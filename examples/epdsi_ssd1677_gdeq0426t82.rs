//! # Good Display GDEQ0426T82 4.26" Monochrome E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1677_gdeq0426t82_epd` example — full
//! parity: same panel, same wiring, same four-phase content and driver call sequence (RAM
//! auto-fill check, full refresh, fast differential logo-swap loop, full-waveform cleanup),
//! built against `epdsi`'s async API (`default-features = false, features = ["graphics"]`) over
//! `embassy-rp`'s async SPI and GPIO instead of the blocking `rp235x-hal` API. Every `epdsi` call
//! below is `.await`ed; nothing else about the driver-level code differs from the blocking
//! example.
//!
//! See the blocking example's module doc for the full detail on the portrait rotation
//! ([`DisplayRotation::Rotate270`]), the reversed Y axis handled inside [`Ssd1677Controller`],
//! and why Phase 0 clears to black before white to actually prove the RAM auto-fill path ran —
//! none of that changes here, only the transport does.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Dalian Good Display GDEQ0426T82 4.26" Monochrome E-Paper Display (800x480)
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
//! cargo run --example epdsi_ssd1677_gdeq0426t82
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
use embassy_time::{Delay, Instant, Timer};

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

/// Full frame buffer size: 100 bytes per RAM row x 480 rows = 48,000 bytes.
const FRAME_BYTES: usize = GDEQ0426T82::WIDTH.div_ceil(8) as usize * GDEQ0426T82::HEIGHT as usize;

/// Visible width in the rotated portrait frame (the panel's 480 px axis).
const VIEW_W: u32 = GDEQ0426T82::HEIGHT;

/// Visible height in the rotated portrait frame (the panel's 800 px axis).
const VIEW_H: u32 = GDEQ0426T82::WIDTH;

/// Bottom Y coordinate of the header bar.
const HEADER_H: u32 = 60;

/// Integer scale factor applied to both 64 px logo bitmaps.
const LOGO_SCALE: u32 = 4;

/// X coordinate that horizontally centres a 256 px scaled logo in the 480 px portrait frame.
const LOGO_X: i32 = (VIEW_W as i32 - 64 * LOGO_SCALE as i32) / 2;

/// Y coordinate of the top of the upper logo slot.
const LOGO_TOP_Y: i32 = 110;

/// Y coordinate both logo stacks are bottom-aligned to.
const LOGO_BOTTOM_Y: i32 = 574;

/// Draws a 1 bpp bitmap scaled up by [`LOGO_SCALE`], with each source pixel becoming a filled
/// square.
fn draw_scaled(display: &mut PageBuffer, bmp: &Bmp<BinaryColor>, origin: Point, ink: BinaryColor) {
    let fill = PrimitiveStyle::with_fill(BinaryColor::On);
    let scale = LOGO_SCALE as i32;

    for pixel in bmp.pixels() {
        if pixel.1 == ink {
            Rectangle::new(
                origin + Point::new(pixel.0.x * scale, pixel.0.y * scale),
                Size::new(LOGO_SCALE, LOGO_SCALE),
            )
            .into_styled(fill)
            .draw(display)
            .unwrap();
        }
    }
}

/// Draws the Ferris and Rust logos stacked vertically, horizontally centred.
fn draw_logos(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
) {
    let ferris_h = (ferris_bmp.size().height * LOGO_SCALE) as i32;
    let rust_h = (rust_bmp.size().height * LOGO_SCALE) as i32;

    let (ferris_y, rust_y) = if swapped {
        (LOGO_BOTTOM_Y - ferris_h, LOGO_TOP_Y)
    } else {
        (LOGO_TOP_Y, LOGO_BOTTOM_Y - rust_h)
    };

    draw_scaled(
        display,
        ferris_bmp,
        Point::new(LOGO_X, ferris_y),
        BinaryColor::Off,
    );
    draw_scaled(
        display,
        rust_bmp,
        Point::new(LOGO_X, rust_y),
        BinaryColor::On,
    );
}

