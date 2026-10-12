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
- [ ] Does CI need `-Z build-std` for `riscv32imc-esp-espidf` the way rustyfarian-network's job does?

## State
- [x] Design approved
- [x] `stoker`: chip mask constants, `validate_gpio_level_source_against`, `Gpio(mask)`, `try_sleep` on trait and `NoopSleepManager`
- [ ] IDF tier: C3/C2 GPIO wake path, stale-mask handling, `Gpio(mask)` mapping, isolation rule per chip
- [ ] `just check-c3` recipe, `.cargo/config.toml.dist` target, CI job
- [ ] Example `idf_c3_deep_sleep_gpio` wakes on GPIO4, shows one rejected `try_sleep`, sleeps again; documents GPIO4 = MTMS (JTAG) and the pull against the wake level
- [ ] Bench case `wake-pin-held-at-entry` on the C3 DevKitM-1: PIR held HIGH when `try_sleep` is called, expect `Err` with `kind() == Rejected` (ADR-001 § 5)
- [x] S3 claim fixed in `sleep.rs`, `esp_sleep.rs`, `docs/key-insights.md`
- [ ] Tests passing (`just verify`, `just check-c3`)
- [ ] CHANGELOG `[Unreleased]`: Added (C3 wake, `try_sleep`) and Breaking (`WakeCause::Gpio(GpioWakeMask)`)

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog from the spookhouse request
- 2026-10-12 — ADR-001 settles `try_sleep` as a required method, the reject error, wake-pin validity by chip constant, the GPIO4/JTAG note, the named bench case and the slept-time scope
- 2026-10-12 — `stoker` part landed (gap 1 of the C3 bench plan): `try_sleep`, `NoopSleepManager::rejecting()`, `Gpio(GpioWakeMask)`, chip masks incl. C3 `0x3F`; IDF tier got `try_sleep` and the `Gpio(mask)` read (cfg `esp_idf_soc_gpio_support_deepsleep_wakeup`), not yet the C3 wake path
