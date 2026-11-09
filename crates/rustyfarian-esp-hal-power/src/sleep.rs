//! Deep sleep with GPIO and timer wake on esp-hal, for the ESP32-C3, ESP32-C6, ESP32-S3 and ESP32.
//!
//! Implements [`SleepManager`] and [`WakeCauseSource`] from [`stoker`]; the design is recorded
//! in `docs/adr/001-hal-neutral-sleep-contract.md`.
//!
//! # Chips
//!
//! | Chip     | Wake pads | Wake path in esp-hal                                  | Wake cause            |
//! |:---------|:----------|:------------------------------------------------------|:----------------------|
//! | ESP32-C3 | GPIO0-5   | per-pin deep-sleep GPIO wake                          | `Gpio(mask)`          |
//! | ESP32-C6 | GPIO0-7   | EXT1 (one level per pad)                              | `Ext1(mask)`          |
//! | ESP32-S3 | GPIO0-21  | EXT1 for a lone level, spilling to EXT0 / per-pin     | `Ext1(mask)`          |
//! | ESP32    | RTC pads  | EXT1 (one level for the whole mask)                   | `Ext1(mask)`          |
//!
//! The wake cause is normalised per chip, not per tier: the same chip reports the same
//! [`WakeCause`] variant on the ESP-IDF tier.
//!
//! On the S3, esp-hal allocates the wake hardware itself. A single pin level on GPIO0-21 goes to
//! EXT1 (one level for the whole mask). Mixed levels spill to EXT0 and per-pin wake, and those
//! keep the low-power peripherals powered, which costs current in sleep.
//!
//! On the ESP32 the wake pads are the RTC GPIOs (0, 2, 4, 12-15, 25-27, 32-39) and EXT1 combines
//! several low pads with AND, not OR. esp-hal would spill the extra low pins to other paths, so
//! [`GpioWakeLevel::AnyLow`] accepts a single pin only: several pins return
//! [`SleepError::AnyLowMultiplePins`] (kind `Unsupported`) before anything is armed, on both
//! tiers. `AnyHigh` takes any number of pads.
//!
//! # Ownership
//!
//! [`EspHalSleepManager`] owns only the `LPWR` peripheral. The firmware keeps owning its wake
//! pins and polls them between sleeps. [`EspHalSleepManager::bind_wake_pins`] borrows them as
//! `&mut Input` for one sleep attempt and returns a [`BoundSleep`] that implements
//! [`SleepManager`]. The borrow ends when the [`BoundSleep`] is dropped, so the same `Input` can
//! be read again after a rejected [`SleepManager::try_sleep`].
//!
//! ```ignore
//! let mut manager = EspHalSleepManager::new(peripherals.LPWR);
//! let mut int = Input::new(peripherals.GPIO5, InputConfig::default());
//! let outcome = manager.bind_wake_pins(&mut [&mut int]).try_sleep(&sources);
//! ```
//!
//! The pins carry their own lifetime `'i`. `&mut Input<'i>` is invariant in `'i`, so tying it to
//! the manager's `'d` makes independent lifetimes fail with "lifetime may not live long enough"
//! at the call site.
//!
//! # Interrupt contract
//!
//! Arming, the sleep call and the cleanup after a rejection run in one critical section. A GPIO
//! interrupt handler therefore never sees an armed, already-asserted pin. A custom handler must
//! not call `unlisten()` on a bound pin: that drops the pin from the wake set. The critical
//! section implementation comes from esp-hal's `rt` feature; firmware binaries need it.
//!
//! # Pulls and pad state
//!
//! esp-hal copies the pad pulls into the sleep configuration and adds none. Configure the pull
//! *against* the wake level (pull-up for a low-level wake) or rely on an external one; a floating
//! pad can wake spuriously. Wake pads stay held at their level from sleep entry and are not
//! isolated. On the S3, GPIO22 and above are isolated and are not restored after a rejected
//! sleep.
//!
//! # Wake cause
//!
//! Create the [`EspHalWakeCauseSource`] at boot, before the first `try_sleep`: a rejected sleep
//! overwrites the per-pin wake record. esp-hal has no wake-pin status mask, so the pins whose
//! `caused_wakeup()` is set are the ones the firmware hands to [`EspHalWakeCauseSource::capture`].

