//! # DS18B20 1-Wire Temperature Sensor Example (Embassy)
//!
//! Reads temperature from a DS18B20 temperature sensor over a 1-Wire bus using Embassy.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2
//! - **Sensor:** DS18B20 Waterproof Temperature Sensor Probe (or compatible)
//!
//! ## Wiring Schematic
//!
//! ```text
//!                               Raspberry Pi Pico 2
//!                            +-----------------------+
//!                            |                       |
//!                            | [ ] 1      40 [ ] USB |
//!                            | [ ] 2      39 [ ]     |
//!                            | [ ] 3      38 [G]ND --+-------+ (black)
//!                            | [ ] 4      37 [ ]     |       |
//!                            | [ ] 5      36 [3]V3 --+---+   |
//!                            |  ...        ...       |   |   |
//!                            | [ ] 20     21 [ ] ----+---+---|---+ (white, GPIO16)
//!                            +-----------------------+   |   |   |
//!                                                        |   |   |
//!                                                        |   |   |
//!                    +-----------------------------+     |   |   |
//!                    |     DS18B20 Sensor / Probe  |     |   |   |
//!                    |      (Bottom/Flat Side)     |     |   |   |
//!                    |                             |     |   |   |
//!                    |     [GND]   [DAT]   [VCC]   |     |   |   |
//!                    +-------|-------|-------|-----+     |   |   |
//!                            |       |       |           |   |   |
//!                            |       +-------+--[5K1]----+   | (Pull-Up Resistor
//!                            |       |       |   Resistor    |  between DAT & VCC)
//!                            |       |       +---------------+ (red)
//!                            +-------|-----------------------+ (black)
//!                                    |
//!                                    +--------------------------- (white)
//! ```
//!
//! ## Breadboard Layout Diagram
//!
//! ```text
//!                          Breadboard Columns (e.g. Columns 14-16)
//!                          
//!              Column 16 (GND)    Column 15 (VCC)    Column 14 (DAT)
//!              +-------------+    +-------------+    +-------------+
//!              |             |    |             |    |             |
//!              | [GND Pin]   |    | [VCC Pin]   |    | [DAT Pin]   |  <-- Sensor Breakout Pins
//!              |             |    |             |    |             |
//!              | [GND Wire]  |    | [VCC Wire]  |    | [DAT Wire]  |  <-- To Pico (GND, 3V3, GPIO16)
//!              |   (black)   |    |    (red)    |    |   (white)   |
//!              |             |    |             |    |             |
//!              |             |    |             |    | [Resistor]  |  <-- Pull-Up Resistor connected
//!              +-------------+    +-------------+    +------|------+      between Column 14 (DAT)
//!                                                           |             and the Red (+) Power Rail
//!                                                           |
//!                                                     [Red (+) Rail]
//! ```
//!
//! > [!IMPORTANT]
//! > **Pull-up Resistor Required:**
//! > You MUST connect a **4.7kΩ to 5.1kΩ pull-up resistor** between the Data (yellow)
//! > line and VCC (red) line. Without it, the 1-Wire bus will not function.
//!
//! > [!CAUTION]
//! > **Verify Your Sensor's Pinout Layout!**
//! > - A standalone TO-92 package sensor typically uses: **[GND] [DAT] [VCC]** (flat side facing you).
//! > - However, many pluggable breakout adapter boards use: **[GND] [VCC] [DAT]** (like the one in this example).
//! > Always check your specific hardware's markings! Wiring it incorrectly can cause the sensor to reverse-bias, heat up rapidly, and potentially damage the device.
//!
//! ## Run
//!
//! ```bash
//! cargo run --example ds18b20
//! ```
//!
//! ## Expected Output
//!
//! When successfully connected, you should see logs similar to:
//! ```text
//! [INFO ] Initializing DS18B20 Example (Embassy)... (ds18b20 rust-rpico2-embassy-examples/examples/ds18b20.rs:155)
//! [INFO ] Searching for devices on the 1-Wire bus... (ds18b20 rust-rpico2-embassy-examples/examples/ds18b20.rs:178)
//! [INFO ] Found 1-Wire device address: [0x28, 0x93, 0xff, 0x77, 0x0, 0x0, 0x0, 0x11] (ds18b20 rust-rpico2-embassy-examples/examples/ds18b20.rs:203)
//! [INFO ] DS18B20 sensor identified! Starting periodic temperature readings... (ds18b20 rust-rpico2-embassy-examples/examples/ds18b20.rs:218)
//! [INFO ] Current temperature: 22.0 °C (ds18b20 rust-rpico2-embassy-examples/examples/ds18b20.rs:240)
//! [INFO ] Current temperature: 22.0 °C (ds18b20 rust-rpico2-embassy-examples/examples/ds18b20.rs:240)
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
use embassy_rp::gpio::{Level, OutputOpenDrain};
use embassy_time::{Duration, Timer};

