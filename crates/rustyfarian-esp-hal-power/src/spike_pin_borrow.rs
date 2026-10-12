//! ADR-001 compile-only spike: borrow `&mut Input` wake pins at the `try_sleep` call.
//!
//! This module is **not** the sleep driver. It exists to prove, against esp-hal `=1.2.2`, that
//! the ownership model decided in `docs/adr/001-hal-neutral-sleep-contract.md` type-checks:
//!
//! - the manager owns only the `LowPower` driver (the `LPWR` peripheral);
//! - the firmware keeps owning its wake pins and polls them between sleeps;
//! - the pins are borrowed as `&mut Input` only for the duration of one sleep attempt;
//! - `WakeCause::Gpio(mask)` can be rebuilt from `caused_wakeup()` on the same pins at boot.
//!
//! It compiles under `just check-hal-c3` and is deleted when the real sleep module lands
//! (`docs/features/esp-hal-sleep-tier-v1.md`). Nothing here is exported.
//!
//! Error variants that the ADR adds to `stoker::SleepError` do not exist yet, so the spike
//! carries a local error enum with the same shape.
//!
//! Spike log: the first version tied the pins' `Input<'d>` lifetime to the manager's `'d`
//! (`&'a mut [&'p mut Input<'d>]`). `&mut Input<'d>` is invariant in `'d`, so a manager and a
//! pin with independent lifetimes failed with "lifetime may not live long enough" at the
//! consumer's call site. The pin lifetime must be its own parameter (`'i` below).
//!
//! Review round 1 (2026-10-12): validation now builds a `Plan` without touching hardware, and
//! arming, the sleep call and the post-rejection cleanup run inside one `critical_section`,
//! with the deadline armed last and re-checked against `POST_ARM_MARGIN_US`.
//!
//! Review round 2 (2026-10-12): timers above `MAX_TIMER_MS` are rejected (the RTC comparator is
//! 48 bits wide and esp-hal truncates the rest); inside the critical section the manager first
//! refuses foreign low-power wake pins, then waits for every UART transmitter to drain, and only
//! then arms pins and the deadline, so esp-hal's own FIFO drain at sleep entry finds nothing to do.

#![allow(dead_code)]

use esp_hal::{
    gpio::{Event, Input, WakeupConfig},
    peripherals::{GPIO, LPWR, UART0, UART1},
    rtc_cntl::{
        sleep::{LowPower, RtcSleepConfig},
        wakeup_cause, WakeupSource,
    },
    time::{Duration, Instant},
};
use stoker::{
    GpioWakeLevel, GpioWakeMask, SleepError, SleepManager, WakeCause, WakeCauseSource, WakeSource,
};

/// ESP32-C3 pads with a low-power wake path (GPIO0-5).
///
/// Same value as ESP-IDF's `SOC_GPIO_DEEP_SLEEP_WAKE_VALID_GPIO_MASK` and as the pads that
/// esp-hal 1.2.2 accepts for `WakeupConfig::low_power_path` (`gpio/wakeup/per_pin.rs`).
const C3_DEEP_SLEEP_WAKE_PINS: u64 = 0x3F;

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

/// Longest timer the manager accepts: `u32::MAX` ms, about 49.7 days.
///
/// esp-hal writes the deadline into a 48-bit RTC comparator and drops the higher bits
/// (`sleep/timer.rs` `arm`). With a slow clock of at most 150 kHz, 2^48 ticks are about 59
/// years; this cap keeps more than 400x headroom including RTC uptime, and it is the same on
/// the IDF tier (`stoker` owns the constant in 0.3.0).
const MAX_TIMER_MS: u64 = u32::MAX as u64;

/// Longest the manager waits for the UART transmitters to drain before arming, in µs.
///
/// A full 128-byte C3 TX FIFO takes about 133 ms at 9600 baud. A transmitter that is still busy
/// after this (CTS flow control held off, DMA feeding the FIFO) is a `UartBusy` error instead of
/// the endless spin esp-hal's own drain would give inside sleep entry.
const UART_DRAIN_TIMEOUT_US: u64 = 200_000;

