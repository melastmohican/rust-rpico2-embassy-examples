//! # Good Display GDEY0266T90H 2.66" Monochrome E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1685_gdey0266t90h_epd` example — full
//! parity: same panel, same wiring, same three-phase content and driver call sequence, built
//! against `epdsi`'s async API (`default-features = false, features = ["graphics"]`) over
//! `embassy-rp`'s async SPI and GPIO instead of the blocking `rp235x-hal` API. Every `epdsi` call
//! below is `.await`ed; nothing else about the driver-level code differs from the blocking
//! example.
//!
//! This is a **different panel** from `GDEY0266T90` (`epdsi_ssd1680_gdey0266t90`): the same 2.66"
//! size, but a 184x360 raster behind an **SSD1685** instead of an SSD1680, driven through the
//! shared SSD168x controller's `Ssd168xVariant::Ssd1685`. The phases below follow that example's
//! structure so the two read side by side: Full, then a partial-window loop that swaps two logos on
//! every pass, then a full-waveform cleanup pass.
//!
//! **Hardware-verified on the Pico 2 (RP2350, async)** (11 Oct 2026). Needs `epdsi` 0.8.0 or later for the variant.
//! Register sequence follows Good Display's `A32-GDEY0266T90H-2FP-20230915` demo and the SSD1685
//! datasheet; see the `GDEY0266T90H` panel doc in `epdsi` for the two deliberate departures
//! (`0xF7` full, `0xDC` partial).
//!
//! Demonstrates:
//! 1. **Phase 1**: Full monochrome refresh — header, side-by-side Ferris/Rust logos, footer
//!    labels, and a status line. Writes zeros to the secondary RAM (`0x26`) for the full
//!    update, as Good Display's demo does, then copies the image into it so Phase 2's differential
//!    update has a correct base to diff against.
//! 2. **Phase 2**: Fast *differential* partial-window refresh loop over the content band (logos
//!    through the bottom status line), swapping the Ferris and Rust logos on every pass and
//!    advancing a progress bar — the same "logo swap" idiom `epdsi_ssd1680_gdem0213b74` uses
//!    (there, the two logos are stacked and swap top/bottom because its 122px panel is too narrow
//!    for them side by side; here, at 184px, they swap left/right instead).
//! 3. **Phase 3**: Full-waveform cleanup pass over the whole content band (logos, footer and
//!    status line), restoring the ink density the shortened Phase 2 differential waveform leaves
//!    behind — the same idiom `epdsi_ssd1680_gdem0213b74`/`epdsi_uc8253_gdey037t03` use.
//!
//! ## Note on refresh speed
//!
//! See the blocking example's module doc. This async run measured Full 1190 ms, Partial 355 ms and
//! cleanup 1190 ms on the Pico 2 (11 Oct 2026), within 3 ms of the three blocking boards, against ~4.1 s for both on the SSD1680 `GDEY0266T90`. Each refresh
//! is timed and logged here too; only the transport differs.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Good Display GDEY0266T90H 2.66" e-Paper (FPC-H011, 184x360 Monochrome)
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
//! cargo run --example epdsi_ssd1685_gdey0266t90h
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

/// Row stride in bytes. 184 px is byte-aligned, so this is exactly 23 with no padding.
const STRIDE: usize = GDEY0266T90H::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size: 23 x 360 = 8,280 bytes.
const FRAME_BYTES: usize = STRIDE * GDEY0266T90H::HEIGHT as usize;

/// Top Y coordinate of the content band repainted in Phases 2 and 3 — everything from just below
/// the title/subtitle separator down to the bottom of the panel, so the logo swap, footer labels
/// and status line are all inside the partial-refresh window. Only the border, title and subtitle
/// above it are painted once in Phase 1 and never touched again.
const BAND_Y: u32 = 52;

/// Height of the content band in pixels (y = 52..359).
const BAND_H: u32 = GDEY0266T90H::HEIGHT - BAND_Y;

/// Content band buffer size: 23 x 308 = 7,084 bytes.
const BAND_BYTES: usize = STRIDE * BAND_H as usize;

/// X coordinate of the left logo slot.
const LOGO_X_LEFT: i32 = 10;

/// X coordinate of the right logo slot.
const LOGO_X_RIGHT: i32 = 78;

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

