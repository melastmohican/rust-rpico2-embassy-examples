//! # GDEY0266Z90 Tri-Color `PageBufferPair` Draw Target Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1680_gdey0266z90_tri_epd` example, which is
//! itself the full-parity `PageBufferPair` rewrite of `epdsi_ssd1680_gdey0266z90` (this repo's
//! `PageBuffer`-based example). Same four phases, same content, same driver call sequence, over
//! `epdsi`'s async API instead of blocking — every `epdsi` call below is `.await`ed, nothing else
//! differs at the driver level.
//!
//! What differs from `epdsi_ssd1680_gdey0266z90` is only the drawing code:
//!
//! - One `page: &mut PageBufferPair` replaces the two `bw`/`red` `PageBuffer` parameters
//!   `draw_static_content`/`draw_band`/`draw_band_bar` used to take.
//! - `TriColor::{Black, Accent}` replaces every `BinaryColor::On`/`Off` choice.
//! - `PageBufferPair::clear()` replaces the `clear_byte(0xFF)` / `clear_byte(0x00)` pair before
//!   each windowed redraw, and the static `NO_RED_BAND` workaround is gone — `page.accent()`'s
//!   own background bytes, derived from `PlanePolarity`, already are that array.
//!
//! See `ssd1680_gdey0266z90_epd`'s module doc (in `rust-rpico2-discovery`) for the full
//! narrative on refresh timing, duty cycle, ink polarity, and glass provenance.
//!
//! ## Hardware
//!
//! Same board, panel and wiring as `epdsi_ssd1680_gdey0266z90` — see that example for the pin
//! table.
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_ssd1680_gdey0266z90_tri_epd
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
use embassy_time::{Delay, Instant, Timer};

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_10X20};
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

/// Row stride in bytes. 152 px is byte-aligned, so this is exactly 19 with no padding.
const STRIDE: usize = GDEY0266Z90::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size per plane: 19 x 296 = 5,624 bytes.
const FRAME_BYTES: usize = STRIDE * GDEY0266Z90::HEIGHT as usize;

/// Top Y coordinate of the status band repainted in Phases 2 and 4.
const BAND_Y: u32 = 220;

/// Height of the status band in pixels (y = 220..295).
const BAND_H: u32 = 76;

/// Status band buffer size: 19 x 76 = 1,444 bytes.
const BAND_BYTES: usize = STRIDE * BAND_H as usize;

/// The polarity this panel needs — Black/White plane normal, accent plane inverted.
const POLARITY: PlanePolarity = PlanePolarity::SSD168X;

/// Refreshes the panel, reporting how long it took and returning the elapsed milliseconds.
async fn timed_refresh<BUS, C, P>(
    epd: &mut EpdDriver<BUS, C, P>,
    delay: &mut Delay,
    label: &str,
) -> u64
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    let start = Instant::now();
    epd.refresh(delay).await.unwrap();
    let elapsed_ms = start.elapsed().as_millis();
    info!("{}: refresh took {} ms", label, elapsed_ms);
    elapsed_ms
}

/// Draws the Phase 1 / Phase 3 static content: everything above the status band.
///
/// One `page` addresses both RAM planes — no separate `bw`/`red` buffers, and no
/// polarity-aware `BinaryColor` choice.
fn draw_static_content(
    page: &mut PageBufferPair,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    mode_label: &str,
) {
    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, TriColor::Black);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, TriColor::Black);
    let accent_small_text_style = MonoTextStyle::new(&FONT_6X10, TriColor::Accent);

    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT),
    )
    .into_styled(stroke)
    .draw(page)
    .unwrap();

    Text::new("GDEY0266Z90", Point::new(8, 22), text_style)
        .draw(page)
        .unwrap();

    Text::new("Tri-Color ", Point::new(8, 40), small_text_style)
        .draw(page)
        .unwrap();
    Text::new("BWR", Point::new(68, 40), accent_small_text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(8, 48), Point::new(143, 48))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    Rectangle::new(Point::new(8, 56), Size::new(136, 18))
        .into_styled(stroke)
        .draw(page)
        .unwrap();
    Rectangle::new(Point::new(10, 58), Size::new(64, 14))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Black))
        .draw(page)
        .unwrap();
    Rectangle::new(Point::new(78, 58), Size::new(64, 14))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
        .draw(page)
        .unwrap();

    let ferris_pos = Point::new(10, 92);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, TriColor::Accent)
                .draw(page)
                .unwrap();
        }
    }

    let rust_pos = Point::new(78, 82);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, TriColor::Black)
                .draw(page)
                .unwrap();
        }
    }

    Text::new("RP2350 Pico 2", Point::new(8, 170), small_text_style)
        .draw(page)
        .unwrap();
    Text::new(
        "epdsi async PageBufferPair",
        Point::new(8, 184),
        small_text_style,
    )
    .draw(page)
    .unwrap();
    Text::new(mode_label, Point::new(8, 198), small_text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(8, 210), Point::new(143, 210))
        .into_styled(stroke)
        .draw(page)
        .unwrap();
}

