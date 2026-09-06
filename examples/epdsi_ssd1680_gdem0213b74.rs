//! # Good Display GDEM0213B74 2.13" Monochrome E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1680_gdem0213b74_epd` example — full
//! parity: same panel, same wiring, same three-phase content and driver call sequence, built
//! against `epdsi`'s async API (`default-features = false, features = ["graphics"]`) over
//! `embassy-rp`'s async SPI and GPIO instead of the blocking `rp235x-hal` API. Every `epdsi` call
//! below is `.await`ed; nothing else about the driver-level code differs from the blocking
//! example.
//!
//! 1. **Phase 1**: Full monochrome refresh — header, separator, Ferris logo (top), Rust logo
//!    (bottom), text labels.
//! 2. **Phase 2**: Fast differential partial-window refresh loop repainting only the content band
//!    (y = 50..249), swapping the Ferris/Rust logos each pass and advancing a progress bar.
//! 3. **Phase 3**: Full-waveform cleanup pass over the band, restoring ink density the shortened
//!    differential waveform leaves behind.
//!
//! Part of the Phase 5 hardware gate ahead of `epdsi` 0.2.0's breaking release.
//!
//! ## Note on the 122 pixel panel width
//!
//! See the blocking example's module doc for the byte-alignment detail (`PageBuffer` rounds the
//! row stride up to 16 bytes for a 122 px panel) — unchanged here.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Dalian Good Display GDEM0213B74 2.13" Monochrome E-Paper Display (122x250)
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
//! cargo run --example epdsi_ssd1680_gdem0213b74
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
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use embedded_hal_bus::spi::ExclusiveDevice;
use epdsi::prelude::*;
use tinybmp::Bmp;

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

/// Row stride in bytes. 122 px rounds up to 16 bytes; see the module docs.
const STRIDE: usize = GDEM0213B74::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size: 16 x 250 = 4,000 bytes.
const FRAME_BYTES: usize = STRIDE * GDEM0213B74::HEIGHT as usize;

/// Top Y coordinate of the content band repainted in Phase 2. The header and the separator
/// above it (y = 0..49) are painted once in Phase 1 and never touched again.
const BAND_Y: u32 = 50;

/// Height of the content band in pixels (y = 50..249).
const BAND_H: u32 = 200;

/// Content band buffer size: 16 x 200 = 3,200 bytes.
const BAND_BYTES: usize = STRIDE * BAND_H as usize;

/// All-white fill for the content band, used to blank the secondary RAM before the Phase 3
/// cleanup pass. Lives in flash rather than on the stack.
static WHITE_BAND: [u8; BAND_BYTES] = [0xFFu8; BAND_BYTES];

/// X coordinate that horizontally centres a 64 px logo on the 122 px panel.
const LOGO_X: i32 = (GDEM0213B74::WIDTH as i32 - 64) / 2;

/// Top Y coordinate of the upper logo slot.
const LOGO_TOP_Y: i32 = 52;

