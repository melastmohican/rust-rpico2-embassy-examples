//! # GDEY0266Z90 2.66" Tri-Color E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `ssd1680_gdey0266z90_epd` example — full
//! parity: same panel, same wiring, same four-phase content and driver call sequence
//! (`Full`/windowed-`Full`/`FastFull`/`BaseMap`+`Partial`), built against `epdsi`'s async API
//! (`default-features = false, features = ["graphics"]`) over `embassy-rp`'s async SPI and GPIO
//! instead of the blocking `rp235x-hal` API. Every `epdsi` call below is `.await`ed; nothing else
//! about the driver-level code differs from the blocking example.
//!
//! See the blocking example's module doc for the full detail on refresh timing, duty cycle,
//! ink polarity (the Red plane is inverted: `0x00` is no red, red content draws as
//! [`BinaryColor::Off`]), and why Phase 4 must not copy the monochrome differential idiom onto a
//! colour panel — none of that changes here, only the transport does.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Good Display GDEY0266Z90 / Waveshare 2.66inch e-Paper Module (B), 152x296 BWR
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
//! Power-cycle the board first, then run once and let it finish — an interrupted run can leave
//! the controller latched busy, and the *next* run then looks broken for reasons that are not in
//! the code.
//!
//! ```bash
//! cargo run --example epdsi_ssd1680_gdey0266z90
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

/// Empty fill for the Red plane over the status band. `0x00`, not `0xFF` — see the blocking
/// example's ink-polarity note.
static NO_RED_BAND: [u8; BAND_BYTES] = [0x00u8; BAND_BYTES];

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
fn draw_static_content(
    bw: &mut PageBuffer,
    red: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    mode_label: &str,
) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT),
    )
    .into_styled(stroke)
    .draw(bw)
    .unwrap();

    Text::new("GDEY0266Z90", Point::new(8, 22), text_style)
        .draw(bw)
        .unwrap();

    Text::new("Tri-Color ", Point::new(8, 40), small_text_style)
        .draw(bw)
        .unwrap();
    Text::new(
        "BWR",
        Point::new(68, 40),
        MonoTextStyle::new(&FONT_6X10, BinaryColor::Off),
    )
    .draw(red)
    .unwrap();

    Line::new(Point::new(8, 48), Point::new(143, 48))
        .into_styled(stroke)
        .draw(bw)
        .unwrap();

    Rectangle::new(Point::new(8, 56), Size::new(136, 18))
        .into_styled(stroke)
        .draw(bw)
        .unwrap();
    Rectangle::new(Point::new(10, 58), Size::new(64, 14))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(bw)
        .unwrap();
    Rectangle::new(Point::new(78, 58), Size::new(64, 14))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
        .draw(red)
        .unwrap();

    let ferris_pos = Point::new(10, 92);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::Off)
                .draw(red)
                .unwrap();
        }
    }

    let rust_pos = Point::new(78, 82);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On).draw(bw).unwrap();
        }
    }

    Text::new("RP2350 Pico 2", Point::new(8, 170), small_text_style)
        .draw(bw)
        .unwrap();
    Text::new("epdsi async", Point::new(8, 184), small_text_style)
        .draw(bw)
        .unwrap();
    Text::new(mode_label, Point::new(8, 198), small_text_style)
        .draw(bw)
        .unwrap();

    Line::new(Point::new(8, 210), Point::new(143, 210))
        .into_styled(stroke)
        .draw(bw)
        .unwrap();
}

/// Writes both colour planes for the full frame, resetting the RAM window and cursor first.
async fn write_full_frame<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, bw: &[u8], red: &[u8])
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEY0266Z90::WIDTH - 1, GDEY0266Z90::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, bw).await.unwrap();

    epd.set_window(0, 0, GDEY0266Z90::WIDTH - 1, GDEY0266Z90::HEIGHT - 1)
        .await
        .unwrap();
    epd.set_cursor(0, 0).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, red).await.unwrap();
}

/// Draws the Black/White half of the status band: label, counter and progress bar outline.
fn draw_band(band: &mut PageBuffer, count: u32, label: &str) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    Text::new(label, Point::new(8, BAND_Y as i32 + 14), small_text_style)
        .draw(band)
        .unwrap();

    let mut count_buf = [0u8; 32];
    let count_str = format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
    Text::new(
        count_str,
        Point::new(8, BAND_Y as i32 + 28),
        small_text_style,
    )
    .draw(band)
    .unwrap();

    Rectangle::new(Point::new(8, BAND_Y as i32 + 38), Size::new(136, 16))
        .into_styled(stroke)
        .draw(band)
        .unwrap();
}

