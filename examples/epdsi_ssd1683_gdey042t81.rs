//! # Good Display GDEY042T81 4.2" Monochrome E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1683_gdey042t81_epd` example: same panel,
//! same wiring, same four-phase content and driver call sequence, built against `epdsi`'s async
//! API (`default-features = false, features = ["graphics"]`) over `embassy-rp`'s async SPI and
//! GPIO instead of the blocking `rp235x-hal` API. Every `epdsi` call below is `.await`ed;
//! nothing else about the driver-level code differs from the blocking example.
//!
//! The panel is an **SSD1683**, a variant of the shared SSD168x controller, and nothing infers the
//! variant from the panel type: pairing `GDEY042T81` with another variant does not error, it
//! renders wrongly. This is not the `GDEY042T81-FL02` front-light variant.
//!
//! **Hardware-verified on the Pico 2 (RP2350, async)** (4 Nov 2026): Full 1584 ms, Partial 410 ms, FastFull 1665 ms,
//! so `FastFull` is slower than `Full` on this panel at room temperature. Needs an `epdsi` release newer than 0.10.0 for the variant.
//! Register sequences follow Good Display's `AU-GDEY042T81-FP4G-20240801` demo and the SSD1683
//! datasheet; see the `GDEY042T81` panel doc in `epdsi` for the deliberate departures (data entry
//! `0x03`, `0x18 80` at init, no hard reset before a partial).
//!
//! Demonstrates:
//! 1. **Phase 1**: Full monochrome refresh: header, side-by-side Ferris/Rust logos, footer
//!    labels and a status line. Writes zeros to the secondary RAM (`0x26`) for the full update,
//!    as Good Display's demo writes its own image to both planes, then copies the image into it so
//!    Phase 2's differential update has a correct base to diff against.
//! 2. **Phase 2**: Fast *differential* partial-window refresh loop over the content band (logos
//!    through the bottom status line), swapping the Ferris and Rust logos left and right on every
//!    pass and advancing a progress bar. The vendor partial byte is `0xFF`.
//! 3. **Phase 3**: Full-waveform cleanup pass over the whole content band, restoring the ink
//!    density the shortened Phase 2 waveform leaves behind.
//! 4. **Phase 4**: `FastFull`, the vendor's temperature-override fast update (`1A 6E`, then
//!    `0xC7`), on a full-frame redraw, timed against Phase 1. It runs last on a full redraw so it
//!    cannot reuse the band buffer the cleanup pass leaves behind.
//!
//! For 4-level grayscale on the same panel see `epdsi_ssd1683_gdey042t81_gray4`. This example's
//! twin for the other API is `ssd1683_gdey042t81_epd`. Each refresh is timed and logged.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Good Display GDEY042T81 4.2" e-Paper (400x300 Monochrome, 24-pin FPC)
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
//! cargo run --example epdsi_ssd1683_gdey042t81
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

/// Row stride in bytes. 400 px is byte-aligned, so this is exactly 50 with no padding.
const STRIDE: usize = GDEY042T81::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size: 50 x 300 = 15,000 bytes.
const FRAME_BYTES: usize = STRIDE * GDEY042T81::HEIGHT as usize;

/// Top Y coordinate of the content band repainted in Phases 2 and 3 — everything from just below
/// the title/subtitle separator down to the bottom of the panel, so the logo swap, footer labels
/// and status line are all inside the partial-refresh window. Only the border, title and subtitle
/// above it are painted once in Phase 1 and never touched again.
const BAND_Y: u32 = 52;

/// Height of the content band in pixels (y = 52..299).
const BAND_H: u32 = GDEY042T81::HEIGHT - BAND_Y;

/// Content band buffer size: 50 x 248 = 12,400 bytes.
const BAND_BYTES: usize = STRIDE * BAND_H as usize;

/// X coordinate of the left logo slot.
const LOGO_X_LEFT: i32 = 60;

/// X coordinate of the right logo slot.
const LOGO_X_RIGHT: i32 = 276;

/// Ferris's own Y offset (64x42 — shorter than Rust, so it sits a little lower to bottom-align).
const FERRIS_Y: i32 = 92;

