//! # ST7735 Slint UI Example (Embassy)
//!
//! Slint UI framework running in `no_std` mode on Raspberry Pi Pico 2 (RP2350),
//! rendered using Slint's software renderer and flushed asynchronously to an 80x160 ST7735S LCD module via `display-driver`.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Waveshare 0.96" ST7735S LCD Module
//!
//! ## Wiring for Waveshare 0.96 inch LCD Module
//!
//! ```text
//!      Raspberry Pi Pico 2          Waveshare 0.96" ST7735S LCD
//!    +-----------------------+      +---------------------------+
//!    |                       |      |                           |
//!    |  3V3 (Pin 36) --------+------+-> VCC                     |
//!    |  GND (Pin 38) --------+------+-> GND                     |
//!    |  GPIO17 (Pin 22) -----+------+-> CS                      |
//!    |  GPIO21 (Pin 27) -----+------+-> RST                     |
//!    |  GPIO20 (Pin 26) -----+------+-> DC                      |
//!    |  GPIO19 (Pin 25) -----+------+-> DIN(MOSI)               |
//!    |  GPIO18 (Pin 24) -----+------+-> CLK(SCK)                |
//!    |  GPIO14 (Pin 19) -----+------+-> BL (Backlight)          |
//!    |                       |      |                           |
//!    +-----------------------+      +---------------------------+
//! ```
//!
//! ## About Slint UI
//!
//! [Slint](https://slint.dev/) is a modern, declarative GUI toolkit designed for embedded and desktop applications.
//! - **Website:** <https://slint.dev/>
//! - **Documentation:** <https://slint.dev/docs>
//! - **Software Renderer Docs:** <https://slint.dev/docs/rust/slint/platform/software_renderer/index.html>
//!
//! ## Run
//!
//! ```bash
//! cargo run --example st7735_slint
//! ```

#![no_std]
#![no_main]

extern crate alloc;
use alloc::boxed::Box;
use alloc::rc::Rc;
use embedded_alloc::LlffHeap as Heap;

const HEAP_SIZE: usize = 64 * 1024;
static mut HEAP_MEM: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: Heap = Heap::empty();

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;

use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::spi::{Config, Spi};
use embassy_time::{Delay, Instant, Timer};

use display_driver::{ColorFormat, DisplayDriver, Orientation, panel::reset::LCDResetOption};
use display_driver_spi::SpiDisplayBus;
use display_driver_st7735::{St7735, spec::PanelSpec, spec::generic::Generic80x160Type3};
use embedded_hal_bus::spi::ExclusiveDevice;

use slint::platform::software_renderer::{MinimalSoftwareWindow, Rgb565Pixel};

// Include generated Slint modules compiled with EmbedForSoftwareRenderer
slint::include_modules!();

/// Slint platform adapter for Embassy RP2350
struct McuPlatform {
    window: Rc<MinimalSoftwareWindow>,
    start_time: Instant,
}

impl slint::platform::Platform for McuPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> core::time::Duration {
        self.start_time.elapsed().into()
    }
}

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

const WIDTH: usize = Generic80x160Type3::PHYSICAL_HEIGHT as _;
const HEIGHT: usize = Generic80x160Type3::PHYSICAL_WIDTH as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("ST7735 Slint UI example starting...");

    // Initialize Heap Allocator for Slint
    unsafe { ALLOCATOR.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) };

    let p = embassy_rp::init(Default::default());

    // Hardware setup:
    // SCLK: GPIO18, MOSI: GPIO19, MISO: GPIO16
    // CS: GPIO17, DC: GPIO20, RST: GPIO21, BL: GPIO14
    let dc = Output::new(p.PIN_20, Level::Low);
    let rst = Output::new(p.PIN_21, Level::High);
    let mut bl = Output::new(p.PIN_14, Level::High);

    // Turn backlight on
    bl.set_high();

    // Setup SPI0 at 16 MHz
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

    let bus = SpiDisplayBus::new(spi_device, dc);
    let reset_opt = LCDResetOption::new_pin(rst);
    let panel = St7735::<Generic80x160Type3, _, _>::new(reset_opt);

    let mut driver = match DisplayDriver::builder(bus, panel)
        .with_color_format(ColorFormat::RGB565)
        .with_orientation(Orientation::Deg90)
        .init(&mut Delay)
        .await
    {
        Ok(driver) => driver,
        Err(e) => {
            error!("Display init failed: {:?}", Debug2Format(&e));
            return;
        }
    };

    info!("Display initialized! Setting up Slint Platform...");

    // Set up Slint Software Window
    let window = MinimalSoftwareWindow::new(
        slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
    );
    window.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));

    slint::platform::set_platform(Box::new(McuPlatform {
        window: window.clone(),
        start_time: Instant::now(),
    }))
    .unwrap();

    let app = AppWindow::new().unwrap();

    info!("Slint App Window created! Entering render loop...");

    static mut FRAME_BUFFER: [Rgb565Pixel; WIDTH * HEIGHT] = [Rgb565Pixel(0); WIDTH * HEIGHT];

    let mut counter = 0i32;

    loop {
        slint::platform::update_timers_and_animations();

        app.set_counter(counter);
        counter = counter.wrapping_add(1);

        let fb = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUFFER) };

        window.draw_if_needed(|renderer| {
            renderer.render(fb, WIDTH);
        });

        static mut SWAPPED_BUFFER: [u16; WIDTH * HEIGHT] = [0; WIDTH * HEIGHT];
        let swapped = unsafe { &mut *core::ptr::addr_of_mut!(SWAPPED_BUFFER) };
        for (i, p) in fb.iter().enumerate() {
            swapped[i] = p.0.swap_bytes();
        }

        let raw_bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(swapped.as_ptr() as *const u8, swapped.len() * 2)
        };

        if let Err(e) = driver.write_frame(raw_bytes).await {
            error!("Flush failed: {:?}", Debug2Format(&e));
        }

        Timer::after_millis(50).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"st7735_slint"),
    hal::binary_info::rp_program_description!(c"ST7735 Slint UI example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
