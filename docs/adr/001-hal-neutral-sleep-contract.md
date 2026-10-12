# ADR-001: HAL-neutral sleep contract for the ESP-IDF and esp-hal tiers

## Status
Proposed (2026-10-12), revised after review rounds 1 and 2 (2026-10-12): § 4 rewritten, spike attempts 3 and 4 added.
Acceptance waits for the bench values of the timer constants in § 4c and the bench cases of § 4b-4d.

Pinned to esp-hal `=1.2.2` (unstable sleep API) and ESP-IDF v5.3.3.
Every esp-hal claim below cites a file in the 1.2.2 crate source; re-verify them before the next esp-hal wave.

## Context
`stoker` defines `SleepManager`, `WakeSource`, `WakeCause` and `WakeCauseSource` for the ESP-IDF tier.
That contract is IDF-shaped: a `GpioLevel` source is a `pin_mask`, deep sleep is one `esp_deep_sleep_start()` call that configures everything, and the wake cause is read from one status register.
`rustyfarian-esp-hal-power` must implement the same traits on esp-hal 1.2.2, whose model is the opposite on every point:

- Wake sources belong to drivers, not to the sleep call.
  A pin becomes a wake source by `Input::listen(Event::HighLevel | LowLevel)` plus `Input::apply_wakeup_config(&WakeupConfig::default().with_low_power_path(true))` (`gpio/mod.rs` ~1360-1410); the timer by `LowPower::set_wakeup_deadline(Instant)` (`rtc_cntl/sleep/mod.rs:94`).
  `LowPower::sleep_deep(config) -> !` and `sleep_deep_with_rejection(config)` take no sources and read the hardware wakeup-enable mask back (`rtc_cntl/sleep/mod.rs:120-151`).
- There is no wake-pin status bitmask.
  `Input::caused_wakeup(&self) -> bool` is per pin (`gpio/mod.rs` ~1418); `rtc_cntl::wakeup_cause()` returns a set of `WakeupSource`s, on the C3 `Gpio` and `Timer` among others.
- esp-hal panics where the IDF returns an error: deep sleep with no armed source, and `sleep_deep` with a deadline the transition cannot catch (`rtc_cntl/sleep/mod.rs:120-131`, `ALARM_TO_SLEEP_TICKS = 16` in `sleep/timer.rs`).
  `sleep_deep_with_rejection` has no deadline assert, so a too-near deadline there is a sleep that never ends.
- A rejected deep sleep returns the wake pads to their drivers but not every other pad; esp-hal isolates digital pads at entry and cannot restore them (`sleep_deep_with_rejection` docs, `rtc_cntl/sleep/mod.rs:134-149`).
- `Input` has no public pin-number method; the number is reachable as `Input::peripheral_input().gpio_number() -> Option<u8>` (`gpio/interconnect.rs:597`).

Consumer input (rustbox-spookhouse, `review-queue/rustyfarian-power-sleep-wake-followup-v1.md`):
the firmware polls its PIR wake pin through `tamer` for the whole awake period and then sleeps on the same pin, so the hal manager must not own the pin.
Its go/no-go gate is 2026-10-25 and this ADR is the first step of its longest dependency chain.

Three feature documents left questions to this ADR: `esp-hal-sleep-tier-v1` (pin ownership, esp-hal panics, re-arming), `embedded-hal-error-convention-v1` (kind variants, `core::error::Error` bound, `ESP_ERR_SLEEP_REJECT`) and `c3-gpio-deep-sleep-wake-v1` (which error covers rejection, wake-pin validity).
All of it ships in the one breaking `SleepManager` change of 0.3.0.
`stoker` stays `#![no_std]` with no dependencies.
C3 first; C6, ESP32 and S3 sleep on esp-hal are out of scope here.

## Decision

### 1. Trait shape for both tiers

```rust
pub trait SleepManager {
    type Error: Error;
    fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), Self::Error>;
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), Self::Error>;
}
```

- `try_sleep` is a required method, not a default that delegates to `sleep`.
  A default that silently sleeps through a held wake pin is the failure the consumer asked to avoid.
- `sleep` keeps the non-rejecting semantics of `esp_deep_sleep_start()` and esp-hal `sleep_deep`.
  On device it returns only with a configuration error.
