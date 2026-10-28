#![no_std]
//! Bare-metal (esp-hal) hardware tier of the rustyfarian power stack.
//!
//! This crate provides the `no_std`, bare-metal esp-hal implementations of the
//! traits defined in the platform-agnostic [`stoker`] crate — battery voltage
//! monitoring, charging detection, and deep sleep — for the ESP32-C3, ESP32-C6,
//! ESP32, and ESP32-S3.
//!
//! Everything public in [`stoker`] is re-exported here, so firmware needs a
//! single import once drivers exist.
//!
//! # Status
//!
//! The `sleep` module (deep sleep with GPIO and timer wake) exists for the ESP32-C3 and
//! ESP32-S3 (`docs/features/esp-hal-sleep-tier-v1.md`). The `reset` and `retain` modules
//! (`EspHalResetReasonSource`, `RtcRetainedCell`) place a `stoker` retention record in RTC fast
//! memory (`docs/features/rtc-retention-v1.md`). The `clock` module (`EspHalRtcClock`) is a
//! drift-free microsecond clock across resets and deep sleep
//! (`docs/features/esp-hal-rtc-clock-v1.md`). The ADC battery monitor and charging
//! monitor are still pending (`docs/features/esp-hal-parity-v1.md`).
//!
//! # Chip features
//!
//! Exactly one of `esp32c3` (default), `esp32c6`, `esp32`, or `esp32s3` must be
//! enabled; they are mutually exclusive. For any chip other than the C3, use
//! `--no-default-features --features <chip>,unstable`.
//!
//! # Building
//!
//! This crate only builds for `*-none-elf` targets. Check it for all four chips
//! with `just check-hal`.

#[cfg(not(any(
    feature = "esp32c3",
    feature = "esp32c6",
    feature = "esp32",
    feature = "esp32s3"
)))]
compile_error!(
    "rustyfarian-esp-hal-power needs a chip feature: pick exactly one of \
     esp32c3, esp32c6, esp32, esp32s3 (non-C3 chips: use `--no-default-features`)"
);

#[cfg(any(
    all(feature = "esp32c3", feature = "esp32c6"),
    all(feature = "esp32c3", feature = "esp32"),
    all(feature = "esp32c3", feature = "esp32s3"),
    all(feature = "esp32c6", feature = "esp32"),
    all(feature = "esp32c6", feature = "esp32s3"),
    all(feature = "esp32", feature = "esp32s3"),
))]
compile_error!(
    "rustyfarian-esp-hal-power chip features are mutually exclusive: enable exactly one of \
     esp32c3, esp32c6, esp32, esp32s3 (use `--no-default-features --features <chip>` to \
     replace the default `esp32c3`)"
);

#[doc(inline)]
pub use stoker::*;

// The drivers need esp-hal's unstable `rtc_cntl` module, sleep API and `Persistable` trait, so
// they are gated on `unstable` as well; without it the crate is only the `stoker` re-export
// (`just check-hal` builds that surface once).
#[cfg(all(
    target_os = "none",
    feature = "unstable",
    any(feature = "esp32c3", feature = "esp32s3")
))]
pub mod reset;
#[cfg(all(
    target_os = "none",
    feature = "unstable",
    any(feature = "esp32c3", feature = "esp32s3")
))]
pub use reset::EspHalResetReasonSource;

#[cfg(all(target_os = "none", feature = "unstable"))]
pub mod retain;
#[cfg(all(target_os = "none", feature = "unstable"))]
pub use retain::RtcRetainedCell;

#[cfg(all(
    target_os = "none",
    feature = "unstable",
    any(feature = "esp32c3", feature = "esp32s3")
))]
pub mod sleep;
#[cfg(all(
    target_os = "none",
    feature = "unstable",
    any(feature = "esp32c3", feature = "esp32s3")
))]
pub use sleep::{BoundSleep, EspHalSleepManager, EspHalWakeCauseSource};

#[cfg(all(target_os = "none", feature = "unstable"))]
pub mod gate;
#[cfg(all(target_os = "none", feature = "unstable"))]
pub use gate::{GpioRailGate, GpioRailGateError, Polarity, HELTEC_V3_VEXT_GPIO};

#[cfg(all(
    target_os = "none",
    feature = "unstable",
    any(feature = "esp32c3", feature = "esp32s3")
))]
pub mod clock;
#[cfg(all(
    target_os = "none",
    feature = "unstable",
    any(feature = "esp32c3", feature = "esp32s3")
))]
pub use clock::EspHalRtcClock;