/// Renders the complete portrait frame: header bar, outer border, both logos, footer labels,
/// update counter, and progress bar.
fn draw_frame(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
    count: u32,
) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    Rectangle::new(Point::new(0, 0), Size::new(VIEW_W, HEADER_H + 1))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    Text::new("GDEQ0426T82  4.26\"", Point::new(24, 38), text_style)
        .draw(display)
        .unwrap();

    Rectangle::new(
        Point::new(0, HEADER_H as i32),
        Size::new(VIEW_W, VIEW_H - HEADER_H),
    )
    .into_styled(stroke)
    .draw(display)
    .unwrap();

    draw_logos(display, ferris_bmp, rust_bmp, swapped);

    Line::new(Point::new(24, 620), Point::new(455, 620))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    Text::new("RP2350 Pico 2", Point::new(24, 656), text_style)
        .draw(display)
        .unwrap();

    Text::new("epdsi async", Point::new(24, 682), text_style)
        .draw(display)
        .unwrap();

    let mut count_buf = [0u8; 32];
    let count_str =
        format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
    Text::new(count_str, Point::new(24, 720), text_style)
        .draw(display)
        .unwrap();

    Rectangle::new(Point::new(24, 732), Size::new(432, 28))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    Rectangle::new(Point::new(27, 735), Size::new(count * 71, 22))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(display)
        .unwrap();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEQ0426T82 4.26\" Monochrome EPD example (epdsi SSD1677, async/Embassy)");

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // SSD1677 BUSY is active-HIGH
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
    let controller = Ssd1677Controller::new(GDEQ0426T82::WIDTH, GDEQ0426T82::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEQ0426T82>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1677 epdsi EPD driver (async)...");
    epd.init(&mut delay).await.unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF).await.unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).await.unwrap();

    info!("--- Phase 0: RAM auto-fill check ---");

    // Black. The elapsed time is the direct evidence: streaming 48,000 bytes over SPI takes
    // milliseconds, while the pattern generator is a handful of bytes plus a BUSY wait.
    let start = Instant::now();
    epd.clear_frame(ColorChannel::BlackWhite, 0x00).await.unwrap();
    info!("clear_frame(black) took {} us", start.elapsed().as_micros());
    info!("Refreshing; expect a uniform BLACK panel, no banding...");
    epd.refresh(&mut delay).await.unwrap();
    Timer::after_millis(4000).await;

    // Back to white, and re-seed the differential base bank alongside it.
    let start = Instant::now();
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF).await.unwrap();
    info!("clear_frame(white) took {} us", start.elapsed().as_micros());
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).await.unwrap();
    info!("Refreshing; expect a uniform WHITE panel, no banding...");
    epd.refresh(&mut delay).await.unwrap();
    Timer::after_millis(3000).await;

    let mut bw_buf = [0xFFu8; FRAME_BYTES];

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let mut display = PageBuffer::new(&mut bw_buf, GDEQ0426T82::WIDTH, GDEQ0426T82::HEIGHT, 0);
    display.set_rotation(DisplayRotation::Rotate270);

    info!("--- Phase 1: Full Monochrome Refresh ---");
    info!("Drawing shapes, text, and logos onto frame buffer...");

    draw_frame(&mut display, &ferris_bmp, &rust_bmp, false, 0);

    info!("Sending Black/White frame (48,000 bytes)...");
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();

    info!("Refreshing display hardware (Full refresh, expect several seconds)...");
    epd.refresh(&mut delay).await.unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, display.as_slice())
        .await
        .unwrap();

    Timer::after_millis(2000).await;

    info!("--- Phase 2: Fast Differential Refresh (logo swap) ---");

    epd.controller_mut()
        .set_refresh_mode(Ssd1677RefreshMode::Partial);

    for count in 1..=6u32 {
        let swapped = count % 2 == 1;

        display.clear_byte(0xFF);
        draw_frame(&mut display, &ferris_bmp, &rust_bmp, swapped, count);

        epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
            .await
            .unwrap();
        epd.set_cursor(0, 0).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
            .await
            .unwrap();

        info!(
            "Differential refresh (Update #{}, logos {})...",
            count,
            if swapped { "swapped" } else { "normal" }
        );
        epd.refresh(&mut delay).await.unwrap();

        epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
            .await
            .unwrap();
        epd.set_cursor(0, 0).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, display.as_slice())
            .await
            .unwrap();

        Timer::after_millis(1000).await;
    }

    info!("--- Phase 3: Full-Waveform Cleanup Pass ---");

    epd.controller_mut()
        .set_refresh_mode(Ssd1677RefreshMode::Full);

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).await.unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();

    info!("Refreshing with the full OTP waveform...");
    epd.refresh(&mut delay).await.unwrap();

    info!("Display complete!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_ssd1677_gdeq0426t82"),
    hal::binary_info::rp_program_description!(c"epdsi async SSD1677/GDEQ0426T82 example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
