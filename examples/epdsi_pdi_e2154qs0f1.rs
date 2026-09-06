//! # Pervasive Displays E2154QS0F1 E-Paper Example (`epdsi`, async / Embassy)
//!
//! Async counterpart to `rust-rpico2-discovery`'s `pdi_e2154qs0f1` example — full parity: same
//! panel, same wiring, same content, built against `epdsi`'s async API
//! (`default-features = false, features = ["graphics"]`) over `embassy-rp`'s async SPI and GPIO
//! instead of the blocking `rp235x-hal` API. Every `epdsi` call below is `.await`ed; nothing else
//! about the driver-level code differs from the blocking example.
//!
//! ## Note on the bit-banged OTP handshake and pin reuse
//!
//! The Pervasive BWRY COG's OTP register read (`PervasiveBwryController::read_otp`, via
//! [`Spi3Bus`]) bit-bangs SCK and a single bidirectional DATA line in software — see the crate
//! doc on `epdsi::bus3` for why. Both wires are the *same physical pins* (GPIO18/GPIO19) later
//! used by the real hardware SPI peripheral for normal 4-wire operation, so this example needs to
//! use each pin first as a bit-banged GPIO, then as an SPI peripheral pin.
//!
//! The blocking example does this through `rp235x-hal`'s pin type-state system
//! (`Pin<I, FunctionSio<_>, _>` -> `.into_function::<FunctionSpi>()`), which requires a hand-rolled
//! `FlexPin` type to switch between `SioInput`/`SioOutput` states, since two different type-states
//! can't share one binding. `embassy-rp` needs none of that ceremony: its `Peri<'d, T>` peripheral
//! handles support `.reborrow()`, so the same GPIO peripheral value is *borrowed* for the OTP
//! block (as [`embassy_rp::gpio::Flex`] for the DATA line, plain `Output`/`Input` for the rest)
//! and only *moved* into `Spi::new()` afterward, once those borrows have dropped. `DATA` still
//! needs a small local [`FlexPin`] wrapper — `embassy_rp::gpio::Flex` isn't `epdsi`'s
//! [`DynamicPin`] itself, just structurally identical to it (`set_as_input`/`set_as_output`/
//! `set_high`/`set_low`/`is_high`, all infallible) — but there's no per-type-state juggling.
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Pervasive Displays E2154QS0F1 1.54" Quad-Color E-Paper Display (EPDK / Driver F)
//!
//! ### Hardware Note for EXT3-1 Extension Boards:
//! - **J3 Jumper Setting**: Ensure the J3 jumper is OPEN (selecting the 10 µH inductor path for panels <= 3.7", e.g. 1.54" E2154QS0F1).
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
//! cargo run --example epdsi_pdi_e2154qs0f1
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
use embassy_rp::gpio::{Flex, Input, Level, Output, Pull};
use embassy_rp::spi::{Config, Spi};
use embassy_time::{Delay, Timer};

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
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

/// Adapts [`embassy_rp::gpio::Flex`] to [`epdsi::bus3::DynamicPin`]. Unlike the blocking
/// example's `FlexPin`, this needs no type-state enum — `Flex` already switches direction at
/// runtime on one type — so this is a thin, infallible forwarding wrapper.
struct FlexPin<'d>(Flex<'d>);

impl embedded_hal::digital::ErrorType for FlexPin<'_> {
    type Error = core::convert::Infallible;
}

impl DynamicPin for FlexPin<'_> {
    type Error = core::convert::Infallible;

    fn set_as_output(&mut self) -> Result<(), Self::Error> {
        self.0.set_as_output();
        Ok(())
    }

    fn set_as_input(&mut self) -> Result<(), Self::Error> {
        self.0.set_as_input();
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.0.set_high();
        Ok(())
    }

    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.0.set_low();
        Ok(())
    }

    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(Flex::is_high(&self.0))
    }
}

