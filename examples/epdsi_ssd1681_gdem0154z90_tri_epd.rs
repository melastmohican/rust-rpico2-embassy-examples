//! # GDEM0154Z90 Tri-Color `PageBufferPair` Draw Target Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1681_gdem0154z90_tri_epd` example, which is
//! itself the full-parity `PageBufferPair` rewrite of `epdsi_ssd1681_gdem0154z90` (this repo's
//! `PageBuffer`-based example). Same two phases, same content, same driver call sequence, over
//! `epdsi`'s async API instead of blocking.
//!
//! What differs from `epdsi_ssd1681_gdem0154z90` is only the drawing code:
//!
//! - One `page: &mut PageBufferPair` replaces the two `display_bw`/`display_red` locals.
//! - `TriColor::{Black, Accent}` replaces every `BinaryColor::On`/`Off` choice — including the
//!   `red_text_style` workaround `epdsi_ssd1681_gdem0154z90` needed for its "BWR" label (a
//!   silent no-render bug from drawing red text with the wrong `BinaryColor` polarity). With
//!   `TriColor::Accent` there is no polarity choice to get backwards at the call site, so that
//!   whole class of bug does not exist here.
//! - `PageBufferPair::clear()` replaces the `clear_byte(0xFF)` / `clear_byte(0x00)` pair before
//!   each windowed redraw.
//!
//! See `ssd1681_gdem0154z90_epd`'s module doc (in `rust-rpico2-discovery`) for the full
//! narrative on refresh speed.
//!
//! ## Hardware
//!
//! Same board, panel and wiring as `epdsi_ssd1681_gdem0154z90` — see that example for the pin
//! table.
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_ssd1681_gdem0154z90_tri_epd
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

