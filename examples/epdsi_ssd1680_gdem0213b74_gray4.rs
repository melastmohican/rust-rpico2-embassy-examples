//! # GDEM0213B74 4-Level Grayscale (Gray4) E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1680_gdem0213b74_gray4_epd` example: full
//! parity, same panel, same wiring, same two-screen content and driver call sequence, built
//! against `epdsi`'s async API (`default-features = false, features = ["graphics"]`) over
//! `embassy-rp`'s async SPI and GPIO instead of the blocking `rp235x-hal` API. Every `epdsi` call
//! below is `.await`ed; nothing else about the driver-level code differs from the blocking
//! example.
//!
//! Companion to `epdsi_ssd1680_gdem0213b74` (plain 1-bit monochrome): same board, panel and
//! wiring, but drives the panel's **4-level grayscale** mode instead: White/Light/Dark/Black
//! instead of just White/Black.
//!
//! Two static screens:
//!
//! 1. **Screen 1**: title/subtitle banner over a 4-band swatch (Black/Dark/Light/White), each
//!    band carrying a single-letter label (`B`/`D`/`L`/`W`) sized to fit this panel's narrower
//!    122px width without overflowing into its neighbor.
//! 2. **Screen 2**: three concentric rounded rectangles (Light/Dark/White) with a centered
//!    "4-Lvl Gray" label in the innermost White ring.
//!
//! Like `epdsi_ssd1680_gdem0213b74`, there is no partial/differential refresh phase here: Gray4
//! mode has exactly one trigger ([`Ssd168xRefreshMode::Gray4`]).
//!
//! ## Provenance: read before trusting this on hardware
//!
//! See the blocking example's module doc for the full caveat: `GDEM0213B74::GRAY4` is **not**
//! Adafruit's own product-page default mode for this breakout. It is transcribed verbatim from
//! Adafruit_EPD's `ti_213mfgn_gray4_init_code`/`ti_213mfgn_gray4_lut_code`, and is byte-identical
//! to the already-shipped `GDEY0266T90::GRAY4` bundle (confirmed by direct byte comparison, not
//! assumed). None of that changes here, only the transport does.
//!
//! ## Note on the 122 pixel panel width
//!
//! See `ssd1680_gdem0213b74_gray4_epd`'s own module doc for the full layout-math rationale: this
//! example uses `FONT_6X10` throughout, single-letter swatch labels, and three rings instead of
//! the four `epdsi_ssd1680_gdey0266t90_gray4`'s wider panel uses, each sized with its text-fit
//! math spelled out in the comments below rather than copied from the wider panel's layout.
//!
//! ## Hardware
//!
//! Same board, panel class, adapter and wiring as `epdsi_ssd1680_gdem0213b74`. See that example
//! for the full pin table. Repeated here for convenience:
//!
//! | Pico 2 Pin    | DESPI-C02 / Breakout Pin | Function              |
//! |---------------|--------------------------|-----------------------|
//! | 3V3 (Pin 36)  | 3.3V / VCC               | 3.3V Power Supply     |
//! | GND (Pin 38)  | GND                      | Ground                |
//! | GPIO11 (Pin 15)| RES / RST               | Reset                 |
//! | GPIO12 (Pin 16)| D/C                     | Data / Command Control|
//! | GPIO13 (Pin 17)| BUSY                    | Busy Status Signal    |
//! | GPIO16 (Pin 21)| MISO                    | SPI MISO              |
//! | GPIO17 (Pin 22)| CS                      | Display Chip Select   |
//! | GPIO18 (Pin 24)| SCK / CLK               | SPI Clock             |
//! | GPIO19 (Pin 25)| SDI / MOSI              | SPI MOSI Data         |
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_ssd1680_gdem0213b74_gray4
//! ```

#![no_std]
#![no_main]

extern crate alloc;
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi::{Config, Spi};
use embassy_time::{Delay, Timer};

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::Text;
use embedded_hal_bus::spi::ExclusiveDevice;
use epdsi::prelude::*;

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

/// Row stride in bytes. 122 px rounds up to 16, same alignment the plain-mono example uses.
const STRIDE: usize = GDEM0213B74::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size per plane: 16 x 250 = 4,000 bytes.
const FRAME_BYTES: usize = STRIDE * GDEM0213B74::HEIGHT as usize;