/// Rust's own Y offset (64x64).
const RUST_Y: i32 = 82;

/// All-zero fill for the content band's secondary RAM, used for the Phase 3 cleanup pass. Good
/// Display's demo writes `0x00` to `0x26` before every full update, so the full waveform runs
/// against an all-black "previous image". Lives in flash rather than on the stack.
static ZERO_BAND: [u8; BAND_BYTES] = [0x00u8; BAND_BYTES];

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

/// Draws Ferris and Rust side by side (400 px fits the 64 px-wide logos with room to spare). `swapped` exchanges which logo occupies the
/// left slot: Phase 2 flips it on every partial update, the same idiom `epdsi_ssd1680_gdem0213b74`
/// uses for its stacked top/bottom swap.
fn draw_logos(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
) {
    let (ferris_x, rust_x) = if swapped {
        (LOGO_X_RIGHT, LOGO_X_LEFT)
    } else {
        (LOGO_X_LEFT, LOGO_X_RIGHT)
    };

    let ferris_pos = Point::new(ferris_x, FERRIS_Y);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    let rust_pos = Point::new(rust_x, RUST_Y);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
}

/// Draws the footer labels, mode line and the separator above them — identical every time it is
/// called, so Phase 2 can redraw it unchanged inside the content band alongside the swapped logos.
fn draw_footer(display: &mut PageBuffer, mode_label: &str) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    Text::new("RP2350 Pico 2", Point::new(8, 170), small_text_style)
        .draw(display)
        .unwrap();
    Text::new("epdsi async", Point::new(8, 184), small_text_style)
        .draw(display)
        .unwrap();
    Text::new(mode_label, Point::new(8, 198), small_text_style)
        .draw(display)
        .unwrap();

    // Separator above the status line that Phase 2's counter/bar sits below.
    Line::new(Point::new(8, 210), Point::new(391, 210))
        .into_styled(stroke)
        .draw(display)
        .unwrap();
}

/// Draws the Phase 1 static content: border, title, subtitle, logos (never swapped here — only
/// Phase 2 swaps them) and footer.
fn draw_static_content(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    mode_label: &str,
) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    // Outer border, so a shifted or wrapped raster is obvious.
    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEY042T81::WIDTH, GDEY042T81::HEIGHT),
    )
    .into_styled(stroke)
    .draw(display)
    .unwrap();

    Text::new("GDEY042T81", Point::new(8, 22), text_style)
        .draw(display)
        .unwrap();

    Text::new("4.2\" Mono", Point::new(8, 40), small_text_style)
        .draw(display)
        .unwrap();

    Line::new(Point::new(8, 48), Point::new(391, 48))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    draw_logos(display, ferris_bmp, rust_bmp, false);
    draw_footer(display, mode_label);
}

/// Writes the full frame to Black/White RAM and zeros to the secondary RAM (`0x26`), the way Good
/// Display's `EPD_WhiteScreen_ALL` does before every full update.
async fn write_full_frame<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, data: &[u8])
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEY042T81::WIDTH - 1, GDEY042T81::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, data)
        .await
        .unwrap();

    epd.set_window(0, 0, GDEY042T81::WIDTH - 1, GDEY042T81::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0x00)
        .await
        .unwrap();
}

/// Copies the frame now on glass into the secondary RAM (`0x26`), the last step of Good Display's
/// `EPD_SetRAMValue_BaseMap`, so Phase 2's differential updates diff against the right base.
async fn seed_previous_frame<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, data: &[u8])
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEY042T81::WIDTH - 1, GDEY042T81::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, data)
        .await
        .unwrap();
}

