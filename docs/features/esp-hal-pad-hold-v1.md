---
gate: none
desk-work: available
---

# Feature: esp-hal public pad-hold API v1

Use esp-hal's `Output::set_pad_hold()` to hold output pad levels through deep sleep without raw register writes.

Request origin: `rustyfarian-knob` esp-hal parity quest (milestone M9), https://github.com/datenkollektiv/rustyfarian-knob/blob/main/docs/ROADMAP.md.

## Decisions

|                                                Decision | Reason                                                                       | Rejected Alternative                                                                                                               |
|--------------------------------------------------------:|:-----------------------------------------------------------------------------|:-----------------------------------------------------------------------------------------------------------------------------------|
| Use `Output::set_pad_hold()` directly, no crate wrapper | `Output` exposes no public pin number, so a wrapper cannot validate GPIO0–21 | Per-pin guards (`PadHold::hold(&Output<'_>) -> Result<Guard>`), mask-based pairs (`hold_pads`/`release_pads`), manager integration |

## Constraints

- esp-hal `=1.2.2` exact pin; pad-hold registers are validated against the registry source.
- S3 only for v1; C3 and other chips arm in a later release if the uptake justifies it.
- Holdable pads are GPIO0–21 on the S3 (via `RTC_CNTL_PAD_HOLD`); pad-hold applies to both wake and non-wake pins.
- The API is safe and does not expose raw registers.
- `Output::new()` calls `init_gpio` which releases the pad hold *before* `set_level` is called, so the output pad is briefly undriven on a wake boot; firmware then sets the level and re-applies the hold via `set_pad_hold(true)`.

## Open Questions

- [x] Which shape fits better: per-pin guard type (like `PadHold::hold(&Output<'_>) -> Result<Guard>`, auto-release on drop), or a mask-based function pair (`hold_pads(mask) / release_pads(mask)`)? — Answer: no crate wrapper needed; use `Output::set_pad_hold()` directly.
- [x] Should the API live on `EspHalSleepManager` or directly on `LPWR`, or both? — Answer: on `Output` (esp-hal 1.2.2 `Output::set_pad_hold(&mut self, bool)` and `Output::is_pad_held(&self) -> bool`).
- [x] Does C3 use the same register arm, or does it need its own gate? — Answer: C3 has the same method (unstable, `#[instability::unstable]` with `lp_io_driver_supported`); not yet benched for non-wake hold scenarios.

## State

- [x] Design approved and shape decided
- [x] Core implementation found in esp-hal 1.2.2 (`Output::set_pad_hold`, unstable)
- [x] `hal_s3_deep_sleep_gpio` example converted to use `Output::set_pad_hold()`
- [ ] Bench validation: CrowPanel touch wake with GPIO1/13 held (open for user)

## Session Log

- 2026-10-14 — Feature doc created from the knob U4b request
- 2026-10-15 — Design panel found esp-hal 1.2.2 already has Output::set_pad_hold; scope reduced to example + docs (user decision)
