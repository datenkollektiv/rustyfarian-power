//! Deep sleep with GPIO and timer wake on esp-hal, for the ESP32-C3 and ESP32-S3.
//!
//! Implements [`SleepManager`] and [`WakeCauseSource`] from [`stoker`]; the design is recorded
//! in `docs/adr/001-hal-neutral-sleep-contract.md`. The ESP32-C6 and the ESP32 have no sleep
//! module yet.
//!
//! # Chips
//!
//! | Chip     | Wake pads | Wake path in esp-hal                                  | Wake cause            |
//! |:---------|:----------|:------------------------------------------------------|:----------------------|
//! | ESP32-C3 | GPIO0-5   | per-pin deep-sleep GPIO wake                          | `Gpio(mask)`          |
//! | ESP32-S3 | GPIO0-21  | EXT1 for a lone level, spilling to EXT0 / per-pin     | `Ext1(mask)`          |
//!
//! The wake cause is normalised per chip, not per tier: the same chip reports the same
//! [`WakeCause`] variant on the ESP-IDF tier.
//!
//! On the S3, esp-hal allocates the wake hardware itself. A single pin level on GPIO0-21 goes to
//! EXT1 (one level for the whole mask). Mixed levels spill to EXT0 and per-pin wake, and those
//! keep the low-power peripherals powered, which costs current in sleep.
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

#[cfg(feature = "esp32s3")]
use esp_hal::peripherals::UART2;
use esp_hal::{
    gpio::{Event, Input, WakeupConfig},
    peripherals::{GPIO, LPWR, UART0, UART1},
    rtc_cntl::{
        sleep::{LowPower, RtcSleepConfig},
        wakeup_cause, WakeupSource,
    },
    time::{Duration, Instant},
};

// The local `sleep` module shadows `stoker::sleep` re-exported by the crate root; re-export its
// items here so `rustyfarian_esp_hal_power::sleep::*` still reaches them. The glob also brings
// the `stoker` sleep types used below into scope (an explicit import would shadow the re-export).
pub use stoker::sleep::*;

#[cfg(all(feature = "bench", not(feature = "esp32c3")))]
compile_error!("the `bench` feature supports the ESP32-C3 only; enable `esp32c3`");

/// Pads with a low-power wake path on this chip.
#[cfg(feature = "esp32c3")]
const WAKE_PIN_MASK: u64 = stoker::sleep::ESP32C3_WAKE_PIN_MASK;

/// Pads with a low-power wake path on this chip (GPIO0-21).
#[cfg(feature = "esp32s3")]
const WAKE_PIN_MASK: u64 = stoker::sleep::ESP32S3_WAKE_PIN_MASK;

// The foreign-pin scan walks `0..WAKE_PIN_MASK.count_ones()`, which only covers the mask when
// the wake pads are contiguous from GPIO0.
const _: () = assert!(
    WAKE_PIN_MASK & (WAKE_PIN_MASK + 1) == 0,
    "WAKE_PIN_MASK must be contiguous from bit 0"
);

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
        // Also rejects an empty slice, multiple timers or GPIO sources, and a timer above
        // `MAX_TIMER_MS`.
        validate_wake_sources(sources)?;

        let mut plan = Plan {
            pin_mask: 0,
            event: Event::HighLevel,
            timer: None,
        };
        for source in sources {
            match *source {
                WakeSource::GpioLevel { pin_mask, level } => {
                    validate_gpio_level_source_against(pin_mask, WAKE_PIN_MASK)?;
                    self.check_bound(pin_mask)?;
                    plan.pin_mask = pin_mask;
                    plan.event = match level {
                        GpioWakeLevel::AnyHigh => Event::HighLevel,
                        GpioWakeLevel::AnyLow => Event::LowLevel,
                    };
                }
                WakeSource::Timer { duration_ms } => {
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
            }
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
        for pin in 0..WAKE_PIN_MASK.count_ones() as u8 {
            if bound & (1u64 << pin) != 0 {
                continue;
            }
            if GPIO::regs().pin(pin as usize).read().int_type().bits() != 0 {
                return Err(SleepError::ForeignWakePin { pin });
            }
        }
        Ok(())
    }

    /// Waits until every UART transmitter is empty and idle, the same condition esp-hal's
    /// `wait_for_suspended` spins on at sleep entry (`uart/low_level/v1.rs`), bounded by
    /// `UART_DRAIN_TIMEOUT_US`. Call with interrupts masked, so no handler refills a FIFO.
    fn drain_uarts() -> Result<(), SleepError> {
        macro_rules! tx_busy {
            ($uart:ident) => {
                $uart::regs().status().read().txfifo_cnt().bits() != 0
                    || $uart::regs().fsm_status().read().st_utx_out().bits() != 0
            };
        }

        let start = Instant::now();
        let timeout = Duration::from_micros(UART_DRAIN_TIMEOUT_US);
        loop {
            let busy0 = tx_busy!(UART0);
            let busy1 = tx_busy!(UART1);
            #[cfg(feature = "esp32s3")]
            let busy2 = tx_busy!(UART2);
            #[cfg(not(feature = "esp32s3"))]
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

    /// Releases every bound pin and clears the standing deadline. Used after a rejection and
    /// after any arming failure, so the firmware's retry loop starts from a clean state.
    fn disarm(&mut self) {
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
            self.manager.low_power.sleep_deep(RtcSleepConfig::deep())
        })
    }

    /// Deep sleep that returns only when the hardware rejects it (a wake pin already at its
    /// wake level). Mirrors `esp_deep_sleep_try_to_start` on the IDF tier.
    ///
    /// Validation runs first and touches nothing. Arming, the sleep call and the cleanup after
    /// a rejection then run in one critical section, so no GPIO handler can observe the armed,
    /// already-asserted pin: a custom handler that calls `unlisten()` would otherwise drop the
    /// pin from the wake set (`gpio/low_level/mod.rs` `set_int_enable` clears `listening`).
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), SleepError> {
        let plan = self.plan(sources)?;
        critical_section::with(|_| {
            #[cfg(feature = "bench")]
            bench::begin_attempt();
            self.arm(plan)?;
            #[cfg(feature = "bench")]
            let armed_at = {
                bench::marker_high();
                bench::now_us()
            };
            self.manager
                .low_power
                .sleep_deep_with_rejection(RtcSleepConfig::deep());
            #[cfg(feature = "bench")]
            bench::set_guard_us(bench::now_us().wrapping_sub(armed_at));
            // Reaching this line *is* the rejection report (esp-hal 1.2.2 `sleep/mod.rs:150`).
            // Release before interrupts come back, so the held level cannot fire a handler.
            self.disarm();
            Err(SleepError::Rejected)
        })
    }
}

