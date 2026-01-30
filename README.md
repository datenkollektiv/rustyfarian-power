# Rustyfarian Battery Power Management

A Rust library for battery voltage monitoring on ESP32 microcontrollers, targeting the **Heltec WiFi LoRa 32 V3**.
Provides battery percentage estimation, power source detection (battery vs USB/external), and configurable voltage thresholds.

## Features

- Battery voltage reading via ADC with voltage divider compensation
- Linear interpolation for battery percentage (0-100%)
- Power source detection: Battery, USB/External, or Unknown
- Configurable thresholds (min/max voltage, USB detection, sample count)
- Hardware-independent core logic behind a `BatteryMonitor` trait for testability

## Prerequisites

This project cross-compiles for the `xtensa-esp32s3-espidf` target using Espressif's custom Rust toolchain (`esp` channel).
See `rust-toolchain.toml` for toolchain configuration.

## Shell Commands

Build the workspace:

```shell
cargo build
```

Run tests (host target, no ESP-IDF dependency):

```shell
cargo test -p battery-monitor --no-default-features --target aarch64-apple-darwin
```

Run a single test:

```shell
cargo test -p battery-monitor --no-default-features --target aarch64-apple-darwin <test_name>
```

Check without building:

```shell
cargo check
```

## Crate Structure

| Module       | Description                                                                           |
|:-------------|:--------------------------------------------------------------------------------------|
| `lib.rs`     | Public API: `PowerSource`, `BatteryStatus`, `BatteryMonitor` trait, `is_sufficient()` |
| `config.rs`  | `BatteryConfig` with voltage thresholds, `voltage_to_percent()`, `evaluate_reading()` |
| `esp_adc.rs` | `EspAdcBatteryMonitor` — ESP-IDF ADC implementation (feature-gated behind `esp-idf`)  |

## License

Licensed under either of [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0) or [MIT License](http://opensource.org/licenses/MIT), at your option.
