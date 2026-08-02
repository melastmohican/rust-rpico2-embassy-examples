//! # GC9A01 Round LCD Display Mipidsi Ferris Example (Embassy)
//!
//! Draw BMP images (Ferris and Rust logo) on a 240x240 GC9A01 round LCD display
//! over SPI using the `mipidsi` display driver crate and `display-interface-spi`.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** GC9A01 240x240 Round LCD Display
//!
//! ## Wiring for GC9A01 Display (7-pin modules)
//!
//! ```text
//!      Raspberry Pi Pico 2           GC9A01 240x240 Round LCD
//!    +-----------------------+      +---------------------------+
//!    |                       |      |                           |
//!    |  3V3 (Pin 36) --------+------+-> VCC                     |
//!    |  GND (Pin 38) --------+------+-> GND                     |
//!    |  GPIO17 (Pin 22) -----+------+-> CS                      |
//!    |  GPIO21 (Pin 27) -----+------+-> RST                     |
//!    |  GPIO20 (Pin 26) -----+------+-> DC                      |
//!    |  GPIO19 (Pin 25) -----+------+-> SDA(MOSI)               |
//!    |  GPIO18 (Pin 24) -----+------+-> SCL(SCK)                |
//!    |                       |      |                           |
//!    +-----------------------+      +---------------------------+
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run --example gc9a01_mipi_ferris
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
    geometry::Point,
    image::Image,
    pixelcolor::{Rgb565, RgbColor},
    prelude::*,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use mipidsi::{
    Builder,
    models::GC9A01,
    options::{ColorInversion, ColorOrder},
};
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
    info!("Initializing GC9A01 round LCD display via mipidsi...");

    let p = embassy_rp::init(Default::default());

    // Hardware setup matching gc9a01_spi.rs:
    // SCLK: GPIO18, MOSI: GPIO19, MISO: GPIO16
    // CS: GPIO17, DC: GPIO20, RST: GPIO21
    let dc = Output::new(p.PIN_20, Level::Low);
    let mut rst = Output::new(p.PIN_21, Level::High);

    // Setup SPI0 at 62.5 MHz (GC9A01 supports up to 62.5 MHz)
    let mut spi_config = Config::default();
    spi_config.frequency = 62_500_000;

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
    let mut display = Builder::new(GC9A01, di)
        .invert_colors(ColorInversion::Inverted)
        .color_order(ColorOrder::Bgr)
        .display_size(240, 240)
        .init(&mut Delay)
        .unwrap();

    info!("Display initialized via mipidsi!");

    // Clear screen to black
    display.clear(Rgb565::BLACK).unwrap();

    info!("Drawing images...");

    // Draw ferris (BMP format)
    let ferris = Bmp::from_slice(include_bytes!("ferris.bmp")).unwrap();
    let ferris = Image::new(&ferris, Point::new(120, 80));
    ferris.draw(&mut display).unwrap();
    info!("Ferris drawn!");

    // Draw Rust logo (BMP format)
    let logo = Bmp::from_slice(include_bytes!("rust.bmp")).unwrap();
    let logo = Image::new(&logo, Point::new(40, 80));
    logo.draw(&mut display).unwrap();
    info!("Rust logo drawn!");

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
    hal::binary_info::rp_program_name!(c"gc9a01_mipi_ferris"),
    hal::binary_info::rp_program_description!(
        c"GC9A01 LCD Mipidsi Ferris example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
