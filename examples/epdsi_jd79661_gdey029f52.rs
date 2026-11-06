//! # GDEY029F52 2.9" Quad-Color E-Paper Example (`epdsi`)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `jd79661_gdey029f52_epd` example: the same
//! panel, wiring, drawing code and driver call sequence, built against `epdsi`'s async API
//! (`default-features = false, features = ["graphics"]`) over `embassy-rp`'s async SPI and GPIO.
//! Every `epdsi` call below is `.await`ed. Drives the 2.9" 128x296 Quad-Color Good Display
//! `GDEY029F52` through `Jd79661Controller` and the `GDEY029F52` panel type.
//!
//! Two pages, one full refresh each, with a 20 s rest between them:
//!
//! 1. **Geometry page.** A black border on all four edges, a red square at the RAM origin and
//!    black, yellow and red bands across the middle third. A wrong resolution shows up here as
//!    a shifted or sheared picture.
//! 2. **Demo page.** The same page as the other colour examples: header, four-swatch colour
//!    bar, Ferris and Rust logos, and footer labels.
//!
//! Do not loop this. Each refresh takes about 16 s.
//!
//! ## Panel check
//!
//! Before the refresh the example resets the panel and reads its `R70H` revision register over
//! a bit-banged 3-wire bus, before SPI takes SCK and MOSI, then logs which unit it is. Seen so
//! far on this panel: `83 02 01`. It never powers on or refreshes the panel at that point, and
//! the example carries on whatever it reads.
//!
//! ## Panel identification
//!
//! Check the rear sticker first. This panel's reads `029SW-F52-B2`. A `GDEY029F51H` (JD79667,
//! 168x384) is a different Good Display product and will not draw correctly here. A unit
//! ordered as an F51H arrived with the F52 sticker, so the order label is not enough.
//!
//! Needs the `GDEY029F52` panel type, which is newer than epdsi 0.10.1.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** GDEY029F52 2.9" Quad-Color, 128x296 (24-pin FPC)
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
//! cargo run --example epdsi_jd79661_gdey029f52
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
use embassy_rp::gpio::{Flex, Input, Level, Output, Pull};
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

/// 4-color options for 2bpp e-Paper display (JD79661)
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
        // RAM row stride is aligned to 8-pixel byte boundary (128 pixels / 32 bytes for 122px width)
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

/// Adapts [`embassy_rp::gpio::Flex`] to [`epdsi::bus3::DynamicPin`] for the pre-flight revision
/// read. `Flex` already switches direction at runtime, so this only forwards.
struct FlexPin<'d>(Flex<'d>);

impl DynamicPin for FlexPin<'_> {
    type Error = core::convert::Infallible;

    fn set_as_output(&mut self) -> Result<(), Self::Error> {
        self.0.set_as_output();
        Ok(())
    }

    fn set_as_input(&mut self) -> Result<(), Self::Error> {
        self.0.set_as_input();
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.0.set_high();
        Ok(())
    }

    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.0.set_low();
        Ok(())
    }

    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(Flex::is_high(&self.0))
    }
}

/// Says which kind of unit a `R70H` reading belongs to, from the units seen so far.
fn describe_revision(rev: [u8; 3]) -> &'static str {
    match rev {
        [0x83, 0x02, 0x01] => "matches the GDEY029F52 unit seen (sticker 029SW-F52-B2)",
        [0x03, 0x02, 0x01] => "matches the JD79661AA/JD79667AA datasheet value",
        [0xFF, 0xFF, 0xFF] | [0x00, 0x00, 0x00] => "read path not reaching the IC, no answer",
        _ => "new pattern, record it with the rear sticker text and lot",
    }
}

/// Page 1: geometry check.
fn draw_geometry_page(display: &mut QuadColorBuffer) {
    display.draw_rect_outline(0, 0, 128, 296, QuadColor::Black);
    display.fill_rect(1, 1, 24, 24, QuadColor::Red);
    // Three bands across the middle third, one per ink.
    display.fill_rect(1, 99, 42, 98, QuadColor::Black);
    display.fill_rect(43, 99, 42, 98, QuadColor::Yellow);
    display.fill_rect(85, 99, 42, 98, QuadColor::Red);
}