#[cfg(any(feature = "esp32", feature = "esp32s3"))]
use esp_hal::peripherals::UART2;
use esp_hal::{
    gpio::{Event, Input, WakeupConfig},
    peripherals::{GPIO, LPWR, UART0, UART1},
    rtc_cntl::sleep::{LowPower, RtcSleepConfig},
    time::{Duration, Instant},
};

// The local `sleep` module shadows `stoker::sleep` re-exported by the crate root; re-export its
// items here so `rustyfarian_esp_hal_power::sleep::*` still reaches them. The glob also brings
// the `stoker` sleep types used below into scope (an explicit import would shadow the re-export).
pub use stoker::sleep::*;
use stoker::{
    drive, ChipPolicy, DriveError, Entered, SleepAttemptMachine, SleepAttemptPort, SleepPlan,
};

#[cfg(all(feature = "bench", not(feature = "esp32c3")))]
compile_error!("the `bench` feature supports the ESP32-C3 only; enable `esp32c3`");

/// Pre-read guard policy of this chip for the attempt machine.
#[cfg(feature = "esp32c3")]
const CHIP_POLICY: ChipPolicy = ChipPolicy::Esp32C3;

/// Pre-read guard policy of this chip for the attempt machine.
///
/// `Never`: the C6 has no global digital-pad hold (no `dig_iso` register), so the pad-hold hang
/// class does not exist here and the pins are read only if the hardware rejects. Whether a held
/// EXT1 pin is rejected or wakes at once is a bench question (one-line change to `Always`).
#[cfg(feature = "esp32c6")]
const CHIP_POLICY: ChipPolicy = ChipPolicy::Esp32C6;

/// Pre-read guard policy of this chip for the attempt machine.
#[cfg(feature = "esp32s3")]
const CHIP_POLICY: ChipPolicy = ChipPolicy::Esp32S3;

/// Pre-read guard policy of this chip for the attempt machine.
///
/// `Always`: EXT0/EXT1 wakes cannot be rejected by the hardware here, so the pins are always read
/// and the global pad-hold register (`ReadHold`) is never consulted.
#[cfg(feature = "esp32")]
const CHIP_POLICY: ChipPolicy = ChipPolicy::Esp32;

/// esp-hal's deep-sleep configuration, with RTC fast memory forced on on the ESP32.
///
/// esp-hal 1.2.2 `RtcSleepConfig::deep()` on the ESP32 clears `fastmem_force_pu`, `force_lpu`
/// and `force_noiso` (through `rtc_fastmem_pd_en`; it never writes `fastmem_pd_en`), and the
/// fast memory then loses its contents in deep sleep: `RtcRetainedCell` loaded `Invalid::Magic`
/// and `EspHalRtcClock` restarted after every timer wake. Forcing the memory on keeps
/// `.rtc_fast.persistent` (bench 2026-11-08, ESP32-WROOM-32: `load=ok` across two timer wakes).
/// Costs the fast-memory retention current in deep sleep (a few µA, not yet measured).
/// The other chips keep esp-hal's default.
fn deep_sleep_config() -> RtcSleepConfig {
    #[allow(unused_mut)]
    let mut cfg = RtcSleepConfig::deep();
    #[cfg(feature = "esp32")]
    cfg.set_rtc_fastmem_pd_en(false);
    cfg
}

/// Pads with a low-power wake path on this chip, from the shared chip table.
const WAKE_PIN_MASK: u64 = match CHIP_POLICY.wake_pin_mask() {
    Some(mask) => mask,
    None => panic!("the esp-hal tier chips all have a wake-pin mask"),
};

/// Shortest timer the manager accepts, in milliseconds (request-level check, before any
/// mutation).
///
/// esp-hal 1.2.2 cannot catch a deadline nearer than 16 slow-clock ticks (about 100 µs,
/// `sleep/timer.rs` `ALARM_TO_SLEEP_TICKS`); `sleep_deep` asserts on it, but
/// `sleep_deep_with_rejection` does not. The floor is provisional until the bench measures the
/// arm-to-sleep path (ADR-001 § 4).
const MIN_TIMER_MS: u64 = 1;

/// Margin the deadline must still have after arming, re-checked with interrupts masked right
/// before the sleep call. Covers esp-hal's own entry path (hooks, `apply`, UART suspend) up to
/// its `deadline_missed` check. Provisional: set from the bench measurement.
const POST_ARM_MARGIN_US: u64 = 200;

