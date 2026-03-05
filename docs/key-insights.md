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
- **Linker and `ldproxy`:** `linker = "ldproxy"` is required in `.cargo/config.toml` for every Xtensa ESP-IDF target.
  `battery-monitor/build.rs` calls `embuild::espidf::sysenv::output()`, which emits `--ldproxy-linker` and `--ldproxy-cwd` link args that are consumed by `ldproxy`.
  Without it, those args are passed directly to `xtensa-esp-elf-gcc`, which rejects them with "unrecognized command-line option" and exits with code 101.
  Both `xtensa-esp32s3-espidf` and `xtensa-esp32-espidf` must have `linker = "ldproxy"` in `.cargo/config.toml`.
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
- **EXT1 mode chip difference — `ANY_LOW` vs `ALL_LOW`:** `ESP_EXT1_WAKEUP_ANY_LOW` does not exist on the original ESP32; only `ESP_EXT1_WAKEUP_ALL_LOW` is available (all configured pins must be low simultaneously).
  On ESP32-S3 and newer chips, `ESP_EXT1_WAKEUP_ANY_LOW` is the preferred API and `ESP_EXT1_WAKEUP_ALL_LOW` is deprecated.
  `esp_sleep.rs` uses `#[cfg(esp32)]` to select the correct constant at compile time.
  The semantics differ (any vs all), but `ALL_LOW` is the closest available mode on original ESP32.
  This was confirmed by a compile failure against `xtensa-esp32-espidf` when adding Adafruit Feather V2 support.
- **`cfg(esp32)` requires a `build.rs` in the crate:** `cargo:rustc-cfg` outputs from a build script only apply to that crate, not its dependents.
  Even though `esp-idf-sys` emits `cargo:rustc-cfg=esp32`, that flag is not visible to `battery-monitor`.
  `battery-monitor/build.rs` reads `TARGET` and emits `cargo:rustc-cfg=esp32` / `cargo:rustc-cfg=esp32s3` directly, along with `cargo:rustc-check-cfg` to silence the `unexpected_cfg` lint.
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
- **Feather V2 charging detection — GPIO13 (STAT) + GPIO34 (VBUS):** The MCP73831 STAT pin is open-drain, driven LOW while charging, HIGH-Z otherwise.
  The board provides an external 4.7 kΩ pull-up to 3.3 V on GPIO13 — configure it as `Pull::Floating` (do not enable the internal pull-up).
  GPIO34 is a USB VBUS detect pin (100 kΩ + 100 kΩ divider); it is an input-only strapping pin on the original ESP32 — calling `set_pull()` returns `ESP_ERR_INVALID_ARG`.
  Cross-referencing both pins fully resolves all four states: STAT LOW + VBUS HIGH = Charging; STAT HIGH + VBUS HIGH = Full; STAT HIGH + VBUS LOW = NoBattery; STAT LOW + VBUS LOW = Unknown (pathological).
  STAT alone cannot distinguish Full from NoBattery — both read HIGH. Disambiguate using battery ADC voltage if needed.
  **`EspChargingMonitor` STAT bound:** `set_pull` in esp-idf-hal requires `T: InputPin + OutputPin`.
  The STAT generic must be `InputPin + OutputPin` even though the pin is only read; GPIO34 (VBUS, input-only) stays `InputPin` only since `set_pull` is never called on it.
- **Heltec V3 charge controller hardware path is unconfirmed:** The actual charge controller pin and bus must be verified against the V3-specific schematic before writing an `EspChargingMonitor` implementation for that board.
- **Module layout for new traits:** New trait modules (`sleep.rs`, `radio_gate.rs`, `charging.rs`) are unconditionally compiled (no feature gate).
  ESP-IDF implementations (`esp_sleep.rs`, `esp_radio.rs`, `esp_charging.rs`) are feature-gated behind `esp-idf`.
  This mirrors the existing `lib.rs` / `esp_adc.rs` split.
- **WakeSource::GpioLevel uses `pin_mask: u64` (decision record):** Mirrors the ESP-IDF `esp_sleep_enable_ext1_wakeup_io()` API directly; preserves `WakeSource: Copy` (consistent with `Timer { duration_ms: u64 }`).
  Range validation (pins 0–21 only, non-zero mask) is enforced in `EspSleepManager::sleep()` before the FFI call with a clear error message.
  `GpioWakeMask(u64)` on the read side provides a `contains_pin(u8) -> bool` helper so callers do not need raw bit manipulation.

## ADC Calibration: Original ESP32 vs ESP32-S3

- **`Calibration::None` (the `AdcChannelConfig` default) is unreliable on original ESP32 at 11 dB attenuation.**
  On ESP32-S3 the default is acceptable; on original ESP32 the ADC non-linearity is severe enough that
  uncalibrated readings can be off by hundreds of mV, producing `PowerSource::Unknown` for a healthy battery.
  Always use `Calibration::Line` for original ESP32 ADC at 11 dB.
