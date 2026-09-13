//! # Good Display GDEM0154F51H E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async example for the Waveshare "1.54inch e-Paper (G)" module, built against `epdsi`'s
//! async API (`default-features = false, features = ["graphics"]`) over `embassy-rp`'s async
//! SPI and GPIO. Every `epdsi` call below is `.await`ed; the local
//! `QuadColorBuffer`/`QuadColor` 2bpp framebuffer type is shared with the sibling JD79661
//! example — it never touches `epdsi` or the HAL directly, so there's nothing panel-specific
//! in it.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Good Display GDEM0154F51H, sold by Waveshare as the "1.54inch e-Paper (G)"
//!   module — 1.54" 4-Color (Black/White/Yellow/Red) E-Paper Display (200x200, square)
//! - **Adapter Board:** [Good Display DESPI-C02](https://www.good-display.com/product/516.html)
//!
//! ## Wiring Connection
//!
//! | Pico 2 Pin    | DESPI-C02 / Breakout Pin | Function             |
//! |---------------|--------------------------|----------------------|
//! | 3V3 (Pin 36)  | VCC                      | 3.3V Power Supply    |
//! | GND (Pin 38)  | GND                      | Ground               |
//! | GPIO18 (Pin 24)| SCK                     | SPI Clock            |
//! | GPIO19 (Pin 25)| MOSI                    | SPI Data             |
//! | GPIO16 (Pin 21)| MISO                    | SPI MISO             |
//! | GPIO17 (Pin 22)| CS                      | Display Chip Select  |
//! | GPIO12 (Pin 16)| DC                      | Data / Command Control|
//! | GPIO11 (Pin 15)| RST                     | Reset                |
//! | GPIO13 (Pin 17)| BUSY                    | Busy Status Signal   |
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_jd79660_gdem0154f51h
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

use embedded_graphics::geometry::{Dimensions, Point, Size};
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

/// 4-color options for 2bpp e-Paper display (JD79660)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadColor {
    Black = 0b00,
    White = 0b01,
    Yellow = 0b10,
    Red = 0b11,
}

/// 2-bit per pixel buffer for 4-color displays (1 byte = 4 pixels)
pub struct QuadColorBuffer<'a> {
    buffer: &'a mut [u8],
    width: u32,
    height: u32,
    ram_stride: u32,
    rotation: DisplayRotation,
}

impl<'a> QuadColorBuffer<'a> {
    pub fn new(buffer: &'a mut [u8], width: u32, height: u32) -> Self {
        // Fill with White (0b01010101 = 0x55)
        buffer.fill(0x55);
        // RAM row stride is aligned to 8-pixel byte boundary (200 is already a multiple of
        // 8, so this is a no-op round-up for this panel)
        let ram_stride = width.div_ceil(8) * 8;
        Self {
            buffer,
            width,
            height,
            ram_stride,
            rotation: DisplayRotation::Rotate0,
        }
    }

    pub fn set_rotation(&mut self, rotation: DisplayRotation) {
        self.rotation = rotation;
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, color: QuadColor) {
        let (mapped_x, mapped_y) = match self.rotation {
            DisplayRotation::Rotate0 => (x, y),
            DisplayRotation::Rotate90 => (self.width.saturating_sub(1).saturating_sub(y), x),
            DisplayRotation::Rotate180 => (
                self.width.saturating_sub(1).saturating_sub(x),
                self.height.saturating_sub(1).saturating_sub(y),
            ),
            DisplayRotation::Rotate270 => (y, self.height.saturating_sub(1).saturating_sub(x)),
        };

        if mapped_x >= self.width || mapped_y >= self.height {
            return;
        }

        let pixel_index = mapped_y * self.ram_stride + mapped_x;
        let byte_index = (pixel_index / 4) as usize;
        let pixel_offset = 3 - (pixel_index % 4);
        let bit_shift = pixel_offset * 2;

        if byte_index < self.buffer.len() {
            let mask = !(0b11 << bit_shift);
            let val = (color as u8) << bit_shift;
            self.buffer[byte_index] = (self.buffer[byte_index] & mask) | val;
        }
    }

    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: QuadColor) {
        for px in x..(x + w) {
            for py in y..(y + h) {
                self.set_pixel(px, py, color);
            }
        }
    }

    pub fn draw_rect_outline(&mut self, x: u32, y: u32, w: u32, h: u32, color: QuadColor) {
        if w == 0 || h == 0 {
            return;
        }
        for px in x..(x + w) {
            self.set_pixel(px, y, color);
            self.set_pixel(px, y + h - 1, color);
        }
        for py in y..(y + h) {
            self.set_pixel(x, py, color);
            self.set_pixel(x + w - 1, py, color);
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.buffer
    }
}