/// Adafruit's SSD1680 Gray4 convention: neither RAM plane inverted, a set bit is the code bit
/// directly. The only convention `epdsi` has evidence for so far.
const POLARITY: Gray4Polarity = Gray4Polarity::ADAFRUIT_SSD1680;

/// Writes both bit-planes for the full frame, resetting the RAM window and cursor first.
async fn write_full_frame<BUS, C>(
    epd: &mut EpdDriver<BUS, C, GDEM0213B74>,
    page: &GrayBufferPair<'_>,
) where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
{
    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, page.plane_a().as_slice())
        .await
        .unwrap();

    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, page.plane_b().as_slice())
        .await
        .unwrap();
}

/// Screen 1: title/subtitle banner over a 4-band Black/Dark/Light/White swatch, each band
/// labeled with a single letter. All three text lines and all four band widths are sized to
/// this panel's 122px width explicitly (see the widths/positions computed below), not copied
/// from the wider `GDEY0266T90` layout.
fn draw_banner(page: &mut GrayBufferPair) {
    let dark_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Dark);
    let light_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Light);
    let black_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let white_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::White);

    // "SSD1680 Gray4" is 13 chars at 6px = 78px, centered in the 122px width: x = (122-78)/2 = 22.
    Text::new("SSD1680 Gray4", Point::new(22, 10), black_small_style)
        .draw(page)
        .unwrap();

    // "GDEM0213B74 122x250" is 19 chars at 6px = 114px: x = (122-114)/2 = 4.
    Text::new("GDEM0213B74 122x250", Point::new(4, 24), dark_style)
        .draw(page)
        .unwrap();

    // "epdsi Gray4 demo" is 16 chars at 6px = 96px: x = (122-96)/2 = 13.
    Text::new("epdsi Gray4 demo", Point::new(13, 38), light_style)
        .draw(page)
        .unwrap();

    // Four bands spanning the full 122px width. 122/4 = 30 remainder 2, so the last band is
    // widened to 32px rather than leaving a 2px gap: 30+30+30+32 = 122.
    const BAR_Y: i32 = 55;
    const BAR_H: u32 = 40;
    const BAR_W: u32 = GDEM0213B74::WIDTH / 4;
    const LAST_BAR_W: u32 = GDEM0213B74::WIDTH - 3 * BAR_W;

    let bands = [
        (0u32, BAR_W, Gray4Color::Black, "B", white_small_style),
        (1u32, BAR_W, Gray4Color::Dark, "D", white_small_style),
        (2u32, BAR_W, Gray4Color::Light, "L", black_small_style),
        (3u32, LAST_BAR_W, Gray4Color::White, "W", black_small_style),
    ];
    let mut x = 0i32;
    for (_index, width, fill, label, label_style) in bands {
        Rectangle::new(Point::new(x, BAR_Y), Size::new(width, BAR_H))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
        // Single 6px-wide character, centered in the band: x + (width-6)/2.
        Text::new(
            label,
            Point::new(x + (width as i32 - 6) / 2, BAR_Y + BAR_H as i32 / 2 + 3),
            label_style,
        )
        .draw(page)
        .unwrap();
        x += width as i32;
    }
    // Outline around the White band so its edge is visible against the page background.
    Rectangle::new(
        Point::new(3 * BAR_W as i32, BAR_Y),
        Size::new(LAST_BAR_W, BAR_H),
    )
    .into_styled(PrimitiveStyle::with_stroke(Gray4Color::Black, 1))
    .draw(page)
    .unwrap();
}