use onewire::ds18b20::DS18B20;
use onewire::{DeviceSearch, OneWire};

/// Custom cycle-accurate blocking delay implementing `embedded_hal::delay::DelayNs`.
/// This is required because `embassy_time::Delay` uses the coarse 1 MHz hardware timer
/// which has a 1-microsecond quantization error. Since the 1-Wire protocol relies on precise
/// microsecond-level pulse widths (e.g. 1us to 15us), a 1us timing error can cause
/// communication failures (e.g. device search misses or CRC mismatches).
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

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Initializing DS18B20 Example (Embassy)...");

    let p = embassy_rp::init(Default::default());

    // Configure GPIO16 (Pin 21) as an Open Drain output
    // OneWire is active-low, pulled up to 3.3V externally, so we start with the output Level::High
    // to release the bus initially.
    let mut ow_pin = OutputOpenDrain::<'static>::new(p.PIN_16, Level::High);
    ow_pin.set_pullup(true);
    let mut wire = OneWire::new(&mut ow_pin, false);

    // Initialize our precise CPU-cycle blocking delay
    let mut delay = PreciseDelay::new();

    let mut first_iteration = true;
    'infinite: loop {
        // Prevent continuous hot looping in case we reset/continue early
        if !first_iteration {
            Timer::after(Duration::from_secs(2)).await;
        } else {
            first_iteration = false;
        }

        info!("Searching for devices on the 1-Wire bus...");

        // Reset the bus to verify electrical state and check if any devices are present
        if wire
            .reset(&mut delay)
            .inspect_err(|err| error!("Failed to reset 1-Wire bus: {:?}", defmt::Debug2Format(err)))
            .is_err()
        {
            warn!("Is the 4.7kΩ - 5.1kΩ pull-up resistor connected between DAT and 3.3V?");
            continue 'infinite;
        }

        // Start device search
        let mut search = DeviceSearch::new();
        let search_result = wire
            .search_next(&mut search, &mut delay)
            .inspect_err(|err| {
                error!(
                    "Failed to search for devices: {:?}",
                    defmt::Debug2Format(err)
                )
            });

        let Ok(Some(device)) = search_result else {
            if let Ok(None) = search_result {
                info!("No 1-Wire devices found on the bus.");
            }
            continue 'infinite;
        };

        info!(
            "Found 1-Wire device address: {=[u8; 8]:#02x}",
            device.address
        );

        // Check if the device is a DS18B20 temperature sensor
        if device.address[0] != onewire::ds18b20::FAMILY_CODE {
            error!("Found device is not a DS18B20 sensor (family code mismatch).");
            continue 'infinite;
        }

        // Initialize the DS18B20 driver
        let Ok(sensor) = DS18B20::new(device).inspect_err(|err| {
            error!(
                "Failed to create DS18B20 sensor driver: {:?}",
                defmt::Debug2Format(err)
            )
        }) else {
            continue 'infinite;
        };

        info!("DS18B20 sensor identified! Starting periodic temperature readings...");

        'measure: loop {
            // Initiate a temperature measurement conversion
            let Ok(resolution) = sensor
                .measure_temperature(&mut wire, &mut delay)
                .inspect_err(|err| {
                    error!(
                        "Failed to start temperature measurement: {:?}",
                        defmt::Debug2Format(err)
                    )
                })
            else {
                continue 'measure;
            };

            // Wait for measurement conversion to complete asynchronously
            // 12-bit resolution requires up to 750ms. Using Timer::after lets the executor run other tasks.
            Timer::after(Duration::from_millis(resolution.time_ms() as u64)).await;

            // Read the scratchpad to retrieve the measured temperature
            match sensor.read_temperature(&mut wire, &mut delay) {
                Ok(raw_temp) => {
                    let (whole, frac) = onewire::ds18b20::split_temp(raw_temp);

                    // Convert whole & fraction to f32 for concise floating-point logging
                    let temp_f32 = (whole as f32) + (frac as f32) / 10000.0;
                    info!("Current temperature: {=f32} °C", temp_f32);
                }
                Err(e) => {
                    error!(
                        "Failed to read temperature from scratchpad: {:?}",
                        defmt::Debug2Format(&e)
                    );
                    continue 'measure;
                }
            }

            // Wait 2 seconds before the next reading
            Timer::after(Duration::from_secs(2)).await;
        }
    }
}

// Program metadata for `picotool info`.
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"ds18b20"),
    hal::binary_info::rp_program_description!(
        c"DS18B20 1-Wire Temperature Sensor example for RPi Pico 2 (Embassy)"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
