//! # Good Display GDEY042T81 4.2" 4-Level Grayscale (Gray4) E-Paper Example (`epdsi`, async / Embassy)
//!
//! Companion to `epdsi_ssd1683_gdey042t81` (plain 1-bit monochrome): same board, panel and wiring, but it
//! drives the panel's **4-level grayscale** mode, White/Light/Dark/Black, through
//! `Ssd168xRefreshMode::Gray4Otp`. Its twin for the other API is `ssd1683_gdey042t81_gray4_epd`.
//!
//! ## Provenance, read before trusting this on hardware
//!
//! Unlike the Adafruit-sourced Gray4 examples in this repo, this one comes from the panel vendor:
//! Good Display's `AU-GDEY042T81-FP4G-20240801` demo ships `EPD_HW_Init_4G` and `EPD_Update_4G`,
//! and the specification sheet lists a 2 s "4 Gray update". The controller runs the built-in
//! waveform (`0x21 = 00 00`, `1A 5A`, `22 91`, then `22 CF`), with no LUT upload. The vendor splits a
//! 2 bits-per-pixel image into the `0x24` and `0x26` planes so that white is `1` on both, which is
//! `Gray4Polarity::ADAFRUIT_SSD1677` (both planes inverted).
//!
//! **Hardware-verified on the Pico 2 (RP2350, async)** (4 Nov 2026): both screens rendered correctly, with the
//! four swatches in the expected order.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Good Display GDEY042T81 4.2" (400x300), driven in Gray4 mode
//! - **Adapter Board:** [Good Display DESPI-C02](https://www.good-display.com/product/516.html)
//!
//! ## Wiring Connection
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
//! cargo run --example epdsi_ssd1683_gdey042t81_gray4
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
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_10X20};
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

/// Frame buffer size per bit-plane: 50 bytes per RAM row x 300 rows = 15,000 bytes.
const FRAME_BYTES: usize = GDEY042T81::WIDTH.div_ceil(8) as usize * GDEY042T81::HEIGHT as usize;

/// Drawing width. The panel is drawn in its native 400x300 landscape orientation.
const VIEW_W: u32 = GDEY042T81::WIDTH;

/// Drawing height.
const VIEW_H: u32 = GDEY042T81::HEIGHT;

/// Both RAM planes are inverted on this panel: a raw white pixel is `1` on both `0x24` and `0x26`,
/// which is how Good Display's `R24_DTM1`/`R26_DTM2` split a 2 bits-per-pixel image. That is the
/// same polarity as `Gray4Polarity::ADAFRUIT_SSD1677`, confirmed on hardware.
const POLARITY: Gray4Polarity = Gray4Polarity::ADAFRUIT_SSD1677;

/// Screen 1: title/subtitle banner over a 4-band Black/Dark/Light/White swatch.
fn draw_banner(page: &mut GrayBufferPair) {
    let title_style = MonoTextStyle::new(&FONT_10X20, Gray4Color::Black);
    let dark_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Dark);
    let light_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Light);
    let black_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let white_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::White);

    // "SSD1683 Gray4" is 13 chars at 10px = 130px, centered in the 400px width.
    Text::new("SSD1683 Gray4", Point::new(135, 40), title_style)
        .draw(page)
        .unwrap();
    // "GDEY042T81 400x300" is 18 chars at 6px = 108px.
    Text::new("GDEY042T81 400x300", Point::new(146, 62), dark_style)
        .draw(page)
        .unwrap();
    // "epdsi Gray4 demo" is 16 chars at 6px = 96px.
    Text::new("epdsi Gray4 demo", Point::new(152, 84), light_style)
        .draw(page)
        .unwrap();

    // Four equal bands spanning the full 400px width, one per gray level.
    const BAR_Y: i32 = 120;
    const BAR_H: u32 = 100;
    const BAR_W: u32 = VIEW_W / 4;

    let bands = [
        (0u32, Gray4Color::Black, "Black", white_small_style),
        (1u32, Gray4Color::Dark, "Dark", white_small_style),
        (2u32, Gray4Color::Light, "Light", black_small_style),
        (3u32, Gray4Color::White, "White", black_small_style),
    ];
    for (index, fill, label, label_style) in bands {
        let x = (index * BAR_W) as i32;
        Rectangle::new(Point::new(x, BAR_Y), Size::new(BAR_W, BAR_H))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
        Text::new(label, Point::new(x + 8, BAR_Y + 56), label_style)
            .draw(page)
            .unwrap();
    }
    // Outline around the White band so its edge is visible against the page background.
    Rectangle::new(Point::new(3 * BAR_W as i32, BAR_Y), Size::new(BAR_W, BAR_H))
        .into_styled(PrimitiveStyle::with_stroke(Gray4Color::Black, 1))
        .draw(page)
        .unwrap();
}

