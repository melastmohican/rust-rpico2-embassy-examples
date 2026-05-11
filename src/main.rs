#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp as hal;
use embassy_rp::block::ImageDef;
use embassy_time::Timer;

//Panic Handler
use panic_probe as _;
// Defmt Logging
use defmt_rtt as _;

/// Tell the Boot ROM about our application
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: ImageDef = hal::block::ImageDef::secure_exe();

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // Log some startup info via defmt
    info!("Booting {}", env!("CARGO_PKG_NAME"));
    info!("Version {}", env!("CARGO_PKG_VERSION"));
    info!("Examples for RP2350 (Embassy)");

    // Initialize embassy-rp
    let _p = embassy_rp::init(Default::default());

    // Idle loop
    loop {
        Timer::after_millis(1000).await;
    }
}

// Program metadata for `picotool info`.
// This isn't needed, but it's recommended to have these minimal entries.
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"rust-rpico2-embassy-examples"),
    embassy_rp::binary_info::rp_program_description!(
        c"Rust Embassy examples for Raspberry Pi Pico 2"
    ),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

// End of file
