//! # Pervasive Displays E2266KS0C1 E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `pdi_e2266ks0c1` example — full parity: same
//! panel, same wiring, same two-phase content (normal full refresh, then a 5-iteration fast
//! differential loop), built against `epdsi`'s async API
//! (`default-features = false, features = ["graphics"]`) over `embassy-rp`'s async SPI and GPIO
//! instead of the blocking `rp235x-hal` API. Every `epdsi` call below is `.await`ed, including
//! `write_fast_frame` (a `PervasiveBwController` inherent method, not part of the `EpdController`
//! trait, but dual-mode the same way); nothing else about the driver-level code differs from the
//! blocking example.
//!
//! Unlike the BWRY panels (`epdsi_pdi_e2154qs0f1`), this monochrome Pervasive panel's
//! `PervasiveBwController` needs no bit-banged OTP handshake, so the SPI/GPIO setup here is as
//! plain as the SSD16xx/UC8253 examples.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Pervasive Displays E2266KS0C1 2.66" Monochrome E-Paper Display (EPDK)
//!
//! ### Hardware Note for EXT3-1 Extension Boards:
//! - **J3 Jumper Setting**: Ensure the J3 jumper is OPEN (selecting the 10 µH inductor path for panels <= 3.7", e.g. 2.66" E2266KS0C1).
//!   - If J3 is closed (47 µH path for large screens), the DC-DC booster chokes during current bursts,
//!     causing voltage sags and busy-wait hangs.
//!
//! ## Wiring (EXT3/EPD connection)
//!
//! Connection using the **10-way rainbow bridging cable** provided with the EPDK.
//!
//! | Pico Pin       | Cable Color | EXT3 Pin / Function  |
//! |----------------|-------------|----------------------|
//! | 3V3 (Pin 36)   | **Black**   | 1 / VCC              |
//! | GPIO18 (Pin 24)| **Brown**   | 2 / SCK (SPI Clock)  |
//! | GPIO13 (Pin 17)| **Red**     | 3 / BUSY             |
//! | GPIO12 (Pin 16)| **Orange**  | 4 / DC (Data/Cmd)    |
//! | GPIO11 (Pin 15)| **Yellow**  | 5 / RST (Reset)      |
//! | GPIO16 (Pin 21)| **Green**   | 6 / MISO             |
//! | GPIO19 (Pin 25)| **Blue**    | 7 / MOSI             |
//! | NC             | **Violet**  | 8 / FCSM (Flash CS)  |
//! | GPIO17 (Pin 22)| **Grey**    | 9 / ECSM (Display CS)|
//! | GND (Pin 38)   | **White**   | 10 / GND             |
//!
//! ## Run
//!
//! ```bash
//! cargo run --example epdsi_pdi_e2266ks0c1
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
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle, Triangle};
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

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Program start (epdsi PervasiveBwController, async/Embassy)");

    let p = embassy_rp::init(Default::default());
    let mut delay = Delay;

    // Hardware Note for EXT3-1 extension boards: J3 jumper must be OPEN (10 µH path) for panels
    // <= 3.7" such as this 2.66" E2266KS0C1 — see the module doc.

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

    let bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = PervasiveBwController::new(E2266KS0C1::WIDTH, E2266KS0C1::HEIGHT);
    let mut driver = EpdBuilder::<_, E2266KS0C1>::new(controller).build(bus);

    info!("Initializing E2266KS0C1 display via epdsi driver...");
    driver.init(&mut delay).await.unwrap();
    info!("E-Paper display initialized");

    // Clear Red frame buffer (DTM2) to 0x00 (no red pixels, preventing controller RAM noise)
    driver.clear_frame(ColorChannel::RedYellow, 0x00).await.unwrap();

    // Full frame buffer (152 width x 296 height / 8 = 5,624 bytes RAM)
    let mut buffer = [0u8; (E2266KS0C1::WIDTH as usize * E2266KS0C1::HEIGHT as usize) / 8];
    let mut prev_buffer = [0u8; (E2266KS0C1::WIDTH as usize * E2266KS0C1::HEIGHT as usize) / 8];
    let mut display = PageBuffer::new(&mut buffer, E2266KS0C1::WIDTH, E2266KS0C1::HEIGHT, 0);

    display.clear_byte(0xFF);

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    info!("--- Phase 1: Normal Full Refresh ---");
    info!("Drawing initial shapes and text onto frame buffer...");

    Text::new("Hello World", Point::new(10, 15), text_style)
        .draw(&mut display)
        .unwrap();

    Line::new(Point::new(10, 25), Point::new(140, 25))
        .into_styled(style)
        .draw(&mut display)
        .unwrap();

    Rectangle::new(Point::new(10, 35), Size::new(40, 40))
        .into_styled(style)
        .draw(&mut display)
        .unwrap();

    Circle::new(Point::new(60, 35), 40)
        .into_styled(style)
        .draw(&mut display)
        .unwrap();

    Triangle::new(
        Point::new(110, 75),
        Point::new(130, 35),
        Point::new(150, 75),
    )
    .into_styled(style)
    .draw(&mut display)
    .unwrap();

    let offset = Point::new(10, 85);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + offset, BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    let offset = Point::new(80, 85);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + offset, BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    Text::new("RP2350", Point::new(10, 170), text_style)
        .draw(&mut display)
        .unwrap();

    Text::new("epdsi async", Point::new(10, 195), text_style)
        .draw(&mut display)
        .unwrap();

    info!("Sending frame buffer data to display...");
    driver
        .write_frame(ColorChannel::BlackWhite, display.as_slice())
        .await
        .unwrap();

    info!("Refreshing display hardware (Normal full refresh)...");
    driver.refresh(&mut delay).await.unwrap();

    prev_buffer.copy_from_slice(display.as_slice());
    Timer::after_millis(2000).await;

    info!("--- Phase 2: Fast Differential Refresh ---");
    info!("Switching PervasiveBwController to Fast refresh mode...");
    driver
        .controller_mut()
        .set_refresh_mode(PervasiveRefreshMode::Fast);
    driver.init(&mut delay).await.unwrap();

    for count in 1..=5 {
        display.clear_byte(0xFF);

        Text::new("Fast Refresh", Point::new(10, 15), text_style)
            .draw(&mut display)
            .unwrap();

        Line::new(Point::new(10, 25), Point::new(140, 25))
            .into_styled(style)
            .draw(&mut display)
            .unwrap();

        let mut count_buf = [0u8; 32];
        let count_str =
            format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
        Text::new(count_str, Point::new(10, 42), text_style)
            .draw(&mut display)
            .unwrap();

        let bar_width = count * 25;
        Rectangle::new(Point::new(10, 65), Size::new(bar_width, 10))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut display)
            .unwrap();

        let (ferris_pos, rust_pos) = if count % 2 == 1 {
            (Point::new(80, 85), Point::new(10, 85))
        } else {
            (Point::new(10, 85), Point::new(80, 85))
        };

        for pixel in ferris_bmp.pixels() {
            if pixel.1 == BinaryColor::Off {
                Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                    .draw(&mut display)
                    .unwrap();
            }
        }

        for pixel in rust_bmp.pixels() {
            if pixel.1 == BinaryColor::On {
                Pixel(pixel.0 + rust_pos, BinaryColor::On)
                    .draw(&mut display)
                    .unwrap();
            }
        }

        Text::new("RP2350", Point::new(10, 170), text_style)
            .draw(&mut display)
            .unwrap();

        Text::new("epdsi fast mode", Point::new(10, 195), text_style)
            .draw(&mut display)
            .unwrap();

        let (bus, controller) = driver.split_mut();
        controller
            .write_fast_frame(bus, &prev_buffer, display.as_slice())
            .await
            .unwrap();

        info!("Refreshing display in fast mode...");
        driver.refresh(&mut delay).await.unwrap();

        prev_buffer.copy_from_slice(display.as_slice());
        Timer::after_millis(1000).await;
    }

    info!("Powering off / sleeping display...");
    driver.sleep(&mut delay).await.unwrap();
    info!("Display complete!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_pdi_e2266ks0c1"),
    hal::binary_info::rp_program_description!(c"epdsi async Pervasive Bw/E2266KS0C1 example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
