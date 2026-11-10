# ADR-001: HAL-neutral sleep contract for the ESP-IDF and esp-hal tiers

## Status
Proposed (2026-10-12).
Decided for the C3 and ESP32-S3; the ESP32 and C6 joined the same contract under [ADR-003](003-tier-parity-contract.md) and [ADR-004](004-shared-logic-ladder.md).
Pinned to esp-hal `=1.2.2` (unstable sleep API) and ESP-IDF v5.3.3; re-verify every esp-hal claim before the next esp-hal wave.
The acceptance benches still open (short-timer sweep for the provisional timer constants, held-level custom handler, rejection retry loop, foreign wake pin, wake cause after a rejection) are tracked in `docs/features/esp-hal-sleep-tier-v1.md`.

## Context
`stoker` defines `SleepManager`, `WakeSource`, `WakeCause` and `WakeCauseSource` for the ESP-IDF tier.
That contract is IDF-shaped: a `GpioLevel` source is a `pin_mask`, deep sleep is one `esp_deep_sleep_start()` call that configures everything, and the wake cause is read from one status register.
`rustyfarian-esp-hal-power` must implement the same traits on esp-hal 1.2.2, whose model is the opposite on every point:

- Wake sources belong to drivers, not to the sleep call: a pin becomes a wake source by `Input::listen` plus `Input::apply_wakeup_config(low_power_path)`, the timer by `LowPower::set_wakeup_deadline`.
- `sleep_deep` and `sleep_deep_with_rejection` take no sources and read the hardware wakeup-enable mask back.
- There is no wake-pin status bitmask: `Input::caused_wakeup()` is per pin, and `wakeup_cause()` returns a set of `WakeupSource`s.
- esp-hal panics where the IDF returns an error: deep sleep with no armed source, and `sleep_deep` with a deadline the transition cannot catch (16 slow-clock ticks).
- `sleep_deep_with_rejection` has no deadline assert, so a too-near deadline there is a sleep that never ends.
- A rejected deep sleep returns the wake pads to their drivers but not every other pad; esp-hal isolates digital pads at entry and cannot restore them.
- `Input` has no public pin-number method; the number is reachable as `Input::peripheral_input().gpio_number() -> Option<u8>`.

The consumer (rustbox-spookhouse) polls its PIR wake pin through `tamer` for the whole awake period and then sleeps on the same pin, so the hal manager must not own the pin.
All of it ships in the one breaking `SleepManager` change of 0.3.0.
`stoker` stays `#![no_std]`; its only dependency is `critical-section`, so `RetainedCell` is sound.

## Decision

### 1. Trait shape for both tiers

```rust
pub trait SleepManager {
    type Error: Error;
    fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), Self::Error>;
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), Self::Error>;
}
```

- `try_sleep` is a required method, not a default that delegates to `sleep`, because a default would silently sleep through a held wake pin.
- `sleep` keeps the non-rejecting semantics of `esp_deep_sleep_start()` and esp-hal `sleep_deep`.
  On device it returns only with a configuration error.
- `try_sleep` maps to `esp_deep_sleep_try_to_start()` and esp-hal `sleep_deep_with_rejection`.
  On device `Ok(())` is unreachable; the method returns `Err` whose `kind()` is `Rejected` when a wake source was already asserted.
- `Rejected` may come from a software pre-read instead of the hardware: on C2/C3 always (IDF tier), and on ESP32/S3 while the global digital-pad hold is effective, because a hardware rejection then hangs (ADR-003 point 1).
- `WakeSource` stays HAL-neutral and `Copy`: `Timer { duration_ms }` and `GpioLevel { pin_mask, level }`, one level per source.
  The IDF tier passes `pin_mask` to `esp_deep_sleep_enable_gpio_wakeup` (C3/C6) or EXT1 (ESP32/S3/C6).
  The hal tier matches every set bit against the pins bound for the attempt (decision 2), arms exactly those pins, and reports a bit without a bound pin as `SleepError::PinNotBound { pin }`.
- `WakeCauseSource::last_wake_cause(&self) -> WakeCause` stays a separate trait; `WakeCause::Gpio` becomes `Gpio(GpioWakeMask)`.
- `EspHalWakeCauseSource::capture(pins: &[&Input<'_>]) -> Self` reads `wakeup_cause()` once at boot: an empty set is `PowerOn`, `Timer` is `Timer`, anything else is `Other`.
  On the C3 `Gpio` builds the mask from `caused_wakeup()` on the pins handed in.
  On the S3 `Ext1` builds `Ext1(mask)` the same way (`Ext0` maps to `WakeCause::Ext0`), so both tiers report the same variant per chip.