/// Spike-local stand-in for the `SleepError` variants the ADR adds in 0.3.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpikeError {
    /// A `stoker` validator rejected the sources (`InvalidConfig`).
    Config(SleepError),
    /// `sources` is empty; esp-hal would panic, ESP-IDF would sleep forever (`InvalidConfig`).
    NoWakeSource,
    /// A mask bit names a pin that is not in the borrowed slice (`InvalidConfig`).
    PinNotBound { pin: u8 },
    /// A `Timer` shorter than the floor, or one whose deadline came too near during arming
    /// (`InvalidConfig`).
    TimerTooShort { duration_ms: u64, min_ms: u64 },
    /// A `Timer` longer than `MAX_TIMER_MS`, which the 48-bit comparator cannot hold with
    /// margin (`InvalidConfig`).
    TimerTooLong { duration_ms: u64, max_ms: u64 },
    /// A low-power wake pad outside the binding is listening; it would wake or reject the
    /// sleep and stay armed after cleanup (`InvalidConfig`).
    ForeignWakePin { pin: u8 },
    /// A UART transmitter did not drain within `UART_DRAIN_TIMEOUT_US` (`Platform`).
    UartBusy { uart: u8 },
    /// esp-hal refused the low-power path for a pad the mask check let through (`Platform`).
    NoLowPowerPath { pin: u8 },
    /// The hardware rejected the sleep because a wake source was already asserted (`Rejected`).
    Rejected,
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

/// Owns the `LowPower` driver only. No pin lives here.
struct EspHalSleepManager<'d> {
    low_power: LowPower<'d>,
}

impl<'d> EspHalSleepManager<'d> {
    fn new(lpwr: LPWR<'d>) -> Self {
        Self {
            low_power: LowPower::new(lpwr),
        }
    }

    /// Borrows the wake pins for one sleep attempt.
    ///
    /// The borrow ends when the returned value is dropped, so the firmware can poll the same
    /// `Input` again after a rejected `try_sleep`. The pins' own lifetime `'i` is independent
    /// of the manager's `'d`.
    fn bind_wake_pins<'a, 'p, 'i>(
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
struct BoundSleep<'a, 'p, 'i, 'd> {
    manager: &'a mut EspHalSleepManager<'d>,
    pins: &'a mut [&'p mut Input<'i>],
}

impl BoundSleep<'_, '_, '_, '_> {
    /// Deep sleep that returns only when the hardware rejects it (a wake pin already at its
    /// wake level). Mirrors `esp_deep_sleep_try_to_start` on the IDF tier.
    ///
    /// Validation runs first and touches nothing. Arming, the sleep call and the cleanup after
    /// a rejection then run in one critical section, so no GPIO handler can observe the armed,
    /// already-asserted pin: a custom handler that calls `unlisten()` would otherwise drop the
    /// pin from the wake set (`gpio/low_level/mod.rs` `set_int_enable` clears `listening`).
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), SpikeError> {
        let plan = self.plan(sources)?;
        critical_section::with(|_| {
            self.arm(plan)?;
            self.manager
                .low_power
                .sleep_deep_with_rejection(RtcSleepConfig::deep());
            // Reaching this line *is* the rejection report (esp-hal 1.2.2 `sleep/mod.rs:150`).
            // Release before interrupts come back, so the held level cannot fire a handler.
            self.disarm();
            Err(SpikeError::Rejected)
        })
    }

