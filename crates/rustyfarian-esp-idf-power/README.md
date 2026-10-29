# rustyfarian-esp-idf-power

ESP-IDF (std) battery monitoring, charging detection, and deep-sleep drivers for
ESP32, ESP32-S3 and ESP32-C3.

This is the hardware tier of the
[rustyfarian-power](https://github.com/datenkollektiv/rustyfarian-power) stack.
It provides the ESP-IDF implementations of the traits defined in the
platform-agnostic [`stoker`](https://crates.io/crates/stoker) crate, and
re-exports `stoker`'s public surface so firmware needs only one import.

## Drivers

- `EspAdcBatteryMonitor` — battery voltage via the ESP-IDF ADC oneshot driver
- `EspSleepManager` — deep sleep with configured wake sources
- `EspWakeCauseSource` — the reason for the last wake
- `EspResetReasonSource` — the reason for the last reset, which gates `RetainedCell::load`
- `EspRtcClock` — the RTC timer in microseconds, for slept-time accounting across deep sleep
- `EspChargingMonitor` — charging state from MCP73831 STAT + USB VBUS pins

## Features

The opt-in feature `critical-section-impl` provides the firmware's `critical-section` implementation over the FreeRTOS port critical section, which `RetainedCell` needs to exclude interrupts and the other core.
A firmware links exactly one provider, and `RetainedCell` is only as safe as that provider.
Enable this feature unless the graph already links a provider; that provider must then also exclude interrupts and the other core before a cell is shared with an ISR.
esp-idf-hal's `critical-section` feature (enabled, for example, by `rustyfarian-esp-idf-network`'s `lora`) is a task-level mutex that does not.
Leaving this feature off avoids the duplicate symbols but makes `RetainedCell` safe between tasks only; resolve the feature conflict before an interrupt touches the cell.

```toml
rustyfarian-esp-idf-power = { git = "https://github.com/datenkollektiv/rustyfarian-power", features = ["critical-section-impl"] }
```

## Supported boards / examples

| Board                     | Chip     | Example                  |
|:--------------------------|:---------|:-------------------------|
| Heltec WiFi LoRa 32 V3.1  | ESP32-S3 | `idf_esp32s3_battery`    |
| Adafruit ESP32 Feather V2 | ESP32    | `idf_esp32_battery`      |
| ESP32-C3-DevKitM-1        | ESP32-C3 | `idf_c3_deep_sleep_gpio` |
| ESP32-C3-DevKitM-1        | ESP32-C3 | `idf_c3_rtc_retention`   |
| CrowPanel 1.28" HMI       | ESP32-S3 | `idf_s3_deep_sleep_gpio` |

```rust,ignore
use rustyfarian_esp_idf_power::{BatteryConfig, BatteryMonitor, EspAdcBatteryMonitor};

let peripherals = esp_idf_hal::peripherals::Peripherals::take()?;
let mut battery = EspAdcBatteryMonitor::new(
    peripherals.adc1,
    peripherals.pins.gpio1,
    BatteryConfig::heltec_v3(),
)?;
let status = battery.read();
```

## Known limitations

- `EspChargingMonitor::new()` requires the `STAT` pin to satisfy
  `InputPin + OutputPin`; an input-only GPIO (e.g. Feather V2 GPIO34) cannot be
  used for STAT.
- `BatteryConfig::heltec_v3()` carries an empirical, per-unit `divider_ratio`
  (5.55) — verify the reported voltage against a multimeter and adjust.
- **docs.rs:** this crate is unlikely to build on docs.rs — `esp-idf-sys`
  requires network access and the ESP-IDF C toolchain. This README is the
  primary documentation; the canonical build target is `xtensa-esp32s3-espidf`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
