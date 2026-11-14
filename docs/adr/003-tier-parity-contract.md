# ADR-003: Tier parity contract for the ESP-IDF and esp-hal tiers

## Status
Accepted; implemented 2026-11-04.
Point 3 is partially superseded by ADR-004.

Aligned with `rustyfarian-network` ADR-022 (tier parity, one async API surface) and ADR-021 (binary-owned providers), and `rustyfarian-ws2812` ADR-010 (interchangeable driver semantics).
Platform pins follow ADR-001: ESP-IDF v6.1 and esp-hal `=1.2.2` in lockstep with the sibling repos.

## Context
`stoker` and the two tier crates implement the three-crate shape the whole family uses: a pure, host-testable core and one thin driver crate per HAL (ADR-001).
`rustyfarian-ws2812` ADR-010 records two drivers whose methods had identical names but different semantics, so one downstream firmware behaved differently depending on the tier it was built for.
`rustyfarian-network` ADR-022 records ~17 API and ~8 behaviour gaps between a blocking ESP-IDF tier and an async esp-hal tier.
It answers them with shared traits, sans-IO cores, and per-tier parity compile checks.
This repo is exposed to the same drift.
The esp-hal tier lacks the ADC battery monitor and the charging monitor that the ESP-IDF tier has.
An async surface is deferred on both tiers, and neither tier manifests an async feature.
The provider-selection link failure (duplicate `_critical_section_1_0_acquire` with `rustyfarian-esp-idf-network`'s `lora` feature) is the same failure class as network bug 007.
It is handled by making `critical-section-impl` opt-in.
This ADR adopts the siblings' contract and sets the requirements for making the tiers' shared, implemented capabilities interchangeable.
Implementation gaps (the missing esp-hal monitors) are tracked separately, not folded into the promise.

## Decision

### 1. Same name means the same observable behaviour
A method shared by name across the tiers delivers the same observable outcomes on both:
- the same validation outcome and error kinds
- the same unsuccessful-attempt outcomes (for sleep, `Rejected`)
- the same successful behaviour
- the same what-remains-usable-after-a-failed-attempt guarantees

Internal sequencing, timing, and peripheral ownership may differ; a shared name requires identical observables, not identical internals.
The esp-hal `BoundSleep` borrowing model remains valid under this rule.

Scope of the parity claim:
- Portable validation (the shared `validate_wake_sources`, called by both tier managers) must be identical.
- Platform constraints that follow from the hardware interface are documented exceptions, not drift: bound-pin checks, the esp-hal `MIN_TIMER_MS` floor, wake-pin masks, isolation conflicts.
- Parity is judged for requests within the shared supported operating range, and by portable error kinds (`stoker`'s kind enums), not by identical concrete error values.

Minimum portable postcondition after any `Rejected` attempt:
- The caller may retry immediately through the same trait object, with no recovery, re-construction, or pin re-binding step.
- The retry is observably a fresh attempt: the same validation outcomes and error kinds apply, and a `Timer` duration counts from the retry.
- Every wake pin named in the rejected request's `pin_mask` is usable as a plain digital input once the call has returned (on esp-hal, once the `BoundSleep` borrow ends: that tier's release step).
- Whether wake sources stay armed between a rejection and its retry is adapter-internal, not an observable.
  - The IDF tier leaves them applied and re-runs configuration on retry.
  - The esp-hal tier disarms on rejection and re-arms on retry.
- Every pad outside the request's `pin_mask` and every pad hold the caller set, RTC or digital, is as the caller left it, because sleep entry changes none of them before the hardware reject decision.
  - Basis: source-read of IDF v6.1 `esp_sleep_start` and esp-hal 1.2.2 sleep entry.
  - Bench R4-S3 checks it on the S3.
- The exception is the global digital-pad hold: while effective on the ESP32, ESP32-S3, ESP32-C2 and ESP32-C3, entry isolates every unheld digital-only pad (S3: GPIO26-48) and nothing restores it.
- The postcondition excludes:
  - the console (UART or USB-Serial-JTAG output may be interrupted)
  - the CPU and APB clock during the attempt
  - power-domain configuration and deep-sleep hooks of WiFi/BT
  - the persistent sleep-mode pad configuration that `isolate_gpio` writes (inert while awake)
  - timer-deadline persistence
- esp-hal clears `WIFI_FORCE_ISO` in `RTC_CNTL_DIG_ISO_REG` at sleep entry and a rejection does not restore it (bench R4-S3).

The three IDF rejection paths, and why none needs a weaker postcondition:
- **EXT1 chips (ESP32, S3, C6), hardware reject:** it fires after `ext1_wakeup_prepare`.
  - The masked pins stay RTC-routed and RTC-held.
  - `ext1_rtc_gpio_mask` stays stale (`esp_sleep_disable_wakeup_source(ALL)` does not clear it).
  - The timer and isolation config stay applied.
- **C2/C3 hardware reject (timer-only, or a pin that reached its level after the pre-read):**
  - Wake pins keep their `gpio_hold_en` hold and the internal pull applied at entry.
  - Per-pin wake enables and the SDK's `gpio_wakeup_mask` survive.
- **C2/C3 software pre-read reject (`gpio_wake_pins_asserted`):** no sleep entry happened, so the residue is configuration only and no pin is held.
- Holds do not block `gpio_get_level`, but `ext1_wakeup_prepare` also RTC-routes the pad, which does.
- The EXT1 reject path therefore calls `rtc_gpio_deinit` to keep the readability clause true; on C2/C3 the pads stay digitally routed.
- A retry that changes the EXT1 pin/level combination can fail with `ESP_ERR_NOT_ALLOWED` in the mask merge, because the disable-all branch does not clear `s_config.ext1_rtc_gpio_mask`.
- The adapter calls `esp_sleep_disable_ext1_wakeup_io(0)` after disable-all to close it.
- The hardware holds are applied before the reject decision, so the rejection does not undo them; the esp-hal adapter instead disarms before returning.

Global pad hold on the ESP32 and ESP32-S3:
- While the hold is effective (`RTC_CNTL_DIG_ISO_REG` `DG_PAD_AUTOHOLD_EN` set, `DG_PAD_FORCE_UNHOLD` clear), a hardware rejection never returns (bench R2-S3).
- Both tiers therefore read the wake pins right before sleep entry in that case and return `Rejected` from software, with the same postcondition (bench R3-S3).
- A wake pin that reaches its wake level after that read, and a non-pin hardware rejection, can still hang while the hold is effective.
- On the C2/C3 no guard exists for non-pin rejections.
- Both cases are outside the shared supported operating range and documented on `try_sleep`.

The original ESP32 hardware never rejects on EXT1 (`RTC_SLEEP_REJECT_MASK` is GPIO and SDIO only), like the C3.
Both tiers close this with the software pre-read: a held EXT1 pin returns `Rejected` on both.
Where a HAL behaviour cannot be mapped into the shared observables at all, the method gets a different name.
This follows `rustyfarian-ws2812` ADR-010: "methods with the same name mean the same thing on both drivers".

### 2. Hardware differences are capabilities, not APIs
A hardware difference is documented once, as a capability in a tier and chip scoped table, instead of being expressed as a diverging API.
This follows `rustyfarian-network` ADR-022 decision 7.
The table distinguishes three classes:
- A hardware limitation: no software path exists.
- An unimplemented driver: the shared capability exists but a tier has not built it.
- A behavioural difference: a same-name method that fails point 1.

The missing esp-hal ADC battery monitor and charging monitor are unimplemented drivers, not evidence of semantic divergence.
Interchangeability claims cover only capabilities that both tiers implement.

Multi-pin `AnyLow` on the original ESP32 is rejected on both tiers with `SleepError::AnyLowMultiplePins` (kind `Unsupported`) before anything is armed.
- The IDF tier offers only EXT1 `ALL_LOW` on the original ESP32.
- esp-hal 1.2.2 has no ESP32 path for a multi-pin low group (`ext1_v1.rs` spills those pins to EXT0 and per-pin wake).
- `SleepAttemptMachine::new_single_pin_any_low` raises the error.
- The `try_sleep` row therefore has no ESP32 `AnyLow` exception; single-pin `AnyLow` is unchanged and still wakes.

### 3. Portable policy lives in `stoker`; sequencing lives in the adapters
ADR-004 partially supersedes this point: sequencing for multi-step flows such as `try_sleep` moves into `stoker` as a sans-IO machine (rung 3 of the ADR-004 ladder).
Register work stays in the adapters.
Portable policy and validation live in `stoker`, where they are host-testable.
Hardware sequencing and register-dependent work remain in the tier adapters.
Draining UARTs, arming pins, guarding deadlines, entering critical sections, and cleaning up after rejection belong beside the hardware they drive.
Pure decisions that benefit from host tests are extracted into `stoker` when useful, without forcing hardware workarounds into the core.
Existing instances: the `GatedSleep` coordinator (policy, in `stoker`), and the esp-hal sleep adapter's UART drain, deadline arming, and rejection cleanup (sequencing, in the adapter).

### 4. API conformance is checked at compile time; behavioural parity is tested separately
Each tier compiles checked-in conformance modules that call the `stoker` traits generically, so a trait change that breaks one tier breaks its build the same way.
- The conformance call must be instantiated with the tier's concrete adapter types inside that tier's build, including `BoundSleep` and its lifetimes.
- Reason: a generic `fn check<S: SleepManager>(..)` alone compiles without proving the real adapter implements the trait.
- Coverage is per capability: a tier is checked for the capabilities it implements, not for every `stoker` trait.
- The modules are `src/conformance.rs` in each tier crate, compiled per supported tier and chip by the chip recipes; a shared conformance crate stays unnecessary.
- Compilation cannot verify rejection handling, wake-pin cleanup, timer accuracy, or wake-cause preservation.
- Behavioural parity is tested in `stoker`'s host tests where portable, and on the hardware benches required by ADR-001, which remain the arbiter for sleep behaviour.

### 5. Providers are owned by the binary, and verified at link time
A library crate in this stack never enables a process-global provider by default: not a critical-section implementation, an executor, a console driver, or a panic handler.
- Tier crates may expose opt-in features for providers (`critical-section-impl` exists exactly this way).
- The firmware selects the provider and must link exactly one implementation; examples do this themselves, because examples are firmware.
- The critical-section provider is the documented instance here and in `rustyfarian-network` ADR-021; executor, console, and panic providers follow the same rule if they gain an opt-in feature.
- A missing or duplicate provider is a link-time fact, only `cargo check`-clean, so validation includes representative firmware example link builds per tier and chip.
- CI links the C3 sleep and retention examples and the S3 retention example.
- Still to add: the remaining tier/chip combinations and composition builds that mirror the sibling-provider failure class.

### 6. Shared example logic and async support are conditional, not goals
Twin examples share application logic where it removes meaningful duplication, and keep per-tier shells for clear hardware setup.
Async is added only where an actual power-management operation benefits from it.
Deep sleep normally never returns and the esp-hal tier is blocking, so network's "one async API surface" rule applies here only if such a surface ships.

## Consequences

Positive:
- Tier drift gets a name, a contract, and a review rule before it produces a cross-tier bug here.
- Downstream firmware that runs on both tiers (as `rustyfarian-knob` does with network) can rely on the same observable guarantees, scoped to capabilities and operating ranges as defined here.
- The `critical-section-impl` opt-in is a stated policy, and its failure class is covered by link-time validation.
- The esp-hal parity work closes against a defined standard that separates implementation gaps from semantic divergence.

Negative:
- Compile-time conformance cannot prove behavioural parity; host tests and the ADR-001 hardware benches carry that load and stay mandatory for sleep changes.
- Conformance modules narrow only API drift; behavioural drift keeps requiring review, host tests, and benches.
- The capability table (point 2) needs maintenance or it goes stale.
- Shared example application modules, where worthwhile, require a workspace or examples layout change.

## Alternatives Considered
| Alternative                          | Pros                                  | Cons                                                | Why Rejected                                      |
|:-------------------------------------|:--------------------------------------|:----------------------------------------------------|:--------------------------------------------------|
| Adopt network's shared async API now | One API surface across both tiers     | No power-management operation here benefits yet     | Conditional (point 6); only if a justified need   |
| Do nothing                           | No effort; the pattern is shared      | The drift triggers are present and bit the siblings | Contract only costs documentation                 |
| Skip the parity compile check        | No code change                        | Point 4 goes unenforced                             | Enforced by the conformance modules               |
| One shared ecosystem core crate      | Single copy of the core logic         | Trait identity is per crate version; coupled releases | Sibling ADR-005 chose per-repo crates; lockstep pins |
