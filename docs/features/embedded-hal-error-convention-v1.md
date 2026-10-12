---
gate: none
desk-work: available
---

# Feature: embedded-hal Error Convention v1

## Decisions
|                                             Decision | Reason                                                              | Rejected Alternative                 |
|-----------------------------------------------------:|:--------------------------------------------------------------------|:-------------------------------------|
|      `SleepManager` has `type Error` now (0.2 style) | IDF impl keeps `anyhow`; landed with the no_std step                | Fixed `SleepError` with IDF variants |
| Adopt embedded-hal 1.0 shape: `ErrorType` + `kind()` | Generic code can tell config errors from platform failures          | Bare `type Error: Debug` long-term   |
|                 Land in 0.3.0, before C3 `try_sleep` | `SleepManager` breaks once; reject error is a `kind()` from day one | 0.4.0 after the hal tier             |

## Constraints
- `stoker` stays `#![no_std]` with no dependencies.
- The `type Error` change and the `ErrorType`/`kind()` convention ship in the same release (0.3.0); the no_std chunk is not released on its own.
- The kind enum is `#[non_exhaustive]` and `Copy`, like embedded-hal's `ErrorKind`.
- `SleepError` keeps its variants and messages; it implements the new error trait.
- `EspSleepManager` keeps `anyhow::Error`, so the IDF crate needs a local error type or wrapper to provide `kind()`.

## Open Questions
- [ ] Which kind variants: `InvalidConfig`, `Rejected`, `Platform`, `Other`?
- [ ] Should the error trait require `core::error::Error` (spookhouse review, 2026-10-12)? `anyhow::Error` does not implement it, so `EspSleepManager` would need a local error type; embedded-hal 1.0 bounds only `Debug` + `kind()`.
- [ ] Mirror the pattern locally, or depend on `embedded-hal` for naming parity only?
- [ ] How does `EspSleepManager` provide `kind()` without losing `anyhow` context?
- [ ] Does `ESP_ERR_SLEEP_REJECT` map to `Rejected` (see c3-gpio-deep-sleep-wake-v1)?

## State
- [x] Design approved
- [ ] Error trait + kind enum in `stoker`, implemented for `SleepError`
- [ ] `EspSleepManager` error provides `kind()`
- [ ] Tests passing (`just verify`, device checks)
- [ ] Documentation updated (CHANGELOG, key-insights)

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog; follows the stoker no_std step
- 2026-10-12 — Updated: open question on a `core::error::Error` bound from the spookhouse review
