# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Starting with 0.1.0 this project follows [Semantic Versioning](https://semver.org/) (pre-1.0: minor bumps may carry breaking changes). Earlier entries are grouped by milestone.

---

## [Unreleased]

### Migration
- **Breaking:** `stoker` is now `#![no_std]` unconditionally.
- **Breaking:** `stoker::sleep::validate_gpio_level_source` and `stoker::sleep::validate_wake_sources` return `Result<(), stoker::SleepError>` instead of `anyhow::Result<()>`; the messages are available via `Display` and unchanged except for the out-of-range case, which now names the valid mask instead of a chip. `SleepError` is `Copy`, `#[non_exhaustive]` and implements `core::error::Error`, so `?` into `anyhow::Error` still works.
- **Breaking:** `SleepManager` gains an associated `type Error: stoker::sleep::Error` and a required `try_sleep(&mut self, sources) -> Result<(), Self::Error>`; `sleep` returns `Result<(), Self::Error>` (ADR-001). Implementors add `type Error` and `try_sleep`; there is deliberately no default, because one delegating to `sleep` would sleep through a held wake pin. Generic code bounded on `S::Error: Debug` keeps compiling (`Debug` is a supertrait) and can branch on `kind()`.
- **Breaking, third-party implementors:** `type Error = anyhow::Error` no longer compiles, and the orphan rule forbids `impl stoker::sleep::Error for anyhow::Error`. Wrap it in a local newtype (`struct MyError(anyhow::Error)` deriving `Debug`) and implement `kind()` on that, usually returning `SleepErrorKind::Platform` or `Other`.

  Before (0.2):

  ```rust
  impl SleepManager for MySleeper {
      type Error = anyhow::Error;
      fn sleep(&mut self, sources: &[WakeSource]) -> anyhow::Result<()> { /* ... */ }
  }
  ```

  After (0.3):

  ```rust
  #[derive(Debug)]
  pub struct MySleepError(pub anyhow::Error);

  impl stoker::sleep::Error for MySleepError {
      fn kind(&self) -> stoker::SleepErrorKind {
          stoker::SleepErrorKind::Platform
      }
  }

  impl SleepManager for MySleeper {
      type Error = MySleepError;
      fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), MySleepError> {
          stoker::sleep::validate_wake_sources(sources).map_err(|e| MySleepError(e.into()))?;
          /* ... */
      }
      fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), MySleepError> {
          /* rejection-aware entry; return an error whose kind() is Rejected when refused */
      }
  }
  ```

  Simpler still, if the implementor has no SDK errors of its own: use `type Error = stoker::SleepError` and return `SleepError::Rejected` from `try_sleep`.

  Consumers matching on the wake cause and calling `sleep` change like this:

  ```rust
  match cause {
      WakeCause::Gpio(mask) if mask.contains_pin(4) => { /* PIR on GPIO4 */ }
      WakeCause::Gpio(_) => { /* other pin, or mask unavailable on this chip */ }
      _ => {}
  }

  match mgr.try_sleep(&sources) {
      Err(e) if e.kind() == SleepErrorKind::Rejected => { /* wait for the pin to clear, retry */ }
      Err(e) => log::error!("sleep config failed: {e:?}"),
      Ok(()) => {}
  }
  ```

  Calls of `sleep(&[])` that relied on "sleep until reset" must pass a wake source; there is no replacement.