    /// Validates the whole request without touching hardware.
    fn plan(&self, sources: &[WakeSource]) -> Result<Plan, SpikeError> {
        if sources.is_empty() {
            return Err(SpikeError::NoWakeSource);
        }
        stoker::sleep::validate_wake_sources(sources).map_err(SpikeError::Config)?;

        let mut plan = Plan {
            pin_mask: 0,
            event: Event::HighLevel,
            timer: None,
        };
        for source in sources {
            match *source {
                WakeSource::GpioLevel { pin_mask, level } => {
                    self.check_pins(pin_mask)?;
                    plan.pin_mask = pin_mask;
                    plan.event = match level {
                        GpioWakeLevel::AnyHigh => Event::HighLevel,
                        GpioWakeLevel::AnyLow => Event::LowLevel,
                    };
                }
                WakeSource::Timer { duration_ms } => {
                    if duration_ms < MIN_TIMER_MS {
                        return Err(SpikeError::TimerTooShort {
                            duration_ms,
                            min_ms: MIN_TIMER_MS,
                        });
                    }
                    if duration_ms > MAX_TIMER_MS {
                        return Err(SpikeError::TimerTooLong {
                            duration_ms,
                            max_ms: MAX_TIMER_MS,
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

    fn check_pins(&self, pin_mask: u64) -> Result<(), SpikeError> {
        if pin_mask == 0 {
            return Err(SpikeError::Config(SleepError::GpioMaskZero));
        }
        if pin_mask & !C3_DEEP_SLEEP_WAKE_PINS != 0 {
            return Err(SpikeError::Config(SleepError::GpioMaskOutOfRange {
                pin_mask,
                valid_mask: C3_DEEP_SLEEP_WAKE_PINS,
            }));
        }
        let unbound = pin_mask & !self.bound_mask();
        if unbound != 0 {
            return Err(SpikeError::PinNotBound {
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
    fn deadline_us(ahead_us: u64, duration_ms: u64) -> Result<u64, SpikeError> {
        Instant::now()
            .duration_since_epoch()
            .as_micros()
            .checked_add(ahead_us)
            .ok_or(SpikeError::TimerTooLong {
                duration_ms,
                max_ms: MAX_TIMER_MS,
            })
    }

    /// Refuses a listening low-power pad that is not bound. Reads registers only.
    ///
    /// esp-hal arms every pad that listens and carries the low-power flag, globally
    /// (`gpio/wakeup/mod.rs` `collect`). The flag itself is a private static, so the check is
    /// conservative: any C3 low-power pad (GPIO0-5) outside the binding whose `int_type` is
    /// set counts, the same field esp-hal reads for the wake level (`armed_level`).
    fn check_foreign_wake_pins(&self) -> Result<(), SpikeError> {
        let bound = self.bound_mask();
        for pin in 0..6u8 {
            if bound & (1u64 << pin) != 0 {
                continue;
            }
            if GPIO::regs().pin(pin as usize).read().int_type().bits() != 0 {
                return Err(SpikeError::ForeignWakePin { pin });
            }
        }
        Ok(())
    }

    /// Waits until every UART transmitter is empty and idle, the same condition esp-hal's
    /// `wait_for_suspended` spins on at sleep entry (`uart/low_level/v1.rs`), bounded by
    /// `UART_DRAIN_TIMEOUT_US`. Call with interrupts masked, so no handler refills a FIFO.
    fn drain_uarts() -> Result<(), SpikeError> {
        let start = Instant::now();
        let timeout = Duration::from_micros(UART_DRAIN_TIMEOUT_US);
        loop {
            let busy0 = UART0::regs().status().read().txfifo_cnt().bits() != 0
                || UART0::regs().fsm_status().read().st_utx_out().bits() != 0;
            let busy1 = UART1::regs().status().read().txfifo_cnt().bits() != 0
                || UART1::regs().fsm_status().read().st_utx_out().bits() != 0;
            if !busy0 && !busy1 {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(SpikeError::UartBusy {
                    uart: if busy0 { 0 } else { 1 },
                });
            }
        }
    }

    /// Arms a validated plan. Call with interrupts masked.
    ///
    /// Order: foreign-pin check and UART drain (both read-only, an error leaves everything
    /// untouched), then pins, then the deadline, then the post-arm margin check. Any failure
    /// after the first mutation releases everything it armed.
    fn arm(&mut self, plan: Plan) -> Result<(), SpikeError> {
        self.check_foreign_wake_pins()?;
        Self::drain_uarts()?;

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
            return Err(SpikeError::TimerTooShort {
                duration_ms: timer.duration_ms,
                min_ms: MIN_TIMER_MS,
            });
        }
        Ok(())
    }

    /// Arms exactly the mask pins and releases every other bound pin. Only the intersection of
    /// bound pins and mask bits may listen: esp-hal arms every listening pin that carries the
    /// low-power flag (`gpio/wakeup/mod.rs` `collect`).
    fn arm_pins(&mut self, pin_mask: u64, event: Event) -> Result<(), SpikeError> {
        let low_power_path = WakeupConfig::default().with_low_power_path(true);
        for pin in self.pins.iter_mut() {
            let Some(n) = pin.peripheral_input().gpio_number() else {
                continue;
            };
            if pin_mask & (1u64 << n) != 0 {
                pin.apply_wakeup_config(&low_power_path)
                    .map_err(|_| SpikeError::NoLowPowerPath { pin: n })?;
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
    type Error = SpikeError;

    /// Non-rejecting deep sleep; returns only on a configuration error. esp-hal's
    /// `deadline_missed` assert stays as the last line of defence behind the post-arm guard.
    fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), SpikeError> {
        let plan = self.plan(sources)?;
        critical_section::with(|_| {
            self.arm(plan)?;
            self.manager.low_power.sleep_deep(RtcSleepConfig::deep())
        })
    }
}

/// Wake cause captured once at boot from the pins the firmware re-creates.
///
/// esp-hal has no wake-pin status bitmask; `caused_wakeup()` is per pin, so the mask is
/// rebuilt from the pins the firmware hands in. Capture it **before** the first `try_sleep`:
/// a rejected sleep runs `record_wakeup()` again and overwrites the per-pin record.
struct EspHalWakeCauseSource {
    cause: WakeCause,
}

impl EspHalWakeCauseSource {
    fn capture(pins: &[&Input<'_>]) -> Self {
        let reason = wakeup_cause();
        let cause = if reason.is_empty() {
            WakeCause::PowerOn
        } else if reason.contains(WakeupSource::Gpio) {
            let mask = pins
                .iter()
                .filter(|pin| pin.caused_wakeup())
                .filter_map(|pin| pin.peripheral_input().gpio_number())
                .fold(0u64, |mask, n| mask | (1u64 << n));
            // `WakeCause::Gpio` is still a unit variant; the C3 wake feature turns it into
            // `Gpio(GpioWakeMask)`. The spike proves the mask can be built.
            let _ = GpioWakeMask(mask);
            WakeCause::Gpio
        } else if reason.contains(WakeupSource::Timer) {
            WakeCause::Timer
        } else {
            WakeCause::Other
        };
        Self { cause }
    }
}

impl WakeCauseSource for EspHalWakeCauseSource {
    fn last_wake_cause(&self) -> WakeCause {
        self.cause
    }
}

const PIR_SOURCES: [WakeSource; 2] = [
    WakeSource::GpioLevel {
        pin_mask: 1 << 4,
        level: GpioWakeLevel::AnyHigh,
    },
    WakeSource::Timer {
        duration_ms: 60_000,
    },
];

/// The consumer's loop shape: read the wake cause once, poll the wake pin, sleep on it, poll
/// again after a rejection. `manager` and `pir` have independent lifetimes on purpose. This
/// function only has to type-check.
fn consumer_shape(mut manager: EspHalSleepManager<'_>, mut pir: Input<'_>) -> ! {
    let wake = EspHalWakeCauseSource::capture(&[&pir]);
    let _cause = wake.last_wake_cause();

    loop {
        let _presence = pir.is_high();

        // Form 1: the binding is a temporary of one statement.
        let outcome = manager
            .bind_wake_pins(&mut [&mut pir])
            .try_sleep(&PIR_SOURCES);

        // The borrow ended with the temporary; `pir` is pollable again.
        if let Err(SpikeError::Rejected) = outcome {
            let _still_high = pir.is_high();
        }

        // Form 2: the binding lives in a local, the pin array in another; both end with
        // the block.
        {
            let mut pins = [&mut pir];
            let mut bound = manager.bind_wake_pins(&mut pins);
            let _ = bound.try_sleep(&PIR_SOURCES);
        }
        let _again = pir.is_high();
    }
}