/// Page 2: the same page as the other colour examples.
fn draw_demo_page(display: &mut QuadColorBuffer) {
    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    // Outer border
    display.draw_rect_outline(0, 0, 128, 296, QuadColor::Black);

    Text::new("GDEY029F52", Point::new(14, 18), text_style)
        .draw(display)
        .unwrap();

    Line::new(Point::new(6, 23), Point::new(121, 23))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(display)
        .unwrap();

    Text::new("JD79661 EPD", Point::new(9, 40), text_style)
        .draw(display)
        .unwrap();

    // Colour bar: one swatch per ink, so a wrong plane mapping is obvious at a glance.
    display.draw_rect_outline(9, 48, 110, 16, QuadColor::Black);
    display.fill_rect(11, 50, 25, 12, QuadColor::Black);
    display.fill_rect(38, 50, 25, 12, QuadColor::Yellow);
    display.fill_rect(65, 50, 25, 12, QuadColor::Red);
    display.fill_rect(92, 50, 25, 12, QuadColor::White);
    display.draw_rect_outline(92, 50, 25, 12, QuadColor::Black);

    // Logos, horizontally centred: (128 - 64) / 2 = 32.
    let ferris_offset = Point::new(32, 78);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_offset, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    let rust_offset = Point::new(32, 150);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_offset, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    Text::new("RP2350", Point::new(34, 262), text_style)
        .draw(display)
        .unwrap();

    Text::new("epdsi BWRY", Point::new(14, 284), text_style)
        .draw(display)
        .unwrap();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEY029F52 2.9\" Quad-Color EPD example (epdsi JD79661, async/Embassy)");

    let mut p = embassy_rp::init(Default::default());

    // --- Which panel is this? Read R70H before SPI0 takes SCK and MOSI. ---
    // Reset with the demo's timings, bit-bang the register read on reborrowed pins, then let
    // them go. Nothing is powered on or refreshed here.
    {
        let mut id_delay = Delay;
        let id_cs = Output::new(p.PIN_17.reborrow(), Level::High);
        let id_sck = Output::new(p.PIN_18.reborrow(), Level::Low);
        let id_sda = FlexPin(Flex::new(p.PIN_19.reborrow()));
        let id_dc = Output::new(p.PIN_12.reborrow(), Level::Low);
        let mut id_rst = Output::new(p.PIN_11.reborrow(), Level::High);
        let id_busy = Input::new(p.PIN_13.reborrow(), Pull::Up);

        Timer::after_millis(20).await;
        id_rst.set_low();
        Timer::after_millis(40).await;
        id_rst.set_high();
        Timer::after_millis(50).await;

        let mut id_bus = Spi3Bus::new(id_cs, id_sck, id_sda, id_dc, id_rst, id_busy);
        id_bus.wait_busy(&mut id_delay).await.unwrap();
        let rev = id_bus.read_jd7966x_revision(&mut id_delay).await.unwrap();
        info!("Panel R70H = {:02X}: {}", rev, describe_revision(rev));
    }

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
    let controller = Jd79661Controller::new(GDEY029F52::WIDTH, GDEY029F52::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEY029F52>::new(controller).build(epd_bus);
    let mut delay = Delay;

    // 2bpp frame buffer: 32 bytes per row x 296 rows = 9,472 bytes
    let mut frame_buf = [0x55u8; 9472];
    let mut display = QuadColorBuffer::new(&mut frame_buf, 128, 296);
    display.set_rotation(DisplayRotation::Rotate0);

    for page in 1..=2 {
        info!("--- page {} of 2 ---", page);
        epd.init(&mut delay).await.unwrap();

        // Start each page from white, so page 1 does not bleed into page 2.
        display.fill_rect(0, 0, 128, 296, QuadColor::White);
        if page == 1 {
            draw_geometry_page(&mut display);
        } else {
            draw_demo_page(&mut display);
        }

        info!("Sending 9,472-byte QuadColor 2bpp frame via epdsi...");
        epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
            .await
            .unwrap();

        info!("Triggering display refresh (about 16 s)...");
        epd.refresh(&mut delay).await.unwrap();

        info!("Powering off DC/DC...");
        epd.sleep(&mut delay).await.unwrap();

        if page == 1 {
            info!("Resting 20 s before page 2...");
            Timer::after_secs(20).await;
        }
    }

    info!("GDEY029F52 QuadColor epdsi demo finished successfully!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_jd79661_gdey029f52"),
    hal::binary_info::rp_program_description!(c"epdsi async JD79661/GDEY029F52 example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
