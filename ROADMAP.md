# Roadmap

> **North star:** Give every rustyfarian application on ESP32 a single, ergonomic power management layer so battery-powered field deployments run reliably for months without intervention.

See [VISION.md](./VISION.md) for goals, success signals, and non-goals.

---

## Current State

The `battery-monitor` crate is the foundation.
It provides battery voltage reading, percentage estimation, and power source detection via a hardware-independent `BatteryMonitor` trait backed by an ESP-IDF ADC implementation.

**What exists:**
- `BatteryMonitor` trait + `BatteryStatus` struct + `PowerSource` enum
- `BatteryConfig` with `evaluate_reading()` and `voltage_to_percent()`
- `EspAdcBatteryMonitor` (ADC1/GPIO1, feature-gated behind `esp-idf`)
- Host-side unit tests covering all branch cases

**What is missing (the whole roadmap below):**
- Deep sleep / wake
- Radio power gating
- Charging / solar awareness
- Light sleep / PM locks

---

## Architecture Decisions (Frozen)

These were decided during the vision session and drive every milestone below.

- **Feature flags:** Single `esp-idf` gate for all hardware implementations.
  Optional `pm-locks` feature for the FreeRTOS PM lock wrapper (requires `CONFIG_PM_ENABLE` in `sdkconfig.defaults`).
- **Module layout:** Trait modules (`sleep.rs`, `radio_gate.rs`, `charging.rs`) are always compiled.
  ESP-IDF implementations (`esp_sleep.rs`, `esp_radio.rs`, `esp_charging.rs`) are feature-gated — mirrors the existing `lib.rs` / `esp_adc.rs` pattern.
- **Embassy:** Not applicable.
  This project runs ESP-IDF std / FreeRTOS, not bare-metal embassy.
  All sleep APIs are direct `esp-idf-sys` FFI calls.
- **Dependency direction:** `rustyfarian-power` defines `RadioPowerGate`; `rustyfarian-network` holds a reference to it.
  The power crate must never import the network crate.
- **Crate naming:** The crate is currently named `battery-monitor`.
  As scope expands to full power management, rename to `power-manager` at the next natural semver boundary.
  Not urgent while the workspace is private.

---

## Milestone 1 — Deep Sleep: Timer Wake

**Goal:** Any rustyfarian app can enter deep sleep and wake on a configurable timer in under a day of integration work.

**Why first:** Board-level deep sleep draws ~100 µA (chip is ~7 µA; LDO, charge controller, and SX1262 add the rest).
Active mode draws 50–240 mA.
A beehive sensor transmitting every 10 minutes spends 99.7% of its time sleeping.
Nothing else in this roadmap matters without this primitive.
With deep sleep, a 3 000 mAh cell sustains approximately nine months of operation on timer-only cycles.

**Deliverables:**

- `sleep.rs` — public trait module (always compiled):
  - `WakeCause` enum: `PowerOn`, `Timer`, `Gpio`, `Touch`, `Other`
  - `WakeSource` enum: `Timer { duration_ms: u64 }` (expand in Milestone 2)
  - `SleepManager` trait: `fn sleep(&mut self, sources: &[WakeSource]) -> anyhow::Result<()>`
  - `WakeCauseSource` trait: `fn last_wake_cause(&self) -> WakeCause` (separate from `SleepManager` — see note below)
- `esp_sleep.rs` — ESP-IDF implementation (feature-gated):
  - `EspSleepManager`: calls `esp_sleep_enable_timer_wakeup()`, `esp_sleep_config_gpio_isolate()`, `esp_deep_sleep_start()`
  - `EspWakeCauseSource`: wraps `esp_sleep_get_wakeup_cause()`
- Mock `NoopSleepManager` for host tests
- Documentation covering the "does not return" semantics and how to persist state across deep sleep via `#[link_section = ".rtc.data"]`
- `sdkconfig.defaults` entry for any required sleep-related Kconfig options

**Design notes:**
- `SleepManager::sleep()` is fallible (`anyhow::Result`) — pre-sleep GPIO configuration can fail before sleep entry; silent failure would drain the battery
- `WakeCauseSource` is a separate trait because `sleep()` never returns on real hardware; the same instance cannot call both methods in one boot session
- `esp_sleep_get_wakeup_cause()` returns `UNDEFINED` on first boot (cold power-on), not only after wake — document and handle this in consumer guidance

**Verification:** `cargo test -p battery-monitor` covers mock path; flash test verifies real wake cycle on device.

---

## Milestone 2 — Deep Sleep: GPIO Wake Sources

