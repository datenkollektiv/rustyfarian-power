# Key Insights

This file is the persistent knowledge base for the rustyfarian-power project.
All agents and Claude Code sessions must read this file at the start of relevant work and update it when new insights are discovered.

Organise entries by topic, not chronologically.
Keep each insight concise: one paragraph or a short bullet list per topic.
Remove or update entries that are superseded.

---

## Hardware

- **Target board:** Heltec Wi-Fi LoRa 32 V3, which uses the ESP32-S3 chip.
  The Cargo target is `xtensa-esp32s3-espidf`, not the generic `xtensa-esp32-none-elf`.
- **Battery ADC pin:** GPIO1 on an ADC1 channel.
  The Heltec V3 routes the battery through a voltage divider before the ADC pin; the divider ratio is captured in `BatteryConfig`.
- **USB detection threshold:** When powered via USB/external, the measured voltage is above the `usb_detection_threshold` configured in `BatteryConfig::heltec_v3()`.
  Do not confuse this with the battery full voltage.

## Architecture

- **Hardware-independence boundary:** Core logic (`config.rs`, `lib.rs`) has no ESP-IDF dependency and compiles on the host for testing.
  Only `esp_adc.rs` is feature-gated behind `esp-idf` (the default feature).
  Always keep this separation clean.
- **Test strategy:** Unit tests live in `lib.rs` and target the host.
  Never add `esp-idf-hal` calls to test code — use the `BatteryConfig` methods directly.

## Build System

