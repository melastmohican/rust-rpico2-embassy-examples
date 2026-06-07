//! # HS3003 Temperature/Humidity Sensor Example (Embassy)
//!
//! Reads temperature and humidity from an HS3003 sensor over I2C0 using Embassy.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2
//! - **Sensor:** Arduino Modulino Thermo (Renesas HS3003)
//!
//! ## Wiring with Qwiic/STEMMA QT
//!
//! Simply connect the Qwiic/STEMMA QT cable between the board and the Modulino Thermo.
//! The cable provides:
//! ```
//!      Modulino -> RPi Pico 2
//! (black)  GND  -> GND
//! (red)    VCC  -> 3.3V
//! (yellow) SCL  -> GPIO5 (Pin 7) (I2C0 SCL)
//! (blue)   SDA  -> GPIO4 (Pin 6) (I2C0 SDA)
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run --example hs3003_i2c
//! ```
//!
//! ## About HS3003
//!
//! The Renesas HS3003 is a high-performance temperature and humidity sensor:
//! - Temperature range: -40°C to +125°C (±0.2°C accuracy)
//! - Humidity range: 0% to 100% RH (±1.5% accuracy)
//! - 14-bit resolution for both measurements
//! - Ultra-low power consumption

#![no_std]
#![no_main]

extern crate alloc;
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::bind_interrupts;
use embassy_rp::i2c::{Config, I2c, InterruptHandler};
use embassy_rp::peripherals::I2C0;
use embassy_time::{Duration, Timer};
use hs3003::Hs3003;
use panic_probe as _;

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    I2C0_IRQ => InterruptHandler<I2C0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("HS3003 Async Sensor Example for RP2350 (Embassy)");

    let p = embassy_rp::init(Default::default());

    // Configure I2C0 (GP4 = SDA, GP5 = SCL)
    let sda = p.PIN_4;
    let scl = p.PIN_5;
    let i2c = I2c::new_async(p.I2C0, scl, sda, Irqs, Config::default());

    // Create sensor instance
    let mut sensor = Hs3003::new(i2c);
    let mut delay = embassy_time::Delay;

    info!("Starting measurements...");

    loop {
        match sensor.read_async(&mut delay).await {
            Ok(measurement) => {
                info!(
                    "Temperature: {}°C, Humidity: {}%",
                    measurement.temperature, measurement.humidity
                );
            }
            Err(_) => {
                error!("Failed to read sensor");
            }
        }
        Timer::after(Duration::from_secs(2)).await;
    }
}