/// Draws the progress bar fill for `count` into `plane`.
fn draw_band_bar(plane: &mut PageBuffer, count: u32, color: BinaryColor) {
    Rectangle::new(
        Point::new(10, BAND_Y as i32 + 40),
        Size::new(count * 33, 12),
    )
    .into_styled(PrimitiveStyle::with_fill(color))
    .draw(plane)
    .unwrap();
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting GDEY0266Z90 2.66\" Tri-Color EPD example (epdsi SSD1680, async/Embassy)");

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

    epd.clear_frame(ColorChannel::BlackWhite, 0xFF)
        .await
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0x00)
        .await
        .unwrap();

    let mut bw_buf = [0xFFu8; FRAME_BYTES];
    let mut red_buf = [0x00u8; FRAME_BYTES];

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    info!("--- Phase 1: Full Tri-Color Refresh ---");

    let full_ms = {
        let mut bw = PageBuffer::new(&mut bw_buf, GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT, 0);
        let mut red = PageBuffer::new(&mut red_buf, GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT, 0);

        draw_static_content(&mut bw, &mut red, &ferris_bmp, &rust_bmp, "mode: Full");

        info!("Sending both planes ({} bytes each)...", FRAME_BYTES);
        write_full_frame(&mut epd, bw.as_slice(), red.as_slice()).await;

        timed_refresh(&mut epd, &mut delay, "Phase 1 (Full)").await
    };

    Timer::after_millis(2000).await;

    info!("--- Phase 2: Windowed Refresh on the Full Waveform ---");

    for count in 1..=2u32 {
        {
            let mut band = PageBuffer::new(
                &mut bw_buf[..BAND_BYTES],
                GDEY0266Z90::WIDTH,
                BAND_H,
                BAND_Y,
            );
            band.clear_byte(0xFF);
            let mut band_red = PageBuffer::new(
                &mut red_buf[..BAND_BYTES],
                GDEY0266Z90::WIDTH,
                BAND_H,
                BAND_Y,
            );
            band_red.clear_byte(0x00);

            draw_band(&mut band, count, "Full window");
            draw_band_bar(&mut band_red, count, BinaryColor::Off);
        }

        epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..BAND_BYTES])
            .await
            .unwrap();
        epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, &red_buf[..BAND_BYTES])
            .await
            .unwrap();

        info!("Refreshing band y={}..{}...", BAND_Y, BAND_Y + BAND_H - 1);
        timed_refresh(&mut epd, &mut delay, "Phase 2 (windowed Full)").await;
        Timer::after_millis(1000).await;
    }

    info!("--- Phase 3: FastFull Full-Screen Refresh ---");

    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::FastFull);

    let fast_ms = {
        let mut bw = PageBuffer::new(&mut bw_buf, GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT, 0);
        bw.clear_byte(0xFF);
        let mut red = PageBuffer::new(&mut red_buf, GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT, 0);
        red.clear_byte(0x00);

        draw_static_content(&mut bw, &mut red, &ferris_bmp, &rust_bmp, "mode: FastFull");

        write_full_frame(&mut epd, bw.as_slice(), red.as_slice()).await;

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
        let mut band = PageBuffer::new(
            &mut bw_buf[..BAND_BYTES],
            GDEY0266Z90::WIDTH,
            BAND_H,
            BAND_Y,
        );
        band.clear_byte(0xFF);
        let mut band_red = PageBuffer::new(
            &mut red_buf[..BAND_BYTES],
            GDEY0266Z90::WIDTH,
            BAND_H,
            BAND_Y,
        );
        band_red.clear_byte(0x00);

        draw_band(&mut band, 0, "BaseMap");
        draw_band_bar(&mut band_red, 0, BinaryColor::Off);
    }

    epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
        .await
        .unwrap();
    epd.set_cursor(0, BAND_Y).await.unwrap();
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..BAND_BYTES])
        .await
        .unwrap();
    epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
        .await
        .unwrap();
    epd.set_cursor(0, BAND_Y).await.unwrap();
    epd.write_frame(ColorChannel::RedYellow, &NO_RED_BAND)
        .await
        .unwrap();

    timed_refresh(&mut epd, &mut delay, "Phase 4 (BaseMap)").await;

    Timer::after_millis(1000).await;

    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Partial);

    for count in 1..=2u32 {
        {
            let mut band = PageBuffer::new(
                &mut bw_buf[..BAND_BYTES],
                GDEY0266Z90::WIDTH,
                BAND_H,
                BAND_Y,
            );
            band.clear_byte(0xFF);
            let mut band_red = PageBuffer::new(
                &mut red_buf[..BAND_BYTES],
                GDEY0266Z90::WIDTH,
                BAND_H,
                BAND_Y,
            );
            band_red.clear_byte(0x00);

            draw_band(&mut band, count, "Partial mode");
            draw_band_bar(&mut band_red, count, BinaryColor::Off);
        }

        epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..BAND_BYTES])
            .await
            .unwrap();
        epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .await
            .unwrap();
        epd.set_cursor(0, BAND_Y).await.unwrap();
        epd.write_frame(ColorChannel::RedYellow, &red_buf[..BAND_BYTES])
            .await
            .unwrap();

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
    hal::binary_info::rp_program_name!(c"epdsi_ssd1680_gdey0266z90"),
    hal::binary_info::rp_program_description!(
        c"epdsi async SSD1680/GDEY0266Z90 example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
