//! Process-wide `critical-section` implementation over the FreeRTOS port critical section.
//!
//! `stoker::RetainedCell` serialises its volatile copies with `critical_section::with`, and that
//! only excludes an interrupt or the other core if the linked implementation masks interrupts.
//! esp-idf-hal's own `critical-section` feature provides a task-level recursive mutex instead:
//! FreeRTOS forbids taking it from an ISR, and an interrupt that preempts the holder runs as the
//! same task and re-enters it, so it cannot back the cell's guarantee.
//!
//! This module mirrors `esp_idf_hal::interrupt::IsrCriticalSection` with the split
//! `acquire`/`release` shape `critical-section` needs: `vPortEnterCritical` on the single-core
//! chips (masks interrupts, nests), `xPortEnterCriticalTimeout` on one `portMUX` for the ESP32
//! and ESP32-S3 (spinlock against the other core plus interrupt mask).
//! Both are the primitives behind ESP-IDF's `portENTER_CRITICAL_SAFE`, so they are valid from
//! tasks and from ISRs.
//!
//! Exactly one `critical-section` implementation may be linked into a firmware.
//! Disable the `critical-section-impl` feature if the firmware brings another provider; do not
//! pair it with esp-idf-hal's `critical-section` feature.

use esp_idf_hal::sys;

/// The one `portMUX` the provider spins on (multi-core chips only).
#[cfg(any(esp32, esp32s3))]
struct Mux(core::cell::UnsafeCell<sys::portMUX_TYPE>);

// SAFETY: the mux is only handed to the FreeRTOS port functions, which implement the
// cross-core locking themselves; no Rust reference to its contents is ever created.
#[cfg(any(esp32, esp32s3))]
unsafe impl Sync for Mux {}

#[cfg(any(esp32, esp32s3))]
static MUX: Mux = Mux(core::cell::UnsafeCell::new(sys::portMUX_TYPE {
    owner: sys::portMUX_FREE_VAL,
    count: 0,
    #[cfg(esp_idf_freertos_portmux_debug)]
    lastLockedFn: b"(never locked)",
    #[cfg(esp_idf_freertos_portmux_debug)]
    lastLockedLine: -1,
}));

#[cfg(not(any(esp32, esp32s3)))]
#[inline(always)]
#[link_section = ".iram1.rustyfarian_cs_enter"]
fn enter() {
    // SAFETY: the single-core FreeRTOS port critical section; nests, and ESP-IDF's
    // portENTER_CRITICAL_SAFE uses the same call from ISRs.
    unsafe { sys::vPortEnterCritical() };
}

#[cfg(not(any(esp32, esp32s3)))]
#[inline(always)]
#[link_section = ".iram1.rustyfarian_cs_exit"]
fn exit() {
    // SAFETY: balances the `vPortEnterCritical` in `enter`; `critical-section` guarantees the
    // release matches an earlier acquire on the same context.
    unsafe { sys::vPortExitCritical() };
}

#[cfg(any(esp32, esp32s3))]
#[inline(always)]
#[link_section = ".iram1.rustyfarian_cs_enter"]
fn enter() {
    // SAFETY: `MUX` is a static the port functions own for their lifetime; the timeout is
    // `portMUX_NO_TIMEOUT`, so the call spins until the other core releases the lock.
    unsafe { sys::xPortEnterCriticalTimeout(MUX.0.get(), sys::portMUX_NO_TIMEOUT) };
}

#[cfg(any(esp32, esp32s3))]
#[inline(always)]
#[link_section = ".iram1.rustyfarian_cs_exit"]
fn exit() {
    // SAFETY: balances the `xPortEnterCriticalTimeout` in `enter` on the same mux.
    unsafe { sys::vPortExitCritical(MUX.0.get()) };
}

struct FreeRtosCriticalSection;

// SAFETY: `acquire` masks interrupts on this core (and takes the cross-core spinlock on the
// multi-core chips) until the matching `release`, which is exactly the exclusion the
// `critical_section::Impl` contract requires; nesting is handled by the FreeRTOS port.
unsafe impl critical_section::Impl for FreeRtosCriticalSection {
    unsafe fn acquire() -> critical_section::RawRestoreState {
        enter();
    }

    unsafe fn release(_restore_state: critical_section::RawRestoreState) {
        exit();
    }
}

critical_section::set_impl!(FreeRtosCriticalSection);
