# AGENTS.md

Embassy async examples for the Raspberry Pi Pico 2 (RP2350, Cortex-M33). `no_std`, `no_main`, logs via `defmt`/RTT.

## Build & CI verification (run before committing)

Local checks mirror `.github/workflows/rust_ci.yml` and must pass before push:

```bash
./download_firmware.sh                                  # required once before building (see below)
cargo fmt --all -- --check                              # or `cargo fmt` to fix
cargo check --examples
cargo clippy --examples -- -D warnings
cargo build --release && cargo build --release --examples
```

- Requires the `thumbv8m.main-none-eabihf` target and the `rust-src` component installed.
- `./download_firmware.sh` fetches CYW43439 firmware into `cyw43-firmware/` (gitignored). Wi-Fi/Matter examples fail to build without it.
- Dependencies pin `[patch.crates-io]` and git deps (`rs-matter`, `rs-matter-stack`, `openthread`, `rs-matter-embassy`) to specific fork branches — builds need network and must not be "upgraded" casually.

## Running an example

`cargo run --example <name>` flashes via SWD using `probe-rs run --chip RP235x --protocol swd` (configured in `.cargo/config.toml`), not picotool. A debug probe (e.g. picoprobe) must be attached.

- `dht11` and `matter_wifi_light` must be run with `--release` (timing/size sensitive).
- Example names, wiring, and sensor/display details are documented in `README.md`.

## Structure conventions

- This is a **single Cargo package** (edition 2024), not a workspace despite README wording. Each example is an independent `--example` binary target in `examples/`.
- Sensor and display driver crates live under `[dev-dependencies]`, not `[dependencies]` — add new example drivers there.
- Two display stacks coexist, distinguished by example filename: async `display-driver` (`*_dd_*`) vs `mipidsi` (`*_mipi_*`). Match the existing pattern for the chosen stack.
- `build.rs` codegens `ui/appwindow.slint` via `slint-build` (software renderer); the Slint example depends on this.
- 1-Wire (`ds18b20`) and `dht11` rely on a cycle-accurate custom `PreciseDelay` for protocol timing.

## Environment quirks

- `DEFMT_LOG = "debug"` is set in `.cargo/config.toml`; log output is viewed over RTT (probe-rs).
- `[profile.dev]` uses `opt-level = 2` — dev builds are already optimized (timing-sensitive code).

## Testing

- No host-runnable unit tests. Verification means flashing to real Pico 2 / Pico 2 W hardware.
- Do not commit changes for an example until it has been run and verified on real hardware.
