//! # SE0352N14TNGA0 Tri-Color `PageBufferPair` Draw Target Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `uc8253_se0352n14_tri_epd` example, which is
//! itself the full-parity `PageBufferPair` rewrite of `epdsi_uc8253_se0352n14` (this repo's
//! `PageBuffer`-based example). Same two phases, same content, over `epdsi`'s async API instead
//! of blocking.
//!
//! See `uc8253_se0352n14_epd`'s module doc (in `rust-rpico2-discovery`) for the full narrative on
//! this panel's quirks (swapped RAM plane routing vs. `GDEY037T03`, inverted ink polarity on
//! *both* planes, no partial/fast waveform, the charge-pump `POWER_ON` requirement, active-low
//! BUSY, and native orientation). What differs from `epdsi_uc8253_se0352n14` is only the drawing
//! code:
//!
//! - `PlanePolarity::UC8253` replaces the `INK`/`WHITE_BYTE` constants — this is the one panel
//!   `epdsi` ships where *both* planes are inverted, unlike SSD168x where only the accent plane
//!   is.
//! - One shared `PageBufferPair` (built once per phase via `frame()`) replaces the two
//!   independently-rotated `PageBuffer`s the original built per phase, removing structurally
//!   (rather than by discipline) the risk of `DisplayRotation::Rotate180` landing on one plane
//!   but not the other.
//! - `blit()` takes a `TriColor` destination color as well as the `BinaryColor` source-pixel
//!   test.
//!
//! ## Hardware
//!
//! Same board, panel and wiring as `epdsi_uc8253_se0352n14` — see that example for the pin table.
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_uc8253_se0352n14_tri_epd
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

/// This panel's polarity: *both* planes are inverted (a set bit is ink).
const POLARITY: PlanePolarity = PlanePolarity::UC8253;

/// Row stride in bytes: 240 / 8 = 30. This panel is already byte-aligned.
const STRIDE: usize = SE0352N14TNGA0::WIDTH.div_ceil(8) as usize;

/// Frame buffer size for one plane: 30 x 360 = 10,800 bytes.
const FRAME_BYTES: usize = STRIDE * SE0352N14TNGA0::HEIGHT as usize;

/// X coordinate of the left logo slot.
const LOGO_LEFT_X: i32 = 30;

/// X coordinate of the right logo slot.
const LOGO_RIGHT_X: i32 = 146;

/// Builds a full-frame drawing surface addressing both RAM planes, rotated to this repo's
/// convention. Going through one constructor for both planes together makes "rotation applied to
/// one plane but not the other" structurally impossible instead of a discipline to maintain.
fn frame<'a>(bw_buf: &'a mut [u8], accent_buf: &'a mut [u8]) -> PageBufferPair<'a> {
    let mut page = PageBufferPair::new(
        bw_buf,
        accent_buf,
        SE0352N14TNGA0::WIDTH,
        SE0352N14TNGA0::HEIGHT,
        0,
        POLARITY,
    );
    page.set_rotation(DisplayRotation::Rotate180);
    page
}

/// Draws a BMP into `page` as `color`, treating source pixels equal to `source_ink` as ink.
fn blit(
    page: &mut PageBufferPair,
    bmp: &Bmp<BinaryColor>,
    origin: Point,
    source_ink: BinaryColor,
    color: TriColor,
) {
    for Pixel(point, pixel_color) in bmp.pixels() {
        if pixel_color == source_ink {
            Pixel(point + origin, color).draw(page).unwrap();
        }
    }
}

/// Draws the panel-test Black content: title, separator, the pattern frame, the black band and
/// the band labels.
fn draw_test_black_content(page: &mut PageBufferPair) {
    let text_style = MonoTextStyle::new(&FONT_10X20, TriColor::Black);
    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);

    Text::new("PLANE TEST", Point::new(10, 24), text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(10, 34), Point::new(229, 34))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    Rectangle::new(Point::new(11, 50), Size::new(218, 240))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    Rectangle::new(Point::new(84, 51), Size::new(72, 238))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Black))
        .draw(page)
        .unwrap();

    for (label, centre_x) in [("W", 48), ("B", 120), ("R", 192)] {
        Text::new(label, Point::new(centre_x - 5, 320), text_style)
            .draw(page)
            .unwrap();
    }
}

/// Draws the panel-test Accent content: the right-hand red band.
fn draw_test_accent_content(page: &mut PageBufferPair) {
    Rectangle::new(Point::new(156, 51), Size::new(72, 238))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
        .draw(page)
        .unwrap();
}

