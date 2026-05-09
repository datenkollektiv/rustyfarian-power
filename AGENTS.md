# AGENTS.md

> Fast-path operating guide for AI coding agents on this project.
> Prefer repository truth over assumptions — check the files referenced below.

## Project Overview

`battery-monitor` is a Rust library for battery voltage monitoring and power management on ESP32 microcontrollers, targeting the Heltec WiFi LoRa 32 V3 (ESP32-S3) and Adafruit ESP32 Feather V2.
It is designed for low-power firmware loops: read battery state, decide whether to transmit, enter deep sleep, repeat.

## Architecture

Single crate: `crates/battery-monitor/`. Two layers split by the `esp-idf` feature flag.

**Core — always compiled, host-testable (`--no-default-features`):**
- `lib.rs` — `BatteryMonitor` + `ChargingMonitor` traits, `PowerSource`, `BatteryStatus`, `Noop*` mocks
- `config.rs` — `BatteryConfig` with board presets (`heltec_v3()`, `adafruit_feather_v2()`) and `evaluate_reading()`
- `sleep.rs` — `SleepManager` + `WakeCauseSource` traits, `WakeCause`/`WakeSource` enums, `NoopSleepManager`
- `charging.rs` — `ChargingMonitor` trait, `ChargingState`, `ChargingSource`, `NoopChargingMonitor`

**ESP-IDF implementations — feature `esp-idf` (default):**
- `esp_adc.rs` — ADC1 reading with averaging and voltage divider compensation
- `esp_sleep.rs` — deep sleep with timer and GPIO wake sources; `EspWakeCauseSource` reads wake reason
- `esp_charging.rs` — MCP73831 STAT pin + USB VBUS detect GPIO

Every hardware concern is behind a trait. Every trait has a `Noop*` mock for host-side testing.
Business logic lives in `BatteryConfig::evaluate_reading()` — hardware-independent and fully unit-tested.

## Development Workflow

Requires the Espressif `esp` Rust toolchain (installed via `espup`). Use `just` for all operations:

```shell
just check          # check platform-independent code — no ESP toolchain needed
just test           # run host-side unit tests — no ESP toolchain needed
just check-all      # check everything including ESP-IDF (requires ESP toolchain)
just build-all      # full build for the ESP32 target
just verify         # non-modifying full verification: fmt-check, deny, check, lint, test
just pre-commit     # full verification with auto-formatting (modifies files)
just build-example <name>   # build a named example inferred from the idf_{chip}_{name} prefix
just run <name>             # build, flash, and open serial monitor
```

Run `just` with no arguments to list all recipes. Host-side tests use `--no-default-features` and require no ESP toolchain. The `esp` toolchain is only needed for `check-all`, `build-all`, `build-example`, and `flash`.

## Key Conventions

**Trait-first:** All hardware reads are behind a trait. Adding a new board means implementing the trait, not changing business logic. See `NoopBatteryMonitor` and `NoopChargingMonitor` for the mock pattern.

**Feature boundary:** Code compilable on the host must not be inside `#[cfg(feature = "esp-idf")]`. This boundary is what makes `just test` work without the ESP toolchain.

**Error handling:** `anyhow::Result` with `.context()` for fallible operations. No `.unwrap()` outside tests. Log with `log::info!`, `log::warn!`, `log::error!`.

**Board presets:** Use `BatteryConfig::heltec_v3()` or `BatteryConfig::adafruit_feather_v2()` — both are calibrated for each board's voltage divider ratio and ADC characteristics. Start from a preset when targeting a new board.

**`is_sufficient` fallback:** `BatteryStatus::is_sufficient()` intentionally returns `true` for `External` and `Unknown` sources — do not block operations when battery state is unclear.

**`EspWakeCauseSource` is a unit struct:** `EspWakeCauseSource.last_wake_cause()` is both a constructor and a method call in one expression. Call it early in `main()`, before peripheral initialisation — the EXT1 status register is hardware-preserved until the next sleep entry.

## Important Files

- `crates/battery-monitor/src/lib.rs` — public API, trait definitions, Noop mocks, usage examples in doc comments
- `crates/battery-monitor/src/config.rs` — board presets and voltage conversion logic
- `docs/key-insights.md` — non-obvious hardware behaviour, build quirks, and resolved gotchas; read before starting any non-trivial task
- `docs/hardware-setup.md` — GPIO wiring tables for Heltec V3 and Feather V2
- `crates/battery-monitor/examples/idf_esp32_battery.rs` — complete Feather V2 example: wake-cause detection, ADC read, charging state, deep sleep
