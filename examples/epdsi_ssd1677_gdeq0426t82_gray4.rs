//! # Good Display GDEQ0426T82 4.26" 4-Level Grayscale (Gray4) E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1677_gdeq0426t82_gray4_epd` example — full
//! parity: same panel, same wiring, same two-screen content and driver call sequence, built
//! against `epdsi`'s async API (`default-features = false, features = ["graphics"]`) over
//! `embassy-rp`'s async SPI and GPIO instead of the blocking `rp235x-hal` API. Every `epdsi` call
//! below is `.await`ed; nothing else about the driver-level code differs from the blocking
//! example.
//!
//! See the blocking example's module doc for the full detail on provenance (this is Adafruit_EPD
//! material, not Good Display/Seeed, and **not yet confirmed on hardware**), the portrait rotation,
//! and why `Adafruit_SSD1677::update()`'s grayscale branch needs a two-pass refresh
//! (`Ssd1677RefreshMode::Gray4Preclear` then `Gray4`, with `reload_gray4_lut` in between) unlike
//! this repo's single-pass SSD1680 Gray4 examples — none of that changes here, only the transport
//! does.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Dalian Good Display GDEQ0426T82 4.26" (800x480), driven in Gray4 mode
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
//! cargo run --example epdsi_ssd1677_gdeq0426t82_gray4
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
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_10X20};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::Text;
use embedded_hal_bus::spi::ExclusiveDevice;
use epdsi::prelude::*;

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

/// Frame buffer size per bit-plane: 100 bytes per RAM row x 480 rows = 48,000 bytes.
const FRAME_BYTES: usize = GDEQ0426T82::WIDTH.div_ceil(8) as usize * GDEQ0426T82::HEIGHT as usize;

/// Visible width in the rotated portrait frame (the panel's 480 px axis).
const VIEW_W: u32 = GDEQ0426T82::HEIGHT;

/// Visible height in the rotated portrait frame (the panel's 800 px axis).
const VIEW_H: u32 = GDEQ0426T82::WIDTH;

/// This panel's two RAM planes are both inverted — see `Gray4Polarity::ADAFRUIT_SSD1677`'s doc
/// for the derivation from `ThinkInk_426_Grayscale4_GDEQ.h`.
const POLARITY: Gray4Polarity = Gray4Polarity::ADAFRUIT_SSD1677;

/// Screen 1: title/subtitle banner over a 4-band Black/Dark/Light/White swatch.
fn draw_banner(page: &mut GrayBufferPair) {
    let title_style = MonoTextStyle::new(&FONT_10X20, Gray4Color::Black);
    let dark_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Dark);
    let light_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Light);
    let black_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let white_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::White);

    // "SSD1677 Gray4" is 13 chars at 10px = 130px, centered in the 480px width.
    Text::new("SSD1677 Gray4", Point::new(175, 40), title_style)
        .draw(page)
        .unwrap();
    // "GDEQ0426T82 800x480" is 20 chars at 6px = 120px.
    Text::new("GDEQ0426T82 800x480", Point::new(180, 62), dark_style)
        .draw(page)
        .unwrap();
    // "epdsi async Gray4" is 17 chars at 6px = 102px.
    Text::new("epdsi async Gray4", Point::new(189, 84), light_style)
        .draw(page)
        .unwrap();

    // Four equal bands spanning the full 480px width, one per gray level.
    const BAR_Y: i32 = 140;
    const BAR_H: u32 = 100;
    const BAR_W: u32 = VIEW_W / 4;

    let bands = [
        (0u32, Gray4Color::Black, "Black", white_small_style),
        (1u32, Gray4Color::Dark, "Dark", white_small_style),
        (2u32, Gray4Color::Light, "Light", black_small_style),
        (3u32, Gray4Color::White, "White", black_small_style),
    ];
    for (index, fill, label, label_style) in bands {
        let x = (index * BAR_W) as i32;
        Rectangle::new(Point::new(x, BAR_Y), Size::new(BAR_W, BAR_H))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
        Text::new(label, Point::new(x + 8, BAR_Y + 56), label_style)
            .draw(page)
            .unwrap();
    }
    // Outline around the White band so its edge is visible against the page background.
    Rectangle::new(Point::new(3 * BAR_W as i32, BAR_Y), Size::new(BAR_W, BAR_H))
        .into_styled(PrimitiveStyle::with_stroke(Gray4Color::Black, 1))
        .draw(page)
        .unwrap();
}

/// Screen 2: four concentric rounded rectangles alternating gray levels, with a centered label.
fn draw_geometric(page: &mut GrayBufferPair) {
    let stroke = PrimitiveStyle::with_stroke(Gray4Color::Black, 1);
    Rectangle::new(Point::new(0, 0), Size::new(VIEW_W, VIEW_H))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    // (padding, corner radius, fill) for each nested ring, outer to inner.
    let rings: [(u32, u32, Gray4Color); 4] = [
        (20, 24, Gray4Color::Light),
        (60, 18, Gray4Color::Dark),
        (100, 12, Gray4Color::Black),
        (140, 12, Gray4Color::White),
    ];
    for (pad, radius, fill) in rings {
        let rect = Rectangle::new(
            Point::new(pad as i32, pad as i32),
            Size::new(VIEW_W - 2 * pad, VIEW_H - 2 * pad),
        );
        RoundedRectangle::with_equal_corners(rect, Size::new(radius, radius))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
    }

    // Centered "4-Level Gray" label inside the innermost White ring.
    let inner_pad = 140i32;
    let inner_width = VIEW_W as i32 - 2 * inner_pad;
    let label_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let label = "4-Level Gray";
    let label_width = label.len() as i32 * 6;
    Text::new(
        label,
        Point::new(
            inner_pad + (inner_width - label_width) / 2,
            VIEW_H as i32 / 2,
        ),
        label_style,
    )
    .draw(page)
    .unwrap();
}

