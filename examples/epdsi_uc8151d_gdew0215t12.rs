//! # Good Display GDEW0215T12 2.15" Monochrome E-Paper Example (`epdsi`, async/Embassy)
//!
//! Raspberry Pi Pico 2 (RP2350) with Embassy, driving the **GDEW0215T12** 2.15" Monochrome
//! (112x208) panel with `Uc8151dController` from `epdsi`, using the async API.
//!
//! The blocking API of this controller is bench verified on an XIAO ESP32-C3 on the Seeed ePaper
//! Driver Board. This is the async twin for the Embassy repo.
//!
//! Four screens, held briefly (`HOLD_MS`) so a run takes about 20 s:
//! 1. Full refresh on the panel's OTP waveform: header, Rust and Ferris logos, text labels.
//! 2. Partial profile, whole frame, no window (the vendor's `EPD_Dis_PartAll`), with a counter band.
//! 3. Partial profile with a window, old then new band data (the vendor's `EPD_Dis_Part`).
//! 4. The same again, counter advanced.
//!
//! Then deep sleep.
//!
//! ## Wiring
//!
//! | Pico 2 Pin      | DESPI-C02 / Breakout Pin | Function                 |
//! |-----------------|--------------------------|--------------------------|
//! | 3V3 (Pin 36)    | 3.3V / VCC               | Power                    |
//! | GND (Pin 38)    | GND                      | Ground                   |
//! | GPIO11 (Pin 15) | RES / RST                | Reset                    |
//! | GPIO12 (Pin 16) | D/C                      | Data / Command           |
//! | GPIO13 (Pin 17) | BUSY                     | Busy (active-LOW)        |
//! | GPIO16 (Pin 21) | MISO                     | SPI MISO                 |
//! | GPIO17 (Pin 22) | CS                       | Chip select              |
//! | GPIO18 (Pin 24) | SCK                      | SPI clock                |
//! | GPIO19 (Pin 25) | MOSI                     | SPI MOSI                 |
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_uc8151d_gdew0215t12
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

/// Row stride in bytes: 112 / 8 = 14.
const STRIDE: usize = GDEW0215T12::WIDTH.div_ceil(8) as usize;

/// Full frame size: 14 x 208 = 2,912 bytes.
const FRAME_BYTES: usize = STRIDE * GDEW0215T12::HEIGHT as usize;

/// How long each screen is held before the next one, in milliseconds.
const HOLD_MS: u32 = 1_500;

/// Top row of the counter band repainted in the partial steps.
const BAND_Y: u32 = 176;

/// Height of the band in rows (y = 176..207).
const BAND_H: u32 = 32;

/// Last row of the band.
const BAND_END: u32 = BAND_Y + BAND_H - 1;

/// Byte offset of the band's first row.
const BAND_START_BYTE: usize = BAND_Y as usize * STRIDE;

/// Byte offset one past the band's last row.
const BAND_END_BYTE: usize = (BAND_END as usize + 1) * STRIDE;

/// X of the 64 px wide logos, centered in the 112 px wide panel.
const LOGO_X: i32 = (GDEW0215T12::WIDTH as i32 - 64) / 2;

/// Draws the static chrome: border, header, Rust and Ferris logos and two text labels.
fn draw_chrome(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEW0215T12::WIDTH, GDEW0215T12::HEIGHT),
    )
    .into_styled(stroke)
    .draw(display)
    .unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    Text::new("GDEW0215T12", Point::new(8, 16), style)
        .draw(display)
        .unwrap();
    Line::new(Point::new(0, 24), Point::new(111, 24))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    // Same polarity handling as the other epdsi examples: the Rust BMP draws where it is `On`,
    // the Ferris BMP where it is `Off`. On `PageBuffer`, `On` is the black ink.
    let rust_pos = Point::new(LOGO_X, 30);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
    let ferris_pos = Point::new(LOGO_X, 100);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    Text::new("RP2350 Pico 2", Point::new(8, 160), style)
        .draw(display)
        .unwrap();
    Text::new("epdsi UC8151D", Point::new(8, 172), style)
        .draw(display)
        .unwrap();
}

/// Draws the counter band: a label and a bar that grows with the count.
fn draw_band(display: &mut PageBuffer, count: u32) {
    let clear = PrimitiveStyle::with_fill(BinaryColor::Off);
    Rectangle::new(
        Point::new(1, BAND_Y as i32),
        Size::new(GDEW0215T12::WIDTH - 2, BAND_H - 1),
    )
    .into_styled(clear)
    .draw(display)
    .unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let label = heapless_label(count);
    Text::new(label.as_str(), Point::new(8, BAND_Y as i32 + 12), style)
        .draw(display)
        .unwrap();

    let fill = PrimitiveStyle::with_fill(BinaryColor::On);
    Rectangle::new(
        Point::new(8, BAND_Y as i32 + 18),
        Size::new(20 * count + 4, 8),
    )
    .into_styled(fill)
    .draw(display)
    .unwrap();
}

