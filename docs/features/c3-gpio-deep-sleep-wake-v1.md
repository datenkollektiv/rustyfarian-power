---
gate: none
desk-work: available
---

# Feature: ESP32-C3 GPIO Deep-Sleep Wake v1

Make `stoker` and `rustyfarian-esp-idf-power` serve the ESP32-C3, which has no EXT0/EXT1 and wakes from deep sleep via `esp_deep_sleep_enable_gpio_wakeup`.
Requested by rustbox-spookhouse (PIR on GPIO4 wakes a battery prop); request copy in `review-queue/rustyfarian-power-c3-gpio-deep-sleep-wake-v1.md`.
Verified 2026-10-12 against ESP-IDF v5.3.3 headers: `SOC_GPIO_SUPPORT_DEEPSLEEP_WAKEUP` is C3/C6, not S3; the S3 claim in `sleep.rs`, `esp_sleep.rs` and `docs/key-insights.md` is wrong.

## Decisions
|                                                                                                                                Decision | Reason                                                                                                                              | Rejected Alternative                                                          |
|----------------------------------------------------------------------------------------------------------------------------------------:|:------------------------------------------------------------------------------------------------------------------------------------|:------------------------------------------------------------------------------|
|                                                                                Ship in the planned 0.3.0, after the `stoker` error enum | `try_sleep` lands on the final error type, one breaking release                                                                     | 0.3.0 from this work alone; additive 0.2.1 with a maskless `Gpio`             |
|                                                               `WakeCause::Gpio(GpioWakeMask)` from `esp_sleep_get_gpio_wakeup_status()` | `contains_pin` works for both wake families                                                                                         | Keep the unit variant and let consumers call the FFI                          |
|                                                                              Caller-supplied valid mask plus chip constants in `stoker` | `riscv32imc-esp-espidf` is C2 and C3; `build.rs` cannot tell them apart                                                             | More `build.rs` cfgs from the target triple                                   |
|                                                                               IDF tier picks the constant from the esp-idf-sys chip cfg | Keeps `stoker` chip-agnostic                                                                                                        | Chip feature flags on `stoker`                                                |
| `SleepManager::try_sleep` (required method, [ADR-001](../adr/001-hal-neutral-sleep-contract.md) § 1) over `esp_deep_sleep_try_to_start` | A latched PIR holds HIGH; the caller needs a retry path, not a silent sleep; a default delegating to `sleep` would sleep through it | Sleep anyway and let the level wake the chip immediately; default `try_sleep` |
|                                                                                                            `check-c3` recipe runs in CI | Folds into the roadmap's dual-target CI matrix item                                                                                 | Local recipe only                                                             |
|                           Wake-pin validity by the `0x3F` chip constant in `stoker`, not `esp_sleep_is_valid_wakeup_gpio` (ADR-001 § 5) | On the C3 the SDK call tests the same `SOC_GPIO_DEEP_SLEEP_WAKE_VALID_GPIO_MASK`; typed, host-tested, same on both tiers            | Per-bit SDK call in the IDF tier                                              |
|                                           Rejection is `SleepError::Rejected` (`kind() == Rejected`), mapped by call site (ADR-001 § 3) | `ESP_ERR_SLEEP_REJECT` aliases `ESP_ERR_INVALID_STATE`                                                                              | Map by code                                                                   |

## Constraints
- C3 deep-sleep wake pins are GPIO0-5 (`0x3F`); C6 is GPIO0-7 (`0xFF`); the EXT1 path stays for ESP32, S3 and C6.
- `esp_sleep_disable_wakeup_source(ALL)` does not clear `gpio_wakeup_mask`; stale pins must be cleared or the limitation documented.
- `GpioLevel` carries one level per source; IDF allows one trigger bit per pin, which may be exposed later.
- `contains_pin` must be bounded for `pin >= 64` before it becomes the main path.
- Depends on parity v1 step "stoker no_std + error enum" landing first.
- Pin the IDF API claims to v5.3.3; IDF 6 renames the esp-idf-hal wrapper.
- GPIO4 is MTMS (JTAG TMS); GPIO4-7 are the JTAG group. The example documents that a PIR on GPIO4 disables JTAG debugging while the firmware runs.
- `sleep(&[])` and `try_sleep(&[])` return `SleepError::NoWakeSource` from 0.3.0 on both tiers (ADR-001 § 4); "off until reset" is no longer reachable through `stoker`.
- `Timer` durations above `MAX_TIMER_MS` (`u32::MAX` ms) return `SleepError::TimerTooLong` on both tiers (ADR-001 § 4a).
- Slept time after a wake is out of scope for 0.3.0; firmware stores `esp_rtc_get_time_us()` in the retention envelope before sleeping (ADR-001 § 5).

