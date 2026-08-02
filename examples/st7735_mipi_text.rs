//! # ST7735S LCD Display Mipidsi Text & Shapes Example (Embassy)
//!
//! Draw text and shapes on an 80x160 ST7735S display (Waveshare 0.96 inch LCD module)
//! over SPI using the `mipidsi` display driver crate and `display-interface-spi`.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Waveshare 0.96" ST7735S LCD Module
//!
//! ## Wiring for Waveshare 0.96 inch LCD Module
//!
//! ```text
//!      Raspberry Pi Pico 2          Waveshare 0.96" ST7735S LCD
//!    +-----------------------+      +---------------------------+
//!    |                       |      |                           |
//!    |  3V3 (Pin 36) --------+------+-> VCC                     |
//!    |  GND (Pin 38) --------+------+-> GND                     |
//!    |  GPIO17 (Pin 22) -----+------+-> CS                      |
//!    |  GPIO21 (Pin 27) -----+------+-> RST                     |
//!    |  GPIO20 (Pin 26) -----+------+-> DC                      |
//!    |  GPIO19 (Pin 25) -----+------+-> DIN(MOSI)               |
//!    |  GPIO18 (Pin 24) -----+------+-> CLK(SCK)                |
//!    |  GPIO14 (Pin 19) -----+------+-> BL (Backlight)          |
//!    |                       |      |                           |
//!    +-----------------------+      +---------------------------+
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run --example st7735_mipi_text
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

use display_interface_spi::SPIInterface;
use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::spi::{Config, Spi};
use embassy_time::{Delay, Timer};

use embedded_graphics::{
    mono_font::{MonoTextStyleBuilder, ascii::FONT_6X10, ascii::FONT_9X15_BOLD},
    pixelcolor::{Rgb565, RgbColor},
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use embedded_hal_bus::spi::ExclusiveDevice;
use mipidsi::{
    Builder,
    models::ST7735s,
    options::{ColorInversion, ColorOrder, Orientation, Rotation},
};

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Initializing ST7735S LCD display text example via mipidsi...");

    let p = embassy_rp::init(Default::default());

    // Control pins
    let dc = Output::new(p.PIN_20, Level::Low);
    let mut rst = Output::new(p.PIN_21, Level::High);
    let mut bl = Output::new(p.PIN_14, Level::High);

    // Setup SPI0 at 16 MHz
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

    let di = SPIInterface::new(spi_device, dc);

    // Reset display
    rst.set_low();
    Timer::after_millis(10).await;
    rst.set_high();
    Timer::after_millis(120).await;

    // Create and initialize display using mipidsi
    let mut display = Builder::new(ST7735s, di)
        .invert_colors(ColorInversion::Inverted)
        .color_order(ColorOrder::Bgr)
        .orientation(Orientation::new().rotate(Rotation::Deg90))
        .display_size(80, 160)
        .display_offset(26, 1)
        .init(&mut Delay)
        .unwrap();

    info!("Display initialized via mipidsi!");

    // Turn on backlight
    bl.set_high();
    info!("Backlight enabled!");

    // Clear screen to black
    display.clear(Rgb565::BLACK).unwrap();

    info!("Drawing text and shapes...");

    // Create text styles
    let title_style = MonoTextStyleBuilder::new()
        .font(&FONT_9X15_BOLD)
        .text_color(Rgb565::WHITE)
        .background_color(Rgb565::BLUE)
        .build();

    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(Rgb565::YELLOW)
        .build();

    // Draw title background
    Rectangle::new(Point::new(0, 0), Size::new(160, 16))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
        .draw(&mut display)
        .unwrap();

    // Draw title text
    Text::with_baseline(
        "Pico 2 ST7735S",
        Point::new(5, 2),
        title_style,
        Baseline::Top,
    )
    .draw(&mut display)
    .unwrap();

    // Draw a separator line
    Line::new(Point::new(0, 18), Point::new(159, 18))
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::GREEN, 2))
        .draw(&mut display)
        .unwrap();

    // Draw a red rectangle
    Rectangle::new(Point::new(10, 25), Size::new(40, 30))
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::RED, 2))
        .draw(&mut display)
        .unwrap();

    // Draw a filled green circle
    Circle::new(Point::new(70, 30), 20)
        .into_styled(PrimitiveStyle::with_fill(Rgb565::GREEN))
        .draw(&mut display)
        .unwrap();

    // Draw a filled orange rectangle
    Rectangle::new(Point::new(110, 28), Size::new(40, 24))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::CSS_ORANGE))
        .draw(&mut display)
        .unwrap();

    // Draw text at bottom
    Text::with_baseline(
        "ST7735S Display",
        Point::new(15, 62),
        text_style,
        Baseline::Top,
    )
    .draw(&mut display)
    .unwrap();

    Text::with_baseline("Hello Rust!", Point::new(90, 62), text_style, Baseline::Top)
        .draw(&mut display)
        .unwrap();

    info!("Display complete!");

    // Keep display showing
    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"st7735_mipi_text"),
    hal::binary_info::rp_program_description!(c"ST7735 LCD Mipidsi Text example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
