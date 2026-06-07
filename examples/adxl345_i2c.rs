//! # ADXL345 3-Axis Accelerometer Example
//!
//! Reads accelerometer data from an ADXL345 sensor over I2C0 using Embassy.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2
//! - **Sensor:** ADXL345 3-Axis Digital Accelerometer
//!
//! ## Wiring
//!
//! ```
//!      ADXL345 -> RPi Pico 2
//! (black)  GND -> GND
//! (red)    VCC -> 3.3V
//! (yellow) SCL -> GPIO5 (Pin 7)
//! (blue)   SDA -> GPIO4 (Pin 6)
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run --example adxl345_i2c
//! ```

#![no_std]
#![no_main]

extern crate alloc;
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

use adxl345_eh_driver::{Driver as Adxl345, GRange, OutputDataRate};
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::bind_interrupts;
use embassy_rp::i2c::{Config, I2c, InterruptHandler};
use embassy_rp::peripherals::I2C0;
use embassy_time::Timer;
use panic_probe as _;

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    I2C0_IRQ => InterruptHandler<I2C0>;
});

const fn lsb_to_g(range: GRange) -> f32 {
    match range {
        GRange::Two => 0.004,
        GRange::Four => 0.008,
        GRange::Eight => 0.016,
        GRange::Sixteen => 0.031,
    }
}

fn magnitude(x: f32, y: f32, z: f32) -> f32 {
    let sum = x * x + y * y + z * z;
    libm::sqrtf(sum)
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Initializing ADXL345 Example (Embassy)...");

    let p = embassy_rp::init(Default::default());

    // Configure I2C0 (GP4 = SDA, GP5 = SCL)
    let sda = p.PIN_4;
    let scl = p.PIN_5;

    let mut config = Config::default();
    config.frequency = 400_000;

    let i2c = I2c::new_async(p.I2C0, scl, sda, Irqs, config);

    info!("Initializing ADXL345...");

    // Create a new ADXL345 driver instance
    // For I2C, use the secondary address 0x53 (common for breakouts)
    let mut accel = match Adxl345::new(i2c, Some(adxl345_eh_driver::address::SECONDARY)) {
        Ok(accel) => {
            info!("ADXL345 initialized successfully!");
            accel
        }
        Err(e) => {
            error!("Error initializing ADXL345: {:?}", defmt::Debug2Format(&e));
            loop {
                Timer::after_millis(1000).await;
            }
        }
    };

    // Configure sensor
    accel.set_range(GRange::Two).unwrap();
    accel.set_datarate(OutputDataRate::Hz100).unwrap();

    info!("Starting measurements...");

    let scale = lsb_to_g(GRange::Two);

    loop {
        // Read accelerometer data
        match accel.get_accel_raw() {
            Ok((x, y, z)) => {
                let ax = x as f32 * scale;
                let ay = y as f32 * scale;
                let az = z as f32 * scale;
                let mag = magnitude(ax, ay, az);

                // Round to 3 decimal places for cleaner defmt output
                let round3 = |val: f32| libm::roundf(val * 1000.0) / 1000.0;
                let ax_r = round3(ax);
                let ay_r = round3(ay);
                let az_r = round3(az);
                let mag_r = round3(mag);

                info!(
                    "x: {=f32}G, y: {=f32}G, z: {=f32}G, |g|: {=f32}G",
                    ax_r, ay_r, az_r, mag_r
                );
            }
            Err(e) => {
                error!(
                    "Error reading accelerometer data: {:?}",
                    defmt::Debug2Format(&e)
                );
            }
        }

        // Wait between measurements
        Timer::after_millis(50).await;
    }
}