- Firmware must capture the cause before the first `try_sleep`: a rejected sleep ends with `record_wakeup()` after sleep entry cleared the per-pin wake status, so the boot record is overwritten.
- `NoopSleepManager` validates and returns `Ok(())` from both methods for host tests, and `NoopSleepManager::rejecting()` returns `SleepError::Rejected` from `try_sleep`, so a consumer's retry path is host-testable.

### 2. Pin ownership: borrow `&mut Input` per attempt

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

- The pin lifetime `'i` is a separate parameter: `&mut Input<'d>` is invariant in `'d`, so tying the pin to the manager lifetime fails to compile at the consumer loop.
- The borrow ends when `BoundSleep` is dropped, so `pir.is_high()` works again right after a rejected `try_sleep`.
- Both consumer forms type-check: the one-statement temporary `manager.bind_wake_pins(&mut [&mut pir]).try_sleep(&sources)` and a block-scoped `let mut pins = [&mut pir]; let mut bound = manager.bind_wake_pins(&mut pins);`.
- The slice form is kept over `[&'p mut Input<'i>; N]` by value, because a slice lets firmware vary the pin count.

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
  It mirrors embedded-hal 1.0 (`Debug` + `kind()`) and does not require `core::error::Error`, which would add nothing for generic consumers and would rule out `defmt`-only error types.
  `stoker::SleepError` keeps its `core::error::Error` impl, so `?` into `anyhow` still works for IDF firmware.
- Kinds: `InvalidConfig` is a request this chip and tier cannot honour as written; `Unsupported` is a source kind the tier does not implement; `Rejected` is the hardware refusing because a wake source was already asserted; `Platform` is a HAL or SDK failure after validation; `Other` is the catch-all.
- `SleepError` gains variants (additive, `#[non_exhaustive]`): `NoWakeSource`, `Rejected`, `TimerTooShort { duration_ms, min_ms }`, `TimerTooLong { duration_ms, max_ms }`, `PinNotBound { pin }`, `ForeignWakePin { pin }`, `UartBusy { uart }`, `IsolationConflict`.
  `kind()` maps `Rejected` to `Rejected`, `UartBusy` to `Platform`, and every other variant, including the four existing ones, to `InvalidConfig`.
- Hal tier: `type Error = SleepError`, no second error type.
  esp-hal's only fallible call, `apply_wakeup_config`, fails with `WakeConfigError::NoLowPowerPath` for a pad outside GPIO0-5; the mask check makes that unreachable, and the manager maps it to `GpioMaskOutOfRange { pin_mask: 1 << pin, valid_mask }` if it ever happens.
  The manager converts the return of `sleep_deep_with_rejection` to `SleepError::Rejected`.
- IDF tier: `EspSleepManager` drops `anyhow::Error` for a local `EspSleepError { Config(SleepError), Idf { code: esp_err_t, kind: SleepErrorKind } }` with `Display`, `core::error::Error` and `kind()`.
  `ESP_ERR_SLEEP_REJECT` is an alias of `ESP_ERR_INVALID_STATE` (`0x103`), so the mapping to `Rejected` is by call site, the return of `esp_deep_sleep_try_to_start()`, never by code.
  `ESP_ERR_INVALID_ARG` from the enable calls is `Platform` (pre-validation makes it unreachable).
  The `isolate_gpio` versus `GpioLevel` conflict, once an `anyhow::bail!`, becomes `Config(SleepError::IsolationConflict)`, kind `InvalidConfig`.
- Consumer code bounded on `S::Error: Debug` keeps compiling.
  A third-party implementor with `type Error = anyhow::Error` must switch to a newtype, because the orphan rule forbids `impl stoker::sleep::Error for anyhow::Error`; the CHANGELOG migration section says so.

### 4. Validation, interrupt control and timing on the esp-hal tier

#### 4a. Validate the whole request before touching hardware

`try_sleep` and `sleep` first build a validated plan with no side effects.
A validation error leaves every pin, its interrupt enable, its low-power flag and the deadline exactly as they were.
Shared checks live in `stoker` and run on both tiers; the hal-only checks need the bound pins and the chip constants.

