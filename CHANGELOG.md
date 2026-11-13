# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Starting with 0.1.0 this project follows [Semantic Versioning](https://semver.org/) (pre-1.0: minor bumps may carry breaking changes). Earlier entries are grouped by milestone.

---

## [Unreleased]

### Migration
- **Breaking:** `rustyfarian-esp-idf-power` requires ESP-IDF v6.0 or newer (the workspace builds v6.1); a firmware on ESP-IDF v5.x stays on 0.3.x, and an older build now stops with a `compile_error!` naming the requirement.

### Changed
- The workspace builds its ESP-IDF examples against ESP-IDF v6.1 (was v5.3.3); the C2/C3 GPIO wake path uses the v6 calls `esp_sleep_enable_gpio_wakeup_on_hp_periph_powerdown` and `gpio_wakeup_disable_on_hp_periph_powerdown_sleep`.
- ESP32-C2/C3: every sleep call clears the SDK's GPIO wake mask and trigger modes, so a pin armed earlier in the same boot is no longer held and pulled during later GPIO-wake sleeps, and a pin switched from `AnyHigh` to `AnyLow` no longer keeps its pull-down.
- `.cargo/config.toml` is tracked instead of `.cargo/config.toml.dist`, and `just setup-cargo-config` is gone; Cargo never reads it for downstream users.
- The ESP-IDF and hal CI jobs run on `ubuntu-latest` again, because ESP-IDF v6.1's `esp-clang` installs on Ubuntu 26.04.
- CI uses the shared `rustyfarian-ci@v1` workflow and setup actions: the host checks run through `host-checks.yml` (`just ci-fmt`, `ci-clippy`, `ci-test`, `ci-deny`, `ci-audit`), the chip jobs pin the esp toolchain to 1.97.0.0 and cache it and `~/.espressif`, and every job has a timeout; `just act` runs the jobs locally.
- Dependabot bumps GitHub Actions and Cargo dependencies instead of the `rust-toolchain` channel.

### Fixed
- `WakeCause::Gpio` maps ESP-IDF v6's RTC-IO-indexed wake status to GPIO numbers with `esp_sleep_wakeup_io_bit2num`.

## [0.3.0] - 2026-11-10

`rustyfarian-esp-hal-power` is published as the bare-metal tier, `stoker` goes `no_std` with one sleep-attempt machine both tiers drive, on the C3, C6, ESP32 and S3.

### Migration
- **Breaking:** `stoker` is `#![no_std]`, its validators return `stoker::SleepError` instead of `anyhow`, and `SleepManager` gains `type Error: stoker::sleep::Error` plus a required `try_sleep`; implementors wrap `anyhow::Error` in a newtype with `kind()` or use `stoker::SleepError` (ADR-001).
- **Breaking:** `EspSleepManager::Error` is `EspSleepError`, `sleep(&[])` returns `NoWakeSource`, timers above `MAX_TIMER_MS` return `TimerTooLong`, isolation plus `GpioLevel` returns `IsolationConflict`, and `WakeCause::Gpio` carries a `GpioWakeMask`; the new error types are re-exported from `rustyfarian-esp-idf-power`.
- Firmware that uses `RetainedCell` on the IDF tier enables `critical-section-impl` or links another interrupt- and multicore-safe provider; pre-release git pins that relied on the earlier default must opt in.
- **Breaking:** `heltec_v3_vext` moves from an inherent method to the `HeltecV3Vext` extension trait on both tiers, so callers import the trait; the call shape is unchanged (ADR-004).
- ESP-IDF S3/C3/C6 battery readings change with eFuse curve fitting; re-check a custom `divider_ratio` against a meter (see Fixed).

Before:

```rust
use rustyfarian_esp_hal_power::GpioRailGate;
let vext = GpioRailGate::heltec_v3_vext(pin);
```

After:

```rust
use rustyfarian_esp_hal_power::{GpioRailGate, HeltecV3Vext};
let vext = GpioRailGate::heltec_v3_vext(pin);
```

- **Breaking:** on the ESP-IDF ESP32 tier, `try_sleep` and `sleep` reject a multi-pin `AnyLow` request with `SleepError::AnyLowMultiplePins` (kind `Unsupported`) before anything is armed, as the esp-hal tier already did.
  The EXT1 `ALL_LOW` mode that used to accept it is no longer reachable for several pins.
  Name one pin for `AnyLow`, or use `AnyHigh` if the wiring allows.

