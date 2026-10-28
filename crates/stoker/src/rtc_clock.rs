//! Pure core of an accumulating RTC clock for tiers without a clock service.
//!
//! The RTC slow-clock counter keeps running through deep sleep, but turning its
//! whole tick count into microseconds with the period calibrated by the
//! *current* boot lets the calibration error scale with uptime: each boot
//! re-converts every tick since power-up.
//! The ESP-IDF clock avoids that by keeping an accumulator in RTC memory and
//! converting only the ticks since the previous read.
//! [`Accum`] is that accumulator as plain integers: a platform tier reads the
//! 48-bit counter and the calibrated period, calls [`Accum::advance`], and keeps
//! the record across sleep in a [`Retained`](crate::Retained)`<20>` through the
//! [`Retainable`] impl.
//!
//! This is tier-support code, not a trait: it has no hardware access, no
//! dependency and no allocation, and the tier owns the register reads, the
//! critical section and the reset-reason gate.
//!
//! # Units
//!
//! A period is microseconds per tick in Q19 fixed point (19 fractional bits, the
//! format the ROM stores in `STORE1`).
//! The sub-microsecond remainder of every conversion is carried in
//! `frac_q19` instead of being truncated, so polling often does not lose about
//! one microsecond per call.
//!
//! ```
//! use stoker::rtc_clock::Accum;
//!
//! // 32 768 Hz is exactly 30.517578125 us per tick.
//! let period_q19 = 16_000_000;
//! let mut clock = Accum::seed(0);
//! clock.advance(32_768, period_q19);
//! assert_eq!(clock.us(), 1_000_000);
//! ```

use crate::retain::Retainable;

/// Fractional bits of a calibrated period (`RtcClock::CAL_FRACT` in esp-hal).
pub const CAL_FRACT: u32 = 19;

/// Mask of the 48 valid bits of the RTC timer counter.
pub const TICK_MASK: u64 = (1 << 48) - 1;

/// Smallest plausible calibrated period: 2 us per tick, in Q19.
pub const MIN_PERIOD_Q19: u32 = 2 << CAL_FRACT;

/// Largest plausible calibrated period: 64 us per tick, in Q19.
pub const MAX_PERIOD_Q19: u32 = 64 << CAL_FRACT;

const FRAC_MASK: u64 = (1 << CAL_FRACT) - 1;

/// Join the `time_high`/`time_low` register halves into a 48-bit tick count.
///
/// Bits above 47 of `high` are discarded.
#[must_use]
pub const fn compose_ticks(high: u32, low: u32) -> u64 {
    (((high as u64) << 32) | low as u64) & TICK_MASK
}

/// Whether `period_q19` can be a real calibration result.
///
/// An unset `STORE1` reads 0 and a failed calibration can yield nonsense; both
/// fall outside [`MIN_PERIOD_Q19`]..=[`MAX_PERIOD_Q19`].
#[must_use]
pub const fn period_is_plausible(period_q19: u32) -> bool {
    period_q19 >= MIN_PERIOD_Q19 && period_q19 <= MAX_PERIOD_Q19
}

/// Convert `ticks` to microseconds, returned as `(whole_us, frac_q19)`.
///
/// `ticks` is masked to 48 bits.
/// The product is split at the upper and lower 19 bits of the tick count, as
/// esp-hal does, so it cannot overflow `u64` for any `u32` period.
/// `frac_q19` is always below `1 << 19`.
#[must_use]
pub const fn ticks_to_q19(ticks: u64, period_q19: u32) -> (u64, u32) {
    let ticks = ticks & TICK_MASK;
    let period = period_q19 as u64;
    let upper = (ticks & !FRAC_MASK) >> CAL_FRACT;
    let lower = (ticks & FRAC_MASK) * period;
    // `upper < 2^29` and `period < 2^32`, so the product fits; `lower < 2^51`.
    let whole = upper * period + (lower >> CAL_FRACT);
    // Masked to 19 bits, so the narrowing is lossless.
    (whole, (lower & FRAC_MASK) as u32)
}

/// Outcome of [`Accum::advance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Advance {
    /// The delta ticks were converted and added.
    Advanced,
    /// The period was implausible; nothing changed, so the ticks are converted
    /// by a later call that has a valid period.
    NoPeriod,
    /// The counter is below the last seen value; nothing changed.
    /// Re-read once, then [`Accum::rebase`] if it is still lower.
    Backwards,
}

/// Accumulated microseconds and the tick count they were last brought up to.
///
/// `last_ticks` is at most [`TICK_MASK`] and `frac_q19` is below `1 << 19`;
/// every constructor and the [`Retainable`] decoder keep that invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accum {
    last_ticks: u64,
    acc_us: u64,
    frac_q19: u32,
}