/// Writes both colour planes for the full frame, resetting the RAM window and cursor first.
async fn write_full_frame<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, page: &PageBufferPair<'_>)
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEY0266Z90::WIDTH - 1, GDEY0266Z90::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, page.bw().as_slice())
        .await
        .unwrap();

    epd.set_window(0, 0, GDEY0266Z90::WIDTH - 1, GDEY0266Z90::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, page.accent().as_slice())
        .await
        .unwrap();
}

/// Draws the status band's Black content: label, counter and progress bar outline.
fn draw_band(page: &mut PageBufferPair, count: u32, label: &str) {
    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, TriColor::Black);

    Text::new(label, Point::new(8, BAND_Y as i32 + 14), small_text_style)
        .draw(page)
        .unwrap();

    let mut count_buf = [0u8; 32];
    let count_str = format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
    Text::new(
        count_str,
        Point::new(8, BAND_Y as i32 + 28),
        small_text_style,
    )
    .draw(page)
    .unwrap();

    Rectangle::new(Point::new(8, BAND_Y as i32 + 38), Size::new(136, 16))
        .into_styled(stroke)
        .draw(page)
        .unwrap();
}

/// Draws the progress bar fill for `count`, always in Accent.
fn draw_band_bar(page: &mut PageBufferPair, count: u32) {
    Rectangle::new(
        Point::new(10, BAND_Y as i32 + 40),
        Size::new(count * 33, 12),
    )
    .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
    .draw(page)
    .unwrap();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!(
        "Starting GDEY0266Z90 2.66\" Tri-Color EPD example (epdsi SSD1680, async/Embassy, PageBufferPair)"
    );

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // SSD1680 BUSY is active-HIGH
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
    let controller = Ssd1680Controller::new(GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT)
        .with_refresh_mode(Ssd168xRefreshMode::Full);
    let mut epd = EpdBuilder::<_, GDEY0266Z90>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1680 epdsi EPD driver (async)...");
    epd.init(&mut delay).await.unwrap();

    // Frame buffers: 5,624 bytes each, pre-filled to each plane's own background byte under
    // POLARITY.
    let mut bw_buf = [POLARITY.bw_background_byte(); FRAME_BYTES];
    let mut red_buf = [POLARITY.accent_background_byte(); FRAME_BYTES];

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    info!("--- Phase 1: Full Tri-Color Refresh ---");

    let full_ms = {
        let mut page = PageBufferPair::new(
            &mut bw_buf,
            &mut red_buf,
            GDEY0266Z90::WIDTH,
            GDEY0266Z90::HEIGHT,
            0,
            POLARITY,
        );

        draw_static_content(&mut page, &ferris_bmp, &rust_bmp, "mode: Full");

        info!("Sending both planes ({} bytes each)...", FRAME_BYTES);
        write_full_frame(&mut epd, &page).await;

        timed_refresh(&mut epd, &mut delay, "Phase 1 (Full)").await
    };

    Timer::after_millis(2000).await;

    info!("--- Phase 2: Windowed Refresh on the Full Waveform ---");

    for count in 1..=2u32 {
        {
            let mut band = PageBufferPair::new(
                &mut bw_buf[..BAND_BYTES],
                &mut red_buf[..BAND_BYTES],
                GDEY0266Z90::WIDTH,
                BAND_H,
                BAND_Y,
                POLARITY,
            );
            band.clear();

            draw_band(&mut band, count, "Full window");
            draw_band_bar(&mut band, count);

            epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
                .await
                .unwrap();
            epd.set_cursor(0, BAND_Y).await.unwrap();
            epd.write_frame(ColorChannel::BlackWhite, band.bw().as_slice())
                .await
                .unwrap();
            epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
                .await
                .unwrap();
            epd.set_cursor(0, BAND_Y).await.unwrap();
            epd.write_frame(ColorChannel::RedYellow, band.accent().as_slice())
                .await
                .unwrap();
        }

        info!("Refreshing band y={}..{}...", BAND_Y, BAND_Y + BAND_H - 1);
        timed_refresh(&mut epd, &mut delay, "Phase 2 (windowed Full)").await;
        Timer::after_millis(1000).await;
    }

    info!("--- Phase 3: FastFull Full-Screen Refresh ---");

    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::FastFull);

    let fast_ms = {
        let mut page = PageBufferPair::new(
            &mut bw_buf,
            &mut red_buf,
            GDEY0266Z90::WIDTH,
            GDEY0266Z90::HEIGHT,
            0,
            POLARITY,
        );
        page.clear();

        draw_static_content(&mut page, &ferris_bmp, &rust_bmp, "mode: FastFull");

        write_full_frame(&mut epd, &page).await;

        timed_refresh(&mut epd, &mut delay, "Phase 3 (FastFull)").await
    };

    info!(
        "Full {} ms vs FastFull {} ms. Reference for this glass is 20048 vs 16180 (~19% faster). \
         Good Display quote ~20000 vs ~19000 on their own glass, so expect the saving to vary \
         with the OTP waveform rather than assuming either figure.",
        full_ms, fast_ms
    );

    Timer::after_millis(2000).await;

    info!("--- Phase 4: BaseMap and Partial (both full-waveform on this panel) ---");

    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::BaseMap);

    {
        let mut band = PageBufferPair::new(
            &mut bw_buf[..BAND_BYTES],
            &mut red_buf[..BAND_BYTES],
            GDEY0266Z90::WIDTH,
            BAND_H,
            BAND_Y,
            POLARITY,
        );
        band.clear();

        draw_band(&mut band, 0, "BaseMap");
        draw_band_bar(&mut band, 0);

        epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, band.bw().as_slice())
            .await
            .unwrap();
        epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, band.accent().as_slice())
            .await
            .unwrap();
    }

    timed_refresh(&mut epd, &mut delay, "Phase 4 (BaseMap)").await;

    Timer::after_millis(1000).await;

    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Partial);

    for count in 1..=2u32 {
        {
            let mut band = PageBufferPair::new(
                &mut bw_buf[..BAND_BYTES],
                &mut red_buf[..BAND_BYTES],
                GDEY0266Z90::WIDTH,
                BAND_H,
                BAND_Y,
                POLARITY,
            );
            band.clear();

            draw_band(&mut band, count, "Partial mode");
            draw_band_bar(&mut band, count);

            epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
                .await
                .unwrap();
            epd.set_cursor(0, BAND_Y).await.unwrap();
            epd.write_frame(ColorChannel::BlackWhite, band.bw().as_slice())
                .await
                .unwrap();
            epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
                .await
                .unwrap();
            epd.set_cursor(0, BAND_Y).await.unwrap();
            epd.write_frame(ColorChannel::RedYellow, band.accent().as_slice())
                .await
                .unwrap();
        }

        info!("Partial-mode update #{}...", count);
        timed_refresh(&mut epd, &mut delay, "Phase 4 (Partial)").await;

        Timer::after_millis(1000).await;
    }

    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);
    epd.set_window(0, 0, GDEY0266Z90::WIDTH - 1, GDEY0266Z90::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();

    epd.sleep(&mut delay).await.unwrap();
    info!("Display complete, controller asleep.");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_ssd1680_gdey0266z90_tri_epd"),
    hal::binary_info::rp_program_description!(
        c"epdsi async SSD1680/GDEY0266Z90 PageBufferPair example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