| Input                                       | Result                                  | Why                                                  |
|:--------------------------------------------|:----------------------------------------|:-----------------------------------------------------|
| `sources` empty                             | `NoWakeSource`                          | esp-hal panics; the IDF sleeps until reset           |
| `pin_mask == 0`, bits outside the chip mask | `GpioMaskZero`, `GpioMaskOutOfRange`    | existing validators; C3 constant `0x3F`              |
| mask bit without a bound pin (hal tier)     | `PinNotBound { pin }`                   | the manager cannot arm a pin it does not hold        |
| `Timer { duration_ms } < 1`                 | `TimerTooShort { duration_ms, min_ms }` | request-level floor; see 4c                          |
| `Timer { duration_ms } > MAX_TIMER_MS`      | `TimerTooLong { duration_ms, max_ms }`  | esp-hal drops bits above its 48-bit comparator       |

- `NoWakeSource` is the same on both tiers and both methods, and changes `sleep(&[]) == Ok` in 0.3.0.
- `MAX_TIMER_MS = u32::MAX` ms (about 49.7 days) in `stoker`, same on both tiers.
- Below the cap `duration_ms * 1000` and `now + duration` cannot overflow `u64`, so fugit's `Instant + Duration` panic is unreachable; the arithmetic stays checked.
- Two more checks read hardware state and run first inside the critical section of 4b, before any mutation: foreign wake pins (4d, `ForeignWakePin`) and the UART drain (4c, `UartBusy`).
- Both only read registers, so their errors also leave everything untouched.
- Arming can then fail in two ways: `apply_wakeup_config` (unreachable after the mask check) and the post-arm deadline guard (4c).
- Either failure releases every pin already armed and clears the deadline before the error returns.

#### 4b. Interrupt delivery: arm, sleep and clean up with interrupts masked

`Input::listen` enables the pin's CPU interrupt at once, so with the wake pin already at its wake level a handler runs before sleep entry, and again after a rejection until cleanup.
A custom handler (or an async `wait_for_*` on the pin) that calls `unlisten()` removes the pin from the wake set; sleep entry then drops the GPIO source, and with no timer armed `sleep_deep` panics, or with a timer the chip sleeps timer-only.
The default handler only clears `int_ena` and keeps the pin a wake source.

Decision: the manager runs arming, the sleep call and the post-rejection cleanup inside one `critical_section::with`.
- On the C3, esp-hal's implementation clears `mstatus.MIE`, so no GPIO handler runs between `listen` and the sleep, and none runs before the pins are released after a rejection.
- The release is `unlisten`, `clear_interrupt` and `apply_wakeup_config(&WakeupConfig::default())` for every bound pin, then `clear_wakeup_deadline`; the section ends only after that, so the held level cannot fire on exit.
- `sleep_deep_with_rejection` reports the outcome by polling raw interrupt bits, so it needs no CPU interrupt.
- The hal crate depends on `critical-section = "1.2"`; esp-hal's default `rt` feature provides the implementation.

Contract for firmware, stated in the driver docs: a custom GPIO handler must not `unlisten` a pin that is bound for sleep, and the manager owns the pin's interrupt configuration for the duration of the borrow.
After `try_sleep` returns the pin is unlistened; firmware that uses interrupts on it re-arms its own `listen`.

#### 4c. Timer: drain, floor, post-arm guard

esp-hal's `sleep` runs a UART drain after `sleep_deep`'s deadline assert, and the drain spins until every TX FIFO is empty (about 11 ms for a full FIFO at 115200 baud, about 133 ms at 9600 baud, far longer than a 1 ms timer).
Masking interrupts does not shorten a hardware wait, so the manager orders the steps inside the critical section of 4b:
1. **Drain first.** The manager waits until UART0 and UART1 report an empty TX FIFO and an idle transmitter, the condition esp-hal spins on.
   With interrupts masked nothing refills a FIFO, so esp-hal's own drain at sleep entry then finds nothing to wait for.
   The wait is bounded by `UART_DRAIN_TIMEOUT_US` (200 ms, a full FIFO at 9600 baud plus margin); a transmitter still busy then, for example held off by CTS, returns `UartBusy` (kind `Platform`).
   DMA-driven UART transmission (UHCI) must be stopped by the firmware before a sleep; the manager does not see it.
2. **Request floor.** `MIN_TIMER_MS` (provisionally 1 ms) rejects requests that cannot work on any chip, before anything is touched (4a).
3. **Pins, then the deadline, last.** The deadline is computed from `Instant::now()` after the drain, so the requested duration counts from arming, not from the call.
4. **Post-arm guard.** After `set_wakeup_deadline` and before the sleep call, the manager re-reads `Instant::now()`; if less than `POST_ARM_MARGIN_US` (provisionally 200 µs) remains, it releases everything and returns `TimerTooShort`.
   esp-hal's own `deadline_missed` is `pub(crate)`, so the manager keeps an equivalent check.