- `try_sleep` maps to `esp_deep_sleep_try_to_start()` and esp-hal `sleep_deep_with_rejection`.
  On device `Ok(())` is unreachable; the method returns `Err` whose `kind()` is `Rejected` when a wake source was already asserted.
- `WakeSource` stays HAL-neutral and `Copy`: `Timer { duration_ms }` and `GpioLevel { pin_mask, level }`, one level per source as today.
  The IDF tier passes `pin_mask` to `esp_deep_sleep_enable_gpio_wakeup` (C3/C6) or EXT1 (ESP32/S3/C6).
  The hal tier matches every set bit against the pins the firmware bound for the attempt (decision 2) via `peripheral_input().gpio_number()`, arms exactly those pins with `listen` and `apply_wakeup_config`, and reports a bit without a bound pin as `SleepError::PinNotBound { pin }`.
- `WakeCauseSource::last_wake_cause(&self) -> WakeCause` stays a separate trait; `WakeCause::Gpio` becomes `Gpio(GpioWakeMask)` as decided in the C3 wake feature.
  The hal tier provides `EspHalWakeCauseSource::capture(pins: &[&Input<'_>]) -> Self`, which reads `wakeup_cause()` once at boot: an empty set is `PowerOn`; `Gpio` builds the mask from `caused_wakeup()` on the pins handed in; `Timer` is `Timer`; anything else is `Other`.
  Firmware must capture the cause before the first `try_sleep`: a rejected sleep ends with `record_wakeup()` after sleep entry cleared the per-pin wake status (`gpio/wakeup/mod.rs` `record_wakeup`, `per_pin.rs` `prepare_gpio_wakeup`), so the boot record is overwritten.
- `NoopSleepManager` validates and returns `Ok(())` from both methods for host tests, and gains `NoopSleepManager::rejecting()` whose `try_sleep` returns `SleepError::Rejected`, so a consumer's retry path is host-testable.

Rejected alternatives: a `WakeSource::Pin(&mut dyn WakePad)` variant with a `stoker` pad trait (puts a lifetime into `WakeSource`, makes it non-`Copy`, and adds a trait the IDF tier would implement for nothing); a default `try_sleep`; dropping `sleep` (fire-and-forget timer sleep is the common battery-logger case and costs nothing to keep).

### 2. Pin ownership: borrow `&mut Input` per attempt, settled by spike

The esp-hal manager owns only the `LowPower<'d>` driver (the `LPWR` peripheral).
Wake pins stay with the firmware and are borrowed for one sleep attempt:

```rust
impl<'d> EspHalSleepManager<'d> {
    pub fn bind_wake_pins<'a, 'p, 'i>(
        &'a mut self,
        pins: &'a mut [&'p mut Input<'i>],
    ) -> BoundSleep<'a, 'p, 'i, 'd>;
}
impl SleepManager for BoundSleep<'_, '_, '_, '_> { /* ... */ }
```

The pin lifetime `'i` is a separate parameter.
The borrow ends when `BoundSleep` is dropped, so `pir.is_high()` works again right after a rejected `try_sleep`.

Spike result (2026-10-12, `crates/rustyfarian-esp-hal-power/src/spike_pin_borrow.rs`, compile-only, not exported):

| Attempt | Shape                                                                                                                              | `just check-hal-c3`                                                                                                                                               |
|--------:|:-----------------------------------------------------------------------------------------------------------------------------------|:------------------------------------------------------------------------------------------------------------------------------------------------------------------|
|       1 | `pins: &'a mut [&'p mut Input<'d>]`, pin tied to manager `'d`                                                                      | exit 101: two `lifetime may not live long enough` errors at the consumer loop (`&mut Input<'d>` is invariant in `'d`; manager and pin have independent lifetimes) |
|       2 | `pins: &'a mut [&'p mut Input<'i>]`, own `'i`                                                                                      | exit 0                                                                                                                                                            |
|       3 | attempt 2 plus validate-then-arm, `critical_section::with` around arm, sleep and cleanup, post-arm deadline guard (§ 4)            | exit 0                                                                                                                                                            |
|       4 | attempt 3 plus timer maximum, foreign-wake-pin check and UART drain before arming (§ 4, review round 2); module gated to `esp32c3` | exit 0                                                                                                                                                            |

