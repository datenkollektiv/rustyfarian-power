---
gate: none
desk-work: available
---

# Feature: ESP32-S3 Deep Sleep with GPIO Wake v1

S3 deep sleep with a GPIO wake on both tiers, upstream request U4 from rustyfarian-knob.
Scope extension of the C3-first esp-hal sleep tier; size M.
Origin: rustyfarian-knob request U4 (filed 2026-10-13), https://github.com/datenkollektiv/rustyfarian-knob/blob/main/docs/ROADMAP.md.
Follows [esp-hal sleep tier v1](esp-hal-sleep-tier-v1.md) and ADR-001 (amended to C3+S3).

## Decisions
|                                                Decision | Reason                                                             | Rejected Alternative                       |
|--------------------------------------------------------:|:-------------------------------------------------------------------|:-------------------------------------------|
| Promote the ADR-001 spike to an exported `sleep` module | A hal example needs public API; the spike is private and C3-only   | cfg arms on the spike only, no hal example |
|                      One module with C3 and S3 cfg arms | Keeps the ADR's C3-first order; the S3 is a second arm, not a fork | S3-only module                             |
|                   Keep the `bind_wake_pins` binding API | esp-hal 1.2.2 assigns S3 pins to ext0/ext1/per-pin internally      | S3-specific ext1 binding API               |
|                   S3 wake-pin mask from `stoker` (0-21) | Same validation and `ForeignWakePin` scan range on both tiers      | Hard-coded C3 `0x3F` set                   |
|                IDF tier: example only, no driver change | EXT1 path and GPIO0-21 validation already exist (`esp_sleep.rs`)   | New S3-specific IDF arm                    |
|                          ADR-001 amended to C3+S3 scope | Contract is chip-neutral; S3 facts and bench results belong there  | Separate S3 ADR                            |

## Consumer Requirements

The knob ships power features on both runtimes in lockstep, so the S3 needs the same sleep contract on esp-hal and ESP-IDF.
Validation hardware is the CrowPanel 1.28" ESP32-S3 with CST816S touch INT on GPIO5 (an RTC/LP pad).
Knob maintainer contributes the implementation; contribution is offered.

## Constraints
- esp-hal `=1.2.2` exact pin; every S3 claim is checked against the registry source.
- Wake source on the knob is CST816S touch INT on GPIO5, active-low, `Pull::Floating`.
- The CST816S needs `LCD_3V3` (GPIO1) on and RST (GPIO13) high, held through sleep, or INT idles LOW.
- GPIO41/42/45 (encoder, button) are not RTC pads and cannot wake from deep sleep; must be rejected with `GpioMaskOutOfRange`.
- The bench CrowPanel's console is native USB-Serial-JTAG, not a CH340; it drops off the bus in deep sleep.
- `ext1` has one level for its whole mask; mixed levels keep the LP peripherals powered.
- Bench only on the CrowPanel 1.28" ESP32-S3; the C3 hal bench stays open in esp-hal-sleep-tier-v1.
- Release is lockstep with `stoker` first and decided separately; nothing is committed without asking.

## Open Questions
- [x] Does the S3 refuse deep sleep with the ext1 pin at its wake level? Yes, on both tiers; no pre-check needed.
- [x] Does GPIO5 keep its idle level in deep sleep? Yes, held HIGH, once `LCD_3V3` and touch RST are on and held.
- [x] Normalise the wake cause? Per chip: S3 reports `Ext1(mask)` on both tiers, C3 `Gpio(mask)` (ADR-001 § 1).
- [x] A resting finger pulses INT and loops wakes; which CST816S IRQ mode or firmware wait should the knob use? — Measured 0.3–0.6 s here and ~0.7 s in the knob run; the IRQ mode or wait is the knob's decision.

## Acceptance Criteria
- `just check-hal` and the clippy recipe pass for `esp32s3` with the sleep module enabled.
- `hal_s3_deep_sleep_gpio` wakes on a touch and reports bit 5 in the agreed `WakeCause` variant.
- The IDF twin wakes on the same touch and reports `WakeCause::Ext1` with bit 5 set.
- A mask containing GPIO41, 42 or 45 is rejected with `GpioMaskOutOfRange` on both tiers.
- ADR-001 lists the S3 bench results next to the C3 ones.

## State
- [x] Design approved
- [x] esp-hal `sleep` module exported with C3 and S3 arms; spike removed
- [x] Example builds for the hal crate (`build-example-hal-s3`, `run-hal-s3`)
- [x] `hal_s3_deep_sleep_gpio` and `idf_s3_deep_sleep_gpio` examples
- [x] ADR-001 amended to C3+S3
- [x] S3 bench doc; touch wake 3/3, timer, held finger and held level per tier
- [x] README, CHANGELOG `[Unreleased]`, `maintenance-plan.md` hardware row
- [x] Tests passing (`just verify`, `just check-hal`, `just clippy-hal`)

## Session Log
- 2026-10-14 — Feature doc created via /feature from the knob's U4 request
- 2026-10-14 — Implemented both tiers; CrowPanel bench passed W1/W2/H1 on IDF and esp-hal; open questions answered
- 2026-10-14 — Bench tooling removed; examples reduced to lightweight demos; bench doc kept as a record
- 2026-10-14 — Folded the knob U4 request into this doc; review-queue copy retired
- 2026-10-28 — Knob feedback: examples stay awake 3 s per boot, `serial-capture` keeps DTR/RTS and reopens the port, resting-finger re-wake ~0.7 s
