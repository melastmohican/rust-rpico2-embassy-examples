//! # Zermatt Image falling Snow Example
//!
//! Display a 320x240 image of Zermatt on the Adafruit 2.2" TFT LCD display with animated falling snow.
//!
//! This example demonstrates displaying a full-screen image in landscape mode with a physics-based
//! snow effect, using an asynchronous drawing pipeline and `lcd-async`.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2
//! - **Display:** Adafruit 2.2" 18-bit color TFT LCD display (Product 1480)
//! - **Breakout:** Adafruit Eye-SPI
//!
//! ## Wiring for Eye-SPI Breakout
//!
//! ```
//!      Raspberry Pi Pico 2              Eye-SPI Breakout
//!    +-----------------------+      +---------------------------+
//!    |                       |      |                           |
//!    |  3V3 (Pin 36) --------+------+-> VIN   (Red Wire)        |
//!    |  GND (Pin 38) --------+------+-> GND   (Black Wire)      |
//!    |  GPIO18 (Pin 24) -----+------+-> SCK   (Blue Wire)       |
//!    |  GPIO19 (Pin 25) -----+------+-> MOSI  (Green Wire)      |
//!    |  GPIO16 (Pin 21) -----+------+-> MISO  (Yellow Wire)     |
//!    |  GPIO20 (Pin 26) -----+------+-> DC    (White Wire)      |
//!    |  GPIO21 (Pin 27) -----+------+-> RST   (Orange Wire)     |
//!    |  GPIO17 (Pin 22) -----+------+-> TCS   (Blue Wire)       |
//!    |                       |      |                           |
//!    +-----------------------+      +---------------------------+
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run --example zermatt_snow
//! ```

#![no_std]
#![no_main]

extern crate alloc;
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

use core::cell::RefCell;
use cortex_m::interrupt::Mutex;

use defmt_rtt as _;
use panic_probe as _;