/// Longest the manager waits for the UART transmitters to drain before arming, in µs.
///
/// A full 128-byte TX FIFO takes about 133 ms at 9600 baud. A transmitter that is still busy
/// after this (CTS flow control held off, DMA feeding the FIFO) is a `UartBusy` error instead of
/// the endless spin esp-hal's own drain would give inside sleep entry. Provisional.
const UART_DRAIN_TIMEOUT_US: u64 = 200_000;

/// Bench instrumentation: a GPIO marker around the arm-to-sleep path and timing capture.
///
/// Internal and unstable; enabled by the `bench` feature on the ESP32-C3 only. The marker pin is
/// driven low at the top of the critical section and high right after arming, so a logic analyser
/// or the PPK2 sees the arm-to-sleep window. The pin must already be configured as an output by
/// the firmware (and be below GPIO32); the hook writes the set/clear registers directly and
/// borrows no driver.
#[cfg(feature = "bench")]
pub mod bench {
    use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

    use esp_hal::{peripherals::GPIO, time::Instant};

    /// Marker pin number; `u8::MAX` = no marker.
    static MARKER_PIN: AtomicU8 = AtomicU8::new(u8::MAX);
    static DRAIN_US: AtomicU32 = AtomicU32::new(0);
    static GUARD_US: AtomicU32 = AtomicU32::new(0);

    /// Timings of the most recent sleep attempt, in microseconds.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct BenchTimings {
        /// Time spent in the UART drain wait.
        pub drain_us: u32,
        /// Time from the marker rising edge (armed) to the rejected return of `try_sleep`.
        pub guard_us: u32,
    }

    /// Selects the marker pin (GPIO0-31); a pin of 32 or above disables the marker.
    ///
    /// The firmware configures the pin as an output first.
    pub fn set_marker_pin(pin: u8) {
        let pin = if pin < 32 { pin } else { u8::MAX };
        MARKER_PIN.store(pin, Ordering::Relaxed);
    }

    /// Timings recorded by the most recent `sleep` or `try_sleep`.
    ///
    /// `guard_us` is only meaningful after a rejected `try_sleep`.
    #[must_use]
    pub fn last() -> BenchTimings {
        BenchTimings {
            drain_us: DRAIN_US.load(Ordering::Relaxed),
            guard_us: GUARD_US.load(Ordering::Relaxed),
        }
    }

    fn marker_mask() -> Option<u32> {
        match MARKER_PIN.load(Ordering::Relaxed) {
            pin if pin < 32 => Some(1u32 << pin),
            _ => None,
        }
    }

    /// Starts an attempt: drives the marker low and clears the previous attempt's timings.
    pub(super) fn begin_attempt() {
        DRAIN_US.store(0, Ordering::Relaxed);
        GUARD_US.store(0, Ordering::Relaxed);
        if let Some(mask) = marker_mask() {
            // SAFETY: `mask` has exactly one bit set, for a pin below 32. `out_w1tc` is a
            // write-only clear register, so the write cannot race a driver's read-modify-write.
            GPIO::regs().out_w1tc().write(|w| unsafe { w.bits(mask) });
        }
    }

    pub(super) fn marker_high() {
        if let Some(mask) = marker_mask() {
            // SAFETY: `mask` has exactly one bit set, for a pin below 32. `out_w1ts` is a
            // write-only set register, so the write cannot race a driver's read-modify-write.
            GPIO::regs().out_w1ts().write(|w| unsafe { w.bits(mask) });
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn now_us() -> u32 {
        // Wraps after about 71 minutes; only differences are used.
        Instant::now().duration_since_epoch().as_micros() as u32
    }

    pub(super) fn set_drain_us(us: u32) {
        DRAIN_US.store(us, Ordering::Relaxed);
    }

    pub(super) fn set_guard_us(us: u32) {
        GUARD_US.store(us, Ordering::Relaxed);
    }
}

/// A fully validated request. Building it touches no hardware, so a validation error leaves
/// every pin and the deadline exactly as they were.
#[derive(Clone, Copy)]
struct Plan {
    /// Pins to arm, already checked against the chip mask and the bound pins; 0 = none.
    pin_mask: u64,
    event: Event,
    /// Requested timer, already checked against the floor and `u64` µs overflow.
    timer: Option<TimerPlan>,
    /// The attempt state machine for this request; one machine per attempt.
    machine: SleepAttemptMachine,
}

#[derive(Clone, Copy)]
struct TimerPlan {
    duration_ms: u64,
    ahead_us: u64,
}

/// Deep-sleep manager for esp-hal. Owns the `LowPower` driver only; no pin lives here.
///
/// Bind the wake pins with [`bind_wake_pins`](Self::bind_wake_pins) to sleep.
pub struct EspHalSleepManager<'d> {
    low_power: LowPower<'d>,
}