Before (ESP32 only):

```rust
use rustyfarian_esp_idf_power::{EspSleepManager, GpioWakeLevel, WakeSource};
use stoker::sleep::SleepManager;

let mut manager = EspSleepManager::default();
let sources = [WakeSource::GpioLevel { pin_mask: 0b11, level: GpioWakeLevel::AnyLow }];
manager.try_sleep(&sources)?;
```

After (ESP32 only):

```rust
use rustyfarian_esp_idf_power::{EspSleepManager, GpioWakeLevel, WakeSource};
use stoker::sleep::SleepManager;

let mut manager = EspSleepManager::default();
let sources = [WakeSource::GpioLevel { pin_mask: 1 << 25, level: GpioWakeLevel::AnyLow }];
manager.try_sleep(&sources)?;
```

### Added
- `stoker::bench::variant_index` and `stoker::bench::parse_u64` read the examples' build-time switches (`BENCH_VARIANT`, `*_RET_ACTION`, timer overrides) in a `const` context, and `ResetReason::name()` gives the `reset=` spelling of the bench lines; the per-example copies of these helpers are gone (17 examples, both tiers).
- `EspAdcBatteryMonitor` and `EspHalAdcBatteryMonitor`: `read_raw_mv()` returns the averaged, calibrated pin millivolts before divider compensation, and `config()` exposes the `BatteryConfig`; the `+calib` variants of `hal_s3_battery` and `idf_esp32s3_battery` print that value instead of dividing the compensated voltage back (which truncated one mV low).
- `just bench` runs rustyfarian-bench v0.5.0: `reset = true` steps (used by `heltec/rail-gate-ve`), per-board `mac` pins (both ESP32-S3 boards), the serial reader no longer reboots USB-Serial-JTAG boards when it reopens the port after a deep sleep, and prompts may name the device with `{mac}`/`{port}` placeholders (literal braces in `say`/`confirm` must now be doubled).
- Bench variants: a scenario example named `NAME+VARIANT` is built and flashed with `BENCH_VARIANT=VARIANT` (`just bench-build` / `bench-flash`, ADR-005 decision 7); `{hal,idf}_esp32_deep_sleep_gpio` read it (`anylow`, `anylow-multi`), with the new `esp32/anylow` and `esp32/anylow-multi` scenarios; `{hal,idf}_s3_power_rail_gate` read `meter` (90 s awake, 120 s asleep) for the attended `heltec/rail-gate-ve` meter walk-through. `{hal,idf}_s3_deep_sleep_gpio` read `r2`, `r3`, `r4` and `r4-guard` (the R2-S3 to R4-S3 cases with the 5 s timer) for the new `bench/s3/` CrowPanel set (`sleep`, `touch-wake`, `r2`, `r3`, `r4`, `r4-guard`), which replaces the removed hand-run steps document. The examples' `try_sleep failed:` line now prints the error kind.
- `rustyfarian-esp-idf-power`: `idf_esp32_deep_sleep_gpio`, the ESP-IDF twin of `hal_esp32_deep_sleep_gpio` (GPIO25 EXT1 + timer, `RTC rtc_us` per boot), so the ESP32 sleep bench checks run on both tiers.
- `rustyfarian-esp-hal-power`: `hal_esp32_rtc_retention`, the ESP32 port of `hal_c3_rtc_retention`; `hal_esp32_deep_sleep_gpio` prints `RTC rtc_us` at boot and before sleeping. Both bench the ESP32 RTC fast-memory path.
- `bench/`: a `rustyfarian-bench` definition with C3 (`retention`, `pir-wake`), Heltec V3 (`retention`, `rail-gate`, `battery`) and Feather V2 (`battery`) scenarios over the default example builds; run with `just bench <board>` (e.g. `just bench c3`) against the pinned `rustyfarian-bench` (`RUSTYFARIAN_BENCH` selects a local checkout), with `bench-list`, `bench-detect` and the `bench-build` / `bench-flash` dispatch recipes alongside (ADR-005).
- ESP32-C3/C2 deep-sleep GPIO wake on the IDF tier, with a pre-sleep held-pin check that returns `Rejected`, `WakeCause::Gpio(mask)`, and `just check-c3` in CI.
- New bare-metal crate `rustyfarian-esp-hal-power` (esp-hal `=1.2.2`, MSRV 1.95) with a C3/S3 `sleep` module (`EspHalSleepManager`, `BoundSleep`, `EspHalWakeCauseSource`) and per-chip `just` recipes.
- ESP32-S3 EXT1 wake reports `WakeCause::Ext1(mask)` on both tiers.
- RTC retention: `stoker::retain` (`Retained`, `RetainedCell`, CRC-checked, reset-gated) and `stoker::reset`, with `EspResetReasonSource`, `EspHalResetReasonSource` and `RtcRetainedCell` in the tiers.
- RTC clocks for slept time: `EspRtcClock::now_us()` on the IDF tier and `EspHalRtcClock::now_us()` on esp-hal C3/S3, whose error scales with the read interval (`stoker::rtc_clock`).
- Power rail gate: `stoker::gate` (`PowerRailGate`, `GatedSleep`, `NoopPowerRailGate`) and `GpioRailGate` with a Heltec V3 Vext preset on both tiers (ADR-002).
- Examples and bench tooling: C3/S3 deep-sleep, retention and rail-gate examples on both tiers, `just serial-capture`, `just console`, `just bench-c3`, and records in `docs/bench/`.
- `stoker::bench` (feature `bench-conformance`, off by default): `PinWindow`, the bench line formatters and `LineBuf`, shared by the C3 and S3 deep-sleep bench examples on both tiers.
- Compile-time conformance modules in both tier crates instantiate every `stoker` trait with the concrete adapters (ADR-003 point 4).
- `stoker::sleep::attempt`: `SleepAttemptMachine`, a sans-IO state machine with `SleepPlan` and the `ChipPolicy` guard table, which both tiers drive for `try_sleep` (ADR-004).
- `stoker::average_mv` and `stoker::classify_charging`: pure functions with host tests, used by the ESP-IDF ADC and charging tiers.
- `stoker::SleepError::AttemptOutOfPhase` (kind `Platform`): esp-hal `try_sleep` returns it, disarmed, if the attempt machine ever ignores an input, matching the IDF tier's `Platform` error instead of a false `Rejected`.
- `stoker::gate::GpioRailGate`, `GpioRailGateError` and `Polarity` are generic over an `embedded-hal` 1.0 `OutputPin`; both tiers re-export the one shared type (ADR-004).
- `stoker::charging::GpioChargingMonitor` implements `ChargingMonitor` over an STAT and a VBUS `InputPin`; a pin read error reports `Unknown`.
- `ChipPolicy::wake_pin_mask()` in `stoker` is the single per-chip wake-pin mask table; `SleepAttemptMachine::new` rejects out-of-range `GpioLevel` masks with `GpioMaskOutOfRange`.
- `stoker` depends on `embedded-hal` 1.0 with default features; it stays `#![no_std]` and alloc-free.
- esp-hal tier: `EspHalChargingMonitor` (an alias for `GpioChargingMonitor` over two `Input` pins) is available on every chip.
- ESP-IDF tier builds for ESP32-C6 (`riscv32imac-esp-espidf`): EXT1 deep-sleep wake, RTC retention, and the `idf_c6_deep_sleep_gpio` and `idf_c6_rtc_retention` examples.
- `just check-c6`, `just ci-c6` and the CI job `check-c6` (pinned to `ubuntu-24.04`) for the ESP-IDF C6 tier.
- esp-hal `EspHalAdcBatteryMonitor` (ADC1 only, 11 dB, per-chip calibration, averaged reads) on ESP32, ESP32-C3, ESP32-C6 and ESP32-S3.
- Examples `hal_s3_battery` and `hal_esp32_battery` for the esp-hal ADC battery monitor.
- esp-hal sleep module (`EspHalSleepManager`, `BoundSleep`) on ESP32 and ESP32-C6.
- Examples `hal_esp32_deep_sleep_gpio` and `hal_c6_deep_sleep_gpio`, and the recipes `build-example-hal-c6`, `run-hal-c6` and `flash-hal-c6`.
- esp-hal `EspHalWakeCauseSource` and `EspHalResetReasonSource` on ESP32 and ESP32-C6; the wake source moves to a public `wake` module.
- esp-hal `EspHalRtcClock` on ESP32 and ESP32-C6.
- `stoker::SleepError::AnyLowMultiplePins` (kind `Unsupported`): both tiers reject a multi-pin `AnyLow` request on the original ESP32 before arming anything.
- `ChipPolicy::Esp32C6` on the ESP-IDF tier, selected by `cfg(esp32c6)`.