/// Wake cause captured once at boot from the pins the firmware re-creates.
///
/// esp-hal has no wake-pin status bitmask; `caused_wakeup()` is per pin, so the mask is
/// rebuilt from the pins the firmware hands in. Capture it **before** the first `try_sleep`:
/// a rejected sleep runs `record_wakeup()` again and overwrites the per-pin record.
#[derive(Debug, Clone, Copy)]
pub struct EspHalWakeCauseSource {
    cause: WakeCause,
}

impl EspHalWakeCauseSource {
    /// Reads the wake cause; `pins` are the wake pins whose `caused_wakeup()` build the mask.
    ///
    /// Pin sources win over the timer when both fired. On the S3 an EXT1 wake reports
    /// `Ext1(mask)`, on the C3 a GPIO wake reports `Gpio(mask)`.
    #[must_use]
    pub fn capture(pins: &[&Input<'_>]) -> Self {
        let reason = wakeup_cause();
        let fired_mask = || {
            GpioWakeMask(
                pins.iter()
                    .filter(|pin| pin.caused_wakeup())
                    .filter_map(|pin| pin.peripheral_input().gpio_number())
                    .fold(0u64, |mask, n| mask | (1u64 << n)),
            )
        };

        let cause = if reason.is_empty() {
            WakeCause::PowerOn
        } else if cfg!(feature = "esp32s3") && reason_has_ext1(&reason) {
            WakeCause::Ext1(fired_mask())
        } else if cfg!(feature = "esp32s3") && reason_has_ext0(&reason) {
            WakeCause::Ext0
        } else if reason.contains(WakeupSource::Gpio) {
            WakeCause::Gpio(fired_mask())
        } else if reason.contains(WakeupSource::Timer) {
            WakeCause::Timer
        } else {
            WakeCause::Other
        };
        Self { cause }
    }
}

// `WakeupSource::Ext0/Ext1` exist on the S3 only; these helpers keep the C3 arm compiling.
#[cfg(feature = "esp32s3")]
fn reason_has_ext1(reason: &esp_hal::rtc_cntl::WakeupReason) -> bool {
    reason.contains(WakeupSource::Ext1)
}

#[cfg(not(feature = "esp32s3"))]
fn reason_has_ext1(_: &esp_hal::rtc_cntl::WakeupReason) -> bool {
    false
}

#[cfg(feature = "esp32s3")]
fn reason_has_ext0(reason: &esp_hal::rtc_cntl::WakeupReason) -> bool {
    reason.contains(WakeupSource::Ext0)
}

#[cfg(not(feature = "esp32s3"))]
fn reason_has_ext0(_: &esp_hal::rtc_cntl::WakeupReason) -> bool {
    false
}

impl WakeCauseSource for EspHalWakeCauseSource {
    fn last_wake_cause(&self) -> WakeCause {
        self.cause
    }
}