/// Draws the Ferris and Rust logos stacked vertically, horizontally centred on the panel.
///
/// The two 64 px-wide logos cannot sit side by side on a 122 px panel, so they are stacked.
/// `swapped` exchanges which logo occupies the upper slot: Phase 2 flips it on every partial
/// update so the differential refresh is obvious at a glance. Ferris is 64x42 and Rust is
/// 64x64, and the offsets are chosen so both arrangements end at y = 161.
fn draw_logos(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
) {
    let (ferris_y, rust_y) = if swapped {
        (LOGO_TOP_Y + 68, LOGO_TOP_Y)
    } else {
        (LOGO_TOP_Y, LOGO_TOP_Y + 46)
    };

    // The Ferris BMP has the opposite polarity to the Rust BMP, hence the `Off` test here.
    let ferris_pos = Point::new(LOGO_X, ferris_y);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    let rust_pos = Point::new(LOGO_X, rust_y);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEM0213B74 2.13\" Monochrome EPD example (epdsi SSD1680, async/Embassy)");

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

    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1680Controller::new(GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEM0213B74>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1680 epdsi EPD driver (async)...");
    epd.init(&mut delay).await.unwrap();

    // Clear both display controller RAM banks to white. On this monochrome panel the secondary
    // RAM (0x26) is not a color plane but the "previous image" used by differential updates.
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF).await.unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).await.unwrap();

    // Frame buffer: 128 x 250 / 8 = 4,000 bytes (0xFF = white)
    let mut bw_buf = [0xFFu8; FRAME_BYTES];

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
    // The panel is only 122 px wide, so the footer labels use the smaller 6x10 font
    let small_text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    info!("--- Phase 1: Full Monochrome Refresh ---");
    info!("Drawing shapes, text, and logos onto frame buffer...");

    // Scoped so the full-frame borrow of `bw_buf` ends before Phase 2 re-borrows it as a
    // smaller sub-region buffer.
    {
        let mut display = PageBuffer::new(&mut bw_buf, GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT, 0);

        // Outer border (visible area only)
        Rectangle::new(
            Point::new(0, 0),
            Size::new(GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT),
        )
        .into_styled(style)
        .draw(&mut display)
        .unwrap();

        Text::new("GDEM0213B74", Point::new(6, 18), text_style)
            .draw(&mut display)
            .unwrap();

        Text::new("2.13\" Mono", Point::new(6, 38), text_style)
            .draw(&mut display)
            .unwrap();

        Line::new(Point::new(6, 45), Point::new(115, 45))
            .into_styled(style)
            .draw(&mut display)
            .unwrap();

        // Ferris on top, Rust below. Phase 2 swaps them on every partial update.
        draw_logos(&mut display, &ferris_bmp, &rust_bmp, false);

        Text::new("RP2350 Pico 2", Point::new(6, 180), small_text_style)
            .draw(&mut display)
            .unwrap();

        Text::new("epdsi async", Point::new(6, 195), small_text_style)
            .draw(&mut display)
            .unwrap();

        Line::new(Point::new(6, 203), Point::new(115, 203))
            .into_styled(style)
            .draw(&mut display)
            .unwrap();

        info!("Sending Black/White frame (4,000 bytes)...");
        epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
            .await
            .unwrap();
        epd.set_cursor(0, 0).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
            .await
            .unwrap();

        info!("Refreshing display hardware (Full refresh)...");
        epd.refresh(&mut delay).await.unwrap();

        // Seed the "previous image" RAM (0x26) with what is now physically on the panel, so the
        // Phase 2 differential updates have a correct base to diff against.
        epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
            .await
            .unwrap();
        epd.set_cursor(0, 0).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, display.as_slice())
            .await
            .unwrap();
    }

    Timer::after_millis(2000).await;

    info!("--- Phase 2: Fast Partial Window Refresh (logo swap) ---");

    // Select the SSD1680 built-in fast LUT (0x22 = 0xFC). On this monochrome panel that is a
    // real differential update and completes in well under a second.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Partial);

    for count in 1..=6u32 {
        let swapped = count % 2 == 1;

        let mut band = PageBuffer::new(
            &mut bw_buf[..BAND_BYTES],
            GDEM0213B74::WIDTH,
            BAND_H,
            BAND_Y,
        );
        band.clear_byte(0xFF);

        draw_logos(&mut band, &ferris_bmp, &rust_bmp, swapped);

        Text::new("RP2350 Pico 2", Point::new(6, 180), small_text_style)
            .draw(&mut band)
            .unwrap();

        Text::new("epdsi async", Point::new(6, 195), small_text_style)
            .draw(&mut band)
            .unwrap();

        Line::new(Point::new(6, 203), Point::new(115, 203))
            .into_styled(style)
            .draw(&mut band)
            .unwrap();

        let mut count_buf = [0u8; 32];
        let count_str =
            format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
        Text::new(count_str, Point::new(6, 224), text_style)
            .draw(&mut band)
            .unwrap();

        Rectangle::new(Point::new(6, 230), Size::new(110, 14))
            .into_styled(style)
            .draw(&mut band)
            .unwrap();

        Rectangle::new(Point::new(8, 232), Size::new(count * 17, 10))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut band)
            .unwrap();

        epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, band.as_slice())
            .await
            .unwrap();

        info!(
            "Refreshing band y={}..{} (Update #{}, logos {})...",
            BAND_Y,
            BAND_Y + BAND_H - 1,
            count,
            if swapped { "swapped" } else { "normal" }
        );
        epd.refresh(&mut delay).await.unwrap();

        // Copy the band we just displayed into the "previous image" RAM so the next iteration
        // diffs against what is actually on the panel rather than the Phase 1 content.
        epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, band.as_slice())
            .await
            .unwrap();

        Timer::after_millis(1000).await;
    }

    info!("--- Phase 3: Full-Waveform Cleanup Pass ---");

    // Differential updates drive the pixels with a much shorter waveform than the OTP full-refresh
    // LUT, so pixels that flip white -> black during Phase 2 settle at a dark grey rather than a
    // deep black, while pixels already black from Phase 1 keep their full density. Re-running the
    // final band content through the full waveform restores even ink density across the band.
    // Blanking the secondary RAM first also stops it being interpreted as a second color plane.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);

    epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
        .await
        .unwrap();
    epd.set_cursor(0, BAND_Y).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, &WHITE_BAND)
        .await
        .unwrap();

    // `bw_buf` still holds the last band drawn in Phase 2, so re-send it unchanged.
    epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
        .await
        .unwrap();
    epd.set_cursor(0, BAND_Y).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..BAND_BYTES])
        .await
        .unwrap();

    info!("Refreshing band with the full OTP waveform...");
    epd.refresh(&mut delay).await.unwrap();

    // Restore the full-frame RAM window for any subsequent updates.
    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();

    info!("Display complete!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_ssd1680_gdem0213b74"),
    hal::binary_info::rp_program_description!(c"epdsi async SSD1680/GDEM0213B74 example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
