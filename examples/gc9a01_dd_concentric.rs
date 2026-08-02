//! # GC9A01 240x240 Concentric Gradient Demo (Embassy)
//!
//! Demonstrates driving a round 240x240 GC9A01 display with a concentric dithered
//! gradient pattern and text using `display-driver` and `embedded-graphics`.
//!
//! Adapted from https://github.com/decaday/display-driver/blob/master/examples/rp2040/src/bin/gc9a01-240x240-concentric.rs
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
//! cargo run --example gc9a01_dd_concentric
//! ```

#![no_std]
#![no_main]

extern crate alloc;
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

use defmt::info;
use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::spi::{Config, Spi};
use embassy_time::{Delay, Timer};

use display_driver::{panel::reset::LCDResetOption, ColorFormat, DisplayDriver, Orientation};
use display_driver_gc9a01::{spec::Generic240x240Type1, Gc9a01};
use display_driver_spi::SpiDisplayBus;

use embedded_graphics::{
    framebuffer::{buffer_size, Framebuffer},
    geometry::Point,
    mono_font::{ascii::FONT_9X18, MonoTextStyle},
    pixelcolor::{
        raw::{BigEndian, RawU16},
        Rgb565,
    },
    prelude::*,
    text::{Alignment, Text},
};
use embedded_hal_bus::spi::ExclusiveDevice;
use micromath::F32Ext;
use static_cell::StaticCell;

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

const WIDTH: usize = 240;
const HEIGHT: usize = 240;

type FramebufferType =
    Framebuffer<Rgb565, RawU16, BigEndian, WIDTH, HEIGHT, { buffer_size::<Rgb565>(WIDTH, HEIGHT) }>;

static FB: StaticCell<FramebufferType> = StaticCell::new();

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("START GC9A01 CONCENTRIC GRADIENT DEMO");

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_20, Level::Low);
    let rst = Output::new(p.PIN_21, Level::High);

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
    let bus = SpiDisplayBus::new(spi_device, dc);

    let panel = Gc9a01::<Generic240x240Type1, _, _>::new(LCDResetOption::new_pin(rst));

    info!("Initializing display...");
    let mut driver = DisplayDriver::builder(bus, panel)
        .with_color_format(ColorFormat::RGB565)
        .with_orientation(Orientation::Deg0)
        .init(&mut Delay)
        .await
        .unwrap();

    info!("Display initialized.");

    // Initialize framebuffer in static memory
    let fb = FB.init(Framebuffer::new());

    // Draw content into framebuffer
    info!("Drawing concentric gradient...");
    draw_concentric_gradient(fb);
    draw_text(fb);

    // Flush to display
    info!("Flushing to display...");
    driver.write_frame(fb.data()).await.unwrap();

    info!("Done!");

    loop {
        Timer::after_secs(3600).await;
    }
}

fn draw_text<D>(target: &mut D)
where
    D: DrawTarget<Color = Rgb565>,
{
    const TEXT: &str = "Powered by\ndisplay-driver";
    let shadow_style = MonoTextStyle::new(&FONT_9X18, Rgb565::new(4, 8, 4));
    let text_style = MonoTextStyle::new(&FONT_9X18, Rgb565::WHITE);
    let text_pos = Point::new(120, 180);
    let shadow_offset = Point::new(1, 1);
    let _ = Text::with_alignment(
        TEXT,
        text_pos + shadow_offset,
        shadow_style,
        Alignment::Center,
    )
    .draw(target);
    let _ = Text::with_alignment(TEXT, text_pos, text_style, Alignment::Center).draw(target);
}

fn draw_concentric_gradient<D>(target: &mut D)
where
    D: DrawTarget<Color = Rgb565>,
{
    let center_x: i32 = 120;
    let center_y: i32 = 120;
    let max_radius: f32 = 120.0;
    let center_r: f32 = 255.0;
    let center_g: f32 = 200.0;
    let center_b: f32 = 50.0;
    let edge_r: f32 = 138.0;
    let edge_g: f32 = 43.0;
    let edge_b: f32 = 226.0;

    const BAYER_4X4: [[f32; 4]; 4] = [
        [0.0 / 16.0, 8.0 / 16.0, 2.0 / 16.0, 10.0 / 16.0],
        [12.0 / 16.0, 4.0 / 16.0, 14.0 / 16.0, 6.0 / 16.0],
        [3.0 / 16.0, 11.0 / 16.0, 1.0 / 16.0, 9.0 / 16.0],
        [15.0 / 16.0, 7.0 / 16.0, 13.0 / 16.0, 5.0 / 16.0],
    ];

    for y in 0..240i32 {
        for x in 0..240i32 {
            let dx = (x - center_x) as f32;
            let dy = (y - center_y) as f32;
            let distance = (dx * dx + dy * dy).sqrt();

            let t = if distance >= max_radius {
                1.0
            } else {
                distance / max_radius
            };

            let r_f = lerp(center_r, edge_r, t);
            let g_f = lerp(center_g, edge_g, t);
            let b_f = lerp(center_b, edge_b, t);

            let bayer_threshold = BAYER_4X4[(y & 3) as usize][(x & 3) as usize];

            let dither_r = r_f + (bayer_threshold - 0.5) * 8.226;
            let dither_g = g_f + (bayer_threshold - 0.5) * 4.048;
            let dither_b = b_f + (bayer_threshold - 0.5) * 8.226;

            let r5 = clamp_u8((dither_r / 8.226) as i32, 0, 31) as u8;
            let g6 = clamp_u8((dither_g / 4.048) as i32, 0, 63) as u8;
            let b5 = clamp_u8((dither_b / 8.226) as i32, 0, 31) as u8;

            let color = Rgb565::new(r5, g6, b5);
            let _ = Pixel(Point::new(x, y), color).draw(target);
        }
    }
}

#[inline]
fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

#[inline]
fn clamp_u8(value: i32, min: i32, max: i32) -> i32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"gc9a01_dd_concentric"),
    hal::binary_info::rp_program_description!(
        c"GC9A01 LCD Display Driver concentric gradient demo for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