### Changed
- `{hal,idf}_s3_deep_sleep_gpio` print the `S3BENCH` boot line and record the sleep entry in every build (they were R2-only), stay awake 6 s instead of 3 s after each boot, and repeat the boot lines at 3 s and 5 s so a host that reopens the re-enumerated USB-Serial-JTAG port late still reads them; the bench boot line reads `reset=` from `stoker::ResetReason`.
- `stoker::sleep::attempt::drive` runs the `try_sleep` dispatch over an I/O-only `SleepAttemptPort`; both tiers implement the port privately and keep only register I/O and error mapping (ADR-004).
- The `Release` action contract is tightened: a tier answers `Released` only after undoing its arming and making the ADR-003 point 1 postcondition hold.
- A timer-only rejection now also passes through `Release` (`pins: 0`), so the deadline cleanup is explicit on both tiers.
- The ESP-IDF C6 build's `try_sleep` uses `ChipPolicy::Esp32C6` (guard `Never`, wake mask `0xFF`) instead of `Other`; C3, S3 and ESP32 IDF builds are unchanged.
- The esp-hal ADC battery monitor accepts only ADC1 pins (sealed `Adc1Pin`); an ADC2 pad does not compile.
- The esp-hal `reset` and `wake` modules compile on all four chips; the C6 UART drain treats TX FSM states 0x00 and 0x0F as idle, as esp-hal does.
- CI links the C6 examples (`just ci-c6`), and `just ci-hal` links the new ESP32 and C6 hal examples.
- CI links the C3 and S3 examples on both tiers (`just ci-c3`, `ci-s3`, new `hal.yml` with `ci-hal`) and compiles the ESP-IDF ESP32 tier (`just ci-esp32`, compile-only); `just verify` and `just ci` also lint and test `stoker`'s `bench-conformance` feature.
- `stoker` drops `anyhow` for `critical-section`, the IDF tier ships an ISR-safe `critical-section` provider behind the opt-in `critical-section-impl` feature, and the hal tier gates `sleep`/`reset`/`retain` on `unstable` outside `default-members`.
- `try_sleep` on the original ESP32 pre-reads the EXT1 wake pins before sleep entry and returns `Rejected` (code `ESP_ERR_INVALID_STATE`) when one is already at its wake level; before, a held pin slept and woke at once.
- `try_sleep` on esp-hal C3 pre-reads the bound wake pins in software and returns `Rejected` when one is already at its wake level, instead of relying on the hardware.
- `try_sleep` reports the same error kinds on both tiers; when a request has several invalid fields, the reported one may differ.
- The ESP-IDF ESP32-C6 build checks wake-pin masks against the C6 range (GPIO0-7, `ChipPolicy::Esp32C6`) instead of the ESP32-S3 range it applied by accident.
- The esp-hal wake-pin mask is derived from `ChipPolicy::wake_pin_mask()` instead of per-chip feature constants; observable behaviour is unchanged.
- ESP-IDF arming goes through the shared `stoker::arm_transactionally` (hidden `WakeArmOps` port): validation touches no hardware, and a failed SDK call rolls back by attempting every undo step, each C2/C3 pin independently; the original error is returned.

