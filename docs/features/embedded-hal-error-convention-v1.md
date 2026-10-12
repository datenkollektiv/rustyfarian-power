---
gate: none
desk-work: available
---

# Feature: embedded-hal Error Convention v1

## Decisions
|                                                                                                                                                                           Decision | Reason                                                                                                         | Rejected Alternative                 |
|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------:|:---------------------------------------------------------------------------------------------------------------|:-------------------------------------|
|                                                                                                                                    `SleepManager` has `type Error` now (0.2 style) | IDF impl keeps `anyhow`; landed with the no_std step                                                           | Fixed `SleepError` with IDF variants |
|                                                                                                                               Adopt embedded-hal 1.0 shape: `ErrorType` + `kind()` | Generic code can tell config errors from platform failures                                                     | Bare `type Error: Debug` long-term   |
|                                                                                                                                               Land in 0.3.0, before C3 `try_sleep` | `SleepManager` breaks once; reject error is a `kind()` from day one                                            | 0.4.0 after the hal tier             |
|                                                    Kinds `InvalidConfig`, `Unsupported`, `Rejected`, `Platform`, `Other` ([ADR-001](../adr/001-hal-neutral-sleep-contract.md) § 3) | Config, rejection and platform failures are the branches a consumer takes; `Unsupported` is reserved           | Three kinds only                     |
|                                                                                             `trait sleep::Error: Debug { fn kind() }`, no `core::error::Error` bound (ADR-001 § 3) | embedded-hal 1.0 shape; keeps `defmt`-only errors possible; `SleepError` still implements `core::error::Error` | Require `core::error::Error`         |
|                                                                                                `ESP_ERR_SLEEP_REJECT` maps to `Rejected` by call site, never by code (ADR-001 § 3) | It aliases `ESP_ERR_INVALID_STATE` (`0x103`) in v5.3.3                                                         | Match on the code                    |
|                                                                IDF tier: `EspSleepError { Config(SleepError), Idf { code, kind } }` replaces `anyhow` in the manager (ADR-001 § 3) | `anyhow::Error` cannot implement the trait (orphan rule)                                                       | Wrapper around `anyhow::Error`       |
| `SleepError` gains `NoWakeSource`, `Rejected`, `TimerTooShort`, `TimerTooLong`, `PinNotBound`, `ForeignWakePin`, `UartBusy` (kind `Platform`), `IsolationConflict` (ADR-001 § 3-4) | Additive on a `#[non_exhaustive]` enum; the hal tier uses `SleepError` directly                                | Hal-local error enum                 |

## Constraints
- `stoker` stays `#![no_std]` with no dependencies.
- The `type Error` change and the `ErrorType`/`kind()` convention ship in the same release (0.3.0); the no_std chunk is not released on its own.
- The kind enum is `#[non_exhaustive]` and `Copy`, like embedded-hal's `ErrorKind`.
- `SleepError` keeps its variants and messages; it implements the new error trait.
- `EspSleepManager` moves from `anyhow::Error` to the local `EspSleepError`; consumer code that bounded `S::Error: Debug` keeps compiling, third-party implementors on `anyhow::Error` need a newtype (CHANGELOG migration note).

## Open Questions
- [x] Which kind variants? `InvalidConfig`, `Unsupported`, `Rejected`, `Platform`, `Other` (ADR-001 § 3).
- [x] Should the error trait require `core::error::Error` (spookhouse review, 2026-10-12)? No (ADR-001 § 3). `anyhow::Error` does not implement it, so `EspSleepManager` would need a local error type; embedded-hal 1.0 bounds only `Debug` + `kind()`.
- [x] Mirror the pattern locally, or depend on `embedded-hal` for naming parity only? Locally; `stoker` keeps zero dependencies (ADR-001 § 3).
- [x] How does `EspSleepManager` provide `kind()` without losing `anyhow` context? Local `EspSleepError` carrying the `esp_err_t` code; the `anyhow` context inside the manager goes (ADR-001 § 3).
- [x] Does `ESP_ERR_SLEEP_REJECT` map to `Rejected` (see c3-gpio-deep-sleep-wake-v1)? Yes, by call site (ADR-001 § 3).

## State
- [x] Design approved
- [x] Design questions closed by ADR-001; this feature is implementation only
- [ ] Error trait + kind enum in `stoker`, implemented for `SleepError`
- [ ] `EspSleepManager` error provides `kind()`
- [ ] Tests passing (`just verify`, device checks)
- [ ] Documentation updated (CHANGELOG, key-insights)

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog; follows the stoker no_std step
- 2026-10-12 — Updated: open question on a `core::error::Error` bound from the spookhouse review
- 2026-10-12 — All open questions answered by ADR-001 § 3 (kinds, no `core::error::Error` bound, `EspSleepError`, reject mapping by call site); implementation only from here
- 2026-10-12 — ADR-001 review round 2: `TimerOverflow` replaced by `TimerTooLong`; `ForeignWakePin` and `UartBusy` (the only `Platform` variant) added