- **Breaking:** `EspSleepManager::Error` is the new `rustyfarian_esp_idf_power::EspSleepError` instead of `anyhow::Error`. It implements `core::error::Error`, so `?` into `anyhow::Error` still works; code that called `anyhow` methods on the error directly converts first.
- **Breaking:** `sleep(&[])` (and `try_sleep(&[])`) returns `SleepError::NoWakeSource` instead of `Ok(())` / sleeping until reset, on `NoopSleepManager` and `EspSleepManager`; `validate_wake_sources(&[])` returns the same error.
- **Breaking:** `Timer { duration_ms }` above `stoker::MAX_TIMER_MS` (`u32::MAX` ms, about 49.7 days) returns `SleepError::TimerTooLong` on both tiers.
- **Breaking:** `WakeCause::Gpio` carries the fired-pin mask: `WakeCause::Gpio(GpioWakeMask)`. Matches on `WakeCause::Gpio` become `WakeCause::Gpio(mask)` or `WakeCause::Gpio(_)`.
- `EspSleepManager` with `isolate_gpio: true` and a `GpioLevel` source returns `EspSleepError::Config(SleepError::IsolationConflict)` instead of an `anyhow` message; ESP-IDF failures are `EspSleepError::Idf { code, kind }` and log which call failed.

### Added
- ESP32-C3/C2 deep-sleep GPIO wake in `EspSleepManager` via `esp_deep_sleep_enable_gpio_wakeup`: `GpioLevel` masks are validated against `ESP32C3_WAKE_PIN_MASK` (`0x3F`) before any SDK call, and stale wake pins from an earlier attempt are cleared before arming.
  The C3 hardware does not reject deep sleep for a wake pin already at its wake level (bench H1), so `try_sleep` reads the input-enabled wake pins right before sleep entry and returns the `Rejected` error itself instead of sleeping and waking at once.