### Fixed
- `rustyfarian-esp-idf-power`: `GpioRailGate::heltec_v3_vext` excludes GPIO36 from ESP-IDF's sleep-time GPIO isolation (`gpio_sleep_sel_dis`); the isolation floated `Vext_Ctrl` and the Vext rail came back ON during deep sleep while the gate reported `powered=false`. esp-hal was unaffected.
- `rustyfarian-esp-idf-power`: the battery ADC path now runs eFuse curve fitting (`Calibration::Curve`) on ESP32-S3/C3/C6, mirroring the esp-hal tier; an uncalibrated oneshot read ~11 % low folded into `BatteryConfig::heltec_v3`'s ratio and misclassified a fitted ~4 V cell as `USB/Ext` (no voltage printed). Both tiers now read the same net. This changes the reading for every ESP-IDF consumer on the S3/C3/C6, not only the Heltec: a custom `divider_ratio` calibrated against the previous uncalibrated path reads about 10 % different and must be re-checked against a meter. Line fitting stays on the ESP32/S2/C2, matching esp-idf-hal's per-chip `Calibration` variants.
- `stoker`: `BatteryConfig::heltec_v3`'s `divider_ratio` is 5.05, metered on hardware; the old 5.55 was the uncalibrated-ADC artifact above, not a board property.
- `rustyfarian-esp-idf-power`: on the ESP32, `EspWakeCauseSource` reports the fired pin of a low-level (`AnyLow`) EXT1 wake armed through `EspSleepManager`; it read `Ext1(0x0)` because ESP-IDF's boot code empties that status before `app_main`. The armed configuration is kept in RTC slow memory, resolved by the new `stoker::ext1_fired_mask`.
- `rustyfarian-esp-hal-power`: on the ESP32, `EspHalWakeCauseSource` reports the fired pin of a low-level (`AnyLow`) EXT1 wake from an armed record in `.rtc_fast.persistent`, mirroring the IDF tier through the shared `stoker::ext1_fired_mask`; the hal tier's read of the EXT1 status register is not reliable there.
- `rustyfarian-esp-hal-power`: on the ESP32, deep sleep now keeps RTC fast memory powered, so `RtcRetainedCell` records and `EspHalRtcClock` survive a timer wake; esp-hal 1.2.2's default deep-sleep config lost them (`load=err:Magic`, clock back at 0).
- `try_sleep` no longer hangs on the ESP32/ESP32-S3 when the global digital-pad hold (`gpio_deep_sleep_hold_en`) is effective and a wake pin is already asserted: both tiers return `Rejected` from a pre-entry read instead (`stoker::digital_hold_effective`, `wake_pins_asserted`, `wake_pins_all_low`).
- `GpioWakeMask::contains_pin` returns `false` for `pin >= 64` instead of overflowing, and `EspSleepManager` validates every mask before the first ESP-IDF call.

