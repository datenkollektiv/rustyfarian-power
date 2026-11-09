//! Wake-cause reporting on esp-hal, for every chip.
//!
//! [`EspHalWakeCauseSource`] implements [`WakeCauseSource`] from [`stoker`].
//! The cause is normalised per chip, not per tier: the same chip reports the same [`WakeCause`] variant on the ESP-IDF tier.
//!
//! | Chip     | Pin wake                     | Wake cause            |
//! |:---------|:-----------------------------|:----------------------|
//! | ESP32-C3 | per-pin deep-sleep GPIO wake | `Gpio(mask)`          |
//! | ESP32-C6 | EXT1                         | `Ext1(mask)`          |
//! | ESP32    | EXT0 or EXT1                 | `Ext0` / `Ext1(mask)` |
//! | ESP32-S3 | EXT0 or EXT1                 | `Ext0` / `Ext1(mask)` |
//!
//! The ESP32 `Gpio` source wakes from light sleep only and carries no status mask, so deep sleep never reports it there.

use esp_hal::{
    gpio::Input,
    rtc_cntl::{wakeup_cause, WakeupReason, WakeupSource},
};
use stoker::{GpioWakeMask, WakeCause, WakeCauseSource};

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
    /// Pin sources win over the timer when both fired.
    /// An EXT1 wake (ESP32, ESP32-S3, ESP32-C6) reports `Ext1(mask)`, an EXT0 wake (ESP32, ESP32-S3) reports `Ext0`, and a C3 GPIO wake reports `Gpio(mask)`.
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
        } else if reason_has_ext1(&reason) {
            WakeCause::Ext1(ext1_fired_from_status(fired_mask().0))
        } else if reason_has_ext0(&reason) {
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

// `WakeupSource::Ext1` does not exist on the C3, `Ext0` on the C3 and C6; these helpers keep those arms compiling.
#[cfg(any(feature = "esp32", feature = "esp32s3", feature = "esp32c6"))]
fn reason_has_ext1(reason: &WakeupReason) -> bool {
    reason.contains(WakeupSource::Ext1)
}

// The EXT1 status to report, with the armed `AnyLow` record as the ESP32 fallback.
//
// On the ESP32 a low-level (`AnyLow`) EXT1 wake can read an empty status in the app; the
// record of the armed configuration then names the single low pin through the shared
// `stoker::ext1_fired_mask`, the same flow the ESP-IDF tier runs (`armed_ext1`; see
// `docs/project-lore.md` → "ESP-IDF tier — ESP32 EXT1 wake status").
// Other chips report the status as read.
#[cfg(feature = "esp32")]
fn ext1_fired_from_status(status: u64) -> GpioWakeMask {
    let (armed_mask, armed_low) = crate::armed_ext1::take().unwrap_or((0, false));
    stoker::ext1_fired_mask(status, armed_mask, armed_low)
}

#[cfg(not(feature = "esp32"))]
fn ext1_fired_from_status(status: u64) -> GpioWakeMask {
    GpioWakeMask(status)
}

#[cfg(feature = "esp32c3")]
fn reason_has_ext1(_: &WakeupReason) -> bool {
    false
}

#[cfg(any(feature = "esp32", feature = "esp32s3"))]
fn reason_has_ext0(reason: &WakeupReason) -> bool {
    reason.contains(WakeupSource::Ext0)
}

#[cfg(any(feature = "esp32c3", feature = "esp32c6"))]
fn reason_has_ext0(_: &WakeupReason) -> bool {
    false
}

impl WakeCauseSource for EspHalWakeCauseSource {
    fn last_wake_cause(&self) -> WakeCause {
        self.cause
    }
}
