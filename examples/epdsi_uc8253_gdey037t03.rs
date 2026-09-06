//! # Good Display GDEY037T03 3.7" Monochrome E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `uc8253_gdey037t03_epd` example — full parity:
//! same panel, same wiring, same three-phase content and driver call sequence, built against
//! `epdsi`'s async API (`default-features = false, features = ["graphics"]`) over `embassy-rp`'s
//! async SPI and GPIO instead of the blocking `rp235x-hal` API. Every `epdsi` call below is
//! `.await`ed; nothing else about the driver-level code differs from the blocking example.
//!
//! See the blocking example's module doc for the full detail on the UC8253's command model (RAM
//! area re-opened around every write and refresh, old/new RAM banks rather than colors) — none of
//! that changes here, only the transport does. BUSY is active-**LOW** on this panel, the opposite
//! of the SSD16xx panels in the other async examples in this repo, hence `Pull::Up` below.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Dalian Good Display GDEY037T03 3.7" Monochrome E-Paper Display (240x416)
//! - **Adapter Board:** [Good Display DESPI-C02](https://www.good-display.com/product/516.html)
//!
//! ## Wiring Connection
//!
//! | Pico 2 Pin    | DESPI-C02 / Breakout Pin | Function              |
//! |---------------|--------------------------|-----------------------|
//! | 3V3 (Pin 36)  | 3.3V / VCC               | 3.3V Power Supply     |
//! | GND (Pin 38)  | GND                      | Ground                |
//! | GPIO11 (Pin 15)| RES / RST               | Reset                 |
//! | GPIO12 (Pin 16)| D/C                     | Data / Command Control|
//! | GPIO13 (Pin 17)| BUSY                    | Busy Status Signal    |
//! | GPIO16 (Pin 21)| MISO                    | SPI MISO              |
//! | GPIO17 (Pin 22)| CS                      | Display Chip Select   |
//! | GPIO18 (Pin 24)| SCK / CLK               | SPI Clock             |
//! | GPIO19 (Pin 25)| SDI / MOSI              | SPI MOSI Data         |
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_uc8253_gdey037t03
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

use embedded_graphics::geometry::{Point, Size};
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

/// Row stride in bytes: 240 / 8 = 30. This panel is already byte-aligned.
const STRIDE: usize = GDEY037T03::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size: 30 x 416 = 12,480 bytes.
const FRAME_BYTES: usize = STRIDE * GDEY037T03::HEIGHT as usize;

/// Top Y coordinate of the content band repainted in Phase 2.
const BAND_Y: u32 = 66;

/// Height of the content band in pixels (y = 66..415).
const BAND_H: u32 = 350;

/// Last row of the band.
const BAND_END: u32 = BAND_Y + BAND_H - 1;

/// Byte offset of the band's first row within the frame buffer.
const BAND_START_BYTE: usize = BAND_Y as usize * STRIDE;

/// Byte offset one past the band's last row.
const BAND_END_BYTE: usize = (BAND_END as usize + 1) * STRIDE;

/// X coordinate of the left logo slot.
const LOGO_LEFT_X: i32 = 30;

/// X coordinate of the right logo slot.
const LOGO_RIGHT_X: i32 = 146;

/// Draws the static chrome above the Phase 2 band: border, header, subtitle and separator.
fn draw_chrome(display: &mut PageBuffer) {
    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEY037T03::WIDTH, GDEY037T03::HEIGHT),
    )
    .into_styled(style)
    .draw(display)
    .unwrap();

    Text::new("GDEY037T03", Point::new(10, 24), text_style)
        .draw(display)
        .unwrap();

    Text::new("3.7\" Mono", Point::new(10, 48), text_style)
        .draw(display)
        .unwrap();

    Line::new(Point::new(10, 58), Point::new(229, 58))
        .into_styled(style)
        .draw(display)
        .unwrap();
}