/// Screen 2: four concentric rounded rectangles alternating gray levels, with a centered label.
fn draw_geometric(page: &mut GrayBufferPair) {
    let stroke = PrimitiveStyle::with_stroke(Gray4Color::Black, 1);
    Rectangle::new(Point::new(0, 0), Size::new(VIEW_W, VIEW_H))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    // (padding, corner radius, fill) for each nested ring, outer to inner.
    let rings: [(u32, u32, Gray4Color); 4] = [
        (12, 16, Gray4Color::Light),
        (36, 12, Gray4Color::Dark),
        (60, 12, Gray4Color::Black),
        (84, 8, Gray4Color::White),
    ];
    for (pad, radius, fill) in rings {
        let rect = Rectangle::new(
            Point::new(pad as i32, pad as i32),
            Size::new(VIEW_W - 2 * pad, VIEW_H - 2 * pad),
        );
        RoundedRectangle::with_equal_corners(rect, Size::new(radius, radius))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
    }

    // Centered "4-Level Gray" label inside the innermost White ring.
    let inner_pad = 84i32;
    let inner_width = VIEW_W as i32 - 2 * inner_pad;
    let label_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let label = "4-Level Gray";
    let label_width = label.len() as i32 * 6;
    Text::new(
        label,
        Point::new(
            inner_pad + (inner_width - label_width) / 2,
            VIEW_H as i32 / 2,
        ),
        label_style,
    )
    .draw(page)
    .unwrap();
}

/// Writes the Black/White (`0x24`) and Red/Yellow (`0x26`) planes, each carrying one gray bit.
async fn write_planes<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, plane_a: &[u8], plane_b: &[u8])
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEY042T81::WIDTH - 1, GDEY042T81::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, plane_a)
        .await
        .unwrap();

    epd.set_window(0, 0, GDEY042T81::WIDTH - 1, GDEY042T81::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, plane_b)
        .await
        .unwrap();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!(
        "Starting GDEY042T81 4.2\" 4-Level Grayscale (Gray4) EPD example (epdsi SSD1683, async/Embassy)"
    );

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // SSD1683 BUSY is active-HIGH
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

    // Instantiate epdsi's SPI bus wrapper and the SSD168x controller. The SSD1683 is a variant of
    // the shared SSD168x driver, and nothing infers it from the panel type. Gray4 here is
    // `Gray4Otp`: the controller's built-in waveform, so there is no LUT bundle to pass.
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd168xController::for_panel::<GDEY042T81>(Ssd168xVariant::Ssd1683)
        .with_refresh_mode(Ssd168xRefreshMode::Gray4Otp);
    let mut epd = EpdBuilder::<_, GDEY042T81>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1683 epdsi EPD driver (Gray4, async)...");
    epd.init(&mut delay).await.unwrap();

    // Two 15,000-byte bit-plane buffers, held as plain stack arrays.
    let mut plane_a_buf = [0u8; FRAME_BYTES];
    let mut plane_b_buf = [0u8; FRAME_BYTES];

    info!("--- Screen 1: Banner & 4-Level Swatch ---");
    {
        let mut page = GrayBufferPair::new(
            &mut plane_a_buf,
            &mut plane_b_buf,
            GDEY042T81::WIDTH,
            GDEY042T81::HEIGHT,
            0,
            POLARITY,
        );
        page.clear();
        draw_banner(&mut page);

        info!("Gray4Otp refresh (expect about 2 s)...");
        write_planes(
            &mut epd,
            page.plane_a().as_slice(),
            page.plane_b().as_slice(),
        )
        .await;
        epd.refresh(&mut delay).await.unwrap();
    }

    Timer::after_millis(8000).await;

    info!("--- Screen 2: Concentric Geometric Grayscale Test Pattern ---");
    {
        let mut page = GrayBufferPair::new(
            &mut plane_a_buf,
            &mut plane_b_buf,
            GDEY042T81::WIDTH,
            GDEY042T81::HEIGHT,
            0,
            POLARITY,
        );
        page.clear();
        draw_geometric(&mut page);

        info!("Gray4Otp refresh (expect about 2 s)...");
        write_planes(
            &mut epd,
            page.plane_a().as_slice(),
            page.plane_b().as_slice(),
        )
        .await;
        epd.refresh(&mut delay).await.unwrap();
    }

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
    hal::binary_info::rp_program_name!(c"epdsi_ssd1683_gdey042t81_gray4"),
    hal::binary_info::rp_program_description!(
        c"epdsi async SSD1683/GDEY042T81 Gray4 example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
