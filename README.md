# Rust Embassy Examples for Raspberry Pi Pico 2

This repository contains examples for the Raspberry Pi Pico 2 (RP2350) board, written in Rust using the [Embassy](https://embassy.dev/) async framework.

> 🎉 Featured on the [Adafruit blog](https://blog.adafruit.com/2026/06/10/rust-embassy-examples-for-raspberry-pi-pico-2/) (June 2026).

## Project generated

```shell
cargo generate --git https://github.com/ImplFerris/pico2-template.git --name rust-rpico2-embassy-examples
```

## Hardware

**Board:** Raspberry Pi Pico 2

- **MCU:** RP2350 (Dual-core Arm Cortex-M33 and RISC-V cores)
- **On-board peripherals:**
  - LED on GPIO25

### Pinout

![Raspberry Pi Pico 2 Pinout](https://www.raspberrypi.com/documentation/microcontrollers/images/pico-2-r4-pinout.svg)

### Common Pin Assignments

- **I2C pins:**
  - **I2C0 SDA:** GPIO4
  - **I2C0 SCL:** GPIO5
  - **I2C1 SDA:** GPIO2
  - **I2C1 SCL:** GPIO3
- **UART pins:**
  - **UART0 TX:** GPIO0, **UART0 RX:** GPIO1
  - **UART1 TX:** GPIO8, **UART1 RX:** GPIO9

## Examples

### I2C Examples

#### hs3003_i2c

Reads temperature and humidity from an HS3003 sensor using the Embassy async framework.

```bash
cargo run --example hs3003_i2c
```

**Wiring (Arduino Modulino Thermo):**

```
     Modulino -> RPi Pico 2
----------    --------------
GND (black) -> GND
VCC (red)   -> 3.3V
SCL (yellow)-> GPIO5 (Pin 7) (I2C0 SCL)
SDA (blue)  -> GPIO4 (Pin 6) (I2C0 SDA)
```

**About HS3003:**

The Renesas HS3003 is a high-performance temperature and humidity sensor:
- Temperature range: -40°C to +125°C (±0.2°C accuracy)
- Humidity range: 0% to 100% RH (±1.5% accuracy)
- 14-bit resolution for both measurements
- Ultra-low power consumption

#### adxl345_i2c

Reads accelerometer data from an ADXL345 sensor over I2C0 using Embassy.

```bash
cargo run --example adxl345_i2c
```

**Wiring:**

```
     ADXL345 -> RPi Pico 2
----------    --------------
GND (black) -> GND
VCC (red)   -> 3.3V
SCL (yellow)-> GPIO5 (Pin 7) (I2C0 SCL)
SDA (blue)  -> GPIO4 (Pin 6) (I2C0 SDA)
```

**About ADXL345:**

The ADXL345 is a small, thin, low power, 3-axis accelerometer with high resolution (13-bit) measurement at up to ±16 g. Digital output data is formatted as 16-bit twos complement and is accessible through either an SPI (3- or 4-wire) or I2C digital interface.

### SPI Display Examples

#### zermatt

Displays a 320x240 image of Zermatt on the Adafruit 2.2" TFT LCD display in landscape mode.

```bash
cargo run --example zermatt
```

**Wiring (Eye-SPI Breakout):**

```
     Raspberry Pi Pico 2              Eye-SPI Breakout
   +-----------------------+      +---------------------------+
   |                       |      |                           |
   |  3V3 (Pin 36) --------+------+-> VIN   (Red Wire)        |
   |  GND (Pin 38) --------+------+-> GND   (Black Wire)      |
   |  GPIO18 (Pin 24) -----+------+-> SCK   (Blue Wire)       |
   |  GPIO19 (Pin 25) -----+------+-> MOSI  (Green Wire)      |
   |  GPIO16 (Pin 21) -----+------+-> MISO  (Yellow Wire)     |
   |  GPIO20 (Pin 26) -----+------+-> DC    (White Wire)      |
   |  GPIO21 (Pin 27) -----+------+-> RST   (Orange Wire)     |
   |  GPIO17 (Pin 22) -----+------+-> TCS   (Blue Wire)       |
   |                       |      |                           |
   +-----------------------+      +---------------------------+
```

#### zermatt_snow

Displays a 320x240 image of Zermatt on the Adafruit 2.2" TFT LCD display with animated falling snow, utilizing a physics engine and the Embassy async framework to draw to an off-screen `lcd-async` framebuffer and dispatch via DMA without blocking the CPU.

```bash
cargo run --example zermatt_snow
```

Wiring is identical to the `zermatt` example.

#### Waveshare 0.96" ST7735S Display Examples

The workspace includes examples for the Waveshare 0.96" 80x160 ST7735S LCD module built with both the **`display-driver`** async framework (`st7735_dd_*`) and the **`mipidsi`** driver crate (`st7735_mipi_*`).

**Common Wiring for Waveshare 0.96" ST7735S LCD:**

```text
     Raspberry Pi Pico 2          Waveshare 0.96" ST7735S LCD
   +-----------------------+      +---------------------------+
   |                       |      |                           |
   |  3V3 (Pin 36) --------+------+-> VCC                     |
   |  GND (Pin 38) --------+------+-> GND                     |
   |  GPIO17 (Pin 22) -----+------+-> CS                      |
   |  GPIO21 (Pin 27) -----+------+-> RST                     |
   |  GPIO20 (Pin 26) -----+------+-> DC                      |
   |  GPIO19 (Pin 25) -----+------+-> DIN(MOSI)               |
   |  GPIO18 (Pin 24) -----+------+-> CLK(SCK)                |
   |  GPIO14 (Pin 19) -----+------+-> BL (Backlight)          |
   |                       |      |                           |
   +-----------------------+      +---------------------------+
```

##### st7735_dd_ferris

Draws the Ferris mascot and Rust logo BMP images on the 80x160 ST7735S display using the async `display-driver` crate stack (`display-driver`, `display-driver-spi`, `display-driver-st7735`).

```bash
cargo run --example st7735_dd_ferris
```

##### st7735_dd_animation

Runs a smooth 30 FPS geometric pattern animation (rotating circles, pulsating center ring, and expanding corner accents) rendered into a 160x80 framebuffer and flushed asynchronously via `display-driver`.

```bash
cargo run --example st7735_dd_animation
```

##### st7735_mipi_ferris

Draws the Ferris mascot and Rust logo BMP images using the `mipidsi` display driver crate and `display-interface-spi`.

```bash
cargo run --example st7735_mipi_ferris
```

##### st7735_mipi_text

Draws text header banners, a separator line, colored geometric shapes, and bottom text labels using `mipidsi` and `display-interface-spi`.

```bash
cargo run --example st7735_mipi_text
```

##### st7735_slint

Runs the **[Slint UI framework](https://slint.dev/)** in `no_std` mode using its built-in software renderer, rendering an interactive animated Slint component (custom colors, typography, properties, and layout) flushed via `display-driver`.

- **Slint Website:** <https://slint.dev/>
- **Slint Documentation:** <https://slint.dev/docs>
- **Slint Software Renderer Docs:** <https://slint.dev/docs/rust/slint/platform/software_renderer/index.html>

```bash
cargo run --example st7735_slint
```

#### GC9A01 240x240 Round LCD Display Examples

The workspace includes examples for 240x240 GC9A01 round LCD displays built with both the **`display-driver`** async framework (`gc9a01_dd_*`) and the **`mipidsi`** driver crate (`gc9a01_mipi_*`).

**Common Wiring for GC9A01 240x240 Round LCD:**

```text
     Raspberry Pi Pico 2           GC9A01 240x240 Round LCD
   +-----------------------+      +---------------------------+
   |                       |      |                           |
   |  3V3 (Pin 36) --------+------+-> VCC                     |
   |  GND (Pin 38) --------+------+-> GND                     |
   |  GPIO17 (Pin 22) -----+------+-> CS                      |
   |  GPIO21 (Pin 27) -----+------+-> RST                     |
   |  GPIO20 (Pin 26) -----+------+-> DC                      |
   |  GPIO19 (Pin 25) -----+------+-> SDA(MOSI)               |
   |  GPIO18 (Pin 24) -----+------+-> SCL(SCK)                |
   |                       |      |                           |
   +-----------------------+      +---------------------------+
```

**Breadboard Layout:**

![GC9A01 Breadboard Wiring Layout](rpico2_gc9a01_bb.png)

##### gc9a01_dd_ferris

Draws the Ferris mascot and Rust logo BMP images on the round 240x240 GC9A01 display using the async `display-driver` crate stack (`display-driver`, `display-driver-spi`, `display-driver-gc9a01`).

```bash
cargo run --example gc9a01_dd_ferris
```

##### gc9a01_dd_concentric

Renders a 4x4 Bayer dithered concentric radial gradient (golden-yellow to purple) with a shadow text overlay on the 240x240 GC9A01 display using `display-driver`.

```bash
cargo run --example gc9a01_dd_concentric
```

##### gc9a01_mipi_ferris

Draws the Ferris mascot and Rust logo BMP images on the 240x240 GC9A01 display using `mipidsi` and `display-interface-spi`.

```bash
cargo run --example gc9a01_mipi_ferris
```

#### Adafruit 1.14" Color Newxie ST7789 Display Examples

##### newxie_dd_thermometer

Reads temperature and pressure from an Adafruit BMP580 sensor over I2C and displays an analog-style thermometer graphic with cycling digital measurements (Fahrenheit, Celsius, pressure hPa, sensor ID) on an Adafruit 1.14" 240x135 ST7789 display using `display-driver` (`display-driver-st7789`).

```bash
cargo run --example newxie_dd_thermometer
```

### E-Paper Examples (epdsi)

Async twins of every e-paper example in [`rust-rpico2-discovery`](https://github.com/melastmohican/rust-rpico2-discovery) — same board, same panels, same wiring, same content and phases; only the transport differs (`embassy-rp`'s async SPI/GPIO instead of `rp235x-hal`'s blocking API, `epdsi` built with `default-features = false, features = ["graphics"]`). See that repo's README for full per-panel narrative (refresh-mode behavior, ink polarity, timing notes, provenance caveats) — not repeated here.

Fourteen use the [Good Display DESPI-C02 adapter board](https://www.good-display.com/product/516.html); four (the Pervasive Displays panels) use the EXT3-1 board with a 10-way rainbow cable.

**Wiring (DESPI-C02 adapter board):**

| Pico 2 Pin       | DESPI-C02 Pin / Function |
|------------------|--------------------------|
| 3V3 (Pin 36)     | VCC                      |
| GND (Pin 38)     | GND                      |
| GPIO11 (Pin 15)  | RST                      |
| GPIO12 (Pin 16)  | DC                       |
| GPIO13 (Pin 17)  | BUSY                     |
| GPIO16 (Pin 21)  | MISO                     |
| GPIO17 (Pin 22)  | CS                       |
| GPIO18 (Pin 24)  | SCK                      |
| GPIO19 (Pin 25)  | MOSI                     |

**Wiring (EXT3-1/EPDK, 10-way rainbow cable):**

| Pico Pin        | Cable Color | EXT3 Pin / Function    |
|-----------------|-------------|-------------------------|
| 3V3 (Pin 36)    | Black       | 1 / VCC                 |
| GPIO18 (Pin 24) | Brown       | 2 / SCK (SPI Clock)     |
| GPIO13 (Pin 17) | Red         | 3 / BUSY                |
| GPIO12 (Pin 16) | Orange      | 4 / DC (Data/Cmd)       |
| GPIO11 (Pin 15) | Yellow      | 5 / RST (Reset)         |
| GPIO16 (Pin 21) | Green       | 6 / MISO                |
| GPIO19 (Pin 25) | Blue        | 7 / MOSI                |
| NC              | Violet      | 8 / FCSM (Flash CS, unused) |
| GPIO17 (Pin 22) | Grey        | 9 / ECSM (Display CS)   |
| GND (Pin 38)    | White       | 10 / GND                |

> **EXT3-1 J3 jumper:** OPEN (10 µH) for panels ≤ 3.7" (`E2266KS0C1`, `E2290KS0F1`, `E2154QS0F1`); CLOSED (47 µH) for `E2417QS0A3` (4.2"). Wrong setting chokes the DC-DC booster on current bursts, causing voltage sags and BUSY hangs.

#### epdsi_jd79661_zjy122250

Good Display ZJY122250-0213AJH-E5 2.13" Quad-Color (Black/White/Yellow/Red, 122×250), `Jd79661Controller`. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_jd79661_zjy122250
```

#### epdsi_jd79660_gdem0154f51h

Good Display GDEM0154F51H 1.54" Quad-Color (Black/White/Yellow/Red, 200×200, Waveshare *1.54inch e-Paper (G)*), `Jd79660Controller` — shares its SPI register table with `Jd79661Controller` (both wrap `Jd7966xController`), differing only in which registers `init_sequence` writes. Wiring: DESPI-C02, above.

> Not independently confirmed on hardware in this repo or in `rust-rpico2-discovery`'s own doc for the blocking twin — build and flash to verify before relying on it.

```bash
cargo run --example epdsi_jd79660_gdem0154f51h
```

#### epdsi_ssd1681_gdem0154z90

Dalian Good Display GDEM0154Z90 1.54" Tri-Color (200×200), `Ssd1681Controller`. Full Tri-Color content plus a partial *window* refresh loop — no fast/differential waveform on this panel, every update takes ~14 s. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1681_gdem0154z90
```

#### epdsi_ssd1681_gdem0154z90_tri_epd

Full-parity companion to `epdsi_ssd1681_gdem0154z90` — same content, same hardware — drawn entirely through `PageBufferPair`/`TriColor` instead of two separate `PageBuffer`s. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1681_gdem0154z90_tri_epd
```

#### epdsi_ssd1680_gdem0213b74

Dalian Good Display GDEM0213B74 2.13" Monochrome (122×250), `Ssd1680Controller`. Full refresh, then a fast *differential* partial-window logo-swap loop, then a full-waveform cleanup pass. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1680_gdem0213b74
```

#### epdsi_ssd1680_gdey0266z90

Good Display GDEY0266Z90 2.66" Tri-Color (Black/White/Red, 152×296), `Ssd1680Controller`. Demonstrates every refresh mode the SSD1680 exposes for a Tri-Color panel: `Full`, a partial window loop, `FastFull`, and `BaseMap`/`Partial` at their real (non-differential on colour glass) cost. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1680_gdey0266z90
```

#### epdsi_ssd1680_gdey0266z90_tri_epd

Full-parity companion to `epdsi_ssd1680_gdey0266z90` — drawn entirely through `PageBufferPair`/`TriColor`. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1680_gdey0266z90_tri_epd
```

#### epdsi_ssd1680_gdey0266t90

Good Display GDEY0266T90 / Waveshare 2.66" e-Paper, Monochrome (152×296), `Ssd1680Controller` — a different, monochrome-only glass from the Tri-Color `GDEY0266Z90` above, same nominal size and controller. Genuinely fast: real Full and Partial (differential) refresh. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1680_gdey0266t90
```

#### epdsi_ssd1680_gdey0266t90_gray4

Companion to `epdsi_ssd1680_gdey0266t90` — drives the panel's 4-level grayscale mode (White/Light/Dark/Black) instead of plain monochrome. `GDEY0266T90::GRAY4` is Adafruit_EPD-sourced, not Good Display/Waveshare material. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1680_gdey0266t90_gray4
```

#### epdsi_uc8253_gdey037t03

Dalian Good Display GDEY037T03 3.7" Monochrome (240×416), `Uc8253Controller`. Full refresh, a partial-window logo-swap loop, and a full-waveform cleanup pass. BUSY is active-**low** on this controller. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_uc8253_gdey037t03
```

#### epdsi_uc8253_se0352n14

Waveshare 3.52" e-Paper (B) SE0352N14-TNG-A0 Tri-Color (240×360), `Uc8253Controller` with `Uc8253Variant::Se0352n14` — the variant is not optional: this panel disagrees with `GDEY037T03` on RAM plane routing and ink polarity despite sharing the same controller. Full refresh only (~16-20 s); do not loop, Waveshare specify ≥180 s between refreshes. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_uc8253_se0352n14
```

#### epdsi_uc8253_se0352n14_tri_epd

Full-parity companion to `epdsi_uc8253_se0352n14` — drawn entirely through `PageBufferPair`/`TriColor`; uses `PlanePolarity::UC8253` (the one panel where *both* planes are inverted). Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_uc8253_se0352n14_tri_epd
```

#### epdsi_ssd1677_gdeq0426t82

Dalian Good Display GDEQ0426T82 4.26" Monochrome (800×480), `Ssd1677Controller`. Renders portrait (480×800, ribbon at bottom) via `DisplayRotation::Rotate270`; the panel's reversed gates are compensated in software inside `Ssd1677Controller` (transparent to callers). Full refresh, then a differential logo-swap loop, then a full-waveform cleanup pass. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1677_gdeq0426t82
```

#### epdsi_ssd1677_gdeq0426t82_gray4

Companion to `epdsi_ssd1677_gdeq0426t82` — drives the panel's 4-level grayscale mode. `GDEQ0426T82::GRAY4` is Adafruit_EPD-sourced, not Seeed/Good Display material, and — unlike `GDEY0266T90`'s single-pass Gray4 mode above — needs a **two-pass** refresh (`Ssd1677RefreshMode::Gray4Preclear` then `Gray4`, with an LUT/voltage-register reload between). Confirmed rendering four distinct gray levels correctly on this board. Wiring: DESPI-C02, above.

```bash
cargo run --example epdsi_ssd1677_gdeq0426t82_gray4
```

#### epdsi_pdi_e2266ks0c1

Pervasive Displays E2266KS0C1 2.66" Monochrome, `PervasiveBwController` (Driver C). Both Normal Full Refresh and Fast Differential Refresh. Wiring: EXT3-1/EPDK, above (J3 open).

```bash
cargo run --example epdsi_pdi_e2266ks0c1
```

#### epdsi_pdi_e2290ks0f1

Pervasive Displays E2290KS0F1 2.90" Monochrome (Driver F COG), `PervasiveBwController`. Both Normal Full Refresh and Fast Differential Refresh. Wiring: EXT3-1/EPDK, above (J3 open).

```bash
cargo run --example epdsi_pdi_e2290ks0f1
```

#### epdsi_pdi_e2154qs0f1

Pervasive Displays E2154QS0F1 1.54" Quad-Color (BWRY / Spectra-4, Driver F COG, 152×152), `PervasiveBwryController`. Bit-banged 3-wire OTP register read, 2bpp packed BWRY frame buffers (5,776 bytes). Wiring: EXT3-1/EPDK, above (J3 open).

```bash
cargo run --example epdsi_pdi_e2154qs0f1
```

#### epdsi_pdi_e2417qs0a3

Pervasive Displays E2417QS0A3 4.20" Quad-Color (BWRY / Spectra-4, Driver A COG, 400×300), `PervasiveBwryController`. Bit-banged 3-wire OTP register read, 2bpp packed BWRY frame buffers (30,000 bytes). Wiring: EXT3-1/EPDK, above (**J3 closed**).

```bash
cargo run --example epdsi_pdi_e2417qs0a3
```

### 1-Wire Examples

#### ds18b20

Reads temperature from a DS18B20 waterproof temperature sensor probe over a 1-Wire bus using Embassy. It utilizes a custom, cycle-accurate `PreciseDelay` implementation to achieve jitter-free sub-microsecond timing required by the 1-Wire protocol on the RP2350's Cortex-M33 core.

```bash
cargo run --example ds18b20
```

**Wiring Schematic:**

```text
                              Raspberry Pi Pico 2
                           +-----------------------+
                           |                       |
                           | [ ] 1      40 [ ] USB |
                           | [ ] 2      39 [ ]     |
                           | [ ] 3      38 [G]ND --+-------+ (black)
                           | [ ] 4      37 [ ]     |       |
                           | [ ] 5      36 [3]V3 --+---+   |
                           |  ...        ...       |   |   |
                           | [ ] 20     21 [ ] ----+---+---|---+ (white, GPIO16)
                           +-----------------------+   |   |   |
                                                       |   |   |
                                                       |   |   |
                   +-----------------------------+     |   |   |
                   |     DS18B20 Sensor / Probe  |     |   |   |
                   |      (Bottom/Flat Side)     |     |   |   |
                   |                             |     |   |   |
                   |     [GND]   [DAT]   [VCC]   |     |   |   |
                   +-------|-------|-------|-----+     |   |   |
                           |       |       |           |   |   |
                           |       +-------+--[5K1]----+   | (Pull-Up Resistor
                           |       |       |   Resistor    |  between DAT & VCC)
                           |       |       +---------------+ (red)
                           +-------|-----------------------+ (black)
                                   |
                                   +--------------------------- (white)
```

**Breadboard Layout:**

![DS18B20 Breadboard Wiring Layout](pico-DS18B20_bb.png)

**About DS18B20:**

The DS18B20 is a 1-Wire digital thermometer that provides 9-bit to 12-bit Celsius temperature measurements. It communicates over a 1-Wire bus, requiring only one data line (and ground) to interface with the microcontroller. It has a temperature range of -55°C to +125°C with ±0.5°C accuracy from -10°C to +85°C.

#### dht11

Reads temperature and humidity from a DHT11 sensor using the Embassy async framework. It utilizes the async API of the `dht-sensor` crate combined with our cycle-accurate `PreciseDelay` implementation.

```bash
cargo run --example dht11 --release
```

Note: Due to timing sensitivity of the DHT11 protocol during the bit-read phase, you must run this example in **release** mode.

**Wiring Schematic:**

```text
                     Raspberry Pi Pico 2             DHT11 Module
                   +---------------------+      +---------------------+
                   |                     |      |                     |
                   | GND (Pin 38) -------+----->| GND                 |
                   | 3V3 (Pin 36) -------+----->| VCC                 |
                   | GPIO16 (Pin 21) ----+----->| DAT (Data)          |
                   |                     |      |                     |
                   +---------------------+      +---------------------+
```

> [!IMPORTANT]
> **Pull-up Resistor:**
> - **If using a DHT11 module board:** It likely already has a built-in pull-up resistor. No extra component is needed.
> - **If using a bare 4-pin DHT11 sensor:** You must add an external 4.7kΩ to 10kΩ pull-up resistor between the DAT (Data) and VCC lines.

**About DHT11:**

The DHT11 is a basic, ultra low-cost digital temperature and humidity sensor. It uses a capacitive humidity sensor and a thermistor to measure the surrounding air, and spits out a digital signal on the data pin (no analog input pins needed). It has a temperature range of 0°C to 50°C (±2°C accuracy) and humidity range of 20% to 90% RH (±5% accuracy).

### Wi-Fi & Matter Examples

#### matter_wifi_light

Implements a Matter-compatible Wi-Fi light bulb using the rs-matter stack. It uses BLE for commissioning and Wi-Fi for network connectivity, allowing you to add the Pico 2 W directly into Apple Home, Google Home, or Home Assistant! When toggled from your smart home app, it turns an external LED on and off.

```bash
cargo run --example matter_wifi_light --release
```

**Provisioning in Home Assistant:**

1. Run the example on your Pico 2 W. It will begin advertising over Bluetooth.
2. Open the Home Assistant companion app on your smartphone.
3. Go to **Settings** -> **Devices & Services** -> **Add Integration** -> **Add Matter device**.
4. When prompted for a setup code, enter the default `3497-0112-332` (or scan the QR code link printed in the terminal logs).
5. Home Assistant will connect to the Pico 2 W over BLE, ask for your Wi-Fi credentials, and securely transmit them to the device.
   
   <img src="HAApp.jpg" width="300" alt="Home Assistant Provisioning">

6. The Pico 2 W will connect to your Wi-Fi network and immediately appear as a standard light bulb. You can use the Home Assistant interface to toggle the light on and off!

   <img src="HAToggle.png" width="300" alt="Home Assistant Toggle">

7. The external LED wired to your Pico 2 W will instantly mirror the state!

   ![Matter Wi-Fi Light Circuit](matter_wifi_light.jpg)

**Wiring Schematic:**

```text
           Raspberry Pi Pico 2 W                   External Components
         +------------------------+
         |                        |
         |         GP15 (Pin 20)  |---------[ 220-330 Ohm Resistor ]-----+
         |                        |                                      |
         |         GND (Pin 18)   |------------------[ LED - ] <---+     |
         |                        |                                |     |
         +------------------------+                     (Cathode / |     |
                                                         Short Leg) |     |
                                                                   |     |
                                                        [ LED + ] -+-----+
                                                        (Anode /
                                                         Long Leg)
```

### Basic GPIO Examples

#### blinky

Blinks an external LED connected to GPIO15. This is useful for boards like the Raspberry Pi Pico 2 W, where the onboard LED is connected to the wireless chip rather than a standard microcontroller GPIO.

```bash
cargo run --example blinky
```

**Wiring:** Same wiring as the `matter_wifi_light` example.
