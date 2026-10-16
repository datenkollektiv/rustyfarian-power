//! RTC fast memory retention on esp-hal.
//!
//! [`RtcRetainedCell`] wraps [`stoker::RetainedCell`] so it can sit under esp-hal's
//! `#[ram(unstable(rtc_fast, persistent))]` attribute, which only accepts types that
//! implement `esp_hal::Persistable`.
//! The wrapper adds nothing at runtime: it is `#[repr(transparent)]` and forwards every method.
//! The record format, the reset-reason gate and the volatile access pattern are those of
//! [`stoker::Retained`] and [`stoker::RetainedCell`].
//!
//! # Placement
//!
//! ```rust,ignore
//! use rustyfarian_esp_hal_power::RtcRetainedCell;
//!
//! #[esp_hal::ram(unstable(rtc_fast, persistent))]
//! static RETAINED: RtcRetainedCell<16> = RtcRetainedCell::new();
//! ```
//!
//! The macro locates esp-hal by crate name, so the consuming crate must depend on `esp-hal` directly.
//! It puts the static into `.rtc_fast.persistent`, a `NOLOAD` section, so the initialiser never runs and the bytes survive resets and deep sleep.
//!
//! The runtime zeroes `.rtc_fast.persistent` before `main` on a power-on reset, so a cold boot reads an empty record.
//! Pair every `load` with the reset reason of the current boot from a [`ResetReasonSource`](stoker::ResetReasonSource); on the C3 and S3 that is `EspHalResetReasonSource`.
//! `load` then refuses the record after a power-on or a brownout without touching the memory.
//!
//! # Concurrency
//!
//! `load`, `store` and `invalidate` run inside a critical section, so callers from several tasks or an interrupt are serialised.
//! esp-hal's `rt` feature supplies the `critical-section` implementation; see [`stoker::RetainedCell`].

pub use stoker::retain::*;

use stoker::ResetReason;

/// A [`RetainedCell`] that esp-hal accepts in `.rtc_fast.persistent`.
///
/// See the [module docs](self) for the placement recipe and the concurrency note.
/// The methods forward to [`RetainedCell`] unchanged.
#[repr(transparent)]
pub struct RtcRetainedCell<const N: usize>(RetainedCell<N>);

impl<const N: usize> RtcRetainedCell<N> {
    /// A zero-initialised cell, which [`load`](Self::load) reports as [`Invalid::Magic`].
    ///
    /// In `.rtc_fast.persistent` the initialiser is never applied and the memory keeps what it held.
    #[must_use]
    pub const fn new() -> Self {
        Self(RetainedCell::new())
    }

    /// Validate the record against `reset` and decode a `T` from it.
    ///
    /// See [`RetainedCell::load`].
    ///
    /// # Errors
    ///
    /// Returns the [`Invalid`] reason the record was refused.
    pub fn load<T: Retainable>(&self, reset: ResetReason) -> Result<T, Invalid> {
        self.0.load(reset)
    }

    /// Serialise `value` into the cell and seal it with a fresh checksum.
    ///
    /// See [`RetainedCell::store`].
    ///
    /// # Errors
    ///
    /// - [`StoreError::Length`] — `N < T::SIZE`, or the size does not fit `u16`; the record is unchanged.
    /// - [`StoreError::Payload`] — [`Retainable::write`] returned `None`; the record is invalidated.
    #[must_use = "a failed store leaves no valid record"]
    pub fn store<T: Retainable>(&self, value: &T) -> Result<(), StoreError> {
        self.0.store(value)
    }

    /// Clear the magic so the next [`load`](Self::load) returns [`Invalid::Magic`].
    pub fn invalidate(&self) {
        self.0.invalidate();
    }
}

impl<const N: usize> Default for RtcRetainedCell<N> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: `Persistable` requires a type that is valid for any bit pattern and is built
// from persistable fields and padding. The inner `UnsafeCell<MaybeUninit<Retained<N>>>`
// holds `u32, u32, u16, u16, [u8; N]` plus trailing padding, so every bit pattern is a
// valid value. Reads are volatile and gated on the reset reason, so a stale or arbitrary
// record is rejected by the magic, version, length and CRC checks instead of being trusted.
unsafe impl<const N: usize> esp_hal::Persistable for RtcRetainedCell<N> {}

const _: () = {
    const fn assert_sync<T: Sync>() {}
    assert_sync::<RtcRetainedCell<16>>();
};