/// Preclear pass: writes `data` to *both* the Black/White and Red/Yellow channels, so the
/// baseline OTP-LUT refresh (`Ssd1677RefreshMode::Gray4Preclear`) matches the final image's
/// black/white split.
async fn write_preclear<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, data: &[u8])
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, data)
        .await
        .unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, data)
        .await
        .unwrap();
}

/// Final pass: writes the real Black/White (LSB) and Red/Yellow (MSB) planes.
async fn write_real_planes<BUS, C, P>(
    epd: &mut EpdDriver<BUS, C, P>,
    plane_a: &[u8],
    plane_b: &[u8],
) where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, plane_a)
        .await
        .unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, plane_b)
        .await
        .unwrap();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!(
        "Starting GDEQ0426T82 4.26\" 4-Level Grayscale (Gray4) EPD example (epdsi SSD1677, async/Embassy)"
    );

    let p = embassy_rp::init(Default::default());

    let dc = Output::new(p.PIN_12, Level::Low);
    let rst = Output::new(p.PIN_11, Level::High);
    // SSD1677 BUSY is active-HIGH
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

    // `for_panel` picks up GDEQ0426T82's dimensions; `.with_gray4` layers on the Adafruit_EPD-
    // sourced register bundle. Start on `Gray4Preclear` — the first pass of every screen below.
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1677Controller::for_panel::<GDEQ0426T82>()
        .with_gray4(GDEQ0426T82::GRAY4)
        .with_refresh_mode(Ssd1677RefreshMode::Gray4Preclear);
    let mut epd = EpdBuilder::<_, GDEQ0426T82>::new(controller).build(epd_bus);
    let mut delay = Delay;

    info!("Initializing SSD1677 epdsi EPD driver (Gray4, async)...");
    epd.init(&mut delay).await.unwrap();

    // Two 48,000-byte bit-plane buffers, held as plain stack arrays — the RP2350's 512 KB of SRAM
    // absorbs both comfortably.
    let mut plane_a_buf = [0u8; FRAME_BYTES];
    let mut plane_b_buf = [0u8; FRAME_BYTES];

    info!("--- Screen 1: Banner & 4-Level Swatch ---");
    {
        let mut page = GrayBufferPair::new(
            &mut plane_a_buf,
            &mut plane_b_buf,
            GDEQ0426T82::WIDTH,
            GDEQ0426T82::HEIGHT,
            0,
            POLARITY,
        );
        page.set_rotation(DisplayRotation::Rotate270);
        page.clear();
        draw_banner(&mut page);

        info!("Preclear pass (mono baseline, expect several seconds)...");
        write_preclear(&mut epd, page.plane_a().as_slice()).await;
        epd.refresh(&mut delay).await.unwrap();

        // The preclear refresh's OTP LUT load overwrote the custom LUT/voltage registers
        // uploaded during init — reload them before the real Gray4 refresh.
        {
            let (bus, controller) = epd.split_mut();
            controller.reload_gray4_lut(bus).await.unwrap();
        }
        epd.controller_mut()
            .set_refresh_mode(Ssd1677RefreshMode::Gray4);

        info!("Gray4 pass (real image, expect several seconds)...");
        write_real_planes(
            &mut epd,
            page.plane_a().as_slice(),
            page.plane_b().as_slice(),
        )
        .await;
        epd.refresh(&mut delay).await.unwrap();
    }

    Timer::after_millis(8000).await;

    info!("--- Screen 2: Concentric Geometric Grayscale Test Pattern ---");
    epd.controller_mut()
        .set_refresh_mode(Ssd1677RefreshMode::Gray4Preclear);
    {
        let mut page = GrayBufferPair::new(
            &mut plane_a_buf,
            &mut plane_b_buf,
            GDEQ0426T82::WIDTH,
            GDEQ0426T82::HEIGHT,
            0,
            POLARITY,
        );
        page.set_rotation(DisplayRotation::Rotate270);
        page.clear();
        draw_geometric(&mut page);

        info!("Preclear pass (mono baseline, expect several seconds)...");
        write_preclear(&mut epd, page.plane_a().as_slice()).await;
        epd.refresh(&mut delay).await.unwrap();

        {
            let (bus, controller) = epd.split_mut();
            controller.reload_gray4_lut(bus).await.unwrap();
        }
        epd.controller_mut()
            .set_refresh_mode(Ssd1677RefreshMode::Gray4);

        info!("Gray4 pass (real image, expect several seconds)...");
        write_real_planes(
            &mut epd,
            page.plane_a().as_slice(),
            page.plane_b().as_slice(),
        )
        .await;
        epd.refresh(&mut delay).await.unwrap();
    }

    info!("Display complete!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_ssd1677_gdeq0426t82_gray4"),
    hal::binary_info::rp_program_description!(
        c"epdsi async SSD1677/GDEQ0426T82 Gray4 example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
