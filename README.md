# Rustyfarian Power Management

A Rust library for power management on ESP32 microcontrollers, targeting the **Heltec WiFi LoRa 32 V3**.
Powers the rustyfarian ecosystem's battery-driven field deployments — from battery monitoring to deep sleep and radio power gating.

## Vision

> Give every rustyfarian application on ESP32 a single, ergonomic power management layer so battery-powered field deployments run reliably for months without intervention.

**We are building this for:** developers building battery-powered IoT applications in the rustyfarian ecosystem (e.g., remote beehive monitoring via LoRaWAN)

**Long-term goals:**
- Deep sleep with configurable wake-up sources as first-class primitives
- Radio power gating that coordinates cleanly with `rustyfarian-network` crates
- Solar-assisted deployment support via charging/boost input awareness

**Out of scope:** Wi-Fi, MQTT, and LoRaWAN protocol logic — these belong in `rustyfarian-network`.

*Full vision, success signals, and open questions: [VISION.md](./VISION.md)*

## Features

- Battery voltage reading via ADC with voltage divider compensation
- Linear interpolation for battery percentage (0–100%)
- Power source detection: Battery, USB/External, or Unknown
- Configurable thresholds (min/max voltage, USB detection, sample count)
- Deep sleep with timer wake and deterministic wake-cause detection
- Hardware-independent core logic behind traits for full host-side testability

## Prerequisites

This project cross-compiles for the `xtensa-esp32s3-espidf` target using Espressif's custom Rust toolchain (`esp` channel).
See `rust-toolchain.toml` for toolchain configuration.

First-time setup:

```shell
just setup-toolchain
just setup-cargo-config
```

## Common Tasks

Run `just` with no arguments to list all available recipes.

Check platform-independent code (no ESP toolchain required):

```shell
just check
```

Check all code including ESP-IDF implementations (requires espup):

```shell
just check-all
```

Run host-side tests:

```shell
just test
```

Run a single test by name:

```shell
just test-one <test_name>
```

Format, check, lint, and test in one step:

```shell
just pre-commit
```

## Crate Structure

| Module          | Description                                                                            |
|:----------------|:---------------------------------------------------------------------------------------|
| `lib.rs`        | Public API: `PowerSource`, `BatteryStatus`, `BatteryMonitor` trait, `is_sufficient()` |
| `config.rs`     | `BatteryConfig` with voltage thresholds, `voltage_to_percent()`, `evaluate_reading()` |
| `sleep.rs`      | `SleepManager`, `WakeCauseSource` traits; `WakeCause`, `WakeSource` enums; `NoopSleepManager` |
| `esp_adc.rs`    | `EspAdcBatteryMonitor` — ESP-IDF ADC implementation (feature-gated behind `esp-idf`)  |
| `esp_sleep.rs`  | `EspSleepManager`, `EspWakeCauseSource` — ESP-IDF deep sleep implementation (feature-gated) |

## License

Licensed under either of [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0) or [MIT License](http://opensource.org/licenses/MIT), at your option.
