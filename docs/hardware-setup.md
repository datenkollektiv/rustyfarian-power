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
Battery (+) ──┬─── 390 kΩ ───┬─── GPIO1 (ADC1_CH0)
              │              │
              └─── 100 kΩ ───┴─── GND
```

The on-board divider is always connected on the V3.1; GPIO37 (ADC_CTRL) does not gate it.
The firmware compensates with `divider_ratio` 5.05 in `BatteryConfig::heltec_v3()`, measured as metered pack voltage over raw pin millivolts, about 3 % above the textbook 4.9.
Recalibrate per the `heltec_v3()` doc comment when the board revision or ADC setup changes.

### Why 11 dB Attenuation

The ESP-IDF ADC1 attenuation setting controls the measurable input range:

| Attenuation | Approximate range |
|-------------|-------------------|
| 0 dB        | 0–950 mV          |
| 2.5 dB      | 0–1250 mV         |
| 6 dB        | 0–1750 mV         |
| 11 dB       | 0–3100 mV         |

A full 4200 mV battery produces about 830 mV at GPIO1, well inside every range.
Both tiers select 11 dB attenuation (ESP-IDF names the same setting `DB_12`), which leaves headroom for the charger rail seen with USB in and no pack.
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
use rustyfarian_esp_idf_power::{EspSleepManager, SleepManager, WakeSource, GpioWakeLevel};

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

The stock Feather V2 does not route the charger STAT or VBUS signals to GPIO13 or GPIO34.
Read the Feather V2 entry in `docs/project-lore.md` before wiring the charging monitor.

`EspChargingMonitor::new` takes the STAT and VBUS pins:

```rust
let mut charging = EspChargingMonitor::new(
    peripherals.pins.gpio13,
    peripherals.pins.gpio34,
    ChargingSource::Usb,
)?;
```

### Deep Sleep

`EspSleepManager` supports the original ESP32; `examples/idf_esp32_deep_sleep_gpio.rs` uses it for timer and EXT1 wake.
The Feather V2 battery example, `examples/idf_esp32_battery.rs`, calls the ESP-IDF timer sleep API directly.

---

## Persisting State Across Deep Sleep

Deep sleep clears all RAM except the RTC memory.
Place a `RetainedCell` in `.rtc_noinit` so the record also survives watchdog and software resets, not only a sleep/wake cycle.
The section is `NOLOAD`: the initialiser is never applied, and the cell validates the record on every `load` instead.
Read the reset reason first; `load` refuses the record after a power-on or brownout, when the memory holds arbitrary or partial data.
The cell serialises its callers in a critical section, so tasks and interrupts may share it once the firmware links an interrupt-safe provider.
Enable the opt-in `critical-section-impl` feature of `rustyfarian-esp-idf-power` for that, unless another crate already links a provider.

```rust
use rustyfarian_esp_idf_power::{EspResetReasonSource, ResetReasonSource, RetainedCell};

#[link_section = ".rtc_noinit"]
static BOOTS: RetainedCell<4> = RetainedCell::new();

fn main() {
    let reset = EspResetReasonSource.last_reset_reason();
    let boots = BOOTS.load::<u32>(reset).unwrap_or(0).wrapping_add(1);
    BOOTS.store(&boots).expect("4 bytes fit in RetainedCell<4>");
}
```

On the ESP32-C3 the section lives in the 8 KB RTC fast memory at `0x50000000`; on the ESP32 and ESP32-S3 it lives in RTC slow memory.
ESP-IDF keeps that memory powered during deep sleep.
The example `idf_c3_rtc_retention` walks a boot counter through timer wake, software, panic and watchdog resets.