use defmt::info;
use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::spi::{Config, Spi};
use embedded_graphics::{
    Drawable,
    draw_target::DrawTarget,
    geometry::Point,
    geometry::Size,
    image::{GetPixel, Image},
    pixelcolor::{Rgb565, RgbColor},
    primitives::Rectangle,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use lcd_async::{
    Builder,
    interface::SpiInterface,
    models::ILI9341Rgb565,
    options::{ColorOrder, Orientation, Rotation},
    raw_framebuf::RawFrameBuf,
};
use tinybmp::Bmp;

use embassy_rp::bind_interrupts;
use hal::block::ImageDef;

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: ImageDef = hal::block::ImageDef::secure_exe();

// Display dimensions in landscape mode
const DISPLAY_WIDTH: usize = 320;
const DISPLAY_HEIGHT: usize = 240;

// Physics engine grid size
const PHY_DISP_RATIO: usize = 2; // Physical cell size in pixels
const PHY_WIDTH: usize = DISPLAY_WIDTH / PHY_DISP_RATIO;
const PHY_HEIGHT: usize = DISPLAY_HEIGHT / PHY_DISP_RATIO;

// Grid storage (1 bit per cell)
const BITS_PER_CELL: usize = 1;
const CELLS_PER_BYTE: usize = 8 / BITS_PER_CELL;
const GRID_TOTAL_CELLS: usize = PHY_WIDTH * PHY_HEIGHT;
const GRID_SIZE_BYTES: usize = GRID_TOTAL_CELLS / CELLS_PER_BYTE;

const FLAKE_SIZE: i32 = 2; // Small 2x2 pixel snowflakes
const SNOW_COLOR: Rgb565 = Rgb565::WHITE;

// Allocate frame buffer statically to avoid blowing up the stack
static mut FRAME_BUFFER: [u8; DISPLAY_WIDTH * DISPLAY_HEIGHT * 2] =
    [0; DISPLAY_WIDTH * DISPLAY_HEIGHT * 2];

// Simple PRNG state
static RNG_STATE: Mutex<RefCell<u32>> = Mutex::new(RefCell::new(12345));

// Simple pseudo-random number generator
fn random_range(min: i32, max: i32) -> i32 {
    cortex_m::interrupt::free(|cs| {
        let mut state = RNG_STATE.borrow(cs).borrow_mut();
        // Linear congruential generator
        *state = state.wrapping_mul(1103515245).wrapping_add(12345);
        let random = (*state / 65536) % 32768;
        min + (random as i32 % (max - min))
    })
}

struct SnowGrid {
    grid: [u8; GRID_SIZE_BYTES],
}

impl SnowGrid {
    fn new() -> Self {
        Self {
            grid: [0u8; GRID_SIZE_BYTES],
        }
    }

    fn get_cell(&self, row: usize, col: usize) -> bool {
        let cell_index = row * PHY_WIDTH + col;
        let byte_index = cell_index / CELLS_PER_BYTE;
        let bit_index = cell_index % CELLS_PER_BYTE;
        (self.grid[byte_index] >> bit_index) & 1 == 1
    }

    fn set_cell(&mut self, row: usize, col: usize, value: bool) {
        let cell_index = row * PHY_WIDTH + col;
        let byte_index = cell_index / CELLS_PER_BYTE;
        let bit_index = cell_index % CELLS_PER_BYTE;

        if value {
            self.grid[byte_index] |= 1 << bit_index;
        } else {
            self.grid[byte_index] &= !(1 << bit_index);
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    info!("Initializing Adafruit 2.2\" TFT LCD display...");

    // Control pins
    let cs = Output::new(p.PIN_17, Level::High); // TCS - Chip Select
    let dc = Output::new(p.PIN_20, Level::Low); // DC - Data/Command
    let rst = Output::new(p.PIN_21, Level::Low); // RST - Reset

    // Configure SPI (SPI0: SCK=18, MOSI=19, MISO=16)
    let mut config = Config::default();
    config.frequency = 40_000_000;

    // Async SPI using DMA
    let spi = Spi::new(
        p.SPI0, p.PIN_18, // clk
        p.PIN_19, // mosi
        p.PIN_16, // miso
        p.DMA_CH0, p.DMA_CH1, Irqs, config,
    );

    info!("SPI configured at 40 MHz");

    // Create exclusive SPI device with CS pin
    let spi_device = ExclusiveDevice::new(spi, cs, embassy_time::Delay).unwrap();

    // Create display interface
    let di = SpiInterface::new(spi_device, dc);

    info!("Initializing display in landscape mode...");

    let mut delay = embassy_time::Delay;

    // Create and initialize display
    let mut display = Builder::new(ILI9341Rgb565, di)
        .reset_pin(rst)
        .display_size(240, 320) // Physical dimensions
        .orientation(Orientation::new().rotate(Rotation::Deg90).flip_horizontal())
        .color_order(ColorOrder::Bgr)
        .init(&mut delay)
        .await
        .unwrap();

    info!("Display initialized in landscape mode (320x240)!");

    info!("Loading Zermatt image (320x240 BMP)...");

    // Load the BMP image data using tinybmp
    let bmp_data = include_bytes!("zermatt_320x240.bmp");
    let bmp = Bmp::<Rgb565>::from_slice(bmp_data).expect("Failed to load BMP image");

    let frame_buffer = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUFFER) };
    let mut fbuf = RawFrameBuf::<Rgb565, _>::new(&mut frame_buffer[..], 320, 240);
    fbuf.clear(Rgb565::BLACK).unwrap();

    info!("Drawing Zermatt image...");
    let image = Image::new(&bmp, Point::new(0, 0));
    image.draw(&mut fbuf).unwrap();

    info!("Sending initial framebuffer to display...");
    display
        .show_raw_data(0, 0, 320, 240, fbuf.as_bytes())
        .await
        .unwrap();

    info!("Image displayed! Starting snow animation...");

    // Initialize snow grid
    let mut snow_grid = SnowGrid::new();

    let mut frame_count = 0u32;

    // Main animation loop
    loop {
        // Simulate falling snow (iterate from bottom to top)
        for row in (0..PHY_HEIGHT - 1).rev() {
            for col in 0..PHY_WIDTH {
                if snow_grid.get_cell(row, col) {
                    // Calculate future column with slight randomness
                    let offset = random_range(-1, 2); // -1, 0, or 1
                    let future_col =
                        (col as i32 + offset).max(0).min(PHY_WIDTH as i32 - 1) as usize;

                    // Check if future cell is empty
                    if !snow_grid.get_cell(row + 1, future_col) {
                        // Move snowflake down
                        snow_grid.set_cell(row + 1, future_col, true);
                        render_flake(&mut fbuf, row + 1, future_col);
                    }

                    // Clear current position
                    snow_grid.set_cell(row, col, false);
                    render_void(&mut fbuf, bmp_data, row, col);
                }
            }
        }

        // Clear snowflakes that reached the bottom
        for col in 0..PHY_WIDTH {
            if snow_grid.get_cell(PHY_HEIGHT - 1, col) {
                snow_grid.set_cell(PHY_HEIGHT - 1, col, false);
                render_void(&mut fbuf, bmp_data, PHY_HEIGHT - 1, col);
            }
        }

        // Create new snow at the top
        for col in 0..PHY_WIDTH {
            if random_range(0, 25) < 1 {
                snow_grid.set_cell(0, col, true);
                render_flake(&mut fbuf, 0, col);
            }
        }

        // Flush the framebuffer to the display over DMA SPI
        display
            .show_raw_data(0, 0, 320, 240, fbuf.as_bytes())
            .await
            .unwrap();

        // Delay between frames
        embassy_time::Timer::after_millis(20).await;

        frame_count += 1;
        if frame_count.is_multiple_of(50) {
            info!("Frame: {}", frame_count);
        }
    }
}

// Render a snowflake at the given grid position
fn render_flake(target: &mut impl DrawTarget<Color = Rgb565>, row: usize, col: usize) {
    let x = (col * PHY_DISP_RATIO) as i32;
    let y = (row * PHY_DISP_RATIO) as i32;

    let rect = Rectangle::new(
        Point::new(x, y),
        Size::new(FLAKE_SIZE as u32, FLAKE_SIZE as u32),
    );
    target.fill_solid(&rect, SNOW_COLOR).ok();
}

// Restore the background image at the given grid position
fn render_void(
    target: &mut impl DrawTarget<Color = Rgb565>,
    bmp_data: &[u8],
    row: usize,
    col: usize,
) {
    let x = (col * PHY_DISP_RATIO) as i32;
    let y = (row * PHY_DISP_RATIO) as i32;

    if let Ok(bmp) = Bmp::<Rgb565>::from_slice(bmp_data) {
        let mut colors = [Rgb565::BLACK; 4];
        let mut idx = 0;

        for dy in 0..FLAKE_SIZE {
            for dx in 0..FLAKE_SIZE {
                let px = x + dx;
                let py = y + dy;
                colors[idx] = bmp.pixel(Point::new(px, py)).unwrap_or(Rgb565::BLACK);
                idx += 1;
            }
        }

        let rect = Rectangle::new(
            Point::new(x, y),
            Size::new(FLAKE_SIZE as u32, FLAKE_SIZE as u32),
        );
        target.fill_contiguous(&rect, colors).ok();
    }
}
