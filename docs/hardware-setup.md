# Hardware Setup Guide

This guide covers the physical wiring, ADC configuration, and power budget for running
`rustyfarian-power` on supported boards.

Two boards are used for development and testing:

- **Heltec WiFi LoRa 32 V3** — ESP32-S3, primary target
- **Adafruit ESP32 Feather V2** — original ESP32, secondary target

---

## Heltec WiFi LoRa 32 V3

### Target Board

**Heltec WiFi LoRa 32 V3** (ESP32-S3 based).
This is the primary target and the board the library defaults are calibrated for.
Use `BatteryConfig::heltec_v3()` for this board.

### Battery ADC Path

The battery voltage path on Heltec V3:

```
Battery (+) ──┬─── 1 MΩ ───┬─── GPIO1 (ADC1_CH0)
              │             │
              └─── 1 MΩ ───┴─── GND
```

A 1 MΩ + 1 MΩ resistor divider halves the battery voltage before it reaches the ADC pin.
The firmware compensates with `divider_ratio: 2.0` in `BatteryConfig`.

### Why 11 dB Attenuation

The ESP-IDF ADC1 attenuation setting controls the measurable input range:

| Attenuation | Approximate range |
|-------------|-------------------|
| 0 dB        | 0–950 mV          |
| 2.5 dB      | 0–1250 mV         |
| 6 dB        | 0–1750 mV         |
| 11 dB       | 0–3100 mV         |

The 2:1 divider halves the battery voltage: a full 4200 mV battery produces ~2100 mV at GPIO1.
11 dB attenuation covers the full 0–2100 mV raw signal with comfortable headroom.
Lower attenuation settings would clip readings above ~1750 mV.

### Deep Sleep Power Budget

Approximate current draw in deep sleep on Heltec V3:

| Component                |         Current |
|:-------------------------|----------------:|
| ESP32-S3 chip (RTC only) |           ~7 µA |
| LDO regulator            |       ~50–80 µA |
| LoRa radio (SX1262)      |         ~1.5 µA |
| Charge controller (BQ)   |          ~50 µA |
| **Board total**          | **~110–150 µA** |

At 3700 mV / 1000 mAh: approximately 270–380 days theoretical sleep-only runtime.
Real runtime is lower due to active wake windows.

### GPIO Wake Pin Requirements

GPIO wake sources use the ESP32-S3 EXT1 mechanism.
Only RTC GPIOs 0–21 are valid for EXT1 wakeup.

#### External pull resistors are required

RTC internal pull-up/pull-down resistors are unavailable during deep sleep when
`RTC_PERIPH` is powered down (the default).
A floating pin will have an indeterminate HOLD state and may false-trigger or fail to trigger.

Use 10–100 kΩ external resistors:

- `GpioWakeLevel::AnyHigh` — external pull-down (pin LOW at rest; external signal pulls HIGH).
- `GpioWakeLevel::AnyLow` — external pull-up (pin HIGH at rest; external signal pulls LOW).

#### `isolate_gpio: false` required with GPIO wake sources

`EspSleepManager` defaults to `isolate_gpio: true`, which calls
`esp_sleep_config_gpio_isolate()` before sleep.
GPIO isolation prevents floating digital pins from leaking current — a common missed
optimisation.

However, GPIO isolation may prevent wake-capable pins from triggering.
When using `WakeSource::GpioLevel`, construct `EspSleepManager` with `isolate_gpio: false`:

```rust
use battery_monitor::{EspSleepManager, SleepManager, WakeSource, GpioWakeLevel};

EspSleepManager { isolate_gpio: false }
    .sleep(&[WakeSource::GpioLevel {
        pin_mask: 1u64 << 4,
        level: GpioWakeLevel::AnyLow,
    }])?;
```

---

## Adafruit ESP32 Feather V2

### Target Board

**Adafruit ESP32 Feather V2** (original ESP32, Xtensa LX6 dual-core).
Use `BatteryConfig::adafruit_feather_v2()` for this board.
Build with `MCU=esp32 cargo ...` and target `xtensa-esp32-espidf`.

### Battery ADC Path

No external wiring is needed.
The Feather V2 routes the LiPo battery through an onboard 100 kΩ + 100 kΩ voltage divider
to **GPIO35** (ADC1_CH7).

```
LiPo+ ──[100 kΩ]──┬──[100 kΩ]── GND
                  └──► GPIO35 (ADC1_CH7)
```

The firmware compensates with `divider_ratio: 2.0` in `BatteryConfig::adafruit_feather_v2()`.

### Charging Monitor Pins

The Feather V2 exposes two pins for charge state detection:

| Pin    | Function                   | Notes                                        |
|:-------|:---------------------------|:---------------------------------------------|
| GPIO13 | MCP73831 STAT (open-drain) | Board has external 4.7 kΩ pull-up            |
| GPIO34 | USB VBUS detect            | 100 kΩ + 100 kΩ divider; input-only on ESP32 |

Pass these to `EspChargingMonitor`:

```rust
let mut charging = EspChargingMonitor::new(
    peripherals.pins.gpio13,
    peripherals.pins.gpio34,
    ChargingSource::Usb,
)?;
```

### Deep Sleep

`EspSleepManager` targets ESP32-S3 and uses APIs that are absent on the original ESP32
(`esp_sleep_config_gpio_isolate()`, `ESP_EXT1_WAKEUP_ANY_HIGH`).
The Feather V2 example calls the ESP-IDF timer sleep API directly as a workaround.
See `examples/idf_esp32_battery.rs` for the current implementation and the inline TODO
for what to replace it with once `esp_sleep.rs` is verified to compile for
`xtensa-esp32-espidf`.

---

## Persisting State Across Deep Sleep

Deep sleep clears all RAM except the RTC slow memory.
Place variables in the RTC domain to retain them across a sleep/wake cycle:

```rust
#[link_section = ".rtc.data"]
static mut BOOT_COUNT: u32 = 0;
```