Attempt 2 type-checks both consumer forms: the one-statement temporary `manager.bind_wake_pins(&mut [&mut pir]).try_sleep(&sources)` and a block-scoped `let mut pins = [&mut pir]; let mut bound = manager.bind_wake_pins(&mut pins);`.
It also type-checks `EspHalWakeCauseSource::capture(&[&pir])` and the `try_sleep` body against `sleep_deep_with_rejection`, `set_wakeup_deadline`, `clear_wakeup_deadline`, `apply_wakeup_config`, `listen`, `unlisten`, `clear_interrupt` and `caused_wakeup` on esp-hal 1.2.2.
The spike module is deleted when the sleep module lands.

Rejected alternatives: manager-owned pins (breaks the consumer's polling, and a pin owned from construction cannot be read by `tamer`); `[&'p mut Input<'i>; N]` moved by value (works too, but fixes the pin count at compile time; a slice lets firmware vary it and the lifetime trap is the same either way).

### 3. Error contract

```rust
pub trait Error: core::fmt::Debug {
    fn kind(&self) -> SleepErrorKind;
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepErrorKind {
    InvalidConfig,
    Unsupported,
    Rejected,
    Platform,
    Other,
}
```

- The trait lives in `stoker::sleep` and bounds `SleepManager::Error`.
  It mirrors embedded-hal 1.0 (`Debug` + `kind()`); it does **not** require `core::error::Error`.
  Requiring it would add nothing for generic consumers, who branch on `kind()`, and would rule out `defmt`-only error types in bare-metal firmware.
  `stoker::SleepError` itself keeps its `core::error::Error` impl, so `?` into `anyhow` still works for IDF firmware.
- Kinds: `InvalidConfig` is a request this chip and tier cannot honour as written; `Unsupported` is a source kind the tier does not implement (reserved, no variant maps to it in 0.3.0); `Rejected` is the hardware refusing because a wake source was already asserted; `Platform` is a HAL or SDK failure after validation; `Other` is the catch-all for future variants.
- `SleepError` gains variants (additive, the enum is `#[non_exhaustive]`): `NoWakeSource`, `Rejected`, `TimerTooShort { duration_ms, min_ms }`, `TimerTooLong { duration_ms, max_ms }`, `PinNotBound { pin }`, `ForeignWakePin { pin }`, `UartBusy { uart }`, `IsolationConflict`.
  `kind()` maps `Rejected` to `Rejected`, `UartBusy` to `Platform`, and every other variant, including the four existing ones, to `InvalidConfig`.
- Hal tier: `type Error = SleepError`, no second error type.
  esp-hal's only fallible call, `apply_wakeup_config`, fails with `WakeConfigError::NoLowPowerPath` for a pad outside GPIO0-5; the mask check in decision 4 makes that unreachable, and the manager maps it to `GpioMaskOutOfRange { pin_mask: 1 << pin, valid_mask }` if it ever happens.
  The rejection report is the return of `sleep_deep_with_rejection`; the manager converts it to `SleepError::Rejected`.
- IDF tier: `EspSleepManager` drops `anyhow::Error` for a local `EspSleepError { Config(SleepError), Idf { code: esp_err_t, kind: SleepErrorKind } }` with `Display`, `core::error::Error` and `kind()`.
  `ESP_ERR_SLEEP_REJECT` is an alias of `ESP_ERR_INVALID_STATE` (`0x103`, `esp_sleep.h:135`), so the mapping to `Rejected` is by call site, the return of `esp_deep_sleep_try_to_start()`, never by code.
  `ESP_ERR_INVALID_ARG` from the enable calls is `Platform` (pre-validation makes it unreachable).
  The existing `isolate_gpio` versus `GpioLevel` conflict, today an `anyhow::bail!`, becomes `Config(SleepError::IsolationConflict)`, a new variant of kind `InvalidConfig`.
- Compatibility: consumer code bounded on `S::Error: Debug` keeps compiling (`Debug` is a supertrait).
  A third-party implementor whose `type Error = anyhow::Error` must switch to a newtype, because the orphan rule forbids `impl stoker::sleep::Error for anyhow::Error`; the CHANGELOG migration section says so.

With this, `embedded-hal-error-convention-v1` has no open design question left and is implementation only.

Rejected alternatives: `type Error: core::error::Error + kind()` (see above); a fixed `SleepError` for all tiers with IDF codes inside (puts `esp_err_t` into `stoker`); mapping `ESP_ERR_INVALID_STATE` to `Rejected` by code (collides with every other invalid-state return).

### 4. Validation, interrupt control and timing on the esp-hal tier

Revised 2026-10-12 after review round 1.
The first draft armed pins before validating the timer, left interrupt delivery undefined between arming and cleanup, and treated the 1 ms floor as a proof.
The guarantee is now split into what the manager proves and what the bench must confirm.

#### 4a. Validate the whole request before touching hardware

`try_sleep` and `sleep` first build a validated plan with no side effects.
A validation error leaves every pin, its interrupt enable, its low-power flag and the deadline exactly as they were.
Shared checks live in `stoker` and run on both tiers; the hal-only checks need the bound pins and the chip constants.

| Input                                       | Result                                  | Why                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
|:--------------------------------------------|:----------------------------------------|:--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `sources` empty                             | `NoWakeSource`                          | esp-hal panics on deep sleep with no armed source (`rtc_cntl/sleep/mod.rs:193`); the IDF sleeps until reset. Same error on both tiers and both methods. Changes `sleep(&[]) == Ok` in 0.3.0.                                                                                                                                                                                                                                                                                                                                                      |
| `pin_mask == 0`, bits outside the chip mask | `GpioMaskZero`, `GpioMaskOutOfRange`    | existing validators; C3 constant `0x3F` joins the per-chip selection.                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| mask bit without a bound pin (hal tier)     | `PinNotBound { pin }`                   | the manager cannot arm a pin it does not hold.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `Timer { duration_ms } < 1`                 | `TimerTooShort { duration_ms, min_ms }` | request-level floor; see 4c for why it is not the whole timing story.                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `Timer { duration_ms } > MAX_TIMER_MS`      | `TimerTooLong { duration_ms, max_ms }`  | esp-hal writes the deadline into a 48-bit RTC comparator and drops the higher bits (`sleep/timer.rs` `arm`), so an oversized request would program a different deadline. `MAX_TIMER_MS = u32::MAX` ms (about 49.7 days) in `stoker`, same on both tiers; at a slow clock of at most 150 kHz the comparator holds about 59 years, more than 400x headroom including RTC uptime. Below the cap `duration_ms * 1000` and `now + duration` cannot overflow `u64`, so fugit's `Instant + Duration` panic is unreachable; the arithmetic stays checked. |

Two more checks need the hardware state and run first inside the critical section of 4b, before any mutation: foreign wake pins (4d, `ForeignWakePin`) and the UART drain (4c, `UartBusy`).
Both only read registers, so their errors also leave everything untouched.
Arming can then fail in two ways: `apply_wakeup_config` (unreachable after the mask check) and the post-arm deadline guard (4c).
Either failure releases every pin already armed and clears the deadline before the error returns.

#### 4b. Interrupt delivery: arm, sleep and clean up with interrupts masked

`Input::listen` enables the pin's CPU interrupt at once (`gpio/mod.rs:1944-1957`, `set_int_enable` with `int_ena` set).
With the wake pin already at its wake level, a handler runs before sleep entry, and again after a rejection until cleanup.
What it does depends on which handler is bound (esp-hal 1.2.2, `gpio/interrupt.rs`):

- **Default handler (`rt`, nothing else bound):** clears only `int_ena` and keeps the wake condition and the `listening` record (`gpio/low_level/mod.rs:234`), so it fires once and the pin stays a wake source.
- **Custom handler (`Io::set_interrupt_handler`):** runs on every level re-assertion until it disables the pin.
  If it calls `unlisten()`, as esp-hal's own example suggests, `set_int_enable` clears the pin from `listening` and `int_type`; sleep entry then drops the GPIO source, and with no timer armed `sleep_deep` panics on "no wakeup source", or with a timer the chip sleeps timer-only.
- **Async `wait_for_*` on the same pin:** the handler completes the future and disables the interrupt; the same loss applies if the task then re-listens or unlistens.

Decision: the manager runs arming, the sleep call and the post-rejection cleanup inside one `critical_section::with`.
On the C3, esp-hal's implementation (esp-sync) clears `mstatus.MIE`, so no GPIO handler runs between `listen` and the sleep, and none runs before the pins are released after a rejection.
The release is `unlisten`, `clear_interrupt` and `apply_wakeup_config(&WakeupConfig::default())` for every bound pin, then `clear_wakeup_deadline`; the section ends only after that, so the held level cannot fire on exit.
`sleep_deep_with_rejection` reports the outcome by polling the raw interrupt bits (`wait_for_sleep_result`, `rtc_cntl/sleep/mod.rs`), so it needs no CPU interrupt; ESP-IDF's `deep_sleep_start` also runs in a critical section (`sleep_modes.c:1130`).
This adds `critical-section = "1.2"` to the hal crate; esp-hal's default `rt` feature provides the implementation.

Contract for firmware, stated in the driver docs: a custom GPIO handler must not `unlisten` a pin that is bound for sleep, and the manager owns the pin's interrupt configuration for the duration of the borrow.
After `try_sleep` returns the pin is unlistened; firmware that uses interrupts on it re-arms its own `listen`.

Bench cases (both on the C3 DevKitM-1, with a custom GPIO handler that counts calls and calls `unlisten`):
- `held-level-custom-handler`: PIR held HIGH, `try_sleep` returns `Rejected` and the handler count stays 0.
- `rejection-retry-loop`: PIR held HIGH for 1000 `try_sleep` attempts, then released; every attempt returns `Rejected`, the next attempt sleeps, and the PIR edge wakes the chip.

#### 4c. Timer: drain, floor, post-arm guard, and a narrowed guarantee

esp-hal cannot catch a deadline nearer than 16 slow-clock ticks (`ALARM_TO_SLEEP_TICKS`, about 80-120 µs, `sleep/timer.rs`).
`sleep_deep` asserts on that; `sleep_deep_with_rejection` does not, and a deadline that passes inside the transition is a sleep that never ends.

The entry path after both guards is not short by construction (review round 2).
esp-hal's `sleep` runs `sleep_uart_prepare` after the hooks and `apply`, and therefore after `sleep_deep`'s `deadline_missed` assert (`rtc_cntl/sleep/mod.rs:181-200`).
On the C3 (UART version 1) that call spins until every TX FIFO is empty and the transmitter is idle (`uart/low_level/v1.rs` `wait_for_suspended`).
A full 128-byte FIFO takes about 11 ms at 115200 baud and about 133 ms at 9600 baud, far longer than a 1 ms timer, and masking interrupts does not shorten a hardware wait.

Strategy, in this order inside the critical section of 4b:
1. **Drain first.** The manager waits until UART0 and UART1 report an empty TX FIFO and an idle transmitter, the same condition esp-hal spins on, read through esp-hal's unstable `UART0::regs()` and `UART1::regs()`.
   With interrupts masked nothing refills a FIFO, so esp-hal's own drain at sleep entry then finds nothing to wait for.
   The wait is bounded by `UART_DRAIN_TIMEOUT_US` (200 ms, a full FIFO at 9600 baud plus margin); a transmitter still busy then, for example held off by CTS, returns `UartBusy` (kind `Platform`) instead of spinning forever inside sleep entry.
   DMA-driven UART transmission (UHCI) must be stopped by the firmware before a sleep; the manager does not see it.
2. **Request floor.** `MIN_TIMER_MS` (provisionally 1 ms) rejects requests that cannot work on any chip, before anything is touched (4a).
3. **Pins, then the deadline, last.** The deadline is computed from `Instant::now()` after the drain, so the requested duration counts from arming, not from the call.
4. **Post-arm guard.** After `set_wakeup_deadline` and before the sleep call, the manager re-reads `Instant::now()`; if less than `POST_ARM_MARGIN_US` (provisionally 200 µs) remains, it releases everything and returns `TimerTooShort`.
   esp-hal's own `deadline_missed` is `pub(crate)`, so the manager keeps an equivalent check of its own.
5. **What remains unguarded** is esp-hal's entry path after the guard: hooks, `apply`, the now-empty UART drain, pad isolation and `start_sleep`, running from RAM (`#[ram] fn sleep`) with interrupts masked.

Narrowed guarantee: the manager guarantees no panic and no endless sleep for empty sources, timers outside `[MIN_TIMER_MS, MAX_TIMER_MS]`, bad or foreign pins, by construction.
For short timers it guarantees a typed error when the margin is already gone at the guard, and relies on the bench to show that guard-to-sleep-entry is shorter than `POST_ARM_MARGIN_US`.
On the hal tier, `sleep` keeps `sleep_deep`, so esp-hal's assert stays as the last line of defence; a panic there is a defect in these constants, not a condition the consumer handles.
Both timer constants move to their bench values before the ADR is accepted.

Bench case `short-timer-sweep` (C3 DevKitM-1, GPIO handler of 4b active), 1000 cycles per cell:

| Sources          | Timer (ms)  | UART state at the call                | Methods              |
|:-----------------|:------------|:--------------------------------------|:---------------------|
| timer only       | 1, 2, 5, 10 | idle; 128 bytes queued at 115200 baud | `try_sleep`, `sleep` |
| GPIO4 plus timer | 1, 2, 5, 10 | idle; 128 bytes queued at 115200 baud | `try_sleep`, `sleep` |

Measure **guard-to-sleep-entry**, not only arm-to-guard: toggle a spare GPIO at the guard and capture it with a logic analyser against the 3V3 supply current dropping into deep sleep; on the rejected path, the guard-to-return time is an upper bound for the same code.
Also record drain time and every outcome.
Acceptance: zero hangs, zero panics, `POST_ARM_MARGIN_US` at least 4x the worst measured guard-to-sleep-entry, and `MIN_TIMER_MS` at least the margin plus `ALARM_TO_SLEEP_TICKS`.

#### 4d. Exclusive low-power wake pins, re-arming, other pads, pulls

Exclusivity rule: during a sleep attempt the binding is the complete set of low-power wake pins (review round 2).
esp-hal collects every pad that listens and carries the low-power flag, globally, not only the bound ones (`gpio/wakeup/mod.rs` `collect`).
A pad that firmware configured elsewhere, for example GPIO5 with `apply_wakeup_config(low_power_path)` left listening, would wake or reject an attempt that binds only GPIO4, and the cleanup would leave it armed.

Enforcement:
- Firmware must not call `apply_wakeup_config` itself; the manager is the only code that sets the low-power flag, and it clears it on every bound pin on release, so outside a sleep attempt no pad carries the flag.
- Inside the critical section and before any mutation, the manager reads `int_type` of every C3 low-power pad (GPIO0-5) outside the binding via esp-hal's unstable `GPIO::regs()`, the same field esp-hal uses for the wake level (`armed_level`).
  A set trigger returns `ForeignWakePin { pin }` (kind `InvalidConfig`).
  esp-hal keeps the low-power flag in a private static, so the check is conservative: it also refuses an unbound pad that only listens for an ordinary interrupt, which firmware fixes by `unlisten` before the sleep or by binding it.
- Bench case `foreign-wake-pin`: GPIO5 armed and listening by firmware, `try_sleep` binding only GPIO4; expect `ForeignWakePin { pin: 5 }` with GPIO4 and GPIO5 untouched, and after `unlisten` on GPIO5 a GPIO5 edge does not wake the chip.

Re-arming: every attempt re-applies `apply_wakeup_config` and `listen` for the mask pins only.
Only the intersection of bound pins and mask bits may listen; other bound pins are released.
Other digital pads that esp-hal isolated at entry are the firmware's to reconfigure after a rejection; the manager documents this and does not snapshot them.
Wake pins need a pull against their wake level; esp-hal keeps the pin's pull and adds none, and a floating pad wakes immediately (`WakeupConfig` docs).

Rejected alternatives:
- Let esp-hal panic: a battery prop that never wakes and reports nothing.
- Clean up only after the sleep call, with interrupts enabled: handlers run before cleanup, and an `unlisten` in a custom handler silently removes the wake source (review round 1).
- A fixed timer floor as the whole guarantee: compilation cannot establish the margin.
- A margin large enough to cover esp-hal's UART drain: the drain depends on queued bytes and baud rate, which the manager does not know (review round 2).
- Arm only the bound pins and ignore foreign ones: esp-hal collects listening low-power pads globally, so a foreign pad still wakes or rejects the sleep (review round 2).
- Snapshot and restore all pads: the manager cannot know their configuration either.

### 5. Consumer follow-up points 2 to 4

- **Wake-pin validity.** `stoker` validates against the chip constant (`0x3F` on the C3) before the SDK call on both tiers; the IDF tier does not call `esp_sleep_is_valid_wakeup_gpio` per bit.
  On the C3 that function is `GPIO_IS_DEEP_SLEEP_WAKEUP_VALID_GPIO`, a test against the same `SOC_GPIO_DEEP_SLEEP_WAKE_VALID_GPIO_MASK` (`soc_caps.h:175`, `sleep_modes.c:1710-1716`), and `esp_deep_sleep_enable_gpio_wakeup` repeats the check and returns `ESP_ERR_INVALID_ARG` (`sleep_modes.c:1988-1994`).
  esp-hal has no equivalent call; its `NoLowPowerPath` is the same GPIO0-5 set (`per_pin.rs`).
  A typed, host-tested error that is identical on both tiers beats an IDF-only runtime call.
  GPIO4 is MTMS, the JTAG TMS line (`io_mux_reg.h:100`); GPIO4 to GPIO7 are the JTAG group.
  Both examples document that a PIR on GPIO4 is fine for firmware and disables JTAG debugging while it runs.
- **Wake pin already at its wake level.** Named bench case `wake-pin-held-at-entry` in both tiers' hardware validation.
  Expected on both: `try_sleep` returns `Err` with `kind() == Rejected` (IDF: `ESP_ERR_SLEEP_REJECT`; esp-hal: `sleep_deep_with_rejection` returns, because the reject mask is the enabled-source mask, `rtc_cntl/sleep/mod.rs:197`).
  Bench H1 (2026-10-12) showed the C3 hardware does not reject a held GPIO wake pin: it sleeps and wakes about 20 ms later.
  The IDF tier therefore reads the input-enabled wake pins right before sleep entry and returns `Rejected` itself; the esp-hal tier uses the same reject hardware and will likely need the same pre-check.
  Re-run on the bench with the pre-check (2026-10-13): 5 of 5 held wakes returned `Rejected` and the boot loop is gone.
  A second named case, `wake-cause-survives-rejection`, checks whether `caused_wakeup()` and `wakeup_cause()` still report the boot wake after one rejection on esp-hal; the code reading says they do not.
- **Slept time.** Out of scope for `stoker` and both tiers in 0.3.0.
  Recipe: esp-hal `Rtc::time_since_power_up()` (the RTC timer runs through deep sleep and every reset except power-up, `rtc_cntl/mod.rs:257-264`), IDF `esp_rtc_get_time_us()`; the firmware stores the pre-sleep stamp in the RTC retention envelope and subtracts after the wake.
  If a second consumer needs it, an additive `stoker` trait in a 0.3.x release is the path; `WakeCause::Timer { slept_ms }` is rejected because it would break the enum again.

## Consequences

Positive:
- One `SleepManager` contract compiles against both tiers; consumer code differs only in how it obtains the manager and, on esp-hal, in one `bind_wake_pins` call.
- The spike removes the main schedule risk: borrowing `&mut Input` at the call compiles on esp-hal 1.2.2, so the consumer's S2 firmware design can assume it.
- Empty sources, timers outside the floor and the maximum, bad masks and foreign wake pins are typed `InvalidConfig` errors before the HAL is touched; no validation error leaves a pin armed.
- No GPIO handler can run between arming and sleep, or before cleanup after a rejection.
- `embedded-hal-error-convention-v1` and the open questions of the two sleep features are closed; what remains is implementation and bench work.

Negative:
- 0.3.0 breaks `SleepManager` implementors (new required `try_sleep`, new `Error` bound) and changes `sleep(&[])` from `Ok` to `NoWakeSource`.
- The IDF tier needs a new error type and loses `anyhow` context inside the manager.
- The esp-hal wake cause must be captured at boot before any `try_sleep`; a consumer that reads it later gets `Other` or an empty mask.
- Everything on the esp-hal side is pinned to an unstable API at `=1.2.2`; the next esp-hal wave re-verifies every cited line.
- The short-timer guarantee rests on two provisional constants (`MIN_TIMER_MS`, `POST_ARM_MARGIN_US`) until the `short-timer-sweep` bench sets them; the non-rejecting `sleep` keeps esp-hal's assert as a last line of defence.
- Firmware loses the pin's interrupt configuration across a `try_sleep` and must not `unlisten` a bound pin from a custom handler.
- The hal crate gains a `critical-section` dependency.
- `try_sleep` waits for queued UART output before arming, up to `UART_DRAIN_TIMEOUT_US`; log output just before a sleep delays it.
- Firmware must not listen on an unbound low-power pad while sleeping and must not call `apply_wakeup_config` itself; the foreign-pin check is conservative and also refuses a pad that only listens for an ordinary interrupt.
- The UART and GPIO checks use esp-hal's unstable `regs()` accessors and register field names; the next esp-hal wave re-verifies them.

## Alternatives Considered
|                                           Alternative |                                          Pros | Cons                                                                  | Why Rejected                                                    |
|------------------------------------------------------:|----------------------------------------------:|:----------------------------------------------------------------------|:----------------------------------------------------------------|
|                     Manager-owned wake pins (esp-hal) |            Simple `new(lpwr, pins)` signature | Firmware cannot poll the pin while awake                              | Consumer requirement 1 (tamer presence polling)                 |
|       `WakeSource::Pin(&mut dyn WakePad)` in `stoker` |            No mask-to-pin matching on esp-hal | Lifetime in `WakeSource`, loses `Copy`, dead trait on IDF             | Mask plus `PinNotBound` keeps `WakeSource` HAL-neutral          |
|             Default `try_sleep` delegating to `sleep` |                 Non-breaking for implementors | Silently sleeps through a held wake pin                               | The exact failure the consumer asked to avoid                   |
|                           Drop `sleep` from the trait |                          One method, no drift | Loses fire-and-forget timer sleep used by the battery examples        | Both are two lines on every implementor                         |
|        `type Error: core::error::Error` plus `kind()` | `?` into `anyhow`/`Box<dyn Error>` everywhere | Excludes `defmt`-only errors; generic code uses `kind()` anyway       | embedded-hal 1.0 shape wins; `SleepError` still implements it   |
|     Map `ESP_ERR_INVALID_STATE` to `Rejected` by code |                                 One match arm | `ESP_ERR_SLEEP_REJECT` aliases `0x103`, which other calls also return | Mapping by call site is exact                                   |
|      Let esp-hal panic on empty sources / short timer |                                 No validators | Battery prop never wakes, no report                                   | Typed `InvalidConfig` before the HAL                            |
| Clean up after the sleep call with interrupts enabled |                           No critical section | A handler runs first; an `unlisten` in it drops the wake source       | Arm, sleep and clean up with interrupts masked (§ 4b)           |
|         Fixed 1 ms floor as the whole timer guarantee |                                        Simple | Compilation cannot establish the margin                               | Floor plus post-arm guard plus bench sweep (§ 4c)               |
|            Margin sized to cover esp-hal's UART drain |                            No register access | Drain time depends on queued bytes and baud                           | Drain before arming, bounded (§ 4c)                             |
|                         Ignore unbound low-power pads |                             No register reads | A foreign pad wakes or rejects the sleep and stays armed              | Exclusivity rule plus `ForeignWakePin` (§ 4d)                   |
|              `esp_sleep_is_valid_wakeup_gpio` per bit |                               Follows the SDK | IDF-only runtime call, same `0x3F` test, no esp-hal equivalent        | Chip constant in `stoker`, host-tested, identical on both tiers |
|        Slept time via `WakeCause::Timer { slept_ms }` |                                  No new trait | Breaks `WakeCause` again, GPIO wakes also need it                     | Out of scope; retention envelope recipe; additive trait later   |
