//! # Newxie Digital-to-Analog Thermometer Example (Embassy)
//!
//! Reads temperature and pressure from a BMP580 sensor over I2C and displays a
//! graphical thermometer and sensor data on an Adafruit 1.14" 240x135 Color Newxie TFT Display (ST7789)
//! over SPI using the async `display-driver` crate family (`display-driver`, `display-driver-spi`, `display-driver-st7789`).
//!
//! ## Hardware
//!
//! - **Board:** Raspberry Pi Pico 2 (RP2350)
//! - **Display:** Adafruit 1.14" 240x135 Color Newxie TFT Display (ST7789)
//! - **Sensor:** Adafruit BMP580 (I2C)
//!
//! ## Wiring for Adafruit 1.14" Color Newxie TFT Display
//!
//! ```text
//!      Raspberry Pi Pico 2          Adafruit 1.14" Newxie TFT
//!    +-----------------------+      +---------------------------+
//!    |                       |      |                           |
//!    |  3V3 (Pin 36) --------+------+-> V+ / VIN                |
//!    |  GND (Pin 38) --------+------+-> G / GND                 |
//!    |  GPIO17 (Pin 22) -----+------+-> CS                      |
//!    |  GPIO21 (Pin 27) -----+------+-> RST                     |
//!    |  GPIO20 (Pin 26) -----+------+-> DC                      |
//!    |  GPIO19 (Pin 25) -----+------+-> DA / MOSI               |
//!    |  GPIO18 (Pin 24) -----+------+-> CL / SCK                |
//!    |  GPIO14 (Pin 19) -----+------+-> BL (Backlight)          |
//!    |                       |      |                           |
//!    +-----------------------+      +---------------------------+
//! ```
//!
//! ## Wiring for BMP580 (I2C)
//!
//! ```text
//!      Sensor Pin    ->  RPi Pico 2 GPIO
//!      SCL           ->  GPIO5 (Pin 7) (I2C0 SCL)
//!      SDA           ->  GPIO4 (Pin 6) (I2C0 SDA)
//!      VIN           ->  3.3V (Pin 36)
//!      GND           ->  GND (Pin 38)
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run --example newxie_dd_thermometer
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
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{Config as I2cConfig, I2c};
use embassy_rp::spi::{Config as SpiConfig, Spi};
use embassy_time::{Delay, Duration, Timer};

use display_driver::{ColorFormat, DisplayDriver, Orientation, panel::reset::LCDResetOption};
use display_driver_spi::SpiDisplayBus;
use display_driver_st7789::{St7789, spec::generic::Generic135x240Type1};