/// Draws the Phase 2 content's Black half: border, header, separators, labels and the Rust logo.
fn draw_black_content(page: &mut PageBufferPair, rust_bmp: &Bmp<BinaryColor>) {
    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, TriColor::Black);

    Rectangle::new(
        Point::new(0, 0),
        Size::new(SE0352N14TNGA0::WIDTH, SE0352N14TNGA0::HEIGHT),
    )
    .into_styled(stroke)
    .draw(page)
    .unwrap();

    Text::new("SE0352N14", Point::new(10, 24), text_style)
        .draw(page)
        .unwrap();

    Text::new("3.52\" BWR", Point::new(10, 48), text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(10, 58), Point::new(229, 58))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    blit(
        page,
        rust_bmp,
        Point::new(LOGO_RIGHT_X, 90),
        BinaryColor::On,
        TriColor::Black,
    );

    Text::new("RP2350 Pico 2", Point::new(10, 200), text_style)
        .draw(page)
        .unwrap();

    Text::new("epdsi async PageBufferPair", Point::new(10, 225), text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(10, 240), Point::new(229, 240))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    Text::new("Waveshare (B)", Point::new(10, 285), text_style)
        .draw(page)
        .unwrap();
}

/// Draws the Phase 2 content's Accent half: the Ferris logo and an accent bar.
fn draw_accent_content(page: &mut PageBufferPair, ferris_bmp: &Bmp<BinaryColor>) {
    blit(
        page,
        ferris_bmp,
        Point::new(LOGO_LEFT_X, 112),
        BinaryColor::Off,
        TriColor::Accent,
    );

    Rectangle::new(Point::new(10, 300), Size::new(220, 30))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
        .draw(page)
        .unwrap();
}

/// Refreshes the panel and reports how long it took.
///
/// A full refresh on this panel is 16-20 s. Anything under a second means BUSY was never
/// observed asserted — see the blocking example's doc comment on this same check.
async fn refresh_timed<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, delay: &mut Delay)
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    let start = Instant::now();
    epd.refresh(delay).await.unwrap();
    let elapsed_ms = start.elapsed().as_millis();

    info!("Refresh returned after {} ms", elapsed_ms);
    if elapsed_ms < 1_000 {
        warn!(
            "Refresh took {} ms, expected ~16-20 s. BUSY likely had not asserted when polling \
             started; a guard delay between DISPLAY_REFRESH and the busy poll is the known fix.",
            elapsed_ms
        );
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!(
        "Starting SE0352N14TNGA0 3.52\" Tri-Color EPD example (epdsi UC8253, async/Embassy, PageBufferPair)"
    );

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

    // The variant is not optional. The default Gdey037t03 profile's init, plane order and CDI
    // value are all wrong for this panel, and it renders inverted or blank rather than erroring.
    let controller = Uc8253Controller::new(SE0352N14TNGA0::WIDTH, SE0352N14TNGA0::HEIGHT)
        .with_variant(Uc8253Variant::Se0352n14);
    let mut epd = EpdBuilder::<_, SE0352N14TNGA0>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing UC8253 epdsi EPD driver (Se0352n14 profile, async)...");
    epd.init(&mut delay).await.unwrap();

    // Both planes start white — PlanePolarity::UC8253's background byte for both, 0x00.
    epd.clear_frame(ColorChannel::BlackWhite, POLARITY.bw_background_byte())
        .await
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, POLARITY.accent_background_byte())
        .await
        .unwrap();

    let mut bw_buf = [POLARITY.bw_background_byte(); FRAME_BYTES];
    let mut red_buf = [POLARITY.accent_background_byte(); FRAME_BYTES];

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    info!("--- Phase 1: Panel Test ---");

    // The buffers start white, so nothing to clear yet.
    {
        let mut page = frame(&mut bw_buf, &mut red_buf);
        draw_test_black_content(&mut page);
        draw_test_accent_content(&mut page);
    }

    // No set_window: a full-frame write must not be wrapped in a partial-window session, and
    // this panel has no partial mode anyway.
    info!("Sending diagnostic pattern (white | black | red)...");
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, &red_buf).await.unwrap();

    info!("Refreshing display hardware (full waveform, expect ~16-20 s)...");
    refresh_timed(&mut epd, &mut delay).await;

    info!("Bands left to right should read white | black | red.");
    info!("  black/red swapped -> plane routing crossed (0x10/0x13)");
    info!("  white/black swapped -> CDI/DDX polarity or buffer base wrong");
    info!("  cleared panel looking grey -> normal white point, not a fault");

    Timer::after_millis(3000).await;

    info!("--- Phase 2: Full Tri-Color Refresh ---");

    // Drawn second so the panel is left showing the picture rather than the test pattern.
    {
        let mut page = frame(&mut bw_buf, &mut red_buf);
        page.clear();
        draw_black_content(&mut page, &rust_bmp);
        draw_accent_content(&mut page, &ferris_bmp);
    }

    info!("Sending both planes (10,800 bytes each)...");
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, &red_buf).await.unwrap();

    info!("Refreshing display hardware (full waveform, expect ~16-20 s)...");
    refresh_timed(&mut epd, &mut delay).await;

    // After sleep the controller is in deep sleep: init() must be called again before drawing.
    epd.sleep(&mut delay).await.unwrap();
    info!("Display complete, panel asleep!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_uc8253_se0352n14_tri_epd"),
    hal::binary_info::rp_program_description!(c"epdsi async UC8253/SE0352N14 PageBufferPair example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