/// A tiny fixed-size label buffer, so the example needs no allocator or `format_no_std`.
struct Label {
    buf: [u8; 12],
    len: usize,
}

impl Label {
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
}

/// Builds `"update N"` for a single-digit `N`.
fn heapless_label(count: u32) -> Label {
    let mut buf = [0u8; 12];
    let text = b"update ";
    buf[..text.len()].copy_from_slice(text);
    buf[text.len()] = b'0' + (count % 10) as u8;
    Label {
        buf,
        len: text.len() + 1,
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEW0215T12 2.15\" Monochrome EPD example (epdsi UC8151D, async/Embassy)");

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // UC8151D BUSY is active-LOW: pull up so a missing panel reads idle.
    let busy = Input::new(p.PIN_13, Pull::Up);

    // 4 MHz: this chip's serial write cycle is at least 100 ns, so 10 MHz is the ceiling.
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
    let controller = Uc8151dController::new(GDEW0215T12::WIDTH, GDEW0215T12::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEW0215T12>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing UC8151D (Full profile, async)...");
    epd.init(&mut delay).await.unwrap();

    let mut buf = [0xFFu8; FRAME_BYTES];
    let mut display = PageBuffer::new(&mut buf, GDEW0215T12::WIDTH, GDEW0215T12::HEIGHT, 0);
    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    info!("--- Screen 1 of 4: full refresh on the OTP waveform ---");
    draw_chrome(&mut display, &ferris_bmp, &rust_bmp);

    // Old plane white first, as the vendor demo does, then the new plane.
    epd.set_window(0, 0, GDEW0215T12::WIDTH - 1, GDEW0215T12::HEIGHT - 1)
        .await
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF)
        .await
        .unwrap();
    epd.set_window(0, 0, GDEW0215T12::WIDTH - 1, GDEW0215T12::HEIGHT - 1)
        .await
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();
    epd.refresh(&mut delay).await.unwrap();
    Timer::after_millis(HOLD_MS as u64).await;

    // The UC8151D's partial profile uploads its own LUT, so it needs a fresh init.
    epd.controller_mut()
        .set_refresh_mode(Uc8151dRefreshMode::Partial);
    epd.init(&mut delay).await.unwrap();
    epd.controller_mut().clear_window();

    info!("--- Screen 2 of 4: partial profile, whole frame, no window ---");
    epd.set_window(0, 0, GDEW0215T12::WIDTH - 1, GDEW0215T12::HEIGHT - 1)
        .await
        .unwrap();
    epd.write_frame(ColorChannel::RedYellow, display.as_slice())
        .await
        .unwrap();
    epd.controller_mut().clear_window();
    draw_band(&mut display, 1);
    epd.set_window(0, 0, GDEW0215T12::WIDTH - 1, GDEW0215T12::HEIGHT - 1)
        .await
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();
    epd.controller_mut().clear_window();
    epd.refresh(&mut delay).await.unwrap();
    Timer::after_millis(HOLD_MS as u64).await;

    for count in 2..=3u32 {
        info!(
            "--- Screen {} of 4: partial window y={}..{}, update {} ---",
            count + 1,
            BAND_Y,
            BAND_END,
            count
        );
        // Old band = what the panel shows now, which is the previous pass's band.
        epd.set_window(0, BAND_Y, GDEW0215T12::WIDTH - 1, BAND_END)
            .await
            .unwrap();
        epd.write_frame(
            ColorChannel::RedYellow,
            &display.as_slice()[BAND_START_BYTE..BAND_END_BYTE],
        )
        .await
        .unwrap();
        draw_band(&mut display, count);
        epd.write_frame(
            ColorChannel::BlackWhite,
            &display.as_slice()[BAND_START_BYTE..BAND_END_BYTE],
        )
        .await
        .unwrap();
        epd.refresh(&mut delay).await.unwrap();
        epd.controller_mut().clear_window();
        Timer::after_millis(HOLD_MS as u64).await;
    }

    info!("--- Deep sleep ---");
    epd.sleep(&mut delay).await.unwrap();
    info!("Display complete!");

    loop {
        Timer::after_millis(1_000).await;
    }
}