impl<'d> EspHalSleepManager<'d> {
    /// Takes the `LPWR` peripheral.
    #[must_use]
    pub fn new(lpwr: LPWR<'d>) -> Self {
        Self {
            low_power: LowPower::new(lpwr),
        }
    }

    /// Borrows the wake pins for one sleep attempt.
    ///
    /// The borrow ends when the returned value is dropped, so the firmware can poll the same
    /// `Input` again after a rejected `try_sleep`. The pins' own lifetime `'i` is independent
    /// of the manager's `'d`.
    pub fn bind_wake_pins<'a, 'p, 'i>(
        &'a mut self,
        pins: &'a mut [&'p mut Input<'i>],
    ) -> BoundSleep<'a, 'p, 'i, 'd> {
        BoundSleep {
            manager: self,
            pins,
        }
    }
}

/// A sleep manager with its wake pins bound for one attempt.
///
/// Created by [`EspHalSleepManager::bind_wake_pins`]; sleep through its [`SleepManager`] impl.
#[must_use = "a BoundSleep does nothing until `sleep` or `try_sleep` is called"]
pub struct BoundSleep<'a, 'p, 'i, 'd> {
    manager: &'a mut EspHalSleepManager<'d>,
    pins: &'a mut [&'p mut Input<'i>],
}

impl BoundSleep<'_, '_, '_, '_> {
    /// Validates the whole request without touching hardware.
    fn plan(&self, sources: &[WakeSource]) -> Result<Plan, SleepError> {
        // Also rejects an empty slice, multiple timers or GPIO sources, a timer above
        // `MAX_TIMER_MS` and an empty pin mask.
        // On the original ESP32 it also refuses `AnyLow` over several pins: EXT1 combines low
        // pads with AND, which is not the `AnyLow` (any pin) contract (ADR-003 point 2).
        let machine = SleepAttemptMachine::new(CHIP_POLICY, sources)?;
        let request = machine.plan();

        let mut plan = Plan {
            pin_mask: 0,
            event: Event::HighLevel,
            timer: None,
            machine,
        };
        if request.has_pins() {
            self.check_bound(request.pin_mask)?;
            plan.pin_mask = request.pin_mask;
            plan.event = match request.level {
                GpioWakeLevel::AnyHigh => Event::HighLevel,
                GpioWakeLevel::AnyLow => Event::LowLevel,
            };
        }
        if let Some(duration_ms) = request.timer {
            if duration_ms < MIN_TIMER_MS {
                return Err(SleepError::TimerTooShort {
                    duration_ms,
                    min_ms: MIN_TIMER_MS,
                });
            }
            // Cannot overflow below `MAX_TIMER_MS`; checked anyway.
            let ahead_us = duration_ms.saturating_mul(1_000);
            Self::deadline_us(ahead_us, duration_ms)?;
            plan.timer = Some(TimerPlan {
                duration_ms,
                ahead_us,
            });
        }
        Ok(plan)
    }

    fn check_bound(&self, pin_mask: u64) -> Result<(), SleepError> {
        let unbound = pin_mask & !self.bound_mask();
        if unbound != 0 {
            return Err(SleepError::PinNotBound {
                // At most 63: `trailing_zeros` of a non-zero u64.
                pin: unbound.trailing_zeros() as u8,
            });
        }
        Ok(())
    }

    fn bound_mask(&self) -> u64 {
        self.pins
            .iter()
            .filter_map(|pin| pin.peripheral_input().gpio_number())
            .fold(0u64, |mask, n| mask | (1u64 << n))
    }

    /// Absolute deadline in µs since boot. `Instant + Duration` panics on overflow (fugit), so
    /// the sum is checked here and the `Instant` built from `EPOCH`; unreachable below
    /// `MAX_TIMER_MS`, reported as `TimerTooLong` if it ever happens.
    fn deadline_us(ahead_us: u64, duration_ms: u64) -> Result<u64, SleepError> {
        Instant::now()
            .duration_since_epoch()
            .as_micros()
            .checked_add(ahead_us)
            .ok_or(SleepError::TimerTooLong {
                duration_ms,
                max_ms: MAX_TIMER_MS,
            })
    }