/// The polarity this panel needs — Black/White plane normal, accent plane inverted.
const POLARITY: PlanePolarity = PlanePolarity::SSD168X;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!(
        "Starting GDEM0154Z90 1.54\" Tri-Color EPD example (epdsi SSD1681, async/Embassy, PageBufferPair)"
    );

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    let busy = Input::new(p.PIN_13, Pull::Down); // SSD1681 BUSY is active-high

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
    let controller = Ssd1681Controller::new(GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEM0154Z90>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1681 epdsi EPD driver (async)...");
    epd.init(&mut delay).await.unwrap();

    // Clear display controller RAM
    epd.clear_frame(ColorChannel::BlackWhite, POLARITY.bw_background_byte())
        .await
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, POLARITY.accent_background_byte())
        .await
        .unwrap();

    // Frame buffers: 200 x 200 / 8 = 5,000 bytes each
    let mut bw_buf = [POLARITY.bw_background_byte();
        (GDEM0154Z90::WIDTH as usize * GDEM0154Z90::HEIGHT as usize) / 8];
    let mut red_buf = [POLARITY.accent_background_byte();
        (GDEM0154Z90::WIDTH as usize * GDEM0154Z90::HEIGHT as usize) / 8];

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, TriColor::Black);

    info!("--- Phase 1: Normal Full Tri-Color Refresh ---");
    info!("Drawing shapes, text, and logos onto the frame buffer...");

    // Scoped so the full-frame borrows of `bw_buf` / `red_buf` end before Phase 2 re-borrows
    // them as smaller sub-region buffers.
    {
        let mut page = PageBufferPair::new(
            &mut bw_buf,
            &mut red_buf,
            GDEM0154Z90::WIDTH,
            GDEM0154Z90::HEIGHT,
            0,
            POLARITY,
        );

        // Outer border (Black)
        Rectangle::new(
            Point::new(0, 0),
            Size::new(GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT),
        )
        .into_styled(stroke)
        .draw(&mut page)
        .unwrap();

        // Header text (Black)
        Text::new("GDEM0154Z90 1.54\"", Point::new(10, 18), text_style)
            .draw(&mut page)
            .unwrap();

        // Separator line (Black)
        Line::new(Point::new(10, 25), Point::new(190, 25))
            .into_styled(stroke)
            .draw(&mut page)
            .unwrap();

        // Subtitle: "Tri-Color " in Black, "BWR" in Accent
        Text::new("Tri-Color ", Point::new(10, 42), text_style)
            .draw(&mut page)
            .unwrap();
        Text::new(
            "BWR",
            Point::new(110, 42),
            MonoTextStyle::new(&FONT_10X20, TriColor::Accent),
        )
        .draw(&mut page)
        .unwrap();

        // Bounding box for color swatches
        Rectangle::new(Point::new(10, 50), Size::new(180, 16))
            .into_styled(stroke)
            .draw(&mut page)
            .unwrap();

        // Black swatch banner inside bounding box
        Rectangle::new(Point::new(12, 52), Size::new(84, 12))
            .into_styled(PrimitiveStyle::with_fill(TriColor::Black))
            .draw(&mut page)
            .unwrap();

        // Accent swatch banner inside bounding box
        Rectangle::new(Point::new(104, 52), Size::new(84, 12))
            .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
            .draw(&mut page)
            .unwrap();

        // Draw Ferris logo in Accent (left side: x=20, y=75)
        let ferris_pos = Point::new(20, 75);
        for pixel in ferris_bmp.pixels() {
            if pixel.1 == BinaryColor::Off {
                Pixel(pixel.0 + ferris_pos, TriColor::Accent)
                    .draw(&mut page)
                    .unwrap();
            }
        }

        // Draw Rust logo in Black (right side: x=115, y=75)
        let rust_pos = Point::new(115, 75);
        for pixel in rust_bmp.pixels() {
            if pixel.1 == BinaryColor::On {
                Pixel(pixel.0 + rust_pos, TriColor::Black)
                    .draw(&mut page)
                    .unwrap();
            }
        }

        // Draw text labels (Black)
        Text::new("RP2350 Pico 2", Point::new(10, 165), text_style)
            .draw(&mut page)
            .unwrap();

        Text::new(
            "epdsi async PageBufferPair",
            Point::new(10, 185),
            text_style,
        )
        .draw(&mut page)
        .unwrap();

        // Each RAM write starts from the window origin, so reset window + cursor before both
        // channels.
        info!("Sending Black/White frame (5,000 bytes)...");
        epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
            .await
            .unwrap();
        epd.set_cursor(0, 0).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, page.bw().as_slice())
            .await
            .unwrap();

        info!("Sending Accent frame (5,000 bytes)...");
        epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
            .await
            .unwrap();
        epd.set_cursor(0, 0).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, page.accent().as_slice())
            .await
            .unwrap();

        info!("Refreshing display hardware (Full refresh, ~14 s)...");
        epd.refresh(&mut delay).await.unwrap();
    }

    Timer::after_millis(2000).await;

    info!("--- Phase 2: Partial Window Tri-Color Refresh ---");

    // The refresh mode deliberately stays `Full` (0x22 = 0xF7). Trigger 0xFC selects the SSD1681
    // built-in fast LUT, which only exists for monochrome panels — on a BWR panel it is slow
    // *and* discards Accent content. What makes this phase "partial" is the narrowed RAM window
    // below.
    defmt::debug_assert_eq!(epd.controller().refresh_mode(), Ssd1681RefreshMode::Full);

    // Bottom status band, updated in place. The Rust logo ends at y = 139 (75 + 64), so the band
    // starts at y = 140 and the header/logos painted in Phase 1 are never touched.
    const BAND_Y: u32 = 140;
    const BAND_H: u32 = 60;
    const BAND_BYTES: usize = (GDEM0154Z90::WIDTH as usize * BAND_H as usize) / 8;

    for count in 1..=5u32 {
        // Sub-region buffers: 200 x 60 / 8 = 1,500 bytes of the full-frame arrays.
        let mut band = PageBufferPair::new(
            &mut bw_buf[..BAND_BYTES],
            &mut red_buf[..BAND_BYTES],
            GDEM0154Z90::WIDTH,
            BAND_H,
            BAND_Y,
            POLARITY,
        );
        band.clear();

        // Update counter label (Black)
        let mut count_buf = [0u8; 32];
        let count_str =
            format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
        Text::new(count_str, Point::new(10, 157), text_style)
            .draw(&mut band)
            .unwrap();

        // Progress bar outline (Black)
        Rectangle::new(Point::new(10, 164), Size::new(180, 14))
            .into_styled(stroke)
            .draw(&mut band)
            .unwrap();

        // Progress bar fill (Accent) — proves the accent channel survives a partial window update
        Rectangle::new(Point::new(12, 166), Size::new(count * 35, 10))
            .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
            .draw(&mut band)
            .unwrap();

        Text::new("Partial window", Point::new(10, 195), text_style)
            .draw(&mut band)
            .unwrap();

        // Restrict controller RAM to the band, then write BOTH channels for that region. Writing
        // only Black/White would leave stale accent RAM behind for the band.
        epd.set_window(0, BAND_Y, GDEM0154Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, band.bw().as_slice())
            .await
            .unwrap();

        epd.set_window(0, BAND_Y, GDEM0154Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, band.accent().as_slice())
            .await
            .unwrap();

        info!(
            "Refreshing band y={}..{} (Update #{}, ~14 s)...",
            BAND_Y,
            BAND_Y + BAND_H - 1,
            count
        );
        epd.refresh(&mut delay).await.unwrap();

        Timer::after_millis(1000).await;
    }

    // Restore the full-frame RAM window for any subsequent updates.
    epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();

    info!("Display complete!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_ssd1681_gdem0154z90_tri_epd"),
    hal::binary_info::rp_program_description!(
        c"epdsi async SSD1681/GDEM0154Z90 PageBufferPair example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