## [0.2.0] - 2026-09-26

Downstream apps can now combine `rustyfarian-esp-idf-power` with the September 2026 sibling crates.

### Migration
- **Breaking:** `rustyfarian-esp-idf-power` consumers must move to `esp-idf-hal 0.47` — its types are in the public API and `esp-idf-sys` allows one version per graph.
- `rustyfarian-esp-idf-power` declares `rust-version = "1.95"` (ESP-tier family policy); `stoker` stays at the 1.88 workspace floor.
- `stoker` has no changes; it moves to the new version only to keep the two crates in lockstep.

### Changed
- ESP-IDF stack — September 2026 wave: `esp-idf-hal` `0.46` → `0.47` (`esp-idf-sys 0.38.1`) and `embuild` → `0.33.5`, with no source changes needed.
- Picks up upstream `PinDriver` fix (esp-idf-hal #585) on the `EspChargingMonitor` input path.
- Dependency refresh clears RUSTSEC-2026-0190 (`anyhow 1.0.104`) and RUSTSEC-2026-0204 (`crossbeam-epoch 0.9.21`, build-time only).
- CI: `actions/checkout` `v5` and `extractions/setup-just` `v4`, off the removed Node 20 runtime.
- `just verify` now runs `cargo deny check`, matching `just ci`.

## [0.1.0] - 2026-06-21

First publication to crates.io.

### Changed
- **Breaking / packaging:** split the single `battery-monitor` crate into two published crates — `stoker` (pure, host-buildable core: battery math, sleep validation, charging/wake types, `Noop*` mocks) and `rustyfarian-esp-idf-power` (ESP-IDF drivers: `EspAdcBatteryMonitor`, `EspSleepManager`, `EspWakeCauseSource`, `EspChargingMonitor`). Import paths change: `battery_monitor::X` → `stoker::X` (pure types) or `rustyfarian_esp_idf_power::X` (hardware types). The ESP-IDF crate re-exports `stoker`'s surface, so `use rustyfarian_esp_idf_power::*` covers both. The `esp-idf` Cargo feature is removed — the crate boundary replaces the feature gate.
- `PowerSource` is now `#[non_exhaustive]` (a `Solar` variant is planned); downstream `match`es need a wildcard arm.
- `validate_wake_sources` and `validate_gpio_level_source` are now `pub` (were `pub(crate)`), so the ESP-IDF crate can reuse the pure validation.

### Added
- `ChargingMonitor` trait and `ChargingState` enum — charge-state lifecycle as a concern orthogonal to `PowerSource` identity
- Hardware wiring documentation for Heltec WiFi LoRa 32 V3 and Adafruit ESP32 Feather V2 (`docs/hardware-setup.md`)
- Per-crate `README.md` + dual `LICENSE-MIT`/`LICENSE-APACHE`; crates.io publishing metadata and `release-plan.md` / `scripts/release-validate.sh` / `just` release recipes

### Changed (pre-release)
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
