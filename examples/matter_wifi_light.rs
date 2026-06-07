//! # Matter Wi-Fi Light Example
//!
//! This example implements a Matter-compatible Wi-Fi light bulb using the rs-matter stack.
//! It uses BLE for commissioning and Wi-Fi for network connectivity, allowing you to add
//! the Pico 2 W directly into Apple Home, Google Home, or Home Assistant!
//!
//! When toggled from your smart home app, it turns an external LED on and off.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 W
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
//! cargo run --example matter_wifi_light --release
//! ```

#![no_std]
#![no_main]
#![recursion_limit = "256"]

use core::mem::MaybeUninit;
use core::pin::pin;
use core::ptr::addr_of_mut;

use cyw43::{A4, Aligned, aligned_bytes};
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::dma;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::InterruptHandler;
use embedded_alloc::LlffHeap;
use panic_probe as _;

use defmt::{info, unwrap};
use defmt_rtt as _;

use embassy_time::{Duration, Timer};
use portable_atomic::{AtomicBool, Ordering};

use rs_matter_embassy::matter::crypto::{Crypto, default_crypto};
use rs_matter_embassy::matter::dm::clusters::app::on_off::test::TestOnOffDeviceLogic;
use rs_matter_embassy::matter::dm::clusters::app::on_off::{self, OnOffHooks};
use rs_matter_embassy::matter::dm::clusters::desc::{self, ClusterHandler as _};
use rs_matter_embassy::matter::dm::devices::DEV_TYPE_ON_OFF_LIGHT;
use rs_matter_embassy::matter::dm::devices::test::{
    DAC_PRIVKEY, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET,
};
use rs_matter_embassy::matter::dm::{Async, Dataver, EmptyHandler, Endpoint, EpClMatcher, Node};
use rs_matter_embassy::matter::persist::DummyKvBlobStore;
use rs_matter_embassy::matter::utils::init::InitMaybeUninit;
use rs_matter_embassy::matter::{clusters, devices};
use rs_matter_embassy::stack::rand::reseeding_csprng;
use rs_matter_embassy::wireless::rp::RpWifiDriver;
use rs_matter_embassy::wireless::{EmbassyWifi, EmbassyWifiMatterStack};

macro_rules! mk_static {
    ($t:ty) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        STATIC_CELL.uninit()
    }};
    ($t:ty,$val:expr) => {{ mk_static!($t).write($val) }};
}

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

const BUMP_SIZE: usize = 65536;

#[global_allocator]
static HEAP: LlffHeap = LlffHeap::empty();

static LED_STATE: AtomicBool = AtomicBool::new(false);

pub struct LedOnOffLogic;

use rs_matter_embassy::matter::dm::Cluster;
use rs_matter_embassy::matter::dm::clusters::app::on_off::EffectVariantEnum;
use rs_matter_embassy::matter::dm::clusters::app::on_off::StartUpOnOffEnum;
use rs_matter_embassy::matter::error::Error;
use rs_matter_embassy::matter::tlv::Nullable;

impl OnOffHooks for LedOnOffLogic {
    const CLUSTER: Cluster<'static> = TestOnOffDeviceLogic::CLUSTER;

    fn on_off(&self) -> bool {
        LED_STATE.load(Ordering::Relaxed)
    }

    fn set_on_off(&self, on: bool) {
        LED_STATE.store(on, Ordering::Relaxed);
    }

    fn start_up_on_off(&self) -> Nullable<StartUpOnOffEnum> {
        Nullable::none()
    }

    fn set_start_up_on_off(&self, _value: Nullable<StartUpOnOffEnum>) -> Result<(), Error> {
        Ok(())
    }

    async fn handle_off_with_effect(&self, _effect: EffectVariantEnum) {}
}

#[embassy_executor::task]
async fn led_task(mut led: Output<'static>) {
    let mut last_state = false;
    loop {
        let current_state = LED_STATE.load(Ordering::Relaxed);
        if current_state != last_state {
            if current_state {
                led.set_high();
            } else {
                led.set_low();
            }
            last_state = current_state;
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    {
        const HEAP_SIZE: usize = 8192;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }
    }

    let p = embassy_rp::init(Default::default());

    info!("Starting matter_wifi_light example...");

    // Setup the external LED on GP15
    let led = Output::new(p.PIN_15, Level::Low);
    spawner.spawn(led_task(led).unwrap());

    let (fw, clm, btfw, nvram) = (
        Option::<&Aligned<A4, [u8]>>::Some(aligned_bytes!("../cyw43-firmware/43439A0.bin")),
        Option::<&Aligned<A4, [u8]>>::Some(aligned_bytes!("../cyw43-firmware/43439A0_clm.bin")),
        Option::<&Aligned<A4, [u8]>>::Some(aligned_bytes!("../cyw43-firmware/43439A0_btfw.bin")),
        Option::<&Aligned<A4, [u8]>>::Some(aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin")),
    );

    const MY_DEV_DET: rs_matter_embassy::matter::dm::clusters::basic_info::BasicInfoConfig =
        rs_matter_embassy::matter::dm::clusters::basic_info::BasicInfoConfig {
            vendor_name: "melastmohican",
            device_name: "rs-matter rp2350 light",
            product_name: "rs-matter rp2350 light",
            ..TEST_DEV_DET
        };

    let stack = mk_static!(EmbassyWifiMatterStack<BUMP_SIZE, ()>).init_with(
        EmbassyWifiMatterStack::init(&MY_DEV_DET, TEST_DEV_COMM, &TEST_DEV_ATT),
    );

    let crypto = default_crypto(reseeding_csprng(RoscRng, 1000).unwrap(), DAC_PRIVKEY);
    let mut weak_rand = crypto.weak_rand().unwrap();

    let on_off = on_off::OnOffHandler::new_standalone(
        Dataver::new_rand(&mut weak_rand),
        LIGHT_ENDPOINT_ID,
        LedOnOffLogic,
    );

    let handler = EmptyHandler
        .chain(
            EpClMatcher::new(
                Some(LIGHT_ENDPOINT_ID),
                Some(TestOnOffDeviceLogic::CLUSTER.id),
            ),
            on_off::HandlerAsyncAdaptor(&on_off),
        )
        .chain(
            EpClMatcher::new(Some(LIGHT_ENDPOINT_ID), Some(desc::DescHandler::CLUSTER.id)),
            Async(desc::DescHandler::new(Dataver::new_rand(&mut weak_rand)).adapt()),
        );

    let mut kv = DummyKvBlobStore;
    stack.startup(&crypto, &mut kv).await.unwrap();

    let kv = stack.create_shared_kv(kv).unwrap();

    let matter = pin!(stack.run_coex(
        EmbassyWifi::new(
            RpWifiDriver::new(
                p.PIN_23, p.PIN_25, p.PIN_24, p.PIN_29, p.DMA_CH0, p.PIO0, Irqs, fw, clm, btfw,
                nvram,
            ),
            crypto.rand().unwrap(),
            true,
            stack,
        ),
        &crypto,
        (NODE, handler),
        &kv,
        (),
    ));

    unwrap!(matter.await);
}

const LIGHT_ENDPOINT_ID: u16 = 1;

const NODE: Node = Node {
    endpoints: &[
        EmbassyWifiMatterStack::<0, ()>::root_endpoint(),
        Endpoint::new(
            LIGHT_ENDPOINT_ID,
            devices!(DEV_TYPE_ON_OFF_LIGHT),
            clusters!(desc::DescHandler::CLUSTER, TestOnOffDeviceLogic::CLUSTER),
        ),
    ],
};