impl Accum {
    /// A zero accumulator anchored at `now_ticks` (cold start).
    #[must_use]
    pub const fn seed(now_ticks: u64) -> Self {
        Self {
            last_ticks: now_ticks & TICK_MASK,
            acc_us: 0,
            frac_q19: 0,
        }
    }

    /// Microseconds accumulated so far, as of the last [`advance`](Self::advance).
    #[must_use]
    pub const fn us(&self) -> u64 {
        self.acc_us
    }

    /// The tick count the accumulator was last brought up to.
    #[must_use]
    pub const fn last_ticks(&self) -> u64 {
        self.last_ticks
    }

    /// Convert the ticks since the last call with `period_q19` and add them.
    ///
    /// An implausible period is reported before the counter is compared, so a
    /// bad calibration never discards ticks.
    #[must_use = "Backwards means the caller should re-read and rebase"]
    pub fn advance(&mut self, now_ticks: u64, period_q19: u32) -> Advance {
        if !period_is_plausible(period_q19) {
            return Advance::NoPeriod;
        }
        let now = now_ticks & TICK_MASK;
        if now < self.last_ticks {
            return Advance::Backwards;
        }
        let (whole, frac) = ticks_to_q19(now - self.last_ticks, period_q19);
        let frac = u64::from(self.frac_q19) + u64::from(frac);
        self.acc_us = self.acc_us.saturating_add(whole + (frac >> CAL_FRACT));
        // Masked to 19 bits, so the narrowing is lossless.
        self.frac_q19 = (frac & FRAC_MASK) as u32;
        self.last_ticks = now;
        Advance::Advanced
    }

    /// Re-anchor at `now_ticks`, keeping the accumulated time.
    ///
    /// For a counter that went backwards and stayed there; the time spent
    /// before the discontinuity is kept, the discontinuity itself adds nothing.
    pub fn rebase(&mut self, now_ticks: u64) {
        self.last_ticks = now_ticks & TICK_MASK;
    }

    /// Take over a record loaded from retention at tick count `now_ticks`.
    ///
    /// The loaded time is kept as is; a loaded anchor ahead of the counter
    /// (which cannot happen across a sleep) is rebased to `now_ticks` so the
    /// next [`advance`](Self::advance) cannot underflow.
    #[must_use]
    pub const fn adopt(loaded: Self, now_ticks: u64) -> Self {
        let now = now_ticks & TICK_MASK;
        if loaded.last_ticks > now {
            Self {
                last_ticks: now,
                ..loaded
            }
        } else {
            loaded
        }
    }
}

impl Retainable for Accum {
    const VERSION: u16 = 1;
    const SIZE: usize = 20;

    fn write(&self, buf: &mut [u8]) -> Option<()> {
        buf.get_mut(0..8)?
            .copy_from_slice(&self.last_ticks.to_le_bytes());
        buf.get_mut(8..16)?
            .copy_from_slice(&self.acc_us.to_le_bytes());
        buf.get_mut(16..20)?
            .copy_from_slice(&self.frac_q19.to_le_bytes());
        Some(())
    }

