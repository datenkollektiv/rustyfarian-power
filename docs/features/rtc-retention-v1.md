---
gate: none
desk-work: available
---

# Feature: RTC Retention Record v1

A validity-checked, `no_std`, allocation-free retention envelope in `stoker`, placed in both tiers, so a few words survive deep sleep and ordinary resets without per-firmware magic-and-checksum records.
Requested by rustbox-spookhouse; the envelope format is documented in `stoker::retain`, the cell and provider rules in `docs/project-lore.md` (stoker — RTC retention cell).

## Decisions
| Decision | Reason | Rejected Alternative |
|---:|:--|:--|
| IDF placement in `.rtc_noinit` | Watchdog and software resets keep the record | `.rtc.data` plus `empty()`, deep sleep only |
| Envelope of integers and a byte array with magic, version, length, CRC-32 | Arbitrary bits after power loss are never UB; padding is never checksummed | Placing an arbitrary `T` in RTC memory |
| `load` treats `PowerOn` and `Brownout` (also `ESP_RST_PWR_GLITCH`) as invalid | A brownout may leave a partial record that still checksums by chance | Checksum only |
| `ResetReasonSource` trait and `#[non_exhaustive]` `ResetReason` with `Panic` | Resets and sleep stay separate; variants can follow after 0.3.0 | Reset variants on `WakeCause::PowerOn` |
| `RetainedCell<N>` in `stoker`, methods inside `critical_section::with` | One sound volatile read/write path; a `Sync` cell with safe methods cannot race | Per-tier cells; `unsafe fn` methods |
| No A/B slot pair | A torn record is rejected and the consumer starts fresh, as a stale slot would | Two slots with a sequence number |
| `Retainable` carries a required `const VERSION: u16` | A default lets schemas drift silently | Envelope-level version only |
| Tiers add placement only: IDF `#[link_section]`, hal `RtcRetainedCell` newtype | IDF has no `Persistable` bound; the orphan rule forces the hal newtype | `rtc_retained!` macro; `Deref` |

## Constraints
- `N >= T::SIZE` is checked at runtime (`Invalid::Length`); stable Rust cannot express it in the type.
- A `static` initialiser in `.rtc_noinit` is never applied; `empty()` is only for `.rtc.data` placement.
- Two payload types with equal `VERSION` and `SIZE` load each other's bytes.
- Every firmware links exactly one interrupt-safe `critical-section` provider (esp-hal `rt`, or the IDF tier's opt-in `critical-section-impl`).
- IDF `.rtc_noinit` is RTC fast memory on the C3 and RTC slow memory on ESP32 and S3; each tier keeps its domain powered in deep sleep.
- NVS or flash persistence, full `PresenceSession` retention and a pending-report queue are out of scope.

## Consumer Requirements
- rustbox-spookhouse retains about 60 bytes: session id, report sequence, encounter counter, a pending report slot and a session snapshot.
- The record survives GPIO and timer wakes, software and watchdog resets, and is refused after power-on or brownout so the receiver deduplicates on session id plus sequence.
- `new_session_id()` cannot run at every boot (the RNG is weak with the radio off), so it depends on the retained counter.

## State
- [ ] Helper for plain-data `Retainable` payloads, deferred until the spookhouse impl shows what repeats.
- [ ] Heltec V3 (IDF), requested by rustyfarian-beekeeper: 15-minute timer sleep, sleep on battery only, RTC clock drift, brownout.