## Open Questions
- [ ] Do GPIO isolation and a held wake pin coexist on the C3, or must wake-mask pins be excluded from isolation (bench on the C3 DevKitM-1)?
- [x] Which `stoker` error variants cover `ESP_ERR_SLEEP_REJECT` versus configuration failures? `SleepError::Rejected` (kind `Rejected`) by call site; everything else `InvalidConfig` or `Platform` (ADR-001 § 3).
- [x] Does CI need `-Z build-std` for `riscv32imc-esp-espidf` the way rustyfarian-network's job does? Yes: `build-std` comes from `.cargo/config.toml`, so the `check-c3` job installs the `esp` toolchain (`esp-rs/xtensa-toolchain`) and overrides `RUSTUP_TOOLCHAIN` per job. The job has not run yet; it first runs on the push.

## State
- [x] Design approved
- [x] `stoker`: chip mask constants, `validate_gpio_level_source_against`, `Gpio(mask)`, `try_sleep` on trait and `NoopSleepManager`
- [x] IDF tier: C3/C2 GPIO wake path, stale-mask handling, `Gpio(mask)` mapping, isolation rule per chip
- [x] `just check-c3` recipe, `.cargo/config.toml.dist` target, CI job
- [x] Example `idf_c3_deep_sleep_gpio` wakes on GPIO4, shows one rejected `try_sleep`, sleeps again; documents GPIO4 = MTMS (JTAG) and the pull against the wake level
- [x] Bench case `wake-pin-held-at-entry` on a C3: PIR held HIGH when `try_sleep` is called; expected `Err` with `kind() == Rejected` (ADR-001 § 5), observed instead an immediate GPIO wake on every attempt; with the manager's wake-pin pre-check (2026-10-13) the expected `Rejected` now holds, 5 of 5; steps in [c3-idf-pir-bench-steps-v1](../bench/c3-idf-pir-bench-steps-v1.md)
- [x] S3 claim fixed in `sleep.rs`, `esp_sleep.rs`, `docs/key-insights.md`
- [x] Tests passing (`just verify`, `just check-c3`)
- [x] CHANGELOG `[Unreleased]`: Added (C3 wake, `try_sleep`) and Breaking (`WakeCause::Gpio(GpioWakeMask)`)

