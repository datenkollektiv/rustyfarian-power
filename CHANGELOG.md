# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project has not yet adopted semantic versioning; entries are grouped by milestone.

---

## [Unreleased]

### Changed
- ROADMAP: marked Philosophy Compliance Sprint and Feather V2 charging monitor as done
- ROADMAP: updated Milestone 4 to reflect what was shipped vs what remains
- README: added `charging.rs` and `esp_charging.rs` to the crate structure table
- `hal-designer` agent: removed stale references to wrong ESP32 targets and non-existent crates

---

## [Milestone 2+] — Charging Monitor, Feather V2 Example, and Philosophy Sprint

*Commits: `6c10a00`, `9c505f0`, `35c7206`*

### Added
- `charging.rs`: `ChargingMonitor` trait, `ChargingState` enum (`Charging { source }`, `Full`, `NoBattery`, `Unknown`), `ChargingSource` enum (`Usb`, `Solar`)
- `esp_charging.rs`: `EspChargingMonitor` — reads MCP73831 STAT (GPIO13) and USB VBUS detect (GPIO34) to resolve four charging states
- `examples/idf_esp32_battery.rs`: full Adafruit ESP32 Feather V2 example — wake-cause detection, battery ADC, charging state, 60 s deep sleep
- `BatteryConfig::adafruit_feather_v2()` preset for the original ESP32 / Feather V2 board (GPIO35, ADC1_CH7, 100 kΩ + 100 kΩ divider)
- `NoopBatteryMonitor` with `on_battery(mv, pct)`, `on_external()`, `unknown()` constructors
- `NoopChargingMonitor` with `charging(source)`, `full()`, `no_battery()`, `unknown()` constructors
- `NoopSleepManager::with_cause(WakeCause)` constructor — enables testing of consumer code that branches on wake cause
- `validate_wake_sources()` extracted to `sleep.rs` — now host-testable without the `esp-idf` feature
- ADC calibration: `Calibration::Line` selected for ESP32 (original) at 11 dB to correct severe non-linearity; `Calibration::None` for ESP32-S3
- `build.rs`: emits `cargo:rustc-cfg=esp32` and `cargo:rustc-cfg=esp32s3` from `TARGET` env var, plus `cargo:rustc-check-cfg` to suppress unexpected-cfg lint
- `.cargo/config.toml.dist` supports both `xtensa-esp32s3-espidf` (Heltec V3) and `xtensa-esp32-espidf` (Feather V2) targets
- `docs/key-insights.md`: ADC calibration and GPIO configuration insights from Feather V2 bringup
- `docs/hardware-setup.md`: Feather V2 wiring, charging pin table, and deep sleep workaround note

### Changed
- `BatteryStatus`: now derives `Copy` (all fields are `Copy`; `Clone`-only forced unnecessary clones)
- `WakeCause`: split `Gpio(GpioWakeMask)` into `Ext1(GpioWakeMask)`, `Ext0` (single-pin, no mask), `Gpio` (ESP32-S3 deep-sleep GPIO, no mask) — eliminates ambiguous mask==0 case
- `EspSleepManager::sleep()`: `GpioLevel` + `isolate_gpio: true` is now a hard error (fail-fast over silent misconfiguration)
- `esp_sleep.rs`: uses `#[cfg(esp32)]` to select `ESP_EXT1_WAKEUP_ALL_LOW` (original ESP32) vs `ESP_EXT1_WAKEUP_ANY_LOW` (ESP32-S3)
- `justfile`: added `check-esp32`, `check-docs-esp32`, `build-example` (chip-inferred), `flash`, `run`, `verify`, `ci` recipes

---

## [Milestone 2] — Deep Sleep: GPIO Wake Sources

*Commit: `2e27caf`*

### Added
- `WakeSource::GpioLevel { pin_mask: u64, level: GpioWakeLevel }` variant
- `GpioWakeLevel` enum: `AnyHigh`, `AnyLow`
- `GpioWakeMask(pub u64)` newtype with `contains_pin(pin: u8) -> bool` helper
- `validate_gpio_level_source(pin_mask: u64) -> anyhow::Result<()>` — pure host-testable validation
- `EspSleepManager::sleep()` handles `GpioLevel` via `esp_sleep_enable_ext1_wakeup_io()` (ESP-IDF v5.x preferred API)
- 6 host-side tests for `validate_gpio_level_source` boundary conditions
- `WakeCause::Ext1(GpioWakeMask)`, `Ext0`, `Gpio` variants
- `docs/hardware-setup.md`: GPIO wake pin requirements (external pull resistors, `isolate_gpio: false`)

---

## [Milestone 1] — Deep Sleep: Timer Wake

*Commit: `aea4645`*

### Added
- `sleep.rs`: `SleepManager` trait, `WakeCauseSource` trait, `WakeCause` enum, `WakeSource::Timer` variant, `NoopSleepManager`
- `esp_sleep.rs`: `EspSleepManager` (timer wake, GPIO isolation, `esp_deep_sleep_start()`), `EspWakeCauseSource`
- `EspSleepManager::isolate_gpio` field: calls `esp_sleep_config_gpio_isolate()` when `true` (default)
- All previously configured wake sources cleared via `esp_sleep_disable_wakeup_source(ALL)` at sleep entry for deterministic behaviour
- `docs/hardware-setup.md`: Heltec V3 ADC path, attenuation table, deep sleep power budget

---

## [Initial] — Battery Monitor Foundation

*Commits: `452e286`, `aa3b1ef`*

### Added
- `battery-monitor` crate: `BatteryMonitor` trait, `BatteryStatus` struct, `PowerSource` enum
- `BatteryConfig` with `evaluate_reading()`, `voltage_to_percent()` (linear interpolation), `heltec_v3()` preset
- `EspAdcBatteryMonitor`: ADC1 oneshot driver with averaged sampling (configurable sample count), 11 dB attenuation, voltage divider compensation
- `is_sufficient(min_voltage_mv, min_percent)`: graceful fallback — returns `true` for `External` and `Unknown` sources
- `Display` impl for `BatteryStatus` and `PowerSource`
- Host-side unit tests covering voltage-to-percent, `evaluate_reading`, and `is_sufficient` boundary cases
