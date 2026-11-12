//! The armed EXT1 record on the ESP32, for [`EspHalWakeCauseSource`](crate::EspHalWakeCauseSource).
//!
//! On the ESP32 a low-level (`AnyLow`) EXT1 wake can read an empty `ext_wakeup1_status` in the
//! app, so the fired pin is reported instead from the configuration armed before the sleep:
//! the same resolution the ESP-IDF tier runs, `stoker::ext1_fired_mask` (see
//! `docs/project-lore.md` → "ESP-IDF tier — ESP32 EXT1 wake status (IDF v6.1)").
//!
//! `.rtc_fast.persistent` is the only RTC memory that survives deep sleep on this tier:
//! `RtcSleepConfig::deep()` powers down the RTC slow memories, and `deep_sleep_config()`
//! (`sleep.rs`) keeps RTC fast memory powered. The section is `NOLOAD`, so the initialiser
//! never runs: the runtime zeroes it before `main` on a power-on reset, and every other warm
//! boot finds stale bytes until the check word validates them. Atomics give interior
//! mutability, so the compiler cannot assume their initial value; one writer before sleep,
//! one reader at boot, no read-modify-write.

use core::sync::atomic::{AtomicU32, Ordering};

/// Mixed into the check word, so zeroed or random memory does not validate.
const MAGIC: u32 = 0x4558_3141;

/// Mask low word, mask high word, low-level flag, check word.
#[unsafe(link_section = ".rtc_fast.persistent")]
static RECORD: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];

/// This boot's answer, in normal RAM (zero at boot): taken flag, mask low, mask high,
/// bit 1 = record valid, bit 0 = low level.
static TAKEN: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];

fn check(lo: u32, hi: u32, low: u32) -> u32 {
    MAGIC ^ lo ^ hi.rotate_left(8) ^ low.rotate_left(16)
}

/// Records the armed EXT1 mask and whether it wakes on a low level; the check word last.
pub(crate) fn store(mask: u64, low: bool) {
    // The mask is split into its low and high words on purpose.
    #[allow(clippy::cast_possible_truncation)]
    let (lo, hi) = (mask as u32, (mask >> 32) as u32);
    let low_flag = u32::from(low);
    RECORD[0].store(lo, Ordering::Relaxed);
    RECORD[1].store(hi, Ordering::Relaxed);
    RECORD[2].store(low_flag, Ordering::Relaxed);
    RECORD[3].store(check(lo, hi, low_flag), Ordering::Release);
}

/// Invalidates the record: all zero, and `check(0, 0, 0)` is `MAGIC`, not zero.
pub(crate) fn invalidate() {
    for word in &RECORD {
        word.store(0, Ordering::Relaxed);
    }
}

/// The armed mask and low-level flag, if the record validates.
fn load() -> Option<(u64, bool)> {
    let stored = RECORD[3].load(Ordering::Acquire);
    let lo = RECORD[0].load(Ordering::Relaxed);
    let hi = RECORD[1].load(Ordering::Relaxed);
    let low = RECORD[2].load(Ordering::Relaxed);
    (stored == check(lo, hi, low) && low <= 1)
        .then(|| (u64::from(lo) | (u64::from(hi) << 32), low == 1))
}

/// The record of the sleep that ended this boot: the first call loads it and invalidates it
/// in RTC memory, so a later sleep armed outside [`EspHalSleepManager`](crate::sleep::EspHalSleepManager)
/// cannot inherit it; later calls in the same boot return the same answer.
pub(crate) fn take() -> Option<(u64, bool)> {
    if TAKEN[0].load(Ordering::Acquire) == 0 {
        let flags = match load() {
            Some((mask, low)) => {
                #[allow(clippy::cast_possible_truncation)]
                let (lo, hi) = (mask as u32, (mask >> 32) as u32);
                TAKEN[1].store(lo, Ordering::Relaxed);
                TAKEN[2].store(hi, Ordering::Relaxed);
                0b10 | u32::from(low)
            }
            None => 0,
        };
        TAKEN[3].store(flags, Ordering::Relaxed);
        invalidate();
        TAKEN[0].store(1, Ordering::Release);
    }
    let flags = TAKEN[3].load(Ordering::Relaxed);
    (flags & 0b10 != 0).then(|| {
        let mask = u64::from(TAKEN[1].load(Ordering::Relaxed))
            | (u64::from(TAKEN[2].load(Ordering::Relaxed)) << 32);
        (mask, flags & 1 != 0)
    })
}