## Implementation Notes (ESP-IDF v5.3.3)
Moved here from the `esp_sleep.rs` module docs, which describe only the API behaviour; re-verify against the IDF source before the next IDF bump.
- Path selection: `cfg(all(esp_idf_soc_gpio_support_deepsleep_wakeup, not(esp_idf_soc_pm_support_ext1_wakeup)))` is C2/C3; C6 has both and keeps EXT1. esp-idf-sys emits the SOC cfgs and `embuild::espidf::sysenv::output()` relays them.
- Valid pins: `SOC_GPIO_DEEP_SLEEP_WAKE_VALID_GPIO_MASK` is GPIO0-5 on C3 (`soc/esp32c3/include/soc/soc_caps.h:175`) and C2 (`soc/esp32c2/include/soc/soc_caps.h:137`).
- Isolation: `esp_sleep_config_gpio_isolate()` only writes each pad's sleep-mode config (`esp_hw_support/sleep_gpio.c:55-62`), and `gpio_deep_sleep_wakeup_enable` switches the wake pin's sleep-select off, so isolation may coexist with a C2/C3 wake pin; bench I4 decides.
- Internal pulls: `gpio_deep_sleep_wakeup_prepare` (`esp_hw_support/sleep_modes.c:1953-1977`) applies `gpio_hold_en` and, with `CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS` (default y, `esp_hw_support/Kconfig:195`), the pull against the wake level.
- Stale arming: `esp_sleep_disable_wakeup_source(ALL)` only clears `RTC_GPIO_TRIG_EN` (`sleep_modes.c:1565-1566`). The manager disarms with `gpio_deep_sleep_wakeup_disable` (`esp_driver_gpio/src/gpio.c:1001-1017`) only pins whose bit in `RTC_CNTL_GPIO_WAKEUP_REG` is set (the test of the inline `gpio_ll_deepsleep_wakeup_is_enabled`, `hal/esp32c3/include/hal/gpio_ll.h:751-756`), because the disable call also re-enables sleep-select (bench I7).
- Unfixable residuals: `s_config.gpio_wakeup_mask` is only ever OR-ed (`sleep_modes.c:2003`), and `gpio_trigger_mode &= ~(mode << idx)` with `LOW == 0` is a no-op (`sleep_modes.c:2004-2008`), so HIGH→LOW keeps the pull-down (bench I6).
- Rejection: `RTC_GPIO_TRIG_EN` is in the C3 `RTC_SLEEP_REJECT_MASK` (`esp_hw_support/port/esp32c3/include/soc/rtc.h:661-663`), yet on hardware a held wake pin is not rejected.
  `esp_deep_sleep_try_to_start` enters deep sleep and the chip wakes about 20 ms later with `cause=Gpio` (bench H1, 2026-10-12).
  `try_sleep` therefore reads each mask pin right before sleep entry: it tests the pad input enable (`FUN_IE` in the pin's IO_MUX register, since arming does not set it), then `gpio_get_level`, and returns `Idf { code: ESP_ERR_INVALID_STATE, kind: Rejected }`, the same shape as the hardware reject.

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog from the spookhouse request
- 2026-10-12 — ADR-001 settles `try_sleep` as a required method, the reject error, wake-pin validity by chip constant, the GPIO4/JTAG note, the named bench case and the slept-time scope
- 2026-10-12 — `stoker` part landed (gap 1 of the C3 bench plan): `try_sleep`, `NoopSleepManager::rejecting()`, `Gpio(GpioWakeMask)`, chip masks incl. C3 `0x3F`; IDF tier got `try_sleep` and the `Gpio(mask)` read (cfg `esp_idf_soc_gpio_support_deepsleep_wakeup`), not yet the C3 wake path
- 2026-10-12 — IDF C3/C2 wake path landed. It is selected by `cfg(all(esp_idf_soc_gpio_support_deepsleep_wakeup, not(esp_idf_soc_pm_support_ext1_wakeup)))`; C6 stays on EXT1.
  The mask is validated against `ESP32C3_WAKE_PIN_MASK` before any SDK call, and the pins are armed with `esp_deep_sleep_enable_gpio_wakeup`.
  Stale pins: `gpio_deep_sleep_wakeup_disable` runs on every unused GPIO0-5. The SDK's own `s_config.gpio_wakeup_mask` cannot be cleared through the public API (documented, plus the HIGH→LOW trigger-mode no-op).
  Isolation rule: `IsolationConflict` stays on every chip until bench I4, documented per chip.
  `WakeCause::Ext1` mapping cfg-gated off C2/C3 (missing symbol broke the link).
  `check-c3` recipe, `.cargo/config.toml.dist` target, CI job, example `idf_c3_deep_sleep_gpio`, `c3` support in the flash scripts, and a PIR runbook were added.
  Gates: `just verify`, `check-c3`, `check-all`, `check-esp32`, `clippy-all` and `build-example idf_c3_deep_sleep_gpio` all exit 0. C3 clippy, the CI job and the hardware were not run.
- 2026-10-12 — Bench P0 with the real PIR (W1, W2, H1, H4): W1 passed on 7 waves; W2 passed covered, 15 of 15 timer wakes at about 5 s; H1 observed (no rejection, immediate wake); H4 blocked by H1.
- 2026-10-12 — Review of `4d8de80`: the example now configures GPIO4 as an input with a pull-down and keeps the driver alive, fails the build on an out-of-range `C3_SLEEP_TIMER_MS`, falls back to a timer-only sleep instead of a restart loop, and prints per-attempt `BENCH` lines only with `C3_BENCH_LOG`.
  `check-c3` now also checks the example, and the CI job builds and links it (ldproxy on).
  Bench case I6 `gpio-level-high-to-low` names the SDK trigger-mode bug; the IDF tier rejects that transition only if I6 shows wrong behaviour.
- 2026-10-12 — Review of `bf680cb`: stale-pin cleanup now disarms only pins whose `RTC_CNTL_GPIO_WAKEUP_REG` enable bit is set, because `gpio_deep_sleep_wakeup_disable` also re-enables sleep-select and would override an application's `gpio_sleep_sel_dis()` on unrelated pins (bench I7).
  I6 now uses a controlled stimulus with an inactive HIGH phase before the `AnyLow` attempt and a separate pull-direction measurement; the example's expected-output block is labelled as `C3_BENCH_LOG=1` output.
- 2026-10-12 — Third review: the module docs were cut down to API behaviour, with the SDK internals moved to Implementation Notes above, and the register read now explains why no public API suffices.
  CI `check-c3` stays "logically correct, pending first run" until the push.
- 2026-10-12 — Bench H1 on an ESP32-C3-MINI-1 board (CH340 bridge, HC-SR501 on GPIO4): a held wake pin is never rejected; the chip sleeps and wakes about 20 ms later, so a stuck-HIGH PIR boot-loops at about 2.5 Hz.
  This contradicts ADR-001 § 2 and § 5 for GPIO on the C3; the ADR and the example's retry loop need a decision before acceptance.
- 2026-10-13 — `try_sleep` on C2/C3 now reads the input-enabled wake pins before sleep entry and returns `Rejected` itself.
  Bench re-run: H1 passes (5 of 5 held wakes rejected, no boot loop); H4 loop behaviour passes, highest count 80 of the planned 1000.