/// Screen 2: three concentric rounded rectangles (Light/Dark/White, outer to inner) with a
/// centered "4-Lvl Gray" label in the innermost White ring. One ring fewer than
/// `epdsi_ssd1680_gdey0266t90_gray4`'s four: that version's 76px-wide innermost ring has room
/// for a 12-character label; this panel's narrower width does not reach that at the same
/// padding (see the inner-ring math below), so the ring count and label text are sized down
/// instead of risking the overflow that example's own module doc warns about.
fn draw_geometric(page: &mut GrayBufferPair) {
    let stroke = PrimitiveStyle::with_stroke(Gray4Color::Black, 1);
    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT),
    )
    .into_styled(stroke)
    .draw(page)
    .unwrap();

    // (padding, corner radius, fill) for each nested ring, outer to inner. Innermost pad=22
    // leaves an inner rectangle 122 - 2*22 = 78px wide: enough for the 60px label below plus a
    // margin, which pad=30 or higher (as used on the wider panel) would not be.
    let rings: [(u32, u32, Gray4Color); 3] = [
        (6, 6, Gray4Color::Light),
        (14, 5, Gray4Color::Dark),
        (22, 4, Gray4Color::White),
    ];
    for (pad, radius, fill) in rings {
        let rect = Rectangle::new(
            Point::new(pad as i32, pad as i32),
            Size::new(GDEM0213B74::WIDTH - 2 * pad, GDEM0213B74::HEIGHT - 2 * pad),
        );
        RoundedRectangle::with_equal_corners(rect, Size::new(radius, radius))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
    }

    // Innermost ring (pad=22) is 78px wide. "4-Lvl Gray" at FONT_6X10 is 10 chars * 6px = 60px,
    // centered: x = 22 + (78-60)/2 = 31.
    let inner_pad = 22i32;
    let inner_width = GDEM0213B74::WIDTH as i32 - 2 * inner_pad;
    let label_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let label = "4-Lvl Gray";
    let label_width = label.len() as i32 * 6;
    Text::new(
        label,
        Point::new(
            inner_pad + (inner_width - label_width) / 2,
            GDEM0213B74::HEIGHT as i32 / 2,
        ),
        label_style,
    )
    .draw(page)
    .unwrap();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!(
        "Starting GDEM0213B74 4-Level Grayscale (Gray4) EPD example (epdsi SSD1680, async/Embassy)"
    );

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // SSD1680 BUSY is active-HIGH
    let busy = Input::new(p.PIN_13, Pull::Down);

    let mut spi_config = Config::default();
    spi_config.frequency = 16_000_000;
    let spi = Spi::new(
        p.SPI0, p.PIN_18, // CLK
        p.PIN_19, // MOSI
        p.PIN_16, // MISO
        p.DMA_CH0, p.DMA_CH1, Irqs, spi_config,
    );
    let cs = Output::new(p.PIN_17, Level::High);
    let spi_device = ExclusiveDevice::new(spi, cs, Delay).unwrap();

    // `for_panel` picks up `GDEM0213B74`'s dimensions; `.with_gray4` layers on the Adafruit_EPD-
    // sourced register bundle, and `Ssd168xRefreshMode::Gray4` selects its one refresh trigger.
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1680Controller::for_panel::<GDEM0213B74>()
        .with_gray4(GDEM0213B74::GRAY4)
        .with_refresh_mode(Ssd168xRefreshMode::Gray4);
    let mut epd = EpdBuilder::<_, GDEM0213B74>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1680 epdsi EPD driver (Gray4, async)...");
    epd.init(&mut delay).await.unwrap();

    let mut plane_a = [0u8; FRAME_BYTES];
    let mut plane_b = [0u8; FRAME_BYTES];

    info!("--- Screen 1: Banner & 4-Level Swatch ---");
    {
        let mut page = GrayBufferPair::new(
            &mut plane_a,
            &mut plane_b,
            GDEM0213B74::WIDTH,
            GDEM0213B74::HEIGHT,
            0,
            POLARITY,
        );
        page.clear();
        draw_banner(&mut page);
        write_full_frame(&mut epd, &page).await;
    }
    epd.refresh(&mut delay).await.unwrap();

    Timer::after_millis(8000).await;

    info!("--- Screen 2: Concentric Geometric Grayscale Test Pattern ---");
    {
        let mut page = GrayBufferPair::new(
            &mut plane_a,
            &mut plane_b,
            GDEM0213B74::WIDTH,
            GDEM0213B74::HEIGHT,
            0,
            POLARITY,
        );
        page.clear();
        draw_geometric(&mut page);
        write_full_frame(&mut epd, &page).await;
    }
    epd.refresh(&mut delay).await.unwrap();

    // Deep sleep. init() must be called again before any further frame.
    epd.sleep(&mut delay).await.unwrap();
    info!("Display complete, controller asleep.");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_ssd1680_gdem0213b74_gray4"),
    hal::binary_info::rp_program_description!(
        c"epdsi async SSD1680/GDEM0213B74 Gray4 example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