Guarantee: no panic and no endless sleep for empty sources, timers outside `[MIN_TIMER_MS, MAX_TIMER_MS]`, bad or foreign pins, by construction.
For short timers the guarantee is a typed error when the margin is already gone at the guard; esp-hal's entry path after the guard (hooks, pad isolation, `start_sleep`) stays unguarded.
On the hal tier, `sleep` keeps `sleep_deep`, so esp-hal's assert stays as the last line of defence; a panic there is a defect in the constants, not a condition the consumer handles.

#### 4d. Exclusive low-power wake pins, re-arming, other pads, pulls

During a sleep attempt the binding is the complete set of low-power wake pins.
esp-hal collects every pad that listens and carries the low-power flag, globally, so a pad firmware configured elsewhere would wake or reject an attempt that binds only another pin, and the cleanup would leave it armed.

- Firmware must not call `apply_wakeup_config` itself; the manager is the only code that sets the low-power flag, and it clears it on every bound pin on release.
- Inside the critical section and before any mutation, the manager reads the trigger of every C3 low-power pad (GPIO0-5) outside the binding; a set trigger returns `ForeignWakePin { pin }` (kind `InvalidConfig`).
- esp-hal keeps the low-power flag in a private static, so the check is conservative: it also refuses an unbound pad that only listens for an ordinary interrupt, which firmware fixes by `unlisten` before the sleep or by binding it.
- Every attempt re-applies `apply_wakeup_config` and `listen` for the mask pins only; other bound pins are released.
- Other digital pads that esp-hal isolated at entry are the firmware's to reconfigure after a rejection; the manager documents this and does not snapshot them, because it cannot know their configuration.
- Wake pins need a pull against their wake level; esp-hal keeps the pin's pull and adds none, and a floating pad wakes immediately.

### 5. Consumer follow-up points

- **Wake-pin validity.** `stoker` validates against the chip constant (`0x3F` on the C3) before the SDK call on both tiers; the IDF tier does not call `esp_sleep_is_valid_wakeup_gpio` per bit.
  A typed, host-tested error that is identical on both tiers beats an IDF-only runtime call.
- **JTAG pin.** GPIO4 is MTMS, the JTAG TMS line; a PIR on GPIO4 is fine for firmware and disables JTAG debugging while it runs, and both examples say so.
- **Wake pin already at its wake level.** `try_sleep` returns `Err` with `kind() == Rejected` on both tiers.
  Both tiers pre-read the wake pins where the hardware does not reject a held pin (C3, original ESP32) and return `Rejected`; the S3 hardware rejects on its own.
- **Slept time.** Out of scope for `stoker` and both tiers in 0.3.0.
  Recipe: IDF `esp_rtc_get_time_us()`, and on esp-hal `EspHalRtcClock::now_us()`; the firmware stores the pre-sleep stamp in the RTC retention envelope and subtracts after the wake.
  Do not use esp-hal `Rtc::time_since_power_up()`: each boot converts the whole 48-bit count with its own calibrated period, so the error grows with uptime.
  `EspHalRtcClock::now_us()` converts only the ticks since the last read, so its error scales with the interval, not with uptime.
  If a second consumer needs it, an additive `stoker` trait in a 0.3.x release is the path; `WakeCause::Timer { slept_ms }` is rejected because it would break the enum again.

### 6. ESP32-S3

The S3 uses the same `SleepManager` trait and `bind_wake_pins` call as the C3; pad topology and wake-source encoding differ.

- Low-power pads GPIO0–21 support wake sources on both tiers; the hal mask comes from `stoker::sleep::ESP32S3_WAKE_PIN_MASK` and is validated with `validate_gpio_level_source_against`.
- A single level group goes to EXT1; mixed levels spill one pin to EXT0 and the rest to per-pin wake, and any leftover keeps the LP peripherals powered.
- Wake pads are held at entry level, not isolated; only pads without an LP number (GPIO22+) are isolated, and a rejected sleep does not restore them.
- Pulls are copied from the GPIO config, none added.
- GPIO0–21 are scanned for foreign wake pins (same rule as C3); the UART drain covers UART0-2 on the S3 (UART0-1 on the C3).
- The S3 hardware rejects a held EXT1 level on both tiers, so no boot loop occurs and no pre-sleep level check is needed in this section; the hal tier re-arms on each attempt.
- A HAL-neutral pad-hold trait is deferred until the IDF tier needs a library API.