    fn read(buf: &[u8]) -> Option<Self> {
        let last_ticks = u64::from_le_bytes(buf.get(0..8)?.try_into().ok()?);
        let acc_us = u64::from_le_bytes(buf.get(8..16)?.try_into().ok()?);
        let frac_q19 = u32::from_le_bytes(buf.get(16..20)?.try_into().ok()?);
        if last_ticks > TICK_MASK || u64::from(frac_q19) > FRAC_MASK {
            return None;
        }
        Some(Self {
            last_ticks,
            acc_us,
            frac_q19,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reset::ResetReason;
    use crate::retain::{Invalid, Retained};

    const P32K: u32 = 16_000_000;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn ticks_to_q19_matches_u128_reference() {
        let ticks = [
            0,
            1,
            FRAC_MASK,
            FRAC_MASK + 1,
            (1 << 32) - 1,
            1 << 32,
            TICK_MASK - 1,
            TICK_MASK,
            u64::MAX,
        ];
        let periods = [
            0,
            1,
            MIN_PERIOD_Q19,
            P32K,
            3_495_253,
            MAX_PERIOD_Q19,
            u32::MAX,
        ];
        for &t in &ticks {
            for &p in &periods {
                let full = u128::from(t & TICK_MASK) * u128::from(p);
                let (whole, frac) = ticks_to_q19(t, p);
                assert_eq!(u128::from(whole), full >> CAL_FRACT, "t={t} p={p}");
                assert_eq!(u64::from(frac), (full & u128::from(FRAC_MASK)) as u64);
            }
        }
    }

    #[test]
    fn ticks_to_q19_matches_reference_on_random_inputs() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        for _ in 0..10_000 {
            let t = rng.next();
            let p = rng.next() as u32;
            let full = u128::from(t & TICK_MASK) * u128::from(p);
            let (whole, frac) = ticks_to_q19(t, p);
            assert_eq!(u128::from(whole), full >> CAL_FRACT);
            assert_eq!(u128::from(frac), full & u128::from(FRAC_MASK));
        }
    }

    #[test]
    fn known_vector_32768_hz() {
        assert_eq!(ticks_to_q19(32_768, P32K), (1_000_000, 0));
        assert_eq!(ticks_to_q19(1, P32K), (30, 271_360));
    }

    #[test]
    fn known_vector_150_khz() {
        assert_eq!(ticks_to_q19(150_000, 3_495_253), (999_999, 474_288));
    }

    #[test]
    fn chunked_advance_equals_one_big_advance() {
        let period = 3_495_253;
        let mut chunked = Accum::seed(0);
        for tick in 1..=1_000_000u64 {
            assert_eq!(chunked.advance(tick, period), Advance::Advanced);
        }
        let mut whole = Accum::seed(0);
        assert_eq!(whole.advance(1_000_000, period), Advance::Advanced);
        assert_eq!(chunked, whole);
    }

    #[test]
    fn no_period_holds_ticks_until_a_valid_one() {
        let mut clock = Accum::seed(100);
        for bad in [0, MIN_PERIOD_Q19 - 1, MAX_PERIOD_Q19 + 1, u32::MAX] {
            assert_eq!(clock.advance(32_868, bad), Advance::NoPeriod);
            assert_eq!(clock, Accum::seed(100));
        }
        assert_eq!(clock.advance(32_868, P32K), Advance::Advanced);
        assert_eq!(clock.us(), 1_000_000);
    }

    #[test]
    fn plausible_period_bounds_are_inclusive() {
        assert!(period_is_plausible(MIN_PERIOD_Q19));
        assert!(period_is_plausible(MAX_PERIOD_Q19));
        assert!(!period_is_plausible(MIN_PERIOD_Q19 - 1));
        assert!(!period_is_plausible(MAX_PERIOD_Q19 + 1));
    }

    #[test]
    fn backwards_changes_nothing_and_rebase_keeps_time() {
        let mut clock = Accum::seed(0);
        assert_eq!(clock.advance(65_536, P32K), Advance::Advanced);
        let before = clock;
        assert_eq!(clock.advance(65_535, P32K), Advance::Backwards);
        assert_eq!(clock, before);
        clock.rebase(1_000);
        assert_eq!(clock.us(), before.us());
        assert_eq!(clock.last_ticks(), 1_000);
        assert_eq!(clock.advance(1_000 + 32_768, P32K), Advance::Advanced);
        assert_eq!(clock.us(), before.us() + 1_000_000);
    }

    #[test]
    fn adopt_keeps_values_and_never_leaves_the_anchor_ahead() {
        let mut loaded = Accum::seed(0);
        assert_eq!(loaded.advance(5_000, P32K), Advance::Advanced);
        assert_eq!(Accum::adopt(loaded, 9_000), loaded);
        let rebased = Accum::adopt(loaded, 100);
        assert_eq!(rebased.us(), loaded.us());
        assert_eq!(rebased.last_ticks(), 100);
        let mut rebased = rebased;
        assert_eq!(rebased.advance(100, P32K), Advance::Advanced);
        assert_eq!(rebased.us(), loaded.us());
    }

    #[test]
    fn compose_ticks_masks_to_48_bits() {
        assert_eq!(compose_ticks(0, 5), 5);
        assert_eq!(compose_ticks(1, 0), 1 << 32);
        assert_eq!(compose_ticks(0xFFFF, u32::MAX), TICK_MASK);
        assert_eq!(compose_ticks(u32::MAX, u32::MAX), TICK_MASK);
        assert_eq!(compose_ticks(0x1_0000, 7), 7);
    }

    #[test]
    fn seed_and_advance_mask_the_counter() {
        assert_eq!(Accum::seed(u64::MAX).last_ticks(), TICK_MASK);
        let mut clock = Accum::seed(0);
        assert_eq!(clock.advance((1 << 48) + 32_768, P32K), Advance::Advanced);
        assert_eq!(clock.us(), 1_000_000);
    }

    #[test]
    fn retainable_round_trip() {
        let mut clock = Accum::seed(7);
        assert_eq!(clock.advance(123_456_789, 3_495_253), Advance::Advanced);
        let mut buf = [0u8; Accum::SIZE];
        clock.write(&mut buf).unwrap();
        assert_eq!(Accum::read(&buf), Some(clock));
        assert_eq!(&buf[0..8], &123_456_789u64.to_le_bytes());
    }

    #[test]
    fn retainable_rejects_bad_frac_or_ticks() {
        let mut buf = [0u8; Accum::SIZE];
        Accum::seed(5).write(&mut buf).unwrap();
        let mut bad_frac = buf;
        bad_frac[16..20].copy_from_slice(&(1u32 << CAL_FRACT).to_le_bytes());
        assert_eq!(Accum::read(&bad_frac), None);
        let mut bad_ticks = buf;
        bad_ticks[0..8].copy_from_slice(&(TICK_MASK + 1).to_le_bytes());
        assert_eq!(Accum::read(&bad_ticks), None);
        assert_eq!(Accum::read(&buf[..19]), None);
    }

    #[test]
    fn retained_gates_on_reset_reason() {
        let mut record = Retained::<20>::empty();
        let mut clock = Accum::seed(0);
        assert_eq!(clock.advance(32_768, P32K), Advance::Advanced);
        record.store(&clock).unwrap();
        assert_eq!(record.load::<Accum>(ResetReason::DeepSleep), Ok(clock));
        assert_eq!(record.load::<Accum>(ResetReason::Software), Ok(clock));
        assert_eq!(
            record.load::<Accum>(ResetReason::PowerOn),
            Err(Invalid::PowerOn)
        );
        assert_eq!(
            record.load::<Accum>(ResetReason::Brownout),
            Err(Invalid::Brownout)
        );
    }

    /// Simulated true rate 32 768 Hz plus 3 %; each boot calibrates the period
    /// with up to +-8 % error.
    fn scattered_period(rng: &mut Rng, true_hz: f64) -> u32 {
        let scatter = (rng.next() % 1601) as f64 / 10_000.0 - 0.08;
        let period_us = 1e6 / true_hz * (1.0 + scatter);
        (period_us * f64::from(1u32 << CAL_FRACT)) as u32
    }

    #[test]
    fn sleeps_after_an_hour_stay_within_the_single_boot_error() {
        let true_hz = 32_768.0 * 1.03;
        let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
        let mut clock = Accum::seed(0);
        let mut ticks = (3_600.0 * true_hz) as u64;
        assert_eq!(
            clock.advance(ticks, scattered_period(&mut rng, true_hz)),
            Advance::Advanced
        );
        for _ in 0..10 {
            let before = clock.us();
            ticks += (60.0 * true_hz) as u64;
            let period = scattered_period(&mut rng, true_hz);
            assert_eq!(clock.advance(ticks, period), Advance::Advanced);
            let slept = (clock.us() - before) as f64 / 1e6;
            assert!((55.2..=64.8).contains(&slept), "slept {slept}");
        }
    }

    #[test]
    fn half_second_sleep_reads_close_to_half_a_second() {
        let true_hz = 32_768.0 * 0.97;
        let mut rng = Rng(0x0BAD_C0DE_1234_5678);
        let mut clock = Accum::seed(0);
        let mut ticks = (3_600.0 * true_hz) as u64;
        assert_eq!(
            clock.advance(ticks, scattered_period(&mut rng, true_hz)),
            Advance::Advanced
        );
        for _ in 0..100 {
            let before = clock.us();
            ticks += (0.5 * true_hz) as u64;
            let period = scattered_period(&mut rng, true_hz);
            assert_eq!(clock.advance(ticks, period), Advance::Advanced);
            let slept = (clock.us() - before) as f64 / 1e6;
            assert!((0.4..=0.7).contains(&slept), "slept {slept}");
        }
    }

    #[test]
    fn random_non_decreasing_sequences_are_monotonic() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..100 {
            let mut clock = Accum::seed(0);
            let mut ticks = 0u64;
            let mut last = 0u64;
            for _ in 0..200 {
                ticks = (ticks + rng.next() % 5_000_000) & TICK_MASK;
                let period = match rng.next() % 4 {
                    0 => 0,
                    _ => {
                        MIN_PERIOD_Q19
                            + (rng.next() % u64::from(MAX_PERIOD_Q19 - MIN_PERIOD_Q19)) as u32
                    }
                };
                let _ = clock.advance(ticks, period);
                assert!(clock.us() >= last);
                last = clock.us();
            }
        }
    }
}