- Example `idf_c3_deep_sleep_gpio` (ESP32-C3-DevKitM-1, HC-SR501 PIR on GPIO4): GPIO plus timer wake, a `try_sleep` retry loop that survives a held PIR output, and `BENCH` log lines for the bench plan; `C3_SLEEP_TIMER_MS` overrides the timer at build time.
- `scripts/build-example.sh`, `run-example.sh` and `ensure-bootloader.sh` accept the `c3` chip (`idf_c3_*` examples, target `riscv32imc-esp-espidf`); the Xtensa GCC setup is skipped for it.
- `just check-c3`, the C3 target in `.cargo/config.toml.dist`, and a `check-c3` CI job.
- Bench runbook `docs/bench/c3-idf-pir-bench-steps-v1.md` for the P0 cases a real PIR can drive.
- `rustyfarian-esp-hal-power` crate scaffold: the bare-metal (`no_std`, esp-hal `=1.2.2`, MSRV 1.95) hardware tier, with four mutually exclusive chip features (`esp32c3` default, `esp32c6`, `esp32`, `esp32s3`) enforced by `compile_error!`, plus `unstable` and `rt`; re-exports `stoker`. No drivers yet; the sleep module lands first.
- `rustyfarian-esp-hal-power::sleep` for the ESP32-C3 and ESP32-S3 (ADR-001): `EspHalSleepManager` owns only `LPWR`; `bind_wake_pins(&mut [&mut Input])` returns a `BoundSleep` that implements `SleepManager` with `type Error = stoker::SleepError`; `EspHalWakeCauseSource::capture(&[&Input])` reads the wake cause at boot. The whole request is validated before any pin or deadline is touched; arming, sleep and post-rejection cleanup run in one critical section; UART TX is drained before arming; listening wake pads outside the binding are refused with `ForeignWakePin`. Replaces the compile-only ADR-001 spike.
- `rustyfarian-esp-hal-power` feature `bench` (internal, unstable, C3-only): marker pin hooks at critical section entry/exit and post-arm guard in sleep and try_sleep; reads `bench::last() -> BenchTimings { drain_us, guard_us }` via `AtomicU32` storage; no API changes without the feature.
- Examples `hal_c3_deep_sleep_gpio` and `hal_c3_sleep_bench` (ESP32-C3, bare-metal esp-hal tier): C3 deep sleep with GPIO4 PIR wake, timer fallback, and 12 bench cases (W1-W3, H1-H8, R1) with case selected at build time by `BENCH_CASE` env var.
- `just build-example-hal-c3 example [features]` and `run-hal-c3 example`; `check-hal` and `clippy-hal` also build the C3 examples; `[target.riscv32imc-unknown-none-elf]` in `.cargo/config.toml.dist` with espflash runner and `-Tlinkall.x` rustflag.
- Example `hal_esp32_bench_stimulus` (ESP32-WROOM bare-metal, esp-hal 1.2.2): ASCII command server on UART0 that drives GPIO25 and GPIO26 via Flex pins for the C3 sleep bench; schedules pulses from device time `Instant`, accepts `P <ch> <H|L> <width_us> [<delay_us> [<end>]]` and `ZZ` to tri-state both channels.
- `scripts/c3-bench.py` (Python 3, pyserial 3.5, PEP 723 header): host-side bench runner that reads the C3 DUT serial log, drives the WROOM stimulus via UART, detects hangs via RTS-triggered reset, writes one CSV row per cycle per BENCH log line, and summarizes pass/fail, hangs, panics, cause histogram, and manual verification items (W1/W2/H5 measurements with LA/PPK2).
- `just bench-c3 *args` recipe wrapping `uv run scripts/c3-bench.py`, so the script is invoked via `just bench-c3 --case H1 --cycles 100 --port /dev/cu.usbserial-210` etc.
- `[target.xtensa-esp32-none-elf]` in `.cargo/config.toml.dist` for ESP32 (classic) bare-metal target, with espflash runner and `-Tlinkall.x` rustflag.
- `just build-example-hal-esp32 example [features]`, `run-hal-esp32 example` and `flash-hal-esp32 example`; `check-hal` and `clippy-hal` also build esp32 examples under the `esp32` feature.
- ESP32-S3 wake causes are normalised per chip: an EXT1 wake reports `WakeCause::Ext1(mask)` on both tiers (the C3 reports `Gpio(mask)`). Hardware-validated on the CrowPanel 1.28" ESP32-S3 on both tiers: touch wake on GPIO5, timer wake, and `Rejected` for an EXT1 level held at entry (the S3 needs no pre-sleep level check, unlike the C2/C3).
- Examples `idf_s3_deep_sleep_gpio` and `hal_s3_deep_sleep_gpio` (CrowPanel 1.28" ESP32-S3, CST816S touch INT on GPIO5 plus a 60 s timer): both power the touch controller (`LCD_3V3` on GPIO1, reset on GPIO13) and hold it through deep sleep, print the wake cause, and retry a rejected sleep.
- `rustyfarian-esp-hal-power` feature `examples` (`rt` plus the optional `esp-bootloader-esp-idf =0.6.0` and `esp-println =0.18.0`, the versions the sibling repos pin).
- `just build-example-hal-s3` and `run-hal-s3`; `check-hal` and `clippy-hal` also build the S3 example; the example scripts accept the `s3` chip; `[target.xtensa-esp32s3-none-elf]` in `.cargo/config.toml.dist`.
- Bench record `docs/bench/s3-crowpanel-touch-sleep-bench-steps-v1.md`.
- `stoker::sleep::Error` (`Debug` + `kind()`, the embedded-hal 1.0 shape, no `core::error::Error` bound) and `SleepErrorKind` (`InvalidConfig`, `Unsupported`, `Rejected`, `Platform`, `Other`; `#[non_exhaustive]`, `Copy`), both re-exported at the crate root; `SleepError` implements it.
- `stoker::SleepError` variants for the 0.3.0 contract: `NoWakeSource`, `Rejected`, `TimerTooShort { duration_ms, min_ms }`, `TimerTooLong { duration_ms, max_ms }`, `PinNotBound { pin }`, `ForeignWakePin { pin }`, `UartBusy { uart }` (kind `Platform`), `IsolationConflict`.
- `SleepManager::try_sleep`: rejection-aware deep sleep. `EspSleepManager` implements it over `esp_deep_sleep_try_to_start()` and reports a rejection as `EspSleepError::Idf` with `kind() == Rejected`, mapped by call site because `ESP_ERR_SLEEP_REJECT` aliases `ESP_ERR_INVALID_STATE`. `NoopSleepManager::rejecting()` returns `SleepError::Rejected` from `try_sleep`, so retry paths are host-testable.
- `stoker::MAX_TIMER_MS`; `stoker::sleep` wake-pin mask constants `ESP32_WAKE_PIN_MASK`, `ESP32S3_WAKE_PIN_MASK`, `ESP32C3_WAKE_PIN_MASK` (`0x3F`) and `ESP32C6_WAKE_PIN_MASK` (`0xFF`), plus `validate_gpio_level_source_against(pin_mask, valid_mask)` for tiers that pick the chip mask themselves.
- `EspWakeCauseSource` fills `WakeCause::Gpio(mask)` from `esp_sleep_get_gpio_wakeup_status()` on chips with deep-sleep GPIO wakeup (C3/C6); on ESP32 and ESP32-S3 the mask is empty.
- `stoker::SleepError`: `GpioMaskZero`, `GpioMaskOutOfRange { pin_mask, valid_mask }`, `MultipleTimers { count }`, `MultipleGpioLevel`; carrying `valid_mask` keeps the variant stable when the C3/C6 masks land.
- `stoker::retain`: `Retained<const N: usize>`, a `no_std`, allocation-free RTC retention envelope (magic, schema version, length, CRC-32) with `empty()`, `load::<T>(ResetReason) -> Result<T, Invalid>` and `store(&T) -> Result<(), StoreError>`; `Retainable` (`VERSION`, `SIZE`, `write`, `read`) with impls for the fixed-width integers; `Invalid` (`PowerOn`, `Brownout`, `Magic`, `Version`, `Length`, `Checksum`, `Payload`) and `StoreError` (`Length`, `Payload`). `load` refuses records after `PowerOn` and `Brownout`; other resets trust the checksum. `store` clears the magic first and writes it last, so a torn store fails the magic check.
- `stoker::retain::RetainedCell<N>`: the `Sync` cell to place a `Retained` record in a `static` in an RTC section (no `static mut`, no reference to the memory ever handed out). It holds the record as `MaybeUninit` and copies it with volatile accesses, so a `NOLOAD` section is read as the bits it holds; `load` checks the `ResetReason` before touching memory; `store` invalidates, writes the record, then writes the magic, each volatile, with a memory fence after the last write; `invalidate()` clears the magic; `new()` is zeroed so every non-`NOLOAD` read is well defined. Not synchronised (no CAS on riscv32imc, no dependencies): use from one execution context or inside the tier's critical section.
- `stoker::reset`: `ResetReason` (`PowerOn`, `Brownout`, `DeepSleep`, `Software`, `Panic`, `Watchdog`, `Other`; `#[non_exhaustive]`), `ResetReasonSource` and `NoopResetReasonSource`, mirroring `WakeCauseSource`. `Panic` separates the panic handler's reset from a clean `Software` reset; `load` trusts the checksum after both.
- `EspResetReasonSource`: the ESP-IDF `ResetReasonSource`, mapping `esp_reset_reason()` per the `stoker::reset` table (`ESP_RST_PANIC` → `Panic`, the three watchdogs → `Watchdog`, `ESP_RST_PWR_GLITCH` → `Brownout` because a glitch may corrupt RTC memory, unknown codes → `Other`).
- `rustyfarian-esp-idf-power` re-exports the `stoker` retention and reset types (`Retained`, `RetainedCell`, `Retainable`, `Invalid`, `StoreError`, `ResetReason`, `ResetReasonSource`, `NoopResetReasonSource`).
- Example `idf_c3_rtc_retention` (ESP32-C3-DevKitM-1, no wiring): a `RetainedCell` in `.rtc_noinit` carries a session id and boot counter through timer wake, `esp_restart`, a panic and an interrupt-watchdog reset, and is refused after a power cycle; `C3_RET_ACTION` (`sleep`, `restart`, `panic`, `wdt`, `cycle`) picks the exit at build time and `C3_SLEEP_TIMER_MS` the timer. `just check-c3` and the CI job cover it.
- `rustyfarian-esp-hal-power::retain::RtcRetainedCell<N>`: a `#[repr(transparent)]` newtype over `stoker::RetainedCell` that implements esp-hal's `Persistable`, so a record can live in a `#[esp_hal::ram(unstable(rtc_fast, persistent))]` static (the orphan rule forbids the impl on the stoker type); forwards `load`, `store` and `invalidate`.
- `rustyfarian-esp-hal-power::reset::EspHalResetReasonSource`: the bare-metal `ResetReasonSource` over `rtc_cntl::reset_reason(Cpu::ProCpu)`, following the `stoker::reset` table with cfg-split arms for the C3 `Cpu0*` and S3 `Cpu*` variant names; never reports `Panic` (a panic handler that restarts arrives as `Software`).
- Example `hal_c3_rtc_retention` (ESP32-C3-DevKitM-1, bare-metal twin of `idf_c3_rtc_retention`): same payload, `RET` line and `C3_RET_ACTION` matrix; `panic` reads `reset=Software` on this tier and `wdt` arms the TIMG0 main watchdog because `esp_hal::init` disables every watchdog.
- `just serial-capture <port> <seconds> [baud]` (`scripts/serial-capture.py`, pyserial via `uv`): timed, timestamped UART capture that keeps DTR and RTS low, for bench runs that must not reset the board on every read.

### Changed
- Workspace: `default-members` keeps the hal tier out of the host and ESP-IDF builds (bare `cargo`, `check-all`, `build-all`, `clippy-all`); new `just` recipes `check-hal`, `check-hal-c3`, `clippy-hal`, `clean-hal`, `release-dry-run-hal` and `release-publish-hal` build into `target/hal` with `-Zbuild-std=core,alloc`.
- `stoker` drops its `anyhow` dependency and now has no dependencies.
- `stoker::sleep` module docs point at `Retained` instead of the `#[link_section = ".rtc.data"] static mut` recipe.
- `rustyfarian-esp-idf-power` re-exports `SleepError`, the sleep `Error` trait, `SleepErrorKind` and `MAX_TIMER_MS`, and exports `EspSleepError`.
- `GpioWakeMask::contains_pin` returns `false` for `pin >= 64` instead of overflowing the shift.
- `EspSleepManager` validates every `GpioLevel` mask before the first ESP-IDF call, so a validation error leaves the wake configuration untouched.
- Docs: the deep-sleep GPIO wakeup (`esp_deep_sleep_enable_gpio_wakeup`) is a C3/C6 feature, not S3 as previously stated.
- ADR-001 (HAL-neutral sleep contract) is widened from the C3 to the C3 and S3, with the S3 path allocation, pad isolation and bench results in § 6.
- `rustyfarian-esp-idf-power` module docs and `docs/hardware-setup.md` replace the `#[link_section = ".rtc.data"] static mut BOOT_COUNT` recipe with a `RetainedCell` in `.rtc_noinit` read through `EspResetReasonSource`.
- `rustyfarian-esp-hal-power` gates `sleep`, `reset` and `retain` on its `unstable` feature (they use esp-hal's unstable `rtc_cntl`, sleep API and `Persistable`); a build without `unstable` is the `stoker` re-export only instead of a compile failure, and `just check-hal` builds that surface once.
- The `stoker::reset` mapping table names the per-chip esp-hal variants: `Cpu0Sw`/`Cpu0*Wdt` on the C3 and `CpuSw`/`Cpu*Wdt` on the S3, and `CorePwrGlitch` under `Brownout`.
- Example `idf_c3_deep_sleep_gpio` reports the reset through `EspResetReasonSource` instead of its own `esp_reset_reason()` mapping; the `BENCH` `reset=` names are unchanged for the resets the bench drives, while `ESP_RST_UNKNOWN` (which is how ESP-IDF v5.3.3 reports a power glitch on the C3) now reads `PowerOn` instead of `Other`, and the `Reset reason:` line drops the raw code.

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