/// Draws Ferris and Rust side by side (184 px fits 64 px-wide logos, unlike the 122 px
/// `GDEM0213B74` where they have to be stacked). `swapped` exchanges which logo occupies the
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
    Line::new(Point::new(8, 210), Point::new(175, 210))
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
        Size::new(GDEY0266T90H::WIDTH, GDEY0266T90H::HEIGHT),
    )
    .into_styled(stroke)
    .draw(display)
    .unwrap();

    Text::new("GDEY0266T90H", Point::new(8, 22), text_style)
        .draw(display)
        .unwrap();

    Text::new("2.66\" Mono", Point::new(8, 40), small_text_style)
        .draw(display)
        .unwrap();

    Line::new(Point::new(8, 48), Point::new(175, 48))
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
    epd.set_window(0, 0, GDEY0266T90H::WIDTH - 1, GDEY0266T90H::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, data)
        .await
        .unwrap();

    epd.set_window(0, 0, GDEY0266T90H::WIDTH - 1, GDEY0266T90H::HEIGHT - 1)
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
    epd.set_window(0, 0, GDEY0266T90H::WIDTH - 1, GDEY0266T90H::HEIGHT - 1)
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

    Rectangle::new(Point::new(8, STATUS_Y + 38), Size::new(168, 16))
        .into_styled(stroke)
        .draw(band)
        .unwrap();

    // Capped at 164: the outline above is 168px wide starting at x=8, the fill starts 2px in at
    // x=10, so 164 lands the fill's right edge 2px inside the outline's, symmetric with the left
    // inset. `count * 33` alone overshoots that at count=5 (165px) — past the outline *and* past
    // the panel's own 184px width — which is exactly the overflowing bar seen on real hardware.
    Rectangle::new(
        Point::new(10, STATUS_Y + 40),
        Size::new((count * 33).min(164), 12),
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
    let bottom = GDEY0266T90H::HEIGHT as i32 - 1;
    let right = GDEY0266T90H::WIDTH as i32 - 1;

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
    info!("Starting GDEY0266T90H 2.66\" Monochrome EPD example (epdsi SSD1685, async/Embassy)");

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // SSD1685 BUSY is active-HIGH
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

    // Instantiate epdsi's async SPI bus wrapper and the SSD168x controller. The SSD1685 is a variant
    // of the shared SSD168x driver, and nothing infers it from the panel type: pairing GDEY0266T90H
    // with the SSD1680 or SSD1681 variant does not error, it renders wrongly.
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd168xController::for_panel::<GDEY0266T90H>(Ssd168xVariant::Ssd1685)
        .with_refresh_mode(Ssd168xRefreshMode::Full);
    let mut epd = EpdBuilder::<_, GDEY0266T90H>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1685 epdsi EPD driver (async)...");
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
        let mut display =
            PageBuffer::new(&mut bw_buf, GDEY0266T90H::WIDTH, GDEY0266T90H::HEIGHT, 0);
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

    // Select the SSD1685 built-in fast LUT (0x22 = 0xDC). Unlike the Tri-Color GDEY0266Z90, this
    // is a genuine differential update on this monochrome panel and should complete in well under
    // a second.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Partial);

    for count in 1..=5u32 {
        // Flip the logo order on every pass — same idiom `epdsi_ssd1680_gdem0213b74` and
        // `epdsi_uc8253_gdey037t03` use, just left/right instead of top/bottom.
        let swapped = count % 2 == 1;

        let mut band = PageBuffer::new(
            &mut bw_buf[..BAND_BYTES],
            GDEY0266T90H::WIDTH,
            BAND_H,
            BAND_Y,
        );
        band.clear_byte(0xFF);
        draw_band_border(&mut band);
        draw_logos(&mut band, &ferris_bmp, &rust_bmp, swapped);
        draw_footer(&mut band, "mode: Full");
        draw_band(&mut band, count, "Fast partial");

        // Restrict controller RAM to the band, write the new image to Black/White RAM.
        epd.set_window(0, BAND_Y, GDEY0266T90H::WIDTH - 1, BAND_Y + BAND_H - 1)
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
        epd.set_window(0, BAND_Y, GDEY0266T90H::WIDTH - 1, BAND_Y + BAND_H - 1)
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

    epd.set_window(0, BAND_Y, GDEY0266T90H::WIDTH - 1, BAND_Y + BAND_H - 1)
        .await
        .unwrap();
    epd.set_cursor(0, BAND_Y).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, &ZERO_BAND)
        .await
        .unwrap();

    // `bw_buf` still holds the last band drawn in Phase 2 — nothing has touched it since, so
    // re-sending it unchanged is safe. (This is exactly the assumption that broke when an earlier
    // revision inserted a FastFull full-frame phase here: see the blocking example's module doc.)
    epd.set_window(0, BAND_Y, GDEY0266T90H::WIDTH - 1, BAND_Y + BAND_H - 1)
        .await
        .unwrap();
    epd.set_cursor(0, BAND_Y).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..BAND_BYTES])
        .await
        .unwrap();

    timed_refresh(&mut epd, &mut delay, "Phase 3 (cleanup)").await;

    // Restore the full-frame RAM window and the default waveform for any subsequent updates.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);
    epd.set_window(0, 0, GDEY0266T90H::WIDTH - 1, GDEY0266T90H::HEIGHT - 1)
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
    hal::binary_info::rp_program_name!(c"epdsi_ssd1685_gdey0266t90h"),
    hal::binary_info::rp_program_description!(
        c"epdsi async SSD1685/GDEY0266T90H example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