/// Draws the status line: label, counter and progress bar. Fixed at an absolute Y position —
/// deliberately independent of [`BAND_Y`], which is just the partial-refresh window's top edge,
/// not where content starts. This sits well inside that window, below the logos and footer.
fn draw_band(band: &mut PageBuffer, count: u32, label: &str) {
    const STATUS_Y: i32 = 220;

    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    Text::new(label, Point::new(8, STATUS_Y + 14), small_text_style)
        .draw(band)
        .unwrap();

    let mut count_buf = [0u8; 32];
    let count_str = format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
    Text::new(count_str, Point::new(8, STATUS_Y + 28), small_text_style)
        .draw(band)
        .unwrap();

    Rectangle::new(Point::new(8, STATUS_Y + 38), Size::new(384, 16))
        .into_styled(stroke)
        .draw(band)
        .unwrap();

    // Capped at 380: the outline above is 384px wide starting at x=8, the fill starts 2px in at
    // x=10, so 380 lands the fill's right edge 2px inside the outline's, symmetric with the left
    // inset.
    Rectangle::new(
        Point::new(10, STATUS_Y + 40),
        Size::new((count * 77).min(380), 12),
    )
    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
    .draw(band)
    .unwrap();
}

/// Redraws the outer border's left, right and bottom edges for this band's row range.
///
/// Phase 1 draws the full-panel border once, but the content band's `clear_byte` + full redraw
/// on every Phase 2 pass (and the Phase 3 cleanup resend) wipes out whatever of that border falls
/// within the band — everything except the sliver above [`BAND_Y`], which is never touched. Without
/// this, the border only ever appears around the title and looks disconnected from the rest of the
/// content below it.
fn draw_band_border(band: &mut PageBuffer) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let bottom = GDEY042T81::HEIGHT as i32 - 1;
    let right = GDEY042T81::WIDTH as i32 - 1;

    Line::new(Point::new(0, BAND_Y as i32), Point::new(0, bottom))
        .into_styled(stroke)
        .draw(band)
        .unwrap();
    Line::new(Point::new(right, BAND_Y as i32), Point::new(right, bottom))
        .into_styled(stroke)
        .draw(band)
        .unwrap();
    Line::new(Point::new(0, bottom), Point::new(right, bottom))
        .into_styled(stroke)
        .draw(band)
        .unwrap();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEY042T81 4.2\" Monochrome EPD example (epdsi SSD1683, async/Embassy)");

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // SSD1683 BUSY is active-HIGH
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

    // Instantiate epdsi's async SPI bus wrapper and the SSD168x controller. The SSD1683 is a variant
    // of the shared SSD168x driver, and nothing infers it from the panel type: pairing GDEY042T81
    // with the SSD1680 or SSD1681 variant does not error, it renders wrongly.
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd168xController::for_panel::<GDEY042T81>(Ssd168xVariant::Ssd1683)
        .with_refresh_mode(Ssd168xRefreshMode::Full);
    let mut epd = EpdBuilder::<_, GDEY042T81>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1683 epdsi EPD driver (async)...");
    epd.init(&mut delay).await.unwrap();

    // Black/White RAM starts white and the secondary RAM (0x26) starts at zero, as in Good
    // Display's `EPD_WhiteScreen_White`. On this monochrome panel 0x26 is the "previous image"
    // buffer used by differential updates, not a color plane.
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF)
        .await
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0x00)
        .await
        .unwrap();

    let mut bw_buf = [0xFFu8; FRAME_BYTES];

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    info!("--- Phase 1: Full Monochrome Refresh ---");

    {
        let mut display = PageBuffer::new(&mut bw_buf, GDEY042T81::WIDTH, GDEY042T81::HEIGHT, 0);
        draw_static_content(&mut display, &ferris_bmp, &rust_bmp, "mode: Full");

        info!("Sending frame ({} bytes)...", FRAME_BYTES);
        write_full_frame(&mut epd, display.as_slice()).await;

        timed_refresh(&mut epd, &mut delay, "Phase 1 (Full)").await;
    };

    // Prime the secondary RAM (0x26) with what is now on glass, as Good Display's
    // `EPD_SetRAMValue_BaseMap` does last, so Phase 2 diffs against the right base.
    seed_previous_frame(&mut epd, &bw_buf[..]).await;

    Timer::after_millis(2000).await;

    info!("--- Phase 2: Fast Partial Window Refresh (logo swap) ---");

    // Select the SSD1683 built-in fast LUT (0x22 = 0xFF), a genuine differential update on this
    // monochrome panel.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Partial);

    for count in 1..=5u32 {
        // Flip the logo order on every pass — same idiom `epdsi_ssd1680_gdem0213b74` and
        // `epdsi_uc8253_gdey037t03` use, just left/right instead of top/bottom.
        let swapped = count % 2 == 1;

        let mut band =
            PageBuffer::new(&mut bw_buf[..BAND_BYTES], GDEY042T81::WIDTH, BAND_H, BAND_Y);
        band.clear_byte(0xFF);
        draw_band_border(&mut band);
        draw_logos(&mut band, &ferris_bmp, &rust_bmp, swapped);
        draw_footer(&mut band, "mode: Full");
        draw_band(&mut band, count, "Fast partial");

        // Restrict controller RAM to the band, write the new image to Black/White RAM.
        epd.set_window(0, BAND_Y, GDEY042T81::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, band.as_slice())
            .await
            .unwrap();

        let ms = timed_refresh(&mut epd, &mut delay, "Phase 2 (Partial)").await;
        info!(
            "Update #{}: {} ms (logos {})",
            count,
            ms,
            if swapped { "swapped" } else { "normal" }
        );

        // Copy the band we just displayed into the "previous image" RAM so the next iteration
        // diffs against what is actually on the panel.
        epd.set_window(0, BAND_Y, GDEY042T81::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, band.as_slice())
            .await
            .unwrap();

        Timer::after_millis(500).await;
    }

    info!("--- Phase 3: Full-Waveform Cleanup Pass ---");

    // Differential updates drive the pixels with a shorter waveform than the OTP full-refresh
    // LUT, so ink density can drift after several Phase 2 passes. Re-running the final band
    // content through the full waveform restores even density. Blanking the secondary RAM
    // first stops it being read as a stale differential base afterwards.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);

    epd.set_window(0, BAND_Y, GDEY042T81::WIDTH - 1, BAND_Y + BAND_H - 1)
        .await
        .unwrap();
    epd.set_cursor(0, BAND_Y).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, &ZERO_BAND)
        .await
        .unwrap();

    // `bw_buf` still holds the last band drawn in Phase 2 — nothing has touched it since, so
    // re-sending it unchanged is safe. (This is exactly the assumption that broke when an earlier
    // revision inserted a FastFull full-frame phase here: see the blocking example's module doc.)
    epd.set_window(0, BAND_Y, GDEY042T81::WIDTH - 1, BAND_Y + BAND_H - 1)
        .await
        .unwrap();
    epd.set_cursor(0, BAND_Y).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..BAND_BYTES])
        .await
        .unwrap();

    timed_refresh(&mut epd, &mut delay, "Phase 3 (cleanup)").await;

    Timer::after_millis(2000).await;

    info!("--- Phase 4: FastFull Refresh ---");

    // Phase 4 runs last and redraws the WHOLE frame. `bw_buf` holds band-relative content from
    // Phase 3, and `write_full_frame` sends a full-frame window, so reusing it as-is would paint a
    // shifted duplicate (the failure an earlier SSD1680 example hit). The vendor's fast path also
    // zeroes `0x26` before the update, which `write_full_frame` already does.
    {
        let mut display = PageBuffer::new(&mut bw_buf, GDEY042T81::WIDTH, GDEY042T81::HEIGHT, 0);
        display.clear_byte(0xFF);
        draw_static_content(&mut display, &ferris_bmp, &rust_bmp, "mode: FastFull");
        write_full_frame(&mut epd, display.as_slice()).await;
    }
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::FastFull);
    let fast_ms = timed_refresh(&mut epd, &mut delay, "Phase 4 (FastFull)").await;
    info!(
        "FastFull took {} ms against Full's earlier Phase 1 time",
        fast_ms
    );

    // Restore the full-frame RAM window and the default waveform for any subsequent updates.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);
    epd.set_window(0, 0, GDEY042T81::WIDTH - 1, GDEY042T81::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();

    // Deep sleep. init() must be called again before any further frame.
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
    hal::binary_info::rp_program_name!(c"epdsi_ssd1683_gdey042t81"),
    hal::binary_info::rp_program_description!(c"epdsi async SSD1683/GDEY042T81 example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