- **Toolchain:** Requires the `esp` Rust toolchain channel (Espressif's Xtensa fork), installed via `espup`.
  The channel is declared in `rust-toolchain.toml`.
  Standard `rustup` toolchains cannot compile for `xtensa-esp32s3-espidf`.
- **ESP-IDF version:** Pinned to v5.3.3 in `.cargo/config.toml` via `ESP_IDF_VERSION`.
  Do not float this; bindgen output changes with every ESP-IDF version.
- **Host tests:** Run with `cargo test -p battery-monitor` (no cross-compilation needed).
  The `esp-idf` feature is excluded during host tests so they run without the ESP-IDF build tooling.

## ESP32-S3 Sleep and Power Management

- **Deep sleep current:** The ESP32-S3 datasheet specifies ~7 µA in deep sleep (RTC domain only, all others off).
  Real-world Heltec V3 boards measure significantly higher (50–150 µA) due to the LDO regulator, charge controller, and LoRa radio quiescent draw.
  The chip-only number is not achievable in practice on this board without hardware modifications.
- **Wake-up sources in deep sleep (ESP32-S3):** Timer (`esp_sleep_enable_timer_wakeup`), EXT0 (single RTC GPIO, GPIOs 0–21), EXT1 (multiple RTC GPIOs 0–21, ANY_LOW or ANY_HIGH), Touch sensor, ULP FSM co-processor, and GPIO deep sleep wakeup (`esp_deep_sleep_enable_gpio_wakeup` — ESP32-S3 specific, confirmed in `soc_caps.h` as `SOC_GPIO_SUPPORT_DEEPSLEEP_WAKEUP`).
  Timer wakeup is the primary source for a LoRaWAN duty-cycle sensor.
  EXT1 or GPIO wakeup are useful for alarm / interrupt-driven sensors (e.g., rain gauge tip, door sensor).
- **ULP coprocessor:** ESP32-S3 has a ULP FSM coprocessor (`SOC_ULP_FSM_SUPPORTED = 1`, `SOC_ULP_HAS_ADC = 1`).
  It can sample the ADC while the main CPU is in deep sleep, enabling battery monitoring without full wake-up.
  This is a useful capability for intelligent wake/no-wake decisions.
- **Sleep API (ESP-IDF v5.3.3):** Public API is in `esp_sleep.h` (component `esp_hw_support`).
  Key functions: `esp_sleep_enable_timer_wakeup(us)`, `esp_sleep_enable_ext1_wakeup_io(mask, mode)`, `esp_deep_sleep_enable_gpio_wakeup(mask, mode)`, `esp_sleep_pd_config(domain, option)`, `esp_deep_sleep_start()`, `esp_light_sleep_start()`, `esp_sleep_get_wakeup_cause()`.
  `esp_deep_sleep_start()` never returns; use `esp_deep_sleep_try_to_start()` if you need rejection detection.
  Use `esp_sleep_enable_ext1_wakeup_io()` (v5.x preferred API) — not the deprecated `esp_sleep_enable_ext1_wakeup()`.
- **EXT1 wakeup status functions:** Call `esp_sleep_get_ext1_wakeup_status()` when the wakeup cause is `ESP_SLEEP_WAKEUP_EXT1` to get the bitmask of fired pins.
  For `ESP_SLEEP_WAKEUP_GPIO` (ESP32-S3 deep-sleep GPIO wake), use `esp_sleep_get_gpio_wakeup_status()` instead — the EXT1 status register is not populated.
  Both functions return a `u64` bitmask (bit N = GPIO N).
  The EXT1 register is preserved by hardware until the next sleep entry, so it can be read at any point after boot.
  Recommended practice (per Espressif) is to read it early in `main()` before peripheral initialisation, and store the result — not because the register is likely to be cleared, but for clarity and defensive coding.
- **GPIO wake sources (EXT1):** Only RTC GPIOs 0–21 are supported on ESP32-S3.
  External pull resistors (10–100 kΩ) are mandatory on every wake GPIO; RTC internal pulls are unavailable during deep sleep when `RTC_PERIPH` is powered down.
  Missing external resistors are a common field failure mode — floating pins may false-trigger or fail to trigger.
  When using `WakeSource::GpioLevel`, set `EspSleepManager { isolate_gpio: false }` — GPIO isolation (the default) may prevent wake-capable pins from triggering.
- **Power domain control:** `esp_sleep_pd_config(ESP_PD_DOMAIN_*, ESP_PD_OPTION_OFF/ON/AUTO)` controls RTC peripherals, RTC slow/fast memory, XTAL, CPU, VDDSDIO, and Modem domains during sleep.
  Powering down the MODEM domain (`ESP_PD_DOMAIN_MODEM`) in deep sleep is automatic when WiFi/BT are not in use.
- **EXT1 mode deprecation note:** On ESP32-S3, `ESP_EXT1_WAKEUP_ALL_LOW` is deprecated — use `ESP_EXT1_WAKEUP_ANY_LOW` instead.
  The old constant is a compile-time deprecated alias.
- **GPIO isolation:** Call `esp_sleep_config_gpio_isolate()` and `esp_sleep_isolate_digital_gpio()` before deep sleep to prevent floating digital GPIO pins from leaking current.
  This is one of the most commonly missed optimizations on real boards.
- **Deep sleep wake stub:** A minimal function marked `RTC_IRAM_ATTR` can execute immediately on wake before the full boot, useful for deciding whether to fully boot or return to sleep.
  Set via `esp_set_deep_sleep_wake_stub()`.
- **Light sleep vs deep sleep for LoRaWAN:** Deep sleep is the correct choice for a sensor transmitting every few minutes.
  Light sleep preserves RAM and peripheral state but consumes ~800 µA–2 mA (too high for months-long battery life).
  Deep sleep with timer wakeup achieves the lowest average current.
- **Heltec V3 SX1262 radio power:** The SX1262 is powered by a dedicated GPIO-controlled MOSFET/LDO on the Heltec V3 board.
  GPIO 3 (VEXT) controls external power to the radio and OLED display on some Heltec board revisions; the specific power control GPIO must be verified against the V3 schematic.
  The SX1262 has its own sleep mode (400 nA) accessible via SPI command — `SetSleep(0x00)`.
  For deep sleep of the ESP32, the SPI bus must be fully idle and the radio must be put to sleep first.

## Trait Design Decisions

- **BatteryMonitor infallible convention:** `BatteryMonitor::read` returns `BatteryStatus` (not `Result`) by absorbing ADC errors into `PowerSource::Unknown`.
  New traits that read best-effort status (e.g., `ChargingMonitor::read_charging_state`) should follow the same convention for consistency.
  Traits whose failure is operationally critical (e.g., `SleepManager::sleep`, `RadioPowerGate::power_on`) should return `anyhow::Result`.
- **Sleep/wake asymmetry:** On ESP32, `esp_deep_sleep_start()` never returns.
  This means `WakeCause` cannot be returned from a `sleep()` call — it must be read at the next boot via a separate `WakeCauseSource` trait.
  Always keep `SleepManager` and `WakeCauseSource` as two distinct traits; merging them implies a round-trip that does not exist in hardware.
- **Radio power gating — who implements the trait:** `rustyfarian-power` defines and implements `RadioPowerGate`; `rustyfarian-network` holds a reference to it.
  Dependencies must flow downward: application -> network -> power.
  The power crate must never import `rustyfarian-network`.
- **Reference-counted radio gate:** When multiple subsystems share one power rail, use a coordinator that wraps `RadioPowerGate` with a reference count.
  The radio powers on when the count goes 0 -> 1 and off when the count returns to 0.
  The `RadioPowerGate` trait itself exposes only the primitive three methods (`power_on`, `power_off`, `is_powered`); coordination is the implementor's concern.
- **ChargingState vs PowerSource are orthogonal:** `PowerSource` answers "what supplies energy now"; `ChargingState` answers "what is the battery's charge trajectory".
  They can vary independently (e.g., on solar with a full battery: `PowerSource::Solar`, `ChargingState::Full`).
  Do not merge them.
- **`PowerSource::Solar` is a planned breaking change:** Adding a `Solar` variant to the existing `PowerSource` enum is a deliberate semver break.
  Downstream match arms must add the arm.
  If the library is published to crates.io before the solar hardware is confirmed, add `#[non_exhaustive]` first.
- **`ChargingSource` as a field in `ChargingState::Charging`:** Rather than separate `ChargingFromUsb` and `ChargingFromSolar` variants, use `ChargingState::Charging { source: ChargingSource }`.
  This avoids proliferating variants for every source permutation.
- **Heltec V3 charge controller hardware path is unconfirmed:** The `ChargingMonitor` trait assumes an I2C charge controller (likely BQ25896) or a charge-status GPIO.
  The actual pin and bus must be verified against the V3 schematic before writing `EspChargingMonitor`.
  Do not assume the schematic matches common Heltec documentation — check the V3-specific schematic.
- **Module layout for new traits:** New trait modules (`sleep.rs`, `radio_gate.rs`, `charging.rs`) are unconditionally compiled (no feature gate).
  ESP-IDF implementations (`esp_sleep.rs`, `esp_radio.rs`, `esp_charging.rs`) are feature-gated behind `esp-idf`.
  This mirrors the existing `lib.rs` / `esp_adc.rs` split.
- **WakeSource::GpioLevel uses `pin_mask: u64` (decision record):** Mirrors the ESP-IDF `esp_sleep_enable_ext1_wakeup_io()` API directly; preserves `WakeSource: Copy` (consistent with `Timer { duration_ms: u64 }`).
  Range validation (pins 0–21 only, non-zero mask) is enforced in `EspSleepManager::sleep()` before the FFI call with a clear error message.
  `GpioWakeMask(u64)` on the read side provides a `contains_pin(u8) -> bool` helper so callers do not need raw bit manipulation.

## Known Gotchas

- Running `cargo build` without the `esp` toolchain active will fail with a linker error.
  Ensure `espup` has been run and the environment is sourced (`source ~/export-esp.sh`).
- The `esp-idf` feature is enabled by default, which triggers the ESP-IDF CMake build on the first compiler.
  This can take 10–20 minutes on a cold cache.
  Subsequent builds are fast due to the `.embuild` cache.
- **Host tests require an explicit target flag.**
  `.cargo/config.toml` forces `target = "xtensa-esp32s3-espidf"` for all builds, so `cargo test` without `--target` tries to link with `ldproxy` and fails.
  Always run host tests as: `cargo test -p battery-monitor --no-default-features --target aarch64-apple-darwin` (adjust the triple for non-Apple hosts).
- **`esp_sleep_config_gpio_isolate()` returns `void` in ESP-IDF v5.x**, not `esp_err_t`.
  Do not wrap it in an error check; call it directly and document the `SAFETY` comment.
  (The key-insights entry about calling this function before sleep is still correct — just no error check needed.)
