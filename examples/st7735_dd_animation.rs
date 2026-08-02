//! # ST7735 Animated Scene Example (Embassy)
//!
//! Smooth 30 FPS geometric animation rendered on an 80x160 ST7735S display (Waveshare 0.96 inch LCD module)
//! over SPI using the async `display-driver` crate family (`display-driver`, `display-driver-spi`, `display-driver-st7735`).
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
//! cargo run --example st7735_dd_animation
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
use display_driver_spi::SpiDisplayBus;
use display_driver_st7735::{St7735, spec::PanelSpec, spec::generic::Generic80x160Type3};

use embedded_graphics::{
    framebuffer::{Framebuffer, buffer_size},
    geometry::Point,
    mono_font::{MonoTextStyle, ascii::FONT_6X10},
    pixelcolor::{
        Rgb565,
        raw::{BigEndian, RawU16},
    },
    prelude::*,
    primitives::{Circle, PrimitiveStyle, Triangle},
    text::Text,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use micromath::F32Ext;

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

// Rotation 90 degrees: 160 width, 80 height
const WIDTH: usize = Generic80x160Type3::PHYSICAL_HEIGHT as _;
const HEIGHT: usize = Generic80x160Type3::PHYSICAL_WIDTH as _;

type FramebufferType =
    Framebuffer<Rgb565, RawU16, BigEndian, WIDTH, HEIGHT, { buffer_size::<Rgb565>(WIDTH, HEIGHT) }>;

/// Draw a creative animated scene with geometric patterns
fn draw_creative_scene(fb: &mut FramebufferType, frame: u32) {
    fb.clear(Rgb565::BLACK).unwrap_or(());

    let center_x = (WIDTH / 2) as i32;
    let center_y = (HEIGHT / 2) as i32;

    // Animated rotating circles
    let angle = (frame % 360) as f32 * core::f32::consts::PI / 180.0;
    let radius = 28.0;

    for i in 0..4 {
        let offset_angle = angle + (i as f32 * core::f32::consts::PI / 2.0);
        let x = center_x + (radius * offset_angle.cos()) as i32;
        let y = center_y + (radius * offset_angle.sin()) as i32;

        let color = match i {
            0 => Rgb565::RED,
            1 => Rgb565::GREEN,
            2 => Rgb565::YELLOW,
            _ => Rgb565::BLUE,
        };

        Circle::new(Point::new(x - 6, y - 6), 12)
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(fb)
            .ok();
    }

    // Pulsating center circle
    let pulse = ((frame % 60) as f32 / 60.0 * 2.0 * core::f32::consts::PI).sin();
    let pulse_radius = (10.0 + pulse * 4.0) as u32;

    Circle::new(
        Point::new(
            center_x - pulse_radius as i32,
            center_y - pulse_radius as i32,
        ),
        pulse_radius * 2,
    )
    .into_styled(PrimitiveStyle::with_stroke(Rgb565::CYAN, 2))
    .draw(fb)
    .ok();

    // Animated corner triangles
    let corner_offset = ((frame / 2) % 15) as i32;

    // Top-left corner triangle
    Triangle::new(
        Point::new(0, 0),
        Point::new(15 + corner_offset, 0),
        Point::new(0, 15 + corner_offset),
    )
    .into_styled(PrimitiveStyle::with_fill(Rgb565::MAGENTA))
    .draw(fb)
    .ok();

    // Bottom-right corner triangle
    Triangle::new(
        Point::new(WIDTH as i32, HEIGHT as i32),
        Point::new(WIDTH as i32 - (15 + corner_offset), HEIGHT as i32),
        Point::new(WIDTH as i32, HEIGHT as i32 - (15 + corner_offset)),
    )
    .into_styled(PrimitiveStyle::with_fill(Rgb565::CYAN))
    .draw(fb)
    .ok();

    // Text overlay
    let text_style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
    Text::new("Embassy ST7735", Point::new(42, 12), text_style)
        .draw(fb)
        .ok();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("ST7735 Animation example starting...");

    let p = embassy_rp::init(Default::default());

    // Hardware setup matching st7735s_spi.rs:
    // SCLK: GPIO18, MOSI: GPIO19, MISO: GPIO16
    // CS: GPIO17, DC: GPIO20, RST: GPIO21, BL: GPIO14
    let dc = Output::new(p.PIN_20, Level::Low);
    let rst = Output::new(p.PIN_21, Level::High);
    let mut bl = Output::new(p.PIN_14, Level::High);

    // Turn backlight on
    bl.set_high();

    // Setup SPI0 at 16 MHz
    let mut spi_config = Config::default();
    spi_config.frequency = 16_000_000;

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
    let panel = St7735::<Generic80x160Type3, _, _>::new(reset_opt);

    let mut driver = match DisplayDriver::builder(bus, panel)
        .with_color_format(ColorFormat::RGB565)
        .with_orientation(Orientation::Deg90)
        .init(&mut Delay)
        .await
    {
        Ok(driver) => driver,
        Err(e) => {
            error!("Display init failed: {:?}", Debug2Format(&e));
            return;
        }
    };

    info!("Display initialized successfully! Starting animation loop...");

    // Create framebuffer for embedded-graphics drawing
    let mut fb = FramebufferType::new();
    let mut frame = 0u32;

    loop {
        draw_creative_scene(&mut fb, frame);

        if let Err(e) = driver.write_frame(fb.data()).await {
            error!("Flush failed at frame {}: {:?}", frame, Debug2Format(&e));
        }

        frame = frame.wrapping_add(1);
        Timer::after_millis(30).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"st7735_dd_animation"),
    hal::binary_info::rp_program_description!(
        c"ST7735 LCD Display Driver 30FPS animation for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