- **DB_11 usable ceiling differs by chip: 2450 mV on original ESP32, 3100 mV on ESP32-S3.**
  The `DirectConverter` in esp-idf-hal applies the correct chip-specific ceiling at compile time, but the
  doc comment in `EspAdcBatteryMonitor::new()` that cites "0–3100 mV range" is only accurate for ESP32-S3.
  At 2450 mV ceiling with a 2:1 divider, the usable battery range is 0–4900 mV (still covers 4200 mV max).
- **Calibration must be selected per chip via `#[cfg(esp32)]` / `#[cfg(esp32s3)]` in `EspAdcBatteryMonitor::new()`.**
  Pattern:
  ```rust
  #[cfg(esp32)]
  let channel_config = AdcChannelConfig { attenuation: DB_11, calibration: Calibration::Line, ..Default::default() };
  #[cfg(not(esp32))]
  let channel_config = AdcChannelConfig { attenuation: DB_11, ..Default::default() };
  ```
  `build.rs` emits `cargo:rustc-cfg=esp32` and `cargo:rustc-cfg=esp32s3` so this `cfg` is already available.
- **Diagnostic: log raw ADC value before divider compensation.**
  Adding `log::info!("raw_mv={}", raw_mv)` in `read_averaged_mv()` immediately distinguishes between
  ADC not reading at all (0), ADC reading but uncalibrated (unexpectedly low), and correct readings.

## GPIO Configuration and Diagnosis

- **`InputEn: 0` in ESP-IDF GPIO log is a cosmetic artifact, not a misconfiguration.**
  When esp-idf-hal 0.45.2 constructs a `PinDriver` via `PinDriver::input()`, the call chain is:
  `input()` → `into_input()` → `into_mode(GPIO_MODE_INPUT)`.
  Inside `into_mode`, `drop(self)` is called first on the intermediate temporary `PinDriver`.
  That `Drop` impl calls `gpio_reset_without_pull()`, which calls `gpio_config()` with `mode = GPIO_MODE_DISABLE`.
  **This is the call that emits the `InputEn: 0` log line.**
  After drop returns, `gpio_set_direction(pin, GPIO_MODE_INPUT)` is called — this uses the `gpio_set_direction()` C API directly, which does NOT call `gpio_config()` and produces no log.
  The pin IS correctly configured as input; the `InputEn: 0` log reflects the transient reset, not the final state.
  This applies to every pin configured through `PinDriver::input()`, including input-only pins like GPIO34.
- **`set_pull()` uses `gpio_set_pull_mode()`, not `gpio_config()`, and does not reset the pin mode.**
  Calling `stat.set_pull(Pull::Floating)` after `PinDriver::input()` calls `gpio_set_pull_mode(pin, GPIO_FLOATING)`.
  This function sets pull registers independently of the direction register.
  It does not call `gpio_config()` and does not clear the `GPIO_MODE_INPUT` set by `gpio_set_direction()`.
  The direction is preserved.
- **`PinDriver::input()` without `set_pull()` is correct for GPIO34 (Feather V2 VBUS detect).**
  GPIO34, 35, 36, 39 are input-only on the original ESP32 and reject `set_pull()` with `ESP_ERR_INVALID_ARG`.
  The Feather V2's 100 kΩ + 100 kΩ voltage divider from VBUS to GND provides a defined voltage at all times
  (~2.5 V with USB, ~0 V without), so no internal pull is needed.
- **GPIO34 strapping pin: consider `rtc_gpio_deinit` before first use.**
  GPIO34 is an RTC GPIO (RTC_GPIO4) that is sampled at boot for strapping.
  ESP-IDF's boot ROM may leave it in the RTC domain.
  `gpio_reset_without_pull` (called by esp-idf-hal's Drop) does NOT call `rtc_gpio_deinit()`.
  If the first read of GPIO34 after boot seems wrong, calling `rtc_gpio_deinit(34)` explicitly before
  `PinDriver::input(vbus_pin)` would release it from the RTC domain and ensure digital input is active.
- **"Charging: Unknown" from `EspChargingMonitor` is most likely a MCP73831 power-on transient.**
  The `(STAT_LOW=true, VBUS_HIGH=false)` branch fires when GPIO13 reads LOW and GPIO34 reads LOW simultaneously.
  The most probable cause during a timer-wake cycle with no USB connected: the MCP73831 STAT pin
  glitches LOW briefly during voltage rail ramp-up after deep sleep.
  Mitigation strategies: add a 10–50 ms delay before the first read, or read GPIO13 three times and
  take a majority vote to filter the transient.
- **Missing `log::warn!` in the charging monitor output is a log level issue, not a code path issue.**
  If "Charging: Unknown" appears in `println!` output but the `log::warn!` call is not visible in the
  serial log, the crate's log component is filtered below WARN level.
  Configure via `sdkconfig.defaults` (`CONFIG_LOG_DEFAULT_LEVEL=4` for INFO, `3` for WARN) or
  call `esp_log_level_set` for the component at runtime.

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