    /// Refuses a listening low-power pad that is not bound. Reads registers only.
    ///
    /// esp-hal arms every pad that listens and carries the low-power flag, globally
    /// (`gpio/wakeup/mod.rs` `collect`). The flag itself is a private static, so the check is
    /// conservative: any low-power pad of the chip outside the binding whose `int_type` is
    /// set counts, the same field esp-hal reads for the wake level (`armed_level`).
    fn check_foreign_wake_pins(&self) -> Result<(), SleepError> {
        let bound = self.bound_mask();
        // The ESP32 mask is not contiguous, so walk its set bits.
        for pin in (0..64u8).filter(|pin| WAKE_PIN_MASK & (1u64 << pin) != 0) {
            if bound & (1u64 << pin) != 0 {
                continue;
            }
            if GPIO::regs().pin(pin as usize).read().int_type().bits() != 0 {
                return Err(SleepError::ForeignWakePin { pin });
            }
        }
        Ok(())
    }

    /// Waits until every UART transmitter is idle (and, except on the C6, its FIFO empty), the
    /// condition esp-hal's `wait_for_suspended` spins on at sleep entry (`uart/low_level/v1.rs`,
    /// `v2.rs` on the C6), bounded by `UART_DRAIN_TIMEOUT_US`. Call with interrupts masked, so
    /// no handler refills a FIFO.
    fn drain_uarts() -> Result<(), SleepError> {
        // The ESP32 has the transmitter state in `status`, the other chips in `fsm_status`.
        // The C6 (UART v2) also treats TX_WAIT_SEND (0x0F) as suspended and does not wait for
        // the FIFO count, exactly like esp-hal.
        macro_rules! tx_busy {
            ($uart:ident) => {{
                let regs = $uart::regs();
                #[cfg(feature = "esp32")]
                let state = regs.status().read().st_utx_out().bits();
                #[cfg(not(feature = "esp32"))]
                let state = regs.fsm_status().read().st_utx_out().bits();
                #[cfg(feature = "esp32c6")]
                let busy = state != 0 && state != 0x0F;
                #[cfg(not(feature = "esp32c6"))]
                let busy = regs.status().read().txfifo_cnt().bits() != 0 || state != 0;
                busy
            }};
        }

        let start = Instant::now();
        let timeout = Duration::from_micros(UART_DRAIN_TIMEOUT_US);
        loop {
            let busy0 = tx_busy!(UART0);
            let busy1 = tx_busy!(UART1);
            #[cfg(any(feature = "esp32", feature = "esp32s3"))]
            let busy2 = tx_busy!(UART2);
            #[cfg(not(any(feature = "esp32", feature = "esp32s3")))]
            let busy2 = false;
            if !busy0 && !busy1 && !busy2 {
                return Ok(());
            }
            if start.elapsed() > timeout {
                let uart = if busy0 {
                    0
                } else if busy1 {
                    1
                } else {
                    2
                };
                return Err(SleepError::UartBusy { uart });
            }
        }
    }