**Goal:** A sensor with an interrupt-generating peripheral (rain gauge, door contact, magnetic flow meter) can wake the ESP32 from a pulse on an RTC GPIO.

**Why second:** Timer wake covers regular-interval patterns; GPIO wake covers event-driven patterns.
Together they cover the two primary IoT sensor architectures.
Both are first-order requirements for the beehive monitoring use case (e.g., a tipping-bucket rain gauge).

**Deliverables:**

- `WakeSource::GpioLevel { pin_mask: u64, level: GpioWakeLevel }` variant added to `sleep.rs`
- `GpioWakeLevel` enum: `High`, `Low`
- `EspSleepManager::sleep()` handles `GpioLevel` via `esp_sleep_enable_ext1_wakeup_io()` (v5.x preferred API — the old `esp_sleep_enable_ext1_wakeup()` is deprecated in ESP-IDF v5.3.3)
- `WakeCause::Gpio` carries the bitmask of fired pins via `esp_sleep_get_ext1_wakeup_status()`
- Documentation: external pull resistors (10–100 kΩ) are required on all GPIO wake pins.
  RTC internal pull-up/pull-down resistors are unavailable when `RTC_PERIPH` is powered down (the default).
  Floating pins with HOLD state are indeterminate — a missing external resistor is a common field failure mode.

**Open question (resolve before implementation):** Decide between `pin_mask: u64` (mirrors the ESP-IDF API but weakly typed) vs a `&[u8]` pin-number slice (more misuse-resistant, range-checked at the boundary).
See `docs/key-insights.md` → Trait Design Decisions.

**Constraints:**
- EXT1 on ESP32-S3 supports RTC GPIOs 0–21 only
- `ESP_EXT1_WAKEUP_ALL_LOW` is deprecated — use `ESP_EXT1_WAKEUP_ANY_LOW`

---

## Milestone 3 — Radio Power Gating

**Goal:** A rustyfarian app can cut power to the SX1262 LoRa radio and OLED before entering deep sleep, reducing sleep current contribution of the radio from ~400 nA (SX1262 sleep mode) to 0 µA (VEXT off).

**Why third:** Radio power gating is the primary integration boundary with `rustyfarian-network`.
Getting the trait boundary right here (defined in power, implemented in power, consumed by network) is architecturally important.
It also eliminates the radio's quiescent contribution from the sleep current budget.

**Deliverables:**

- `radio_gate.rs` — public trait module (always compiled):
  - `RadioPowerGate` trait: `power_on(&mut self) -> anyhow::Result<()>`, `power_off(&mut self) -> anyhow::Result<()>`, `is_powered(&self) -> bool`
- `esp_radio.rs` — ESP-IDF implementation (feature-gated):
  - `GpioRadioPowerGate`: configurable enable GPIO (default GPIO 3 for Heltec V3 VEXT) and configurable stabilisation delay
- Documentation of the recommended pre-sleep sequence:
  1. Confirm any pending LoRa TX/RX is complete
  2. Send `SetSleep` to SX1262 over SPI
  3. Call `radio_gate.power_off()` (drives VEXT GPIO low)
  4. Call `sleep_manager.sleep(sources)`
- Note that GPIO 3 (VEXT) also gates the OLED display — powering off drops both
- Note that SX1262 requires full register re-initialisation on every wake (all state is lost when VEXT is cut; ~10–50 ms)

**Open questions (resolve before implementation):**
- Confirm GPIO 3 = VEXT on the Heltec WiFi LoRa 32 V3 schematic.
  The assignment varies between V2 and V3 board revisions; hardcoding the wrong pin silently breaks radio operation.
- Decide whether the stabilisation delay is exposed on the `RadioPowerGate` trait (`fn stabilisation_ms(&self) -> u32`) or only on the concrete constructor.
  Defer until there is an async consumer that needs to schedule against it.

---

## Milestone 4 — Charging and Solar Awareness

**Goal:** A rustyfarian app can detect whether the device is discharging, charging from USB, or charging from solar, and make transmission throttling or scheduling decisions accordingly.

**Why fourth:** The solar power boost use case is explicitly named in the vision.
Charge-state awareness enables applications to transmit more aggressively when solar is available and to enter a "low-power hold" when the battery is critically low and uncharged — which is the primary failure mode for unattended field deployments.

**Deliverables:**

- `charging.rs` — public module (always compiled):
  - `ChargingSource` enum: `Usb`, `Solar`, `Unknown`
  - `ChargingState` enum: `Discharging`, `Charging { source: ChargingSource }`, `Full`, `NotPresent`, `Fault`, `Unknown`
  - `ChargingMonitor` trait: `fn read_charging_state(&mut self) -> ChargingState` (infallible, absorbs errors into `Unknown`)