/// Draws everything inside the Phase 2 band: the two logos, the footer labels and the progress
/// indicator.
fn draw_content(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
    count: u32,
) {
    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    let (ferris_x, rust_x) = if swapped {
        (LOGO_RIGHT_X, LOGO_LEFT_X)
    } else {
        (LOGO_LEFT_X, LOGO_RIGHT_X)
    };

    let ferris_pos = Point::new(ferris_x, 112);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    let rust_pos = Point::new(rust_x, 90);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    Text::new("RP2350 Pico 2", Point::new(10, 200), text_style)
        .draw(display)
        .unwrap();

    Text::new("epdsi async", Point::new(10, 225), text_style)
        .draw(display)
        .unwrap();

    Line::new(Point::new(10, 240), Point::new(229, 240))
        .into_styled(style)
        .draw(display)
        .unwrap();

    let mut count_buf = [0u8; 32];
    let label = if count == 0 {
        "Full refresh"
    } else {
        format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap()
    };
    Text::new(label, Point::new(10, 285), text_style)
        .draw(display)
        .unwrap();

    Rectangle::new(Point::new(10, 300), Size::new(220, 22))
        .into_styled(style)
        .draw(display)
        .unwrap();

    if count > 0 {
        Rectangle::new(Point::new(12, 303), Size::new(count * 35, 16))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(display)
            .unwrap();
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEY037T03 3.7\" Monochrome EPD example (epdsi UC8253, async/Embassy)");

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // UC8253 BUSY is active-LOW, unlike the SSD16xx panels in the other async examples
    let busy = Input::new(p.PIN_13, Pull::Up);

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
    let controller = Uc8253Controller::new(GDEY037T03::WIDTH, GDEY037T03::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEY037T03>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing UC8253 epdsi EPD driver (async)...");
    epd.init(&mut delay).await.unwrap();

    // Prime the old plane to white so the update has a clean base.
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).await.unwrap();

    let mut bw_buf = [0xFFu8; FRAME_BYTES];

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let mut display = PageBuffer::new(&mut bw_buf, GDEY037T03::WIDTH, GDEY037T03::HEIGHT, 0);

    info!("--- Phase 1: Full Monochrome Refresh ---");

    draw_chrome(&mut display);
    draw_content(&mut display, &ferris_bmp, &rust_bmp, false, 0);

    info!("Sending Black/White frame (12,480 bytes)...");
    epd.set_window(0, 0, GDEY037T03::WIDTH - 1, GDEY037T03::HEIGHT - 1)
        .await
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();

    info!("Refreshing display hardware (Full refresh)...");
    epd.refresh(&mut delay).await.unwrap();

    epd.set_window(0, 0, GDEY037T03::WIDTH - 1, GDEY037T03::HEIGHT - 1)
        .await
        .unwrap();
    epd.write_frame(ColorChannel::RedYellow, display.as_slice())
        .await
        .unwrap();

    Timer::after_millis(2000).await;

    info!("--- Phase 2: Partial Window Refresh (logo swap) ---");

    epd.controller_mut()
        .set_refresh_mode(Uc8253RefreshMode::FastPartial);

    for count in 1..=6u32 {
        let swapped = count % 2 == 1;

        display.clear_byte(0xFF);
        draw_chrome(&mut display);
        draw_content(&mut display, &ferris_bmp, &rust_bmp, swapped, count);

        epd.set_window(0, BAND_Y, GDEY037T03::WIDTH - 1, BAND_END)
            .await
            .unwrap();
        epd.write_frame(
            ColorChannel::BlackWhite,
            &display.as_slice()[BAND_START_BYTE..BAND_END_BYTE],
        )
        .await
        .unwrap();

        info!(
            "Refreshing band y={}..{} (Update #{}, logos {})...",
            BAND_Y,
            BAND_END,
            count,
            if swapped { "swapped" } else { "normal" }
        );
        epd.refresh(&mut delay).await.unwrap();

        epd.set_window(0, BAND_Y, GDEY037T03::WIDTH - 1, BAND_END)
            .await
            .unwrap();
        epd.write_frame(
            ColorChannel::RedYellow,
            &display.as_slice()[BAND_START_BYTE..BAND_END_BYTE],
        )
        .await
        .unwrap();

        Timer::after_millis(1000).await;
    }

    info!("--- Phase 3: Full-Waveform Cleanup Pass ---");

    epd.controller_mut()
        .set_refresh_mode(Uc8253RefreshMode::Full);

    display.clear_byte(0xFF);
    draw_chrome(&mut display);
    draw_content(&mut display, &ferris_bmp, &rust_bmp, false, 6);

    epd.set_window(0, 0, GDEY037T03::WIDTH - 1, GDEY037T03::HEIGHT - 1)
        .await
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();

    info!("Refreshing full panel with the full waveform...");
    epd.refresh(&mut delay).await.unwrap();

    info!("Display complete!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_uc8253_gdey037t03"),
    hal::binary_info::rp_program_description!(c"epdsi async UC8253/GDEY037T03 example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
