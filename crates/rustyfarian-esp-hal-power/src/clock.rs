//! Drift-free microsecond clock across resets and deep sleep (esp-hal).
//!
//! [`EspHalRtcClock::now_us`] is the esp-hal twin of the IDF tier's `EspRtcClock`.
//! `Rtc::time_since_power_up()` converts the whole 48-bit tick count with the period calibrated on the current boot, so differences of two readings drift with uptime (-8.5 % to +6.1 % for a 60 s sleep after 1 h).
//! This clock instead keeps an accumulator in RTC fast memory and converts only the ticks since the previous call, as ESP-IDF does.
//! The arithmetic is [`stoker::rtc_clock`]; this module only reads the hardware and stores the result.
//!
//! # Usage
//!
//! ```rust,ignore
//! let peripherals = esp_hal::init(esp_hal::Config::default());
//! let t0 = EspHalRtcClock::now_us();
//! ```
//!
//! Call it after `esp_hal::init`: the calibrated slow-clock period in `STORE1` is valid only once the clock has been initialised.
//! Before that, or with an implausible period, the clock does not advance and returns the last value; the ticks are converted once a valid period appears.
//!
//! # What survives
//!
//! - Deep sleep, software reset, watchdog reset: the accumulator is kept and the clock continues.
//! - Power-on and brownout: the record is refused (see [`RtcRetainedCell::load`]) and the clock restarts at the first call of that boot.
//! - A record that fails its checksum (garbage after flashing, a torn write) also restarts the clock.
//!
//! The reset reason is read once, on the first call of each boot, through [`EspHalResetReasonSource`].
//! The ROM latches the cause, so a late first call still sees it.
//!
//! # Accuracy
//!
//! The error is proportional to the interval since the previous call, not to uptime.
//! The sub-microsecond remainder is carried, so polling often does not lose time.
//! A reading takes about 20-40 us with interrupts masked (ten `time_update` latches of 1 us each, plus the store).
//!
//! A counter that reads lower than the last one is read once more; if it is still lower, the anchor is rebased and the accumulated time is kept.
//!
//! # Scope and caveats
//!
//! - ESP32-C3 and ESP32-S3 only.
//! - Calling `Rtc::time_since_power_up()` as well is allowed but its values keep the drift.
//!   On the S3, call it from one core only: both use the `time_update` latch.
//! - The register paths (`RTC_TIMER` latch, `LPWR` `STORE1`) are esp-hal 1.2.2 internals, not public API.
//! - The record is a private `#[ram(unstable(rtc_fast, persistent))]` static, so the firmware must not use the same section for anything that conflicts.

use core::cell::Cell;

use critical_section::Mutex;
use esp_hal::delay::Delay;
use esp_hal::peripherals::{LPWR, RTC_TIMER};
use stoker::retain::Retainable;
use stoker::rtc_clock::{Accum, Advance};
use stoker::ResetReasonSource;

use crate::reset::EspHalResetReasonSource;
use crate::retain::RtcRetainedCell;

/// Persistent accumulator record, sized to `Accum::SIZE` so `store` cannot fail on length.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static RETAINED: RtcRetainedCell<{ <Accum as Retainable>::SIZE }> = RtcRetainedCell::new();

/// This boot's working copy, `None` until the first call has loaded or seeded it.
///
/// The cell cannot serve later calls itself: `load` refuses the record after a power-on reset for the whole boot.
static STATE: Mutex<Cell<Option<Accum>>> = Mutex::new(Cell::new(None));

/// Snapshot of the 48-bit `RTC_TIMER` counter, latched the way `Rtc::time_since_boot_raw` does it.
fn read_ticks() -> u64 {
    let rtc = RTC_TIMER::regs();
    let delay = Delay::new();
    for _ in 0..10 {
        rtc.time_update().write(|w| w.time_update().set_bit());
        delay.delay_micros(1);
    }
    let high = rtc.time_high0().read().timer_value0_high().bits();
    let low = rtc.time_low0().read().timer_value0_low().bits();
    stoker::rtc_clock::compose_ticks(u32::from(high), low)
}

/// Calibrated slow-clock period in microseconds with 19 fractional bits (`STORE1`); 0 before calibration.
fn read_period_q19() -> u32 {
    LPWR::regs().store1().read().bits()
}

/// Microseconds counted from the first call after a power-on or brownout, accumulated across resets and deep sleep.
///
/// See the [module docs](self) for what survives which reset and the accuracy.
pub struct EspHalRtcClock;

impl EspHalRtcClock {
    /// Microseconds since the first call after the last power-on or brownout, so that first read returns 0; only differences between readings are meaningful.
    ///
    /// Never decreases.
    /// Returns the previous value unchanged while the calibration period is not yet valid.
    /// On the S3, do not mix it with `Rtc::time_since_power_up()` on the other core: both use the `time_update` latch.
    pub fn now_us() -> u64 {
        // One section on purpose: the tick read, `advance` and `store` must not interleave with
        // another caller, or a later snapshot is applied before an earlier one (spurious
        // `Backwards`) and the latch is shared mid-read. The first-call load runs once per boot.
        critical_section::with(|cs| {
            let cell = STATE.borrow(cs);
            let mut now = read_ticks();
            let mut accum = match cell.get() {
                Some(accum) => accum,
                None => {
                    let reset = EspHalResetReasonSource.last_reset_reason();
                    match RETAINED.load::<Accum>(reset) {
                        Ok(loaded) => Accum::adopt(loaded, now),
                        Err(_) => Accum::seed(now),
                    }
                }
            };

            let period = read_period_q19();
            if matches!(accum.advance(now, period), Advance::Backwards) {
                now = read_ticks();
                if matches!(accum.advance(now, period), Advance::Backwards) {
                    accum.rebase(now);
                }
            }

            cell.set(Some(accum));
            // Cannot fail: the cell is sized to `Accum::SIZE` and `Accum::write` fills exactly that.
            let _ = RETAINED.store(&accum);
            accum.us()
        })
    }
}