## Consequences

Positive:
- One `SleepManager` contract compiles against both tiers; consumer code differs only in how it obtains the manager and, on esp-hal, in one `bind_wake_pins` call.
- Borrowing `&mut Input` at the call compiles on esp-hal 1.2.2, so the consumer's firmware design can assume it.
- Empty sources, timers outside the floor and the maximum, bad masks and foreign wake pins are typed `InvalidConfig` errors before the HAL is touched; no validation error leaves a pin armed.
- No GPIO handler can run between arming and sleep, or before cleanup after a rejection.

Negative:
- 0.3.0 breaks `SleepManager` implementors (new required `try_sleep`, new `Error` bound) and changes `sleep(&[])` from `Ok` to `NoWakeSource`.
- The IDF tier needs a new error type and loses `anyhow` context inside the manager.
- The esp-hal wake cause must be captured at boot before any `try_sleep`; a consumer that reads it later gets `Other` or an empty mask.
- The esp-hal side is pinned to an unstable API at `=1.2.2`, including the unstable `regs()` accessors and register field names.
- The short-timer guarantee rests on two provisional constants until the sweep sets them.
- Firmware loses the pin's interrupt configuration across a `try_sleep` and must not `unlisten` a bound pin from a custom handler.
- The hal crate gains a `critical-section` dependency.
- `try_sleep` waits for queued UART output before arming, up to `UART_DRAIN_TIMEOUT_US`; log output just before a sleep delays it.
- Firmware must not listen on an unbound low-power pad while sleeping and must not call `apply_wakeup_config` itself.

## Alternatives Considered
| Alternative                                    | Pros                                | Cons                                                | Why Rejected                                       |
|:-----------------------------------------------|:------------------------------------|:----------------------------------------------------|:---------------------------------------------------|
| Manager-owned wake pins (esp-hal)              | Simple `new(lpwr, pins)` signature  | Firmware cannot poll the pin while awake            | Consumer requirement (tamer presence polling)      |
| `WakeSource::Pin(&mut dyn WakePad)`            | No mask-to-pin matching on esp-hal  | Lifetime in `WakeSource`, loses `Copy`, dead on IDF | Mask plus `PinNotBound` keeps `WakeSource` neutral |
| Default `try_sleep` delegating to `sleep`      | Non-breaking for implementors       | Silently sleeps through a held wake pin             | The failure the consumer asked to avoid            |
| Drop `sleep` from the trait                    | One method, no drift                | Loses fire-and-forget timer sleep                   | Both are two lines on every implementor            |
| `type Error: core::error::Error` plus `kind()` | `?` into `anyhow`/`Box<dyn Error>`  | Excludes `defmt`-only errors                        | embedded-hal 1.0 shape; `SleepError` implements it |
| One fixed `SleepError` with IDF codes inside   | One error type                      | Puts `esp_err_t` into `stoker`                      | `stoker` stays SDK-free                            |
| Map `ESP_ERR_INVALID_STATE` to `Rejected`      | One match arm                       | Alias of `0x103`, returned by other calls too       | Mapping by call site is exact                      |
| Let esp-hal panic on bad requests              | No validators                       | Battery prop never wakes, no report                 | Typed `InvalidConfig` before the HAL               |
| Clean up after sleep with interrupts enabled   | No critical section                 | A handler's `unlisten` drops the wake source        | Interrupts masked throughout (4b)                  |
| Fixed 1 ms floor as the whole guarantee        | Simple                              | Compilation cannot establish the margin             | Floor plus post-arm guard (4c)                     |
| Margin sized to cover esp-hal's UART drain     | No register access                  | Drain depends on queued bytes and baud              | Drain before arming, bounded (4c)                  |
| Ignore unbound low-power pads                  | No register reads                   | A foreign pad wakes or rejects and stays armed      | Exclusivity rule plus `ForeignWakePin` (4d)        |
| Snapshot and restore all pads                  | Full restore after rejection        | Manager cannot know pad configuration               | Firmware reconfigures them (4d)                    |
| `esp_sleep_is_valid_wakeup_gpio` per bit       | Follows the SDK                     | IDF-only runtime call, same `0x3F` test             | Chip constant in `stoker`, host-tested             |
| Slept time via `WakeCause::Timer { slept_ms }` | No new trait                        | Breaks `WakeCause` again; GPIO wakes need it too    | Retention envelope recipe; additive trait later    |