impl<'a> DrawTarget for QuadColorBuffer<'a> {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0 && coord.y >= 0 {
                let c = match color {
                    BinaryColor::On => QuadColor::Black,
                    BinaryColor::Off => QuadColor::White,
                };
                self.set_pixel(coord.x as u32, coord.y as u32, c);
            }
        }
        Ok(())
    }
}

impl<'a> Dimensions for QuadColorBuffer<'a> {
    fn bounding_box(&self) -> Rectangle {
        let (w, h) = match self.rotation {
            DisplayRotation::Rotate0 | DisplayRotation::Rotate180 => (self.width, self.height),
            DisplayRotation::Rotate90 | DisplayRotation::Rotate270 => (self.height, self.width),
        };
        Rectangle::new(Point::zero(), Size::new(w, h))
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting JD79660 GDEM0154F51H 1.54\" e-Paper (G) EPD example (async/Embassy)");

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
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
    let controller = Jd79660Controller::new(GDEM0154F51H::WIDTH, GDEM0154F51H::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEM0154F51H>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing JD79660 epdsi EPD driver (async)...");
    epd.init(&mut delay).await.unwrap();

    // Allocate 2bpp frame buffer: 50 bytes/row * 200 rows = 10,000 bytes
    let mut frame_buf = [0x55u8; 10000];
    let mut display = QuadColorBuffer::new(&mut frame_buf, 200, 200);
    display.set_rotation(DisplayRotation::Rotate0);

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    // Outer border
    display.draw_rect_outline(0, 0, 200, 200, QuadColor::Black);

    // Header text
    Text::new("GDEM0154F51H", Point::new(20, 18), text_style)
        .draw(&mut display)
        .unwrap();

    // Separator line
    Line::new(Point::new(6, 23), Point::new(193, 23))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(&mut display)
        .unwrap();

    // Subtitle
    Text::new("JD79660 EPD", Point::new(6, 40), text_style)
        .draw(&mut display)
        .unwrap();

    // Quad-Color preview bounding box & color swatches
    display.draw_rect_outline(6, 48, 188, 16, QuadColor::Black);
    display.fill_rect(8, 50, 42, 12, QuadColor::Black);
    display.fill_rect(54, 50, 42, 12, QuadColor::Yellow);
    display.fill_rect(100, 50, 42, 12, QuadColor::Red);
    display.fill_rect(146, 50, 42, 12, QuadColor::White);
    display.draw_rect_outline(146, 50, 42, 12, QuadColor::Black);

    // Draw Ferris logo (centered: (200 - 64)/2 = 68)
    let ferris_offset = Point::new(68, 78);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_offset, BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    // Draw Rust logo (centered: (200 - 64)/2 = 68)
    let rust_offset = Point::new(68, 130);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_offset, BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    Text::new("RP2350", Point::new(70, 178), text_style)
        .draw(&mut display)
        .unwrap();

    Text::new("epdsi async", Point::new(50, 195), text_style)
        .draw(&mut display)
        .unwrap();

    info!("Sending 10,000-byte QuadColor 2bpp frame via epdsi (async)...");
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();

    info!("Triggering display refresh...");
    epd.refresh(&mut delay).await.unwrap();

    info!("Powering off DC/DC...");
    epd.sleep(&mut delay).await.unwrap();

    info!("JD79660 QuadColor epdsi demo finished successfully!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_jd79660_gdem0154f51h"),
    hal::binary_info::rp_program_description!(c"epdsi async JD79660/GDEM0154F51H example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
