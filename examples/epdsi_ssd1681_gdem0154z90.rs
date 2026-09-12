//! # Good Display GDEM0154Z90 1.54" Tri-Color E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1681_gdem0154z90_epd` example — full
//! parity: same panel, same wiring, same two-phase content and driver call sequence, built
//! against `epdsi`'s async API (`default-features = false, features = ["graphics"]`) over
//! `embassy-rp`'s async SPI and GPIO instead of the blocking `rp235x-hal` API. Every `epdsi` call
//! below is `.await`ed; nothing else about the driver-level code differs from the blocking
//! example, aside from one fix noted at the "BWR" label below.
//!
//! 1. **Phase 1**: Full Tri-Color (Black/White/Red) refresh displaying header, colored accent
//!    banners, Ferris logo (Red), Rust logo (Black), and text labels.
//! 2. **Phase 2**: Partial *window* refresh loop that repaints only the bottom status band with an
//!    animated Black/Red progress bar, leaving the header and logos untouched.
//!
//! Exercises the real `embedded-hal-async` call path on hardware already proven to work under
//! `epdsi`'s blocking API (see `rust-rpico2-discovery`'s Phase 3 hardware gate), as part of the
//! Phase 5 hardware gate ahead of `epdsi` 0.2.0's breaking release.
//!
//! ## Note on refresh speed
//!
//! Tri-color (BWR) panels such as the GDEM0154Z90 have **no fast/differential waveform**: the red
//! pigment needs the long OTP waveform, so *every* update takes ~14 s. Phase 2 therefore keeps
//! [`Ssd1681RefreshMode::Full`] and narrows the RAM window instead of using the SSD1681's fast
//! trigger, which only exists for monochrome panels — see the blocking example's doc for the full
//! explanation.
//!
//! ## Note on `PageBuffer` colour convention
//!
//! `PageBuffer::draw_iter` always treats `BinaryColor::On` as "clear the bit" and `Off` as "set
//! it". That's correct as-written for the Black/White plane (`0xFF` background, ink clears bits to
//! 0) but backwards for the Red plane: it's cleared to `0x00`, and a set bit is what the SSD1681
//! renders as red, so red content must be drawn with `Off`. The blocking reference example gets
//! this right for its red swatch and Ferris logo, but not for the "BWR" text label, which silently
//! draws nothing for the same reason my first pass at this file didn't render "embassy-rp" at all.
//! Fixed here by giving red text its own `Off`-coloured style.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Dalian Good Display GDEM0154Z90 1.54" Tri-Color E-Paper Display (200x200)
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
//! cargo run --example epdsi_ssd1681_gdem0154z90
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
use embedded_graphics::mono_font::ascii::FONT_10X20;
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

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEM0154Z90 1.54\" Tri-Color EPD example (epdsi SSD1681, async/Embassy)");

    let p = embassy_rp::init(Default::default());

    // Control pins. `busy` needs both `InputPin` (sync peek) and `Wait` (async edge-wait) — the
    // same `Input` type provides both, no separate construction needed.
    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    let busy = Input::new(p.PIN_13, Pull::Down); // SSD1681 BUSY is active-high

    // SPI0 at 16 MHz, DMA-backed so it satisfies `embedded_hal_async::spi::SpiBus`.
    let mut spi_config = Config::default();
    spi_config.frequency = 16_000_000;
    let spi = Spi::new(
        p.SPI0, p.PIN_18, // CLK
        p.PIN_19, // MOSI
        p.PIN_16, // MISO
        p.DMA_CH0, p.DMA_CH1, Irqs, spi_config,
    );
    let cs = Output::new(p.PIN_17, Level::High);
    // `embedded-hal-bus`'s "async" feature (already enabled in Cargo.toml) adds an
    // `embedded_hal_async::spi::SpiDevice` impl to this same `ExclusiveDevice` type.
    let spi_device = ExclusiveDevice::new(spi, cs, Delay).unwrap();

    // Instantiate epdsi's async SPI bus wrapper and dedicated SSD1681 controller — identical
    // constructors to the blocking API; only the `EpdDriver` methods below are async.
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1681Controller::new(GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEM0154Z90>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1681 epdsi EPD driver (async)...");
    epd.init(&mut delay).await.unwrap();

    // Clear display controller RAM
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF)
        .await
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0x00)
        .await
        .unwrap();

    // Frame buffers: 200 x 200 / 8 = 5,000 bytes each
    let mut bw_buf = [0xFFu8; (GDEM0154Z90::WIDTH as usize * GDEM0154Z90::HEIGHT as usize) / 8];
    let mut red_buf = [0x00u8; (GDEM0154Z90::WIDTH as usize * GDEM0154Z90::HEIGHT as usize) / 8];

    // Load BMP images
    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
    // See the module doc's note on `PageBuffer` colour convention — red content must use `Off`.
    let red_text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::Off);

    info!("--- Phase 1: Normal Full Tri-Color Refresh ---");
    info!("Drawing shapes, text, and logos onto frame buffers...");

    // Scoped so the full-frame borrows of `bw_buf` / `red_buf` end before Phase 2 re-borrows them
    // as smaller sub-region buffers.
    {
        let mut display_bw =
            PageBuffer::new(&mut bw_buf, GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT, 0);
        let mut display_red =
            PageBuffer::new(&mut red_buf, GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT, 0);

        // Outer border (Black)
        Rectangle::new(
            Point::new(0, 0),
            Size::new(GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT),
        )
        .into_styled(style)
        .draw(&mut display_bw)
        .unwrap();

        // Header text (Black)
        Text::new("GDEM0154Z90 1.54\"", Point::new(10, 18), text_style)
            .draw(&mut display_bw)
            .unwrap();

        // Separator line (Black)
        Line::new(Point::new(10, 25), Point::new(190, 25))
            .into_styled(style)
            .draw(&mut display_bw)
            .unwrap();

        // Subtitle: "Tri-Color " in Black, "BWR" in Red
        Text::new("Tri-Color ", Point::new(10, 42), text_style)
            .draw(&mut display_bw)
            .unwrap();
        Text::new("BWR", Point::new(110, 42), red_text_style)
            .draw(&mut display_red)
            .unwrap();

        // Bounding box for color swatches
        Rectangle::new(Point::new(10, 50), Size::new(180, 16))
            .into_styled(style)
            .draw(&mut display_bw)
            .unwrap();

        // Black swatch banner inside bounding box (on Black/White frame)
        Rectangle::new(Point::new(12, 52), Size::new(84, 12))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut display_bw)
            .unwrap();

        // Red swatch banner inside bounding box (on Red frame, BinaryColor::Off sets bit=1 in 0x00-base buffer)
        Rectangle::new(Point::new(104, 52), Size::new(84, 12))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut display_red)
            .unwrap();

        // Draw Ferris logo in Red (left side: x=20, y=75)
        let ferris_pos = Point::new(20, 75);
        for pixel in ferris_bmp.pixels() {
            if pixel.1 == BinaryColor::Off {
                Pixel(pixel.0 + ferris_pos, BinaryColor::Off)
                    .draw(&mut display_red)
                    .unwrap();
            }
        }

        // Draw Rust logo in Black (right side: x=115, y=75)
        let rust_pos = Point::new(115, 75);
        for pixel in rust_bmp.pixels() {
            if pixel.1 == BinaryColor::On {
                Pixel(pixel.0 + rust_pos, BinaryColor::On)
                    .draw(&mut display_bw)
                    .unwrap();
            }
        }

        // Draw text labels (Black)
        Text::new("RP2350 Pico 2", Point::new(10, 165), text_style)
            .draw(&mut display_bw)
            .unwrap();

        Text::new("epdsi async SSD1681", Point::new(10, 185), text_style)
            .draw(&mut display_bw)
            .unwrap();

        // Each RAM write starts from the window origin, so reset window + cursor before both channels.
        info!("Sending Black/White frame (5,000 bytes)...");
        epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
            .await
            .unwrap();
        epd.set_cursor(0, 0).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, display_bw.as_slice())
            .await
            .unwrap();

        info!("Sending Red frame (5,000 bytes)...");
        epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
            .await
            .unwrap();
        epd.set_cursor(0, 0).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, display_red.as_slice())
            .await
            .unwrap();

        info!("Refreshing display hardware (Full refresh, ~14 s)...");
        epd.refresh(&mut delay).await.unwrap();
    }

    Timer::after_millis(2000).await;

    info!("--- Phase 2: Partial Window Tri-Color Refresh ---");

    // The refresh mode deliberately stays `Full` (0x22 = 0xF7). Trigger 0xFC selects the SSD1681
    // built-in fast LUT, which only exists for monochrome panels — on a BWR panel it is slow *and*
    // discards red. What makes this phase "partial" is the narrowed RAM window below.
    defmt::debug_assert_eq!(epd.controller().refresh_mode(), Ssd1681RefreshMode::Full);

    // Bottom status band, updated in place. The Rust logo ends at y = 139 (75 + 64), so the band
    // starts at y = 140 and the header/logos painted in Phase 1 are never touched.
    const BAND_Y: u32 = 140;
    const BAND_H: u32 = 60;
    const BAND_BYTES: usize = (GDEM0154Z90::WIDTH as usize * BAND_H as usize) / 8;

    for count in 1..=5u32 {
        // Sub-region buffers: 200 x 60 / 8 = 1,500 bytes of the full-frame arrays.
        let mut band_bw = PageBuffer::new(
            &mut bw_buf[..BAND_BYTES],
            GDEM0154Z90::WIDTH,
            BAND_H,
            BAND_Y,
        );
        let mut band_red = PageBuffer::new(
            &mut red_buf[..BAND_BYTES],
            GDEM0154Z90::WIDTH,
            BAND_H,
            BAND_Y,
        );

        band_bw.clear_byte(0xFF);
        band_red.clear_byte(0x00);

        // Update counter label (Black)
        let mut count_buf = [0u8; 32];
        let count_str =
            format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
        Text::new(count_str, Point::new(10, 157), text_style)
            .draw(&mut band_bw)
            .unwrap();

        // Progress bar outline (Black)
        Rectangle::new(Point::new(10, 164), Size::new(180, 14))
            .into_styled(style)
            .draw(&mut band_bw)
            .unwrap();

        // Progress bar fill (Red) — proves the Red channel survives a partial window update
        Rectangle::new(Point::new(12, 166), Size::new(count * 35, 10))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut band_red)
            .unwrap();

        Text::new("Partial window", Point::new(10, 195), text_style)
            .draw(&mut band_bw)
            .unwrap();

        // Restrict controller RAM to the band, then write BOTH channels for that region. Writing
        // only Black/White would leave stale Red RAM behind for the band.
        epd.set_window(0, BAND_Y, GDEM0154Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, band_bw.as_slice())
            .await
            .unwrap();

        epd.set_window(0, BAND_Y, GDEM0154Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, band_red.as_slice())
            .await
            .unwrap();

        info!(
            "Refreshing band y={}..{} (Update #{}, ~14 s)...",
            BAND_Y,
            BAND_Y + BAND_H - 1,
            count
        );
        epd.refresh(&mut delay).await.unwrap();

        Timer::after_millis(1000).await;
    }

    // Restore the full-frame RAM window for any subsequent updates.
    epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
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
    hal::binary_info::rp_program_name!(c"epdsi_ssd1681_gdem0154z90"),
    hal::binary_info::rp_program_description!(
        c"epdsi async SSD1681/GDEM0154Z90 example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