/// Converts a 1bpp `PageBuffer` slice (152x152 bits, 2,888 bytes) into a 2bpp BWRY frame buffer
/// (152x152 2bpp, 5,776 bytes). 1bpp White (bit=1) -> 2bpp `0b01` (White); 1bpp Black (bit=0) ->
/// 2bpp `0b00` (Black).
fn convert_1bpp_to_2bpp_bwry(src_1bpp: &[u8], dst_2bpp: &mut [u8]) {
    for (i, &byte) in src_1bpp.iter().enumerate() {
        let mut high_2bpp = 0u8;
        let mut low_2bpp = 0u8;

        for bit in 0..4 {
            let is_set = (byte & (1 << (7 - bit))) != 0;
            let val = if is_set { 0b01 } else { 0b00 };
            high_2bpp |= val << ((3 - bit) * 2);
        }

        for bit in 0..4 {
            let is_set = (byte & (1 << (3 - bit))) != 0;
            let val = if is_set { 0b01 } else { 0b00 };
            low_2bpp |= val << ((3 - bit) * 2);
        }

        dst_2bpp[i * 2] = high_2bpp;
        dst_2bpp[i * 2 + 1] = low_2bpp;
    }
}

/// Draws a colored rectangle directly onto the 2bpp BWRY frame buffer.
/// `color_code`: 0b00 = Black, 0b01 = White, 0b10 = Yellow, 0b11 = Red.
fn fill_rect_2bpp(dst_2bpp: &mut [u8], width: u32, x: u32, y: u32, w: u32, h: u32, color_code: u8) {
    let color_2bit = color_code & 0x03;
    for py in y..(y + h) {
        for px in x..(x + w) {
            let pixel_idx = (py * width + px) as usize;
            let byte_idx = pixel_idx / 4;
            let pixel_in_byte = pixel_idx % 4;
            let bit_shift = (3 - pixel_in_byte) * 2;
            let mask = !(0x03 << bit_shift);
            dst_2bpp[byte_idx] = (dst_2bpp[byte_idx] & mask) | (color_2bit << bit_shift);
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Program start (epdsi PervasiveBwryController, async/Embassy)");

    let p = embassy_rp::init(Default::default());
    let mut delay = Delay;

    // Hardware Note for EXT3-1 extension boards: J3 jumper must be OPEN (10 µH path) for panels
    // <= 3.7" such as this 1.54" E2154QS0F1 — see the module doc.

    let mut pin_cs = p.PIN_17;
    let mut pin_sck = p.PIN_18;
    let mut pin_mosi = p.PIN_19;
    let mut pin_dc = p.PIN_12;
    let mut pin_rst = p.PIN_11;
    // BUSY pin is active-LOW for Pervasive Displays COG (low = busy).
    let mut pin_busy = p.PIN_13;

    let mut controller = PervasiveBwryController::new(E2154QS0F1::WIDTH, E2154QS0F1::HEIGHT)
        .with_variant(PervasiveBwryVariant::DriverF)
        .with_temperature(25);

    info!("Reading OTP register data from panel (bit-banged 3-wire handshake)...");
    {
        // Each of these borrows its pin via `.reborrow()`, so the originals (`pin_cs`, `pin_sck`,
        // ...) become free to move into the real SPI peripheral once this block ends and the
        // borrows drop — see the module doc's note on pin reuse.
        let cs = Output::new(pin_cs.reborrow(), Level::High);
        let sck_gpio = Output::new(pin_sck.reborrow(), Level::Low);
        let data_flex = FlexPin(Flex::new(pin_mosi.reborrow()));
        let dc = Output::new(pin_dc.reborrow(), Level::Low);
        let rst = Output::new(pin_rst.reborrow(), Level::High);
        let busy = Input::new(pin_busy.reborrow(), Pull::Up);

        let mut bus3 = Spi3Bus::new(cs, sck_gpio, data_flex, dc, rst, busy);
        controller
            .read_otp(&mut bus3, &mut delay)
            .await
            .expect("Pervasive BWRY OTP read failed");
    }
    info!("OTP register data read OK");

    // The OTP-phase borrows have dropped, so `pin_sck`/`pin_mosi` are free to move into the real
    // hardware SPI peripheral for normal 4-wire operation.
    let cs = Output::new(pin_cs, Level::High);
    let dc = Output::new(pin_dc, Level::Low);
    let rst = Output::new(pin_rst, Level::High);
    let busy = Input::new(pin_busy, Pull::Up);

    let mut spi_config = Config::default();
    spi_config.frequency = 16_000_000;
    let spi = Spi::new(
        p.SPI0, pin_sck, pin_mosi, p.PIN_16, // MISO
        p.DMA_CH0, p.DMA_CH1, Irqs, spi_config,
    );
    let spi_device = ExclusiveDevice::new(spi, cs, Delay).unwrap();

    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let mut epd = EpdBuilder::<_, E2154QS0F1>::new(controller).build(epd_bus);

    info!("Initializing E2154QS0F1 display (BWRY Driver F) via epdsi driver...");
    // init() no longer touches OTP data (already read above); it does the non-OTP init steps.
    epd.init(&mut delay).await.unwrap();
    info!("E-Paper display initialized");

    // 1bpp monochrome drawing buffer (152 x 152 / 8 = 2,888 bytes)
    let mut buffer_1bpp = [0u8; (E2154QS0F1::WIDTH as usize * E2154QS0F1::HEIGHT as usize) / 8];
    // 2bpp BWRY packed frame buffer (152 x 152 * 2 / 8 = 5,776 bytes)
    let mut buffer_2bpp = [0u8; (E2154QS0F1::WIDTH as usize * E2154QS0F1::HEIGHT as usize) / 4];

    let mut display = PageBuffer::new(&mut buffer_1bpp, E2154QS0F1::WIDTH, E2154QS0F1::HEIGHT, 0);
    display.clear_byte(0xFF);

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    info!("Drawing shapes, text, and logos onto frame buffer...");

    // The 1.54" E2154QS0F1 panel is 152x152 with no RAM overscan — the whole canvas is visible.
    Text::new("E2154QS0F1 1.54\"", Point::new(5, 15), text_style)
        .draw(&mut display)
        .unwrap();

    Line::new(Point::new(5, 22), Point::new(145, 22))
        .into_styled(style)
        .draw(&mut display)
        .unwrap();

    Text::new("Spectra-4 BWRY", Point::new(5, 38), text_style)
        .draw(&mut display)
        .unwrap();

    Rectangle::new(Point::new(5, 45), Size::new(140, 12))
        .into_styled(style)
        .draw(&mut display)
        .unwrap();

    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + Point::new(7, 60), BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + Point::new(75, 60), BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    Text::new("RP2350 Pico 2", Point::new(5, 125), text_style)
        .draw(&mut display)
        .unwrap();

    Text::new("epdsi async", Point::new(5, 145), text_style)
        .draw(&mut display)
        .unwrap();

    convert_1bpp_to_2bpp_bwry(display.as_slice(), &mut buffer_2bpp);

    // Overlay Red (0b11) and Yellow (0b10) accent color blocks to demonstrate Quad-Color capability.
    fill_rect_2bpp(&mut buffer_2bpp, E2154QS0F1::WIDTH, 7, 47, 65, 8, 0b11);
    fill_rect_2bpp(&mut buffer_2bpp, E2154QS0F1::WIDTH, 75, 47, 65, 8, 0b10);

    info!("Sending BWRY frame buffer data (5,776 bytes) to display...");
    epd.write_frame(ColorChannel::BlackWhite, &buffer_2bpp)
        .await
        .unwrap();

    info!("Refreshing display hardware (Full OTP-driven update)...");
    epd.refresh(&mut delay).await.unwrap();

    info!("Powering off DC/DC...");
    epd.sleep(&mut delay).await.unwrap();

    info!("Display update complete!");

    loop {
        Timer::after_secs(3600).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"epdsi_pdi_e2154qs0f1"),
    hal::binary_info::rp_program_description!(c"epdsi async Pervasive BWRY/E2154QS0F1 example for RP2350"),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