use embedded_graphics::{
    framebuffer::{Framebuffer, buffer_size},
    geometry::Point,
    mono_font::{MonoTextStyleBuilder, ascii::FONT_9X15_BOLD},
    pixelcolor::{
        Rgb565,
        raw::{BigEndian, RawU16},
    },
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use embedded_hal::delay::DelayNs;
use embedded_hal_bus::spi::ExclusiveDevice;

/// Boot ROM definition block for RP2350
#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

const WIDTH: usize = 135;
const HEIGHT: usize = 240;

type FramebufferType =
    Framebuffer<Rgb565, RawU16, BigEndian, WIDTH, HEIGHT, { buffer_size::<Rgb565>(WIDTH, HEIGHT) }>;

// --- BMP580 Driver ---

const BMP580_ADDR: u8 = 0x47;
const REG_CHIP_ID: u8 = 0x01;
const REG_TEMP_DATA_XLSB: u8 = 0x1D;
const REG_OSR_CONFIG: u8 = 0x36;
const REG_ODR_CONFIG: u8 = 0x37;
const REG_CMD: u8 = 0x7E;
const CMD_SOFT_RESET: u8 = 0xB6;
const CHIP_ID_BMP580: u8 = 0x50;

struct Bmp580<I2C> {
    i2c: I2C,
    chip_id: u8,
}

impl<I2C> Bmp580<I2C>
where
    I2C: embedded_hal::i2c::I2c,
{
    pub fn new<D: DelayNs>(i2c: I2C, delay: &mut D) -> Result<Self, I2C::Error> {
        let mut sensor = Bmp580 { i2c, chip_id: 0 };
        sensor.init(delay)?;
        Ok(sensor)
    }

    fn init<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), I2C::Error> {
        let mut id = [0u8];
        self.i2c.write_read(BMP580_ADDR, &[REG_CHIP_ID], &mut id)?;
        self.chip_id = id[0];

        if self.chip_id != CHIP_ID_BMP580 {
            info!(
                "Unexpected Chip ID: 0x{:x} (expected 0x{:x})",
                self.chip_id, CHIP_ID_BMP580
            );
        } else {
            info!("BMP580 detected (Chip ID: 0x{:x})", self.chip_id);
        }

        self.i2c.write(BMP580_ADDR, &[REG_CMD, CMD_SOFT_RESET])?;
        delay.delay_ms(10);

        self.i2c.write(BMP580_ADDR, &[REG_OSR_CONFIG, 0x50])?;
        self.i2c.write(BMP580_ADDR, &[REG_ODR_CONFIG, 0x81])?;
        delay.delay_ms(10);

        Ok(())
    }

    pub fn read_data(&mut self) -> Result<(f32, f32), I2C::Error> {
        let mut buf = [0u8; 6];
        self.i2c
            .write_read(BMP580_ADDR, &[REG_TEMP_DATA_XLSB], &mut buf)?;

        let t_raw = ((buf[2] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[0] as u32);
        let temperature = (t_raw as f32) / 65536.0;

        let p_raw = ((buf[5] as u32) << 16) | ((buf[4] as u32) << 8) | (buf[3] as u32);
        let pressure = (p_raw as f32) / 64.0 / 100.0;

        Ok((pressure, temperature))
    }
}

// --- Thermometer Graphic ---

#[derive(Clone, Copy)]
struct ThermometerColors {
    bg: Rgb565,
    outline: Rgb565,
    bulb_outline: Rgb565,
    bulb_fill: Rgb565,
    tick_major: Rgb565,
    fill_actual: Rgb565,
}

impl Default for ThermometerColors {
    fn default() -> Self {
        Self {
            bg: Rgb565::BLACK,
            outline: Rgb565::WHITE,
            bulb_outline: Rgb565::WHITE,
            bulb_fill: Rgb565::RED,
            tick_major: Rgb565::WHITE,
            fill_actual: Rgb565::RED,
        }
    }
}

struct ThermometerGraphic {
    anchor: Point,
    width: u32,
    height: u32,
    temp_min: f32,
    temp_max: f32,
    colors: ThermometerColors,
}

impl ThermometerGraphic {
    fn new(anchor: Point, width: u32, height: u32, temp_min: f32, temp_max: f32) -> Self {
        Self {
            anchor,
            width,
            height,
            temp_min,
            temp_max,
            colors: ThermometerColors::default(),
        }
    }

    fn draw_static<D>(&self, target: &mut D) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        Rectangle::new(self.anchor, Size::new(self.width, self.height))
            .into_styled(PrimitiveStyle::with_fill(self.colors.bg))
            .draw(target)?;

        let bulb_radius: i32 = 12;
        let bulb_center =
            self.anchor + Point::new(self.width as i32 / 2, self.height as i32 - bulb_radius - 10);

        Circle::with_center(bulb_center, (bulb_radius * 2) as u32)
            .into_styled(PrimitiveStyle::with_fill(self.colors.bulb_fill))
            .draw(target)?;

        Circle::with_center(bulb_center, (bulb_radius * 2) as u32)
            .into_styled(PrimitiveStyle::with_stroke(self.colors.bulb_outline, 1))
            .draw(target)?;

        let tube_width: u32 = 10;
        let tube_height = self.height - (bulb_radius as u32 * 2) - 20;
        let tube_top_left =
            self.anchor + Point::new((self.width as i32 / 2) - (tube_width as i32 / 2), 10);

        Rectangle::new(tube_top_left, Size::new(tube_width, tube_height))
            .into_styled(PrimitiveStyle::with_stroke(self.colors.outline, 1))
            .draw(target)?;

        let margin_top: i32 = 20;
        let margin_bottom: i32 = bulb_radius * 2 + 25;
        let active_height = self.height as i32 - margin_top - margin_bottom;
        let px_per_degree = active_height as f32 / (self.temp_max - self.temp_min);

        for temp in (self.temp_min as i32..=self.temp_max as i32).step_by(10) {
            let y = (self.anchor.y + margin_top + active_height)
                - ((temp as f32 - self.temp_min) * px_per_degree) as i32;
            Line::new(
                Point::new(tube_top_left.x - 5, y),
                Point::new(tube_top_left.x, y),
            )
            .into_styled(PrimitiveStyle::with_stroke(self.colors.tick_major, 1))
            .draw(target)?;
        }

        Ok(())
    }

    fn update_temp<D>(&self, target: &mut D, temp_f: f32) -> Result<(), D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
    {
        let bulb_radius: i32 = 12;
        let margin_top: i32 = 20;
        let margin_bottom: i32 = bulb_radius * 2 + 25;
        let active_height = self.height as i32 - margin_top - margin_bottom;
        let px_per_degree = active_height as f32 / (self.temp_max - self.temp_min);

        let tube_width: i32 = 6;
        let tube_top_left =
            self.anchor + Point::new((self.width as i32 / 2) - (tube_width / 2), 10);

        let inner_tube_height = self.height - (bulb_radius as u32 * 2) - 22;
        Rectangle::new(
            tube_top_left + Point::new(1, 1),
            Size::new((tube_width - 2) as u32, inner_tube_height),
        )
        .into_styled(PrimitiveStyle::with_fill(self.colors.bg))
        .draw(target)?;

        let fill_height =
            ((temp_f - self.temp_min) * px_per_degree).clamp(0.0, active_height as f32);
        let fill_y = (self.anchor.y + margin_top + active_height) - fill_height as i32;

        Rectangle::new(
            Point::new(tube_top_left.x + 2, fill_y),
            Size::new((tube_width - 4) as u32, fill_height as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(self.colors.fill_actual))
        .draw(target)?;

        Ok(())
    }
}

struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Writer<'a> {
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.pos]).unwrap_or("")
    }
}