    /// Arms a validated plan. Call with interrupts masked.
    ///
    /// Order: foreign-pin check and UART drain (both read-only, an error leaves everything
    /// untouched), then pins, then the deadline, then the post-arm margin check. Any failure
    /// after the first mutation releases everything it armed.
    fn arm(&mut self, plan: Plan) -> Result<(), SleepError> {
        // Every arm starts here, timer-only included, so no later wake reads a stale record.
        #[cfg(feature = "esp32")]
        crate::armed_ext1::invalidate();
        self.check_foreign_wake_pins()?;
        #[cfg(feature = "bench")]
        let drain_start = bench::now_us();
        Self::drain_uarts()?;
        #[cfg(feature = "bench")]
        bench::set_drain_us(bench::now_us().wrapping_sub(drain_start));

        if let Err(e) = self.arm_pins(plan.pin_mask, plan.event) {
            self.disarm();
            return Err(e);
        }

        // Multi-pin `AnyLow` was refused in validation, so a low record names one pin
        // (`stoker::ext1_fired_mask` needs exactly that); a timer-only arm stores nothing.
        #[cfg(feature = "esp32")]
        if plan.pin_mask != 0 {
            crate::armed_ext1::store(plan.pin_mask, plan.event == Event::LowLevel);
        }

        let Some(timer) = plan.timer else {
            self.manager.low_power.clear_wakeup_deadline();
            return Ok(());
        };
        let deadline_us = match Self::deadline_us(timer.ahead_us, timer.duration_ms) {
            Ok(us) => us,
            Err(e) => {
                self.disarm();
                return Err(e);
            }
        };
        self.manager
            .low_power
            .set_wakeup_deadline(Instant::EPOCH + Duration::from_micros(deadline_us));

        // Post-arm guard: esp-hal's `deadline_missed` is `pub(crate)`, so re-check our own
        // margin after arming and before the sleep call.
        let now_us = Instant::now().duration_since_epoch().as_micros();
        if now_us.saturating_add(POST_ARM_MARGIN_US) >= deadline_us {
            self.disarm();
            return Err(SleepError::TimerTooShort {
                duration_ms: timer.duration_ms,
                min_ms: MIN_TIMER_MS,
            });
        }
        Ok(())
    }

    /// Arms exactly the mask pins and releases every other bound pin. Only the intersection of
    /// bound pins and mask bits may listen: esp-hal arms every listening pin that carries the
    /// low-power flag (`gpio/wakeup/mod.rs` `collect`).
    fn arm_pins(&mut self, pin_mask: u64, event: Event) -> Result<(), SleepError> {
        let low_power_path = WakeupConfig::default().with_low_power_path(true);
        for pin in self.pins.iter_mut() {
            let Some(n) = pin.peripheral_input().gpio_number() else {
                continue;
            };
            if pin_mask & (1u64 << n) != 0 {
                // A refusal here means a pad the mask check let through has no low-power path.
                pin.apply_wakeup_config(&low_power_path).map_err(|_| {
                    SleepError::GpioMaskOutOfRange {
                        pin_mask: 1u64 << n,
                        valid_mask: WAKE_PIN_MASK,
                    }
                })?;
                pin.listen(event);
            } else {
                Self::release_pin(pin);
            }
        }
        Ok(())
    }

    /// Returns one pin to a plain polled input: no interrupt enable (a latched level with
    /// `rt` would storm), no stale status, and no persistent low-power wake flag
    /// (`apply_wakeup_config` outlives the driver, `gpio/mod.rs` ~1395).
    fn release_pin(pin: &mut Input<'_>) {
        pin.unlisten();
        pin.clear_interrupt();
        let _ = pin.apply_wakeup_config(&WakeupConfig::default());
    }

    /// Reads the bound wake pins as a level mask (bit N set = GPIO N high). Call with
    /// interrupts masked, right before sleep entry. Pins are read through the bound `Input`s,
    /// which carry the input enable, so every masked pin is readable.
    fn read_levels(&self) -> u64 {
        self.pins
            .iter()
            .filter_map(|pin| {
                let n = pin.peripheral_input().gpio_number()?;
                pin.is_high().then_some(1u64 << n)
            })
            .fold(0u64, |mask, bit| mask | bit)
    }

    /// Releases every bound pin and clears the standing deadline. Used after a rejection and
    /// after any arming failure, so the firmware's retry loop starts from a clean state.
    fn disarm(&mut self) {
        // Everything is disarmed here, so no record may claim an armed state that is gone.
        #[cfg(feature = "esp32")]
        crate::armed_ext1::invalidate();
        for pin in self.pins.iter_mut() {
            Self::release_pin(pin);
        }
        self.manager.low_power.clear_wakeup_deadline();
    }
}

impl SleepManager for BoundSleep<'_, '_, '_, '_> {
    type Error = SleepError;

