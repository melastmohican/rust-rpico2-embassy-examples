//! # Good Display GDEW0102I4FC 1.02" Monochrome E-Paper Example (`epdsi`, async/Embassy)
//!
//! Raspberry Pi Pico 2 (RP2350) with Embassy, driving the **GDEW0102I4FC** 1.02" Monochrome
//! (80x128) panel with `Uc8175Controller` from `epdsi`, using the async API.
//!
//! **Bench verified on 2 Nov 2026** (async, Pico 2 wired by hand to a `DESPI-C102`): the picture
//! rendered. The blocking API also ran on an XIAO ESP32-C3 and on this Pico 2.
//!
//! Two full refreshes (about 3 s each, this panel has no partial refresh):
//! 1. Clear to white, as the vendor demo does first.
//! 2. Header, Rust and Ferris logos.
//!
//! Then deep sleep. If the panel shows only noise, reseat the 30-pin FPC first.
//!
//! ## Wiring
//!
//! | Pico 2 Pin      | DESPI-C102 Pin | Function                 |
//! |-----------------|----------------|--------------------------|
//! | 3V3 (Pin 36)    | 3.3V / VCC     | Power                    |
//! | GND (Pin 38)    | GND            | Ground                   |
//! | GPIO11 (Pin 15) | RES / RST      | Reset                    |
//! | GPIO12 (Pin 16) | D/C            | Data / Command           |
//! | GPIO13 (Pin 17) | BUSY           | Busy (active-LOW)        |
//! | GPIO17 (Pin 22) | CS             | Chip select              |
//! | GPIO18 (Pin 24) | SCK            | SPI clock                |
//! | GPIO19 (Pin 25) | MOSI           | SPI MOSI                 |
//!
//! GPIO16 (MISO) is claimed by the SPI driver but not wired.
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_uc8175_gdew0102i4fc
//! ```

#![no_std]
#![no_main]

// Same empty heap the other epdsi examples in this repo declare: a dependency links `alloc`, and
// nothing here ever allocates, so it is never initialised.
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

/// Row stride in bytes: 80 / 8 = 10.
const STRIDE: usize = GDEW0102I4FC::WIDTH.div_ceil(8) as usize;

/// Full frame size: 10 x 128 = 1,280 bytes, the vendor's `EPD_ARRAY`.
const FRAME_BYTES: usize = STRIDE * GDEW0102I4FC::HEIGHT as usize;

/// How long the cleared panel is held before the picture, in milliseconds.
const HOLD_MS: u32 = 1_500;

/// X of the 64 px wide logos, centered in the 80 px wide panel.
const LOGO_X: i32 = (GDEW0102I4FC::WIDTH as i32 - 64) / 2;

/// Draws the border, header, Rust logo (64 x 64) and Ferris logo (64 x 42).
fn draw_picture(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEW0102I4FC::WIDTH, GDEW0102I4FC::HEIGHT),
    )
    .into_styled(stroke)
    .draw(display)
    .unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    Text::new("GDEW0102I4FC", Point::new(4, 12), style)
        .draw(display)
        .unwrap();
    Line::new(Point::new(0, 16), Point::new(79, 16))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    // The Rust BMP draws where it is `On`, the Ferris BMP where it is `Off`. On `PageBuffer`,
    // `On` is the black ink.
    let rust_pos = Point::new(LOGO_X, 19);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
    let ferris_pos = Point::new(LOGO_X, 84);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEW0102I4FC 1.02\" Monochrome EPD example (epdsi UC8175, async/Embassy)");

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // UC8175 BUSY is active-LOW: pull up so a missing panel reads idle.
    let busy = Input::new(p.PIN_13, Pull::Up);

    // 4 MHz, the rate of Good Display's own sketches for this panel.
    let mut spi_config = Config::default();
    spi_config.frequency = 4_000_000;
    let spi = Spi::new(
        p.SPI0, p.PIN_18, // CLK
        p.PIN_19, // MOSI
        p.PIN_16, // MISO
        p.DMA_CH0, p.DMA_CH1, Irqs, spi_config,
    );
    let cs = Output::new(p.PIN_17, Level::High);
    let spi_device = ExclusiveDevice::new(spi, cs, Delay).unwrap();

    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Uc8175Controller::new();
    let mut epd = EpdBuilder::<_, GDEW0102I4FC>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing UC8175 (async)...");
    epd.init(&mut delay).await.unwrap();

    info!("--- Screen 1 of 2: clear to white ---");
    epd.set_window(0, 0, GDEW0102I4FC::WIDTH - 1, GDEW0102I4FC::HEIGHT - 1)
        .await
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF)
        .await
        .unwrap();
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF)
        .await
        .unwrap();
    epd.refresh(&mut delay).await.unwrap();
    Timer::after_millis(HOLD_MS as u64).await;

    let mut buf = [0xFFu8; FRAME_BYTES];
    let mut display = PageBuffer::new(&mut buf, GDEW0102I4FC::WIDTH, GDEW0102I4FC::HEIGHT, 0);
    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();
    draw_picture(&mut display, &ferris_bmp, &rust_bmp);

    info!("--- Screen 2 of 2: Rust and Ferris ---");
    // Old plane white first, as the vendor demo does, then the new plane.
    epd.clear_frame(ColorChannel::RedYellow, 0xFF)
        .await
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();
    epd.refresh(&mut delay).await.unwrap();

    info!("--- Deep sleep ---");
    epd.sleep(&mut delay).await.unwrap();
    info!("Display complete!");

    loop {
        Timer::after_millis(1_000).await;
    }
}