- `esp_charging.rs` — ESP-IDF implementation (feature-gated):
  - `GpioChargingMonitor`: reads TP4054 CHRG status pin via GPIO to detect active charging
- `PowerSource::Solar` variant added to `lib.rs` — this is a **semver break**; add `#[non_exhaustive]` to `PowerSource` before this milestone ships if the library has been published to crates.io
- Mock `NoopChargingMonitor` for host tests

**Open questions (resolve before implementation):**
- Identify the TP4054 CHRG pin on the Heltec WiFi LoRa 32 V3 schematic.
  The CHRG pin may or may not be connected to an ESP32 GPIO on this board revision.
  Do not assume it matches common Heltec documentation — check the V3-specific schematic.
- Confirm whether `PowerSource::Solar` breaks any existing downstream match arms within the workspace before shipping.

**Constraints:**
- The TP4054 charge controller has no I2C/SPI interface and no MPPT capability.
  Solar awareness is limited to detecting the CHRG pin state.
  MPPT interaction is out of scope for this board.

---

## Milestone 5 — Light Sleep and PM Locks

**Goal:** Apps with sub-second wake intervals (e.g., a pulse-counting flow meter) can use light sleep rather than a busy loop, halving active current without the re-initialisation cost of deep sleep.

**Why fifth:** Light sleep retains CPU state and RAM — `esp_light_sleep_start()` returns after wake.
It is the right tool for sub-second intervals or when full hardware re-initialisation on every cycle is prohibitively expensive.
Power consumption (~800 µA–2 mA board-level) is higher than deep sleep and too high for the beehive use case, but appropriate for faster-cycling sensors.

**Deliverables:**

- `LightSleepManager` concrete struct (not a new trait — `SleepManager::sleep()` can dispatch to light sleep based on a configuration parameter or a separate entry point)
- `PmLock` wrapper for `esp_pm_lock_create()` / `esp_pm_lock_acquire()` / `esp_pm_lock_release()` — prevents automatic frequency scaling during critical sections
- `pm-locks` feature flag; document that it requires `CONFIG_PM_ENABLE=y` and `CONFIG_FREERTOS_USE_TICKLESS_IDLE=y` in `sdkconfig.defaults`
- Documentation distinguishing light sleep from deep sleep and guiding the choice

---

## Future — ULP Coprocessor Monitor

**Goal:** The ULP FSM samples battery ADC during deep sleep and wakes the CPU only when voltage crosses a threshold, eliminating unnecessary full-boot cycles.

**Why deferred:** High implementation complexity (ULP assembly or ULP-RISC-V C code compiled separately).
Not required for the initial beehive use case.
The `SleepManager` trait surface is designed to not foreclose this — a future `WakeSource::Ulp { condition }` variant can extend the enum.

**Deliverables (future):**
- ULP FSM program reading ADC1 during deep sleep
- `UlpWakeCondition` configuration (voltage threshold, hysteresis, sample interval)
- `WakeSource::Ulp { condition: UlpWakeCondition }` variant
- `WakeCause::Ulp` variant

**ESP32-S3 capability confirmation:** `SOC_ULP_FSM_SUPPORTED = 1`, `SOC_ULP_HAS_ADC = 1` — confirmed in `soc_caps.h` for the ESP32-S3.

---

## Open Questions

| Question | Blocks | How to resolve |
|:---|:---|:---|
| Is GPIO 3 = VEXT on the Heltec WiFi LoRa 32 V3? | Milestone 3 | Check the V3-specific schematic before writing `GpioRadioPowerGate` |
| Is the TP4054 CHRG pin connected to an ESP32 GPIO on Heltec V3? | Milestone 4 | Check the V3-specific schematic; identify pin number |
| `pin_mask: u64` vs `&[u8]` pin list for `WakeSource::GpioLevel`? | Milestone 2 | Decide before implementing `EspSleepManager` for GPIO sources |
| Add `#[non_exhaustive]` to `PowerSource` before publishing? | Milestone 4 | Decide at first crates.io publish, or when M4 is scoped |
| Radio stabilisation delay on `RadioPowerGate` trait or constructor only? | Milestone 3 | Defer until there is an async consumer requiring it |
| Solar integration depth — CHRG pin only, or richer energy budget? | Milestone 4 | Confirmed as CHRG pin only for Heltec V3; revisit if hardware changes |