impl<'a> core::fmt::Write for Writer<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let len = bytes.len();
        if self.pos + len <= self.buf.len() {
            self.buf[self.pos..self.pos + len].copy_from_slice(bytes);
            self.pos += len;
            Ok(())
        } else {
            Err(core::fmt::Error)
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Initializing Newxie Thermometer Example (Embassy + display-driver)...");

    let p = embassy_rp::init(Default::default());

    // 1. Initialize I2C for BMP580 (GP4 = SDA, GP5 = SCL)
    let i2c = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let mut delay = Delay;

    let mut bmp = match Bmp580::new(i2c, &mut delay) {
        Ok(s) => s,
        Err(_) => {
            error!("Failed to initialize BMP580 sensor");
            loop {
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    };

    // 2. Initialize SPI for ST7789 Display
    // SCLK: GPIO18, MOSI: GPIO19, MISO: GPIO16
    // CS: GPIO17, DC: GPIO20, RST: GPIO21, BL: GPIO14
    let dc = Output::new(p.PIN_20, Level::Low);
    let rst = Output::new(p.PIN_21, Level::High);
    let mut bl = Output::new(p.PIN_14, Level::High);
    bl.set_high();

    let mut spi_config = SpiConfig::default();
    spi_config.frequency = 40_000_000;

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
    let panel = St7789::<Generic135x240Type1, _, _>::new(reset_opt);

    info!("Building Display Driver (135x240 Portrait)...");
    let mut driver = match DisplayDriver::builder(bus, panel)
        .with_color_format(ColorFormat::RGB565)
        .with_orientation(Orientation::Deg0)
        .init(&mut Delay)
        .await
    {
        Ok(driver) => driver,
        Err(e) => {
            error!("Display init failed: {:?}", Debug2Format(&e));
            return;
        }
    };

    info!("Display initialized via display-driver!");

    // 3. Create offscreen framebuffer for drawing
    let mut fb = FramebufferType::new();
    fb.clear(Rgb565::BLACK).unwrap();

    // Setup Thermometer Graphic
    let therm = ThermometerGraphic::new(Point::new(10, 5), 115, 180, 40.0, 100.0);
    therm.draw_static(&mut fb).unwrap();

    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_9X15_BOLD)
        .text_color(Rgb565::WHITE)
        .build();

    info!("Starting main loop...");

    let mut cycle: i32 = 0;
    let mut last_temp_c: f32 = 22.0;
    let mut last_press_hpa: f32 = 1013.25;

    loop {
        if let Ok((press, temp)) = bmp.read_data() {
            last_temp_c = temp;
            last_press_hpa = press;
        }

        info!(
            "Temp: {}.{} C | Pressure: {}.{} hPa",
            last_temp_c as i32,
            ((last_temp_c % 1.0).abs() * 10.0) as u32,
            last_press_hpa as i32,
            ((last_press_hpa % 1.0).abs() * 10.0) as u32
        );

        let temp_f = (last_temp_c * 9.0 / 5.0) + 32.0;
        therm.update_temp(&mut fb, temp_f).unwrap();

        cycle = (cycle + 1) % 4;

        // Clear label area at the bottom of the framebuffer
        Rectangle::new(Point::new(0, 190), Size::new(135, 50))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
            .draw(&mut fb)
            .unwrap();

        let mut buf = [0u8; 32];
        let mut writer = Writer {
            buf: &mut buf,
            pos: 0,
        };

        match cycle {
            0 => {
                let _ = core::fmt::write(&mut writer, format_args!("{:.1} F", temp_f));
            }
            1 => {
                let _ = core::fmt::write(&mut writer, format_args!("{:.1} C", last_temp_c));
            }
            2 => {
                let _ = core::fmt::write(&mut writer, format_args!("{:.1} hPa", last_press_hpa));
            }
            _ => {
                let _ = core::fmt::write(&mut writer, format_args!("BMP ID: 0x{:x}", bmp.chip_id));
            }
        }

        Text::with_baseline(
            writer.as_str(),
            Point::new(10, 200),
            text_style,
            Baseline::Top,
        )
        .draw(&mut fb)
        .unwrap();

        // Flush framebuffer to ST7789 display
        if let Err(e) = driver.write_frame(fb.data()).await {
            error!("Flush failed: {:?}", Debug2Format(&e));
        }

        Timer::after(Duration::from_secs(2)).await;
    }
}

// Metadata for picotool
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 4] = [
    hal::binary_info::rp_program_name!(c"newxie_dd_thermometer"),
    hal::binary_info::rp_program_description!(
        c"Adafruit Newxie ST7789 Display Driver Thermometer example for RP2350"
    ),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_build_attribute!(),
];
