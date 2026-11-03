# ADR-003: Tier parity contract for the ESP-IDF and esp-hal tiers

## Status
Accepted (2026-11-02).

Aligned with `rustyfarian-network` ADR-022 (tier parity, one async API surface) and ADR-021 (binary-owned providers), and `rustyfarian-ws2812` ADR-010 (interchangeable driver semantics), both accepted 2026-11-01.
Platform pins follow ADR-001: ESP-IDF v5.3.3 and esp-hal `=1.2.2` in lockstep with the sibling repos.

## Context
`stoker` and the two tier crates already implement the three-crate shape the whole family uses: a pure, host-testable core and one thin driver crate per HAL (ADR-001).
The sibling repos have since formalised a hardening wave on top of the same shape, triggered by real bugs.
`rustyfarian-ws2812` ADR-010 records two drivers whose methods had identical names but different semantics, so one downstream firmware behaved differently depending on the tier it was built for.
`rustyfarian-network` ADR-022 records ~17 API and ~8 behaviour gaps between a blocking ESP-IDF tier and an async esp-hal tier, and answers them with shared traits, sans-IO cores, and per-tier parity compile checks.

This repo is exposed to the same family of drift, not only to the same pattern.
The esp-hal tier lacks the ADC battery monitor and the charging monitor that the ESP-IDF tier has (tracked in `docs/features/esp-hal-parity-v1.md`); an async surface is deferred on both tiers, and neither tier manifests an async feature today.
Capability skew between tiers is exactly the drift condition that produced the sibling bugs.
The provider-selection link failure of 2026-10-29 (duplicate `_critical_section_1_0_acquire` with `rustyfarian-esp-idf-network`'s `lora` feature) is the same failure class as network bug 007, and it is already handled here by making `critical-section-impl` opt-in.

The decision below adopts the siblings' contract and establishes the requirements for making the tiers' shared, implemented capabilities interchangeable; today it records requirements and deviations, not achieved interchangeability.
Implementation gaps (the missing esp-hal ADC and charging monitors) are tracked separately, not folded into the promise.
It is a documentation decision: no code changes in this wave.
Enforcement is a committed follow-up (see `docs/features/tier-parity-v1.md`), because an accepted contract without completion conditions is unenforceable.

## Decision

### 1. Same name means the same observable behaviour
A method shared by name across the tiers delivers the same observable outcomes on both: the same validation outcome and error kinds, the same unsuccessful-attempt outcomes (for sleep, `Rejected`), the same successful behaviour, and the same what-remains-usable-after-a-failed-attempt guarantees.
Internal sequencing, timing, and peripheral ownership may differ; a shared name requires identical observables, not identical internals.
The esp-hal `BoundSleep` borrowing model remains valid under this rule.
Validation parity is scoped: portable validation — the shared `validate_wake_sources`, called by both tier managers — must be identical, while platform constraints that follow from the hardware interface (bound-pin checks, the esp-hal `MIN_TIMER_MS` floor, wake-pin masks, isolation conflicts) are documented exceptions, not drift.
Parity is judged for requests within the shared supported operating range, and by portable error kinds (`stoker`'s kind enums), not by identical concrete error values.
Known divergence: after a `Rejected` attempt the tiers currently differ.
ESP-IDF keeps the wake sources and the GPIO isolation config applied, and a hardware rejection leaves the wake pins held (RTC pad holds on the EXT1 chips, `gpio_hold_en` with the internal pull on C2/C3 — both are applied before the hardware's reject decision, so the rejection does not undo them); the C2/C3 software pre-read path in contrast holds no pins; the esp-hal adapter disarms before returning (`sleep.rs` `try_sleep`).
The sleep audit (2026-11-03, `docs/features/tier-parity-v1.md`) fixed the minimum portable postcondition: after any `Rejected` attempt the caller may retry immediately through the same trait object, with no recovery, re-construction, or pin re-binding step, and the retry is observably a fresh attempt — the same validation outcomes and error kinds apply, and a `Timer` duration counts from the retry; every wake pin named in the rejected request's `pin_mask` is usable as a plain digital input once the call has returned (on the esp-hal tier, once the `BoundSleep` borrow ends, which is that tier's release step).
Whether wake sources stay armed between a rejection and its retry is adapter-internal, not an observable: the IDF tier leaves them applied and re-runs configuration on retry; the esp-hal tier disarms on rejection and re-arms on retry.
The postcondition does not cover the isolation state of non-wake pads, timer-deadline persistence, or the C2/C3 within-boot GPIO-wake residue (the SDK mask has no public clear in IDF v5.3.3).
On the ESP32 and ESP32-S3, while the global digital-pad hold is effective (`RTC_CNTL_DIG_ISO_REG` `DG_PAD_AUTOHOLD_EN` set, `DG_PAD_FORCE_UNHOLD` clear), a hardware rejection never returns (bench R2-S3, 2026-11-03); both tiers therefore read the wake pins right before sleep entry in that case and return `Rejected` from software, with the same postcondition (bench R3-S3, 2026-11-04).
A wake pin that reaches its wake level after that read, and a non-pin hardware rejection, can still hang while the hold is effective; this is outside the shared supported operating range and documented on `try_sleep`.
The audit confirmed that every IDF rejection path can meet the postcondition in software; the closing adapter change and its host and bench tests are specified in the feature doc.
Closing the gap is the condition for calling the tiers behaviourally interchangeable on `try_sleep`; renaming the method is not an acceptable closing move, because it would weaken the shared `SleepManager` abstraction.
Where a HAL behaviour cannot be mapped into the shared observables at all, the method gets a different name.
This follows `rustyfarian-ws2812` ADR-010: "methods with the same name mean the same thing on both drivers".

### 2. Hardware differences are capabilities, not APIs
A hardware difference is documented once, as a capability in a tier and chip scoped table, instead of being expressed as a diverging API.
This follows `rustyfarian-network` ADR-022 decision 7.
The table distinguishes three classes — a hardware limitation (no software path exists), an unimplemented driver (the shared capability exists but a tier has not built it), and a behavioural difference (a same-name method that fails point 1).
The missing esp-hal ADC battery monitor and charging monitor are unimplemented drivers, not evidence of semantic divergence.
Interchangeability claims cover only capabilities that both tiers implement.

### 3. Portable policy lives in `stoker`; sequencing lives in the adapters
Portable policy and validation live in `stoker`, where they are host-testable.
Hardware sequencing and register-dependent work remain in the tier adapters: draining UARTs, arming pins, guarding deadlines, entering critical sections, and cleaning up after rejection belong beside the hardware they drive.
Pure decisions that benefit from host tests are extracted into `stoker` when useful, without forcing hardware workarounds into the core.
Existing instances: the `GatedSleep` sequencing coordinator (policy, in `stoker`), and the esp-hal sleep adapter's UART drain, deadline arming, and rejection cleanup (sequencing, correctly in the adapter).

### 4. API conformance is checked at compile time; behavioural parity is tested separately
Each tier compiles checked-in conformance modules that call the `stoker` traits generically, so a trait change that breaks one tier breaks its build the same way.
The conformance call must be instantiated or type-checked with the tier's concrete adapter types inside that tier's build: a generic `fn check<S: SleepManager>(..)` alone compiles without proving the real adapter implements the trait.
Each target build therefore instantiates the call with its concrete types, including `BoundSleep` and its lifetimes.
Coverage is per capability: a tier is checked for the capabilities it implements, not for every `stoker` trait.
Compilation verifies trait bounds and callable methods; it cannot verify rejection handling, wake-pin cleanup, timer accuracy, or wake-cause preservation.
Behavioural parity is therefore tested in `stoker`'s host tests where portable, and on the hardware benches required by ADR-001, which remain the arbiter for sleep behaviour.
The conformance modules are a committed follow-up with acceptance criteria in `docs/features/tier-parity-v1.md`; they start as small modules compiled per supported tier and chip, and a shared conformance crate is unnecessary initially.
Until they exist, trait-shape changes across the tiers must be audited by hand in review, guided by the trait list in `stoker`.

### 5. Providers are owned by the binary, and verified at link time
A library crate in this stack never enables a process-global provider by default: not a critical-section implementation, an executor, a console driver, or a panic handler.
Tier crates may expose opt-in features for providers (`critical-section-impl` exists exactly this way), but the firmware selects the provider and must link exactly one implementation; examples do this themselves, because examples are firmware.
The critical-section provider is the documented instance of this rule here and in `rustyfarian-network` ADR-021; the executor, console, and panic providers have no opt-in features in this repo today and follow the same rule if they gain one.
A missing or duplicate provider is a link-time fact, not a compile-time one: it is only `cargo check`-clean (2026-10-29 lore), so the follow-up validation includes representative firmware example link builds per tier and chip.
This formalises the 2026-10-29 decision and follows `rustyfarian-network` ADR-021.

### 6. Shared example logic and async support are conditional, not goals
Twin examples share application logic where it removes meaningful duplication, and keep per-tier shells for clear hardware setup.
Async is added only where an actual power-management operation benefits from it.
Deep sleep normally never returns and the esp-hal tier is blocking today, so network's "one async API surface" rule applies here only if such a surface ships.

## Consequences

Positive:
- Tier drift gets a name, a contract, and a review rule in this repo, before it produces a cross-tier bug here.
- Downstream repos that run one firmware on both tiers (as `rustyfarian-knob` does with network) will be able to rely on the same observable guarantees when consuming `rustyfarian-esp-idf-power` and `rustyfarian-esp-hal-power`, scoped to capabilities and operating ranges as defined here — once the audit closes the recorded deviations; until then the recorded `try_sleep` divergence stands.
- The `critical-section-impl` opt-in becomes a stated policy instead of a one-off lore entry, and its failure class is covered by link-time validation.
- CI already links representative firmware — the C3 sleep and retention examples and the S3 retention example — which is partial link-time provider coverage; the follow-up completes the missing tier/chip combinations and adds composition builds that mirror the sibling-provider failure class.
- The esp-hal parity work in `docs/features/esp-hal-parity-v1.md` now closes against a defined parity standard that separates implementation gaps from semantic divergence.
- The follow-up tasks have acceptance criteria, so the accepted contract has completion conditions.

Negative:
- Compile-time conformance cannot prove behavioural parity; host tests and the ADR-001 hardware benches carry that load and the benches stay mandatory for sleep changes.
- Conformance modules narrow only API drift; behavioural drift keeps requiring review, host tests, and benches after they exist.
- The tiers are not behaviourally interchangeable everywhere today: the post-rejection cleanup divergence (point 1) is on record and stands until the audit closes it.
- The audit against point 1, starting with sleep and `try_sleep`, is required before interchangeability is claimed for that surface.
- The capability table (point 2) needs maintenance or it goes stale.
- Shared example application modules, where worthwhile, require a workspace or examples layout change.

## Alternatives Considered
|                          Alternative | Pros                                                | Cons                                                              | Why Rejected                                                      |
|-------------------------------------:|:----------------------------------------------------|:------------------------------------------------------------------|:------------------------------------------------------------------|
| Adopt network's shared async API now | One API surface across both tiers                   | No power-management operation here benefits from async yet        | Conditional (point 6); adopt only if a justified async need ships |
|                           Do nothing | No effort; the structural pattern is already shared | The drift triggers are present and produced sibling bugs          | Contract only costs documentation today                           |
|   Build the parity compile check now | Enforces point 4 immediately                        | A code change, out of scope for this wave                         | Committed follow-up with acceptance criteria, not optional        |
|      One shared ecosystem core crate | Single copy of the core logic repo-wide             | Trait identity is per crate version; coupling the repos' releases | Sibling ADR-005 chose per-repo crates; pins stay in lockstep      |
