//! # Zermatt Image Display Example
//!
//! Display a 320x240 image of Zermatt on the Adafruit 2.2" TFT LCD display.
//! This example demonstrates displaying a full-screen image in landscape mode.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2
//! - **Display:** Adafruit 2.2" 18-bit color TFT LCD display (Product 1480)
//! - **Breakout:** Adafruit Eye-SPI
//!
//! ## Wiring for Eye-SPI Breakout
//!
//! ```
//!      Raspberry Pi Pico 2              Eye-SPI Breakout
//!    +-----------------------+      +---------------------------+
//!    |                       |      |                           |
//!    |  3V3 (Pin 36) --------+------+-> VIN   (Red Wire)        |
//!    |  GND (Pin 38) --------+------+-> GND   (Black Wire)      |
//!    |  GPIO18 (Pin 24) -----+------+-> SCK   (Blue Wire)       |
//!    |  GPIO19 (Pin 25) -----+------+-> MOSI  (Green Wire)      |
//!    |  GPIO16 (Pin 21) -----+------+-> MISO  (Yellow Wire)     |
//!    |  GPIO20 (Pin 26) -----+------+-> DC    (White Wire)      |
//!    |  GPIO21 (Pin 27) -----+------+-> RST   (Orange Wire)     |
//!    |  GPIO17 (Pin 22) -----+------+-> TCS   (Blue Wire)       |
//!    |                       |      |                           |
//!    +-----------------------+      +---------------------------+
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run --example zermatt
//! ```

#![no_std]
#![no_main]

extern crate alloc;
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

use defmt_rtt as _;
use panic_probe as _;

use defmt::info;
use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::spi::{Config, Spi};
use embedded_graphics::{
    Drawable,
    draw_target::DrawTarget,
    geometry::Point,
    image::Image,
    pixelcolor::{Rgb565, RgbColor},
};
use embedded_hal_bus::spi::ExclusiveDevice;
use lcd_async::{
    Builder,
    interface::SpiInterface,
    models::ILI9341Rgb565,
    options::{ColorOrder, Orientation, Rotation},
    raw_framebuf::RawFrameBuf,
};
use tinybmp::Bmp;

use embassy_rp::bind_interrupts;
use hal::block::ImageDef;

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: ImageDef = hal::block::ImageDef::secure_exe();

// Allocate frame buffer statically to avoid blowing up the stack
static mut FRAME_BUFFER: [u8; 320 * 240 * 2] = [0; 320 * 240 * 2];

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    info!("Initializing Adafruit 2.2\" TFT LCD display...");

    // Control pins
    let cs = Output::new(p.PIN_17, Level::High); // TCS - Chip Select
    let dc = Output::new(p.PIN_20, Level::Low); // DC - Data/Command
    let rst = Output::new(p.PIN_21, Level::Low); // RST - Reset

    // Configure SPI (SPI0: SCK=18, MOSI=19, MISO=16)
    let mut config = Config::default();
    config.frequency = 40_000_000;

    // Async SPI using DMA
    let spi = Spi::new(
        p.SPI0, p.PIN_18, // clk
        p.PIN_19, // mosi
        p.PIN_16, // miso
        p.DMA_CH0, p.DMA_CH1, Irqs, config,
    );

    info!("SPI configured at 40 MHz");

    // Create exclusive SPI device with CS pin
    let spi_device = ExclusiveDevice::new(spi, cs, embassy_time::Delay).unwrap();

    // Create display interface
    let di = SpiInterface::new(spi_device, dc);

    info!("Initializing display in landscape mode...");

    let mut delay = embassy_time::Delay;

    // Create and initialize display
    let mut display = Builder::new(ILI9341Rgb565, di)
        .reset_pin(rst)
        .display_size(240, 320) // Physical dimensions
        .orientation(Orientation::new().rotate(Rotation::Deg90).flip_horizontal())
        .color_order(ColorOrder::Bgr)
        .init(&mut delay)
        .await
        .unwrap();

    info!("Display initialized in landscape mode (320x240)!");

    info!("Loading Zermatt image (320x240 BMP)...");

    // Load the BMP image data using tinybmp
    let bmp = Bmp::<Rgb565>::from_slice(include_bytes!("zermatt_320x240.bmp"))
        .expect("Failed to load BMP image");

    info!("Drawing Zermatt image to framebuffer...");

    let frame_buffer = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUFFER) };
    let mut fbuf = RawFrameBuf::<Rgb565, _>::new(&mut frame_buffer[..], 320, 240);
    fbuf.clear(Rgb565::BLACK).unwrap();

    // Draw the image at origin (0, 0) to fill the entire screen
    let image = Image::new(&bmp, Point::new(0, 0));
    image.draw(&mut fbuf).unwrap();

    info!("Sending framebuffer to display...");
    display
        .show_raw_data(0, 0, 320, 240, fbuf.as_bytes())
        .await
        .unwrap();

    info!("Zermatt image displayed! Enjoy the view!");

    // Main loop - image is now showing
    loop {
        embassy_time::Timer::after_millis(100).await;
    }
}
