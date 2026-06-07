//! # DHT11 Temperature & Humidity Sensor Example (Embassy)
//!
//! Reads temperature and humidity from a DHT11 sensor using a single GPIO pin with Embassy.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2
//! - **Sensor:** DHT11 Temperature & Humidity Sensor
//!
//! ## Wiring Schematic
//!
//! ```text
//!                      Raspberry Pi Pico 2             DHT11 Module
//!                    +---------------------+      +---------------------+
//!                    |                     |      |                     |
//!                    | GND (Pin 38) -------+----->| GND                 |
//!                    | 3V3 (Pin 36) -------+----->| VCC                 |
//!                    | GPIO16 (Pin 21) ----+----->| DAT (Data)          |
//!                    |                     |      |                     |
//!                    +---------------------+      +---------------------+
//! ```
//!
//! > [!IMPORTANT]
//! > **Pull-up Resistor:**
//! > - **If using a DHT11 module board:** It likely already has a built-in pull-up resistor. No extra component is needed.
//! > - **If using a bare 4-pin DHT11 sensor:** You must add an external 4.7kΩ to 10kΩ pull-up resistor between the DAT (Data) and VCC lines.
//!
//! ## Run
//!
//! ```bash
//! cargo run --example dht11 --release
//! ```
//! Note: Due to tight timing sensitivity of the DHT11 protocol during the bit-read phase, you must run this example in **release** mode.
//!

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
use embassy_rp::gpio::{Level, OutputOpenDrain};
use embassy_time::{Duration, Timer};

use dht_sensor::*;

/// Custom cycle-accurate blocking and async delay implementing both `embedded_hal::delay::DelayNs`
/// and `embedded_hal_async::delay::DelayNs`.
///
/// DHT11 communications require microsecond-level pulse width measurement. Using `embassy_time::Delay`
/// introduces jitter which can break the protocol. Therefore, we use cycle-accurate Cortex-M delay loops
/// for sub-millisecond delays. For millisecond-scale delays (such as the initial 18ms start signal),
/// we yield execution back to the Embassy executor via `embassy_time::Timer` to allow other tasks to run.
struct PreciseDelay {
    loops_per_us: u32,
    loops_per_ms: u32,
}

impl PreciseDelay {
    fn new() -> Self {
        let sys_freq = embassy_rp::clocks::clk_sys_freq();
        Self {
            loops_per_us: sys_freq / 2_000_000,
            loops_per_ms: sys_freq / 2_000,
        }
    }
}

impl embedded_hal::delay::DelayNs for PreciseDelay {
    fn delay_ns(&mut self, ns: u32) {
        let loops = ((ns as u64 * self.loops_per_us as u64) / 1000) as u32;
        if loops > 0 {
            cortex_m::asm::delay(loops);
        }
    }

    fn delay_us(&mut self, us: u32) {
        let loops = us.saturating_mul(self.loops_per_us);
        if loops > 0 {
            cortex_m::asm::delay(loops);
        }
    }

    fn delay_ms(&mut self, ms: u32) {
        let loops = ms.saturating_mul(self.loops_per_ms);
        if loops > 0 {
            cortex_m::asm::delay(loops);
        }
    }
}

impl embedded_hal_async::delay::DelayNs for PreciseDelay {
    async fn delay_ns(&mut self, ns: u32) {
        <Self as embedded_hal::delay::DelayNs>::delay_ns(self, ns);
    }

    async fn delay_us(&mut self, us: u32) {
        <Self as embedded_hal::delay::DelayNs>::delay_us(self, us);
    }

    async fn delay_ms(&mut self, ms: u32) {
        // Yield to Embassy executor during millisecond-scale startup delays
        Timer::after(Duration::from_millis(ms as u64)).await;
    }
}

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("DHT11 Temperature & Humidity Sensor Example (Embassy)");

    let p = embassy_rp::init(Default::default());

    // Configure GPIO16 (Pin 21) as an Open Drain output with internal pull-up.
    let mut dht_pin = OutputOpenDrain::<'static>::new(p.PIN_16, Level::High);
    dht_pin.set_pullup(true);

    let mut delay = PreciseDelay::new();

    info!("DHT11 initialized. Starting reading loop (every 2 seconds)...");

    loop {
        // Perform an asynchronous read of the DHT11 sensor (which yields during the startup pulse,
        // and uses cycle-accurate blocking delay for the data readout).
        match dht11::r#async::read(&mut delay, &mut dht_pin).await {
            Ok(reading) => {
                info!(
                    "Temperature: {}°C | Humidity: {}%",
                    reading.temperature, reading.relative_humidity
                );
            }
            Err(e) => {
                // We print errors as info/warn since transient errors are common with DHT11 sensors
                match e {
                    DhtError::Timeout => {
                        warn!(
                            "Error: Reading timed out. Verify wiring and ensure running in --release mode."
                        );
                    }
                    DhtError::ChecksumMismatch => {
                        warn!("Error: Checksum mismatch. The data might have been corrupted.");
                    }
                    _ => {
                        warn!("Error reading sensor: {:?}", defmt::Debug2Format(&e));
                    }
                }
            }
        }

        // The DHT11 is slow and should not be polled more than once every 2 seconds.
        // Embassy's async timer allows other tasks to run during this interval.
        Timer::after(Duration::from_secs(2)).await;
    }
}

// Program metadata for `picotool info`.
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"dht11"),
    hal::binary_info::rp_program_description!(
        c"DHT11 Temperature & Humidity Sensor example for RPi Pico 2 (Embassy)"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
