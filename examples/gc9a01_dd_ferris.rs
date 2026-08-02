//! # GC9A01 Round LCD Display Driver Example (Embassy)
//!
//! Draw BMP images (Ferris and Rust logo) on a 240x240 GC9A01 round LCD display
//! over SPI using the async `display-driver` crate family (`display-driver`, `display-driver-spi`, `display-driver-gc9a01`).
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
//! cargo run --example gc9a01_dd_ferris
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
use embassy_rp::gpio::{Level, Output};
use embassy_rp::spi::{Config, Spi};
use embassy_time::{Delay, Timer};

use display_driver::{ColorFormat, DisplayDriver, Orientation, panel::reset::LCDResetOption};
use display_driver_gc9a01::{Gc9a01, spec::Generic240x240Type1};
use display_driver_spi::SpiDisplayBus;

use embedded_graphics::{
    framebuffer::{Framebuffer, buffer_size},
    geometry::Point,
    image::Image,
    pixelcolor::{
        Rgb565,
        raw::{BigEndian, RawU16},
    },
    prelude::*,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use tinybmp::Bmp;

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

const SCREEN_WIDTH: usize = 240;
const SCREEN_HEIGHT: usize = 240;

type FramebufferType = Framebuffer<
    Rgb565,
    RawU16,
    BigEndian,
    SCREEN_WIDTH,
    SCREEN_HEIGHT,
    { buffer_size::<Rgb565>(SCREEN_WIDTH, SCREEN_HEIGHT) },
>;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("GC9A01 Display Driver example starting (Ferris & Rust logo)...");

    let p = embassy_rp::init(Default::default());

    // Hardware setup matching gc9a01_spi.rs:
    // SCLK: GPIO18, MOSI: GPIO19, MISO: GPIO16
    // CS: GPIO17, DC: GPIO20, RST: GPIO21
    let dc = Output::new(p.PIN_20, Level::Low);
    let rst = Output::new(p.PIN_21, Level::High);

    // Setup SPI0 at 62.5 MHz (GC9A01 supports high speed SPI)
    let mut spi_config = Config::default();
    spi_config.frequency = 62_500_000;

    let spi = Spi::new(
        p.SPI0, p.PIN_18, // CLK
        p.PIN_19, // MOSI
        p.PIN_16, // MISO
        p.DMA_CH0, p.DMA_CH1, Irqs, spi_config,
    );

    // Chip select pin and ExclusiveDevice wrapper for SPI
    let cs = Output::new(p.PIN_17, Level::High);
    let spi_device = ExclusiveDevice::new(spi, cs, Delay).unwrap();

    // Initialize display bus and driver
    let bus = SpiDisplayBus::new(spi_device, dc);
    let reset_opt = LCDResetOption::new_pin(rst);
    let panel = Gc9a01::<Generic240x240Type1, _, _>::new(reset_opt);

    let mut driver = match DisplayDriver::builder(bus, panel)
        .with_color_format(ColorFormat::RGB565)
        .with_orientation(Orientation::Deg0)
        .init(&mut Delay)
        .await
    {
        Ok(driver) => driver,
        Err(e) => {
            error!("Display init failed: {:?}", Debug2Format(&e));
            return;
        }
    };

    info!("Display initialized successfully!");

    // Create framebuffer for embedded-graphics drawing
    let mut fb = FramebufferType::new();

    // Clear screen to black
    fb.clear(Rgb565::BLACK).unwrap();

    info!("Drawing images...");

    // Draw ferris (BMP format)
    let ferris = Bmp::from_slice(include_bytes!("ferris.bmp")).unwrap();
    let ferris = Image::new(&ferris, Point::new(120, 80));
    ferris.draw(&mut fb).unwrap();
    info!("Ferris drawn!");

    // Draw Rust logo (BMP format)
    let logo = Bmp::from_slice(include_bytes!("rust.bmp")).unwrap();
    let logo = Image::new(&logo, Point::new(40, 80));
    logo.draw(&mut fb).unwrap();
    info!("Rust logo drawn!");

    // Flush framebuffer to display
    if let Err(e) = driver.write_frame(fb.data()).await {
        error!("Flush failed: {:?}", Debug2Format(&e));
    } else {
        info!("Display complete!");
    }

    // Keep application running while display shows images
    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"gc9a01_dd_ferris"),
    hal::binary_info::rp_program_description!(
        c"GC9A01 LCD Display Driver example drawing Ferris & Rust logo"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
