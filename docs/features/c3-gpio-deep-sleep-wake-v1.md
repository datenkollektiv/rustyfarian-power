---
gate: none
desk-work: available
---

# Feature: ESP32-C3 GPIO Deep-Sleep Wake v1

Make `stoker` and `rustyfarian-esp-idf-power` serve the ESP32-C3, which has no EXT0/EXT1 and wakes from deep sleep via `esp_deep_sleep_enable_gpio_wakeup`.
Requested by rustbox-spookhouse (PIR on GPIO4 wakes a battery prop); request copy in `review-queue/rustyfarian-power-c3-gpio-deep-sleep-wake-v1.md`.
Verified 2026-10-12 against ESP-IDF v5.3.3 headers: `SOC_GPIO_SUPPORT_DEEPSLEEP_WAKEUP` is C3/C6, not S3; the S3 claim in `sleep.rs`, `esp_sleep.rs` and `docs/key-insights.md` is wrong.

## Decisions
|                                                                    Decision | Reason                                                                      | Rejected Alternative                                              |
|----------------------------------------------------------------------------:|:----------------------------------------------------------------------------|:------------------------------------------------------------------|
|                    Ship in the planned 0.3.0, after the `stoker` error enum | `try_sleep` lands on the final error type, one breaking release             | 0.3.0 from this work alone; additive 0.2.1 with a maskless `Gpio` |
|   `WakeCause::Gpio(GpioWakeMask)` from `esp_sleep_get_gpio_wakeup_status()` | `contains_pin` works for both wake families                                 | Keep the unit variant and let consumers call the FFI              |
|                  Caller-supplied valid mask plus chip constants in `stoker` | `riscv32imc-esp-espidf` is C2 and C3; `build.rs` cannot tell them apart     | More `build.rs` cfgs from the target triple                       |
|                   IDF tier picks the constant from the esp-idf-sys chip cfg | Keeps `stoker` chip-agnostic                                                | Chip feature flags on `stoker`                                    |
| `SleepManager::try_sleep` default method over `esp_deep_sleep_try_to_start` | A latched PIR holds HIGH; the caller needs a retry path, not a silent sleep | Sleep anyway and let the level wake the chip immediately          |
|                                                `check-c3` recipe runs in CI | Folds into the roadmap's dual-target CI matrix item                         | Local recipe only                                                 |

## Constraints
- C3 deep-sleep wake pins are GPIO0-5 (`0x3F`); C6 is GPIO0-7 (`0xFF`); the EXT1 path stays for ESP32, S3 and C6.
- `esp_sleep_disable_wakeup_source(ALL)` does not clear `gpio_wakeup_mask`; stale pins must be cleared or the limitation documented.
- `GpioLevel` carries one level per source; IDF allows one trigger bit per pin, which may be exposed later.
- `contains_pin` must be bounded for `pin >= 64` before it becomes the main path.
- Depends on parity v1 step "stoker no_std + error enum" landing first.
- Pin the IDF API claims to v5.3.3; IDF 6 renames the esp-idf-hal wrapper.

## Open Questions
- [ ] Do GPIO isolation and a held wake pin coexist on the C3, or must wake-mask pins be excluded from isolation (bench on the C3 DevKitM-1)?
- [ ] Which `stoker` error variants cover `ESP_ERR_SLEEP_REJECT` versus configuration failures? (`kind()` mapping: embedded-hal-error-convention-v1)
- [ ] Does CI need `-Z build-std` for `riscv32imc-esp-espidf` the way rustyfarian-network's job does?

## State
- [x] Design approved
- [ ] `stoker`: chip mask constants, `validate_gpio_level_source_against`, `Gpio(mask)`, `try_sleep` on trait and `NoopSleepManager`
- [ ] IDF tier: C3/C2 GPIO wake path, stale-mask handling, `Gpio(mask)` mapping, isolation rule per chip
- [ ] `just check-c3` recipe, `.cargo/config.toml.dist` target, CI job
- [ ] Example `idf_c3_deep_sleep_gpio` wakes on GPIO4, shows one rejected `try_sleep`, sleeps again
- [ ] S3 claim fixed in `sleep.rs`, `esp_sleep.rs`, `docs/key-insights.md`
- [ ] Tests passing (`just verify`, `just check-c3`)
- [ ] CHANGELOG `[Unreleased]`: Added (C3 wake, `try_sleep`) and Breaking (`WakeCause::Gpio(GpioWakeMask)`)

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog from the spookhouse request
