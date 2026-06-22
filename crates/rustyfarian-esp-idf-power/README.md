# rustyfarian-esp-idf-power

ESP-IDF (std) battery monitoring, charging detection, and deep-sleep drivers for
ESP32 and ESP32-S3.

This is the hardware tier of the
[rustyfarian-power](https://github.com/datenkollektiv/rustyfarian-power) stack.
It provides the ESP-IDF implementations of the traits defined in the
platform-agnostic [`stoker`](https://crates.io/crates/stoker) crate, and
re-exports `stoker`'s public surface so firmware needs only one import.

## Drivers

- `EspAdcBatteryMonitor` — battery voltage via the ESP-IDF ADC oneshot driver
- `EspSleepManager` — deep sleep with configured wake sources
- `EspWakeCauseSource` — the reason for the last wake
- `EspChargingMonitor` — charging state from MCP73831 STAT + USB VBUS pins

## Supported boards / examples

| Board                     | Chip     | Example               |
|:--------------------------|:---------|:----------------------|
| Heltec WiFi LoRa 32 V3.1  | ESP32-S3 | `idf_esp32s3_battery` |
| Adafruit ESP32 Feather V2 | ESP32    | `idf_esp32_battery`   |

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