    /// Non-rejecting deep sleep; returns only on a configuration error. esp-hal's
    /// `deadline_missed` assert stays as the last line of defence behind the post-arm guard.
    fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), SleepError> {
        let plan = self.plan(sources)?;
        critical_section::with(|_| {
            #[cfg(feature = "bench")]
            bench::begin_attempt();
            self.arm(plan)?;
            #[cfg(feature = "bench")]
            bench::marker_high();
            self.manager.low_power.sleep_deep(deep_sleep_config())
        })
    }

    /// Deep sleep that returns only when the attempt is rejected (a wake pin already at its
    /// wake level). Mirrors `esp_deep_sleep_try_to_start` on the IDF tier.
    ///
    /// The attempt is driven by [`SleepAttemptMachine`], which decides when to read the wake
    /// pins; this tier only performs the reads, the arming and the sleep call. On the ESP32-C3
    /// the bound wake pins are always read right before sleep entry, and a pin already at its
    /// wake level returns `Rejected` from software (the hardware path is not relied on).
    ///
    /// On the ESP32-S3 the hardware never returns from a rejected sleep while the global
    /// digital-pad hold is effective (`RTC_CNTL_DIG_ISO_REG` bit 11 set, bit 14 clear): sleep
    /// entry isolates every unheld digital pad and the chip stalls silently. When the hold is
    /// effective, the pins are therefore read right before sleep entry and `Rejected` is
    /// returned for a pin already at its wake level. The global hold costs the isolation of
    /// every unheld digital pad; per-pin holds on RTC pads do not need it. A pin that reaches
    /// its wake level after the read, and non-pin hardware rejections, can still stall while
    /// the hold is effective.
    ///
    /// Validation runs first and touches nothing. Arming, the pin reads, the sleep call and the
    /// cleanup after a rejection then run in one critical section, so no GPIO handler can
    /// observe the armed, already-asserted pin: a custom handler that calls `unlisten()` would
    /// otherwise drop the pin from the wake set (`gpio/low_level/mod.rs` `set_int_enable`
    /// clears `listening`).
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), SleepError> {
        let plan = self.plan(sources)?;
        critical_section::with(|_| {
            #[cfg(feature = "bench")]
            bench::begin_attempt();
            let mut port = EspHalPort {
                sleep: self,
                plan,
                #[cfg(feature = "bench")]
                armed_at: 0,
            };
            match drive(plan.machine, &mut port) {
                Ok(()) => Ok(()),
                Err(DriveError::Arm(e)) => Err(e),
                Err(DriveError::Rejected(_)) => Err(SleepError::Rejected),
                Err(DriveError::Protocol) => Err(SleepError::AttemptOutOfPhase),
            }
        })
    }
}

/// The attempt port of this tier: performs the register and pin I/O the shared [`drive`] asks
/// for, inside the caller's critical section. Bench hooks live here.
struct EspHalPort<'s, 'a, 'p, 'i, 'd> {
    sleep: &'s mut BoundSleep<'a, 'p, 'i, 'd>,
    /// The validated request; `arm` uses it instead of the machine's `SleepPlan`.
    plan: Plan,
    #[cfg(feature = "bench")]
    armed_at: u32,
}

impl SleepAttemptPort for EspHalPort<'_, '_, '_, '_, '_> {
    type Error = SleepError;

    fn arm(&mut self, _plan: SleepPlan) -> Result<(), SleepError> {
        self.sleep.arm(self.plan)?;
        #[cfg(feature = "bench")]
        {
            bench::marker_high();
            self.armed_at = bench::now_us();
        }
        Ok(())
    }

    // `RTC_CNTL_DIG_ISO_REG` (S3 policy only): global digital-pad hold state.
    #[cfg(not(feature = "esp32c6"))]
    fn read_hold(&mut self) -> bool {
        digital_hold_effective(LPWR::regs().dig_iso().read().bits())
    }

    // The C6 has no global digital-pad hold; its policy never asks, and a stray request is
    // answered "not effective".
    #[cfg(feature = "esp32c6")]
    fn read_hold(&mut self) -> bool {
        false
    }

    fn read_pins(&mut self, _mask: u64) -> u64 {
        self.sleep.read_levels()
    }

    fn enter(&mut self) -> Entered {
        self.sleep
            .manager
            .low_power
            .sleep_deep_with_rejection(deep_sleep_config());
        // Reaching this line *is* the rejection report (esp-hal 1.2.2 `sleep/mod.rs:150`).
        Entered::Returned
    }

    // The whole attempt runs with interrupts masked, so the pins are plain inputs again before
    // the interrupts come back; a timer-only plan (`pins` = 0) clears its deadline here too.
    fn release(&mut self, _pins: u64) {
        #[cfg(feature = "bench")]
        bench::set_guard_us(bench::now_us().wrapping_sub(self.armed_at));
        self.sleep.disarm();
    }
}

pub use crate::wake::EspHalWakeCauseSource;
