# AGENTS.md

> Fast-path operating guide for AI coding agents on this project.
> Prefer repository truth over assumptions — check the files referenced below.

## Project Overview

Rustyfarian Power is a Rust workspace for battery monitoring and power management on ESP32 microcontrollers, targeting the Heltec WiFi LoRa 32 V3 (ESP32-S3) and Adafruit ESP32 Feather V2.
It is built for low-power firmware loops: read battery state, decide whether to transmit, enter deep sleep, repeat.

## Architecture

Two-crate Cargo workspace (`members = ["crates/*"]`), split by a **crate boundary** — not a feature flag. The pure/hardware separation is what lets the core build and test on the host with no ESP toolchain.

**`crates/stoker`** — platform-agnostic core, host-buildable, depends only on `anyhow`:
- `lib.rs` — `BatteryMonitor` trait, `PowerSource`, `BatteryStatus`, `NoopBatteryMonitor`
- `config.rs` — `BatteryConfig` with board presets (`heltec_v3()`, `adafruit_feather_v2()`) and `evaluate_reading()` — all voltage/percentage logic
- `sleep.rs` — `SleepManager` + `WakeCauseSource` traits, `WakeCause`/`WakeSource` enums, `validate_wake_sources()` / `validate_gpio_level_source()`, `NoopSleepManager`
- `charging.rs` — `ChargingMonitor` trait, `ChargingState`, `ChargingSource`, `NoopChargingMonitor`

**`crates/rustyfarian-esp-idf-power`** — ESP-IDF (std) hardware tier; cannot compile on the host (needs the Xtensa/ESP-IDF toolchain):
- `esp_adc.rs` — `EspAdcBatteryMonitor` (ADC1 averaging + divider compensation)
- `esp_sleep.rs` — `EspSleepManager`, `EspWakeCauseSource` (deep sleep, wake-cause read)
- `esp_charging.rs` — `EspChargingMonitor` (MCP73831 STAT + USB VBUS pins)
- `lib.rs` re-exports all of `stoker`, so device firmware imports from this one crate.

Every hardware concern is behind a trait, and every trait ships a `Noop*` mock for host tests. Business logic lives in `stoker` (`evaluate_reading()`, the `validate_*` fns) — hardware-independent and fully unit-tested.

## Development Workflow

Requires the Espressif `esp` Rust toolchain (installed via `espup`); ESP-IDF is pinned to v5.3.3. The workspace defaults to the `xtensa-esp32s3-espidf` target via `.cargo/config.toml`, so host recipes pass `--target` explicitly and scope to `-p stoker`. Use `just` for all operations:

```shell
just check          # check the pure stoker crate on the host — no ESP toolchain
just test           # host-side unit tests — no ESP toolchain
just check-all      # check everything incl. ESP-IDF crate (requires espup)
just check-esp32    # check the ESP-IDF crate for the ESP32 / Feather V2 target
just verify         # non-modifying gate: fmt-check, check, clippy, test
just pre-commit     # fmt, check, clippy, test (modifies files — local only)
just build-example <name>   # chip inferred from the idf_{chip}_{name} prefix
just run <name>             # build, flash, and open the serial monitor
```

Run `just` with no arguments to list all recipes. Building the ESP32 (Feather V2) target additionally requires the `MCU=esp32` env var (esp-idf-sys reads it). ESP-IDF builds are isolated under `target/idf`; an optional macOS RAM disk at `/Volumes/RustBuilds` speeds incremental builds.

## Key Conventions

**Crate boundary is the host/hardware line.** Anything host-compilable goes in `stoker`; anything touching `esp-idf-hal`/`esp-idf-sys` goes in `rustyfarian-esp-idf-power`. Keeping logic in `stoker` is what makes `just test` work toolchain-free.

**Trait-first:** All hardware reads sit behind a trait; supporting a new board means implementing the trait, not editing logic. Follow the `Noop*` mock pattern for any new trait.

**Board presets:** Start from `BatteryConfig::heltec_v3()` or `adafruit_feather_v2()` — each encodes a calibrated divider ratio and ADC setup. `heltec_v3`'s `divider_ratio` is empirical (folds in ADC loading), not the textbook figure.

**Error handling:** `anyhow::Result` with `.context()`; no `.unwrap()` outside tests. Log via `log::info!/warn!/error!`.

**`is_sufficient` fallback:** `BatteryStatus::is_sufficient()` intentionally returns `true` for `External` and `Unknown` — never block operations when battery state is unclear.

**`EspWakeCauseSource` is a unit struct:** `EspWakeCauseSource.last_wake_cause()` is constructor + call in one expression. Read it early in `main()`, before peripheral init.

**Docs:** one sentence per line (keeps diffs clean); use ` ```shell ` fences and keep comments out of code snippets.

## Coding Principles

- **State assumptions** before starting. If a task has multiple valid interpretations, present them rather than picking silently.
- **Simplicity first.** Minimum code that solves the problem. No features beyond what was asked. No abstractions for single-use code. No error handling for impossible scenarios.
- **Surgical changes.** Touch only what the task requires. Do not improve adjacent code, comments, or formatting. Every changed line should trace directly to the user's request.
- When your changes create orphans (unused imports, variables, functions), remove them. Do not remove pre-existing dead code unless asked.

## Important Files

- `crates/stoker/src/lib.rs` — public API, trait definitions, Noop mocks, usage examples in doc comments
- `crates/stoker/src/config.rs` — board presets and voltage conversion logic
- `docs/key-insights.md` — non-obvious hardware behaviour, build quirks, resolved gotchas; read before any non-trivial task
- `docs/hardware-setup.md` — GPIO wiring tables and power budgets for Heltec V3 and Feather V2
- `crates/rustyfarian-esp-idf-power/examples/idf_esp32_battery.rs` — complete Feather V2 example: wake-cause detection, ADC read, charging state, deep sleep
- `release-plan.md` — staged two-crate publish sequence (`stoker` first, then the ESP-IDF crate)
