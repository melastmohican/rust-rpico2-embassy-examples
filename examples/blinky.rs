//! # External LED Blinky Example (Embassy)
//!
//! This example blinks an external LED connected to GPIO15. This is useful for boards
//! like the Raspberry Pi Pico 2 W, where the onboard LED is connected to the wireless chip
//! rather than a standard microcontroller GPIO.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 / Pico 2 W
//! - **LED & Resistor (220-330 Ohm):** Connected between GP15 (Pin 20) and GND (Pin 18).
//!
//! ## Wiring Schematic
//!
//! ```text
//!            Raspberry Pi Pico 2 W                   External Components
//!          +------------------------+
//!          |                        |
//!          |         GP15 (Pin 20)  |---------[ 220-330 Ohm Resistor ]-----+
//!          |                        |                                      |
//!          |         GND (Pin 18)   |------------------[ LED - ] <---+     |
//!          |                        |                                |     |
//!          +------------------------+                     (Cathode / |     |
//!                                                          Short Leg) |     |
//!                                                                    |     |
//!                                                         [ LED + ] -+-----+
//!                                                         (Anode /
//!                                                          Long Leg)
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run --example blinky
//! ```

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
use embassy_rp::gpio::{Level, Output};
use embassy_time::Timer;
use panic_probe as _;

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Blinky example: Booting {}", env!("CARGO_PKG_NAME"));

    let p = embassy_rp::init(Default::default());

    // Configure the external LED (GPIO15) as a push-pull output
    let mut led = Output::new(p.PIN_15, Level::Low);

    loop {
        led.set_high();
        info!("BLINK ON");
        Timer::after_millis(300).await;

        led.set_low();
        info!("BLINK OFF");
        Timer::after_millis(300).await;
    }
}

// Program metadata for `picotool info`.
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"blinky"),
    hal::binary_info::rp_program_description!(c"Blinky example for Raspberry Pi Pico 2 (Embassy)"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
