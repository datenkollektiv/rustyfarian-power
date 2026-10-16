//! Validity-checked retention record for RTC memory.
//!
//! A few words of firmware state (a session id, a report sequence, a boot
//! counter) need to survive deep sleep and, on the ESP-IDF tier, ordinary
//! resets.
//! RTC memory does that, but it holds arbitrary bits after power loss and may
//! hold a half-written record after a brownout.
//! [`Retained`] is a `no_std`, allocation-free envelope of integers and a byte
//! array with a magic number, a schema version, a length and a CRC-32, so a
//! stale or corrupt record is detected instead of trusted.
//!
//! # Usage
//!
//! Implement [`Retainable`] for the payload (a fixed little-endian layout is
//! recommended), place a `Retained<N>` with `N >= T::SIZE` in RTC memory, and
//! pair every [`Retained::load`] with the [`ResetReason`] of the current boot:
//!
//! ```
//! use stoker::{Retainable, Retained, ResetReason};
//!
//! #[derive(Clone, Copy, PartialEq, Eq, Debug)]
//! struct Session { id: u32, seq: u16 }
//!
//! impl Retainable for Session {
//!     const VERSION: u16 = 1;
//!     const SIZE: usize = 6;
//!     fn write(&self, buf: &mut [u8]) -> Option<()> {
//!         buf.get_mut(0..4)?.copy_from_slice(&self.id.to_le_bytes());
//!         buf.get_mut(4..6)?.copy_from_slice(&self.seq.to_le_bytes());
//!         Some(())
//!     }
//!     fn read(buf: &[u8]) -> Option<Self> {
//!         let id = u32::from_le_bytes(buf.get(0..4)?.try_into().ok()?);
//!         let seq = u16::from_le_bytes(buf.get(4..6)?.try_into().ok()?);
//!         Some(Self { id, seq })
//!     }
//! }
//!
//! let mut record = Retained::<16>::empty();
//! record.store(&Session { id: 0xCAFE, seq: 7 }).unwrap();
//!
//! // After a deep-sleep wake the record is trusted when it checks out.
//! assert_eq!(record.load::<Session>(ResetReason::DeepSleep), Ok(Session { id: 0xCAFE, seq: 7 }));
//!
//! // After power loss or a brownout it is refused even when the bits look fine.
//! assert_eq!(record.load::<Session>(ResetReason::PowerOn), Err(stoker::Invalid::PowerOn));
//! ```
//!
//! # Placement
//!
//! In firmware the record lives in a [`RetainedCell`] `static` (never
//! `static mut`) placed in an RTC section by the platform tier:
//! the ESP-IDF tier uses `#[link_section = ".rtc_noinit"]` for reset survival,
//! the esp-hal tier wraps the cell in its own `Persistable` newtype under
//! `#[ram(unstable(rtc_fast, persistent))]`.
//! The cell holds the record as `MaybeUninit`, so the compiler never assumes
//! the initialiser it sees in source, and copies it in and out with volatile
//! accesses, so a `NOLOAD` section that never applied that initialiser is read
//! as the bits it actually holds.
//! Because every field is an integer or a byte array, every bit pattern is a
//! valid `Retained`, and [`RetainedCell::load`] checks the [`ResetReason`]
//! before it touches the memory at all.
//! [`Retained::empty`] and the `&mut self` API remain for `.rtc.data`
//! placement and for host-side tests.
//!
//! # Encoding
//!
//! The checksum is CRC-32 (IEEE 802.3, reflected polynomial `0xEDB88320`,
//! initial value and final XOR `0xFFFF_FFFF`) over the little-endian header
//! fields `magic`, `version`, `len` followed by the first `len` payload bytes.
//! All-zero and all-`0xFF` memory fails the magic check before the CRC runs.
//! There is no A/B slot pair: a reset during [`Retained::store`] leaves a
//! record the checksum rejects, and the consumer falls back to fresh state,
//! which is the same outcome a stale second slot would give.

use core::cell::UnsafeCell;
use core::fmt;
use core::mem::MaybeUninit;
use core::ptr;
use core::sync::atomic::{compiler_fence, fence, Ordering};

use crate::reset::ResetReason;

/// Magic number identifying a [`Retained`] record (`"STOK"` as little-endian bytes).
const MAGIC: u32 = 0x5354_4F4B;

/// A value that can be retained across sleep and reset through [`Retained`].
///
/// Serialising through `write`/`read` rather than casting the struct means the
/// checksum never covers padding bytes, and the layout is explicit.
/// Use a fixed little-endian layout so a record written by one firmware build
/// is readable by the next.
///
/// Bump [`VERSION`](Self::VERSION) on any change to the layout.
/// Two payload types that share `VERSION` and `SIZE` (for example `u32` and
/// `i32`) will load each other's bytes; the envelope cannot tell them apart.
///
/// The crate implements `Retainable` for the fixed-width integers
/// (`u8`–`u64`, `i8`–`i64`) with `VERSION = 1` and a little-endian layout, so a
/// boot counter needs no custom impl.
pub trait Retainable: Copy {
    /// Schema version of the payload layout, checked on [`Retained::load`].
    const VERSION: u16;
    /// Exact number of payload bytes written by [`write`](Self::write).
    const SIZE: usize;
    /// Serialise `self` into `buf`, which is exactly [`SIZE`](Self::SIZE) bytes long.
    ///
    /// Return `None` if the value cannot be serialised; [`Retained::store`]
    /// then reports [`StoreError::Payload`].
    fn write(&self, buf: &mut [u8]) -> Option<()>;
    /// Deserialise a value from `buf`, which is exactly [`SIZE`](Self::SIZE) bytes long.
    ///
    /// Return `None` if the bytes do not form a valid value (for example an
    /// out-of-range discriminant); [`Retained::load`] then reports
    /// [`Invalid::Payload`].
    fn read(buf: &[u8]) -> Option<Self>;
}

macro_rules! retainable_int {
    ($($t:ty),*) => {$(
        impl Retainable for $t {
            const VERSION: u16 = 1;
            const SIZE: usize = core::mem::size_of::<$t>();
            fn write(&self, buf: &mut [u8]) -> Option<()> {
                buf.get_mut(..Self::SIZE)?.copy_from_slice(&self.to_le_bytes());
                Some(())
            }
            fn read(buf: &[u8]) -> Option<Self> {
                Some(<$t>::from_le_bytes(buf.get(..Self::SIZE)?.try_into().ok()?))
            }
        }
    )*};
}

retainable_int!(u8, u16, u32, u64, i8, i16, i32, i64);

/// Why [`Retained::load`] refused a record.
///
/// Returned so a consumer can count resets by cause even when it discards the
/// record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Invalid {
    /// The boot followed a [`ResetReason::PowerOn`]; RTC memory is arbitrary.
    PowerOn,
    /// The boot followed a [`ResetReason::Brownout`]; the record may be partial.
    Brownout,
    /// The magic number did not match; the memory never held a record.
    Magic,
    /// The record was written by a payload with a different [`Retainable::VERSION`].
    Version,
    /// The record length does not match [`Retainable::SIZE`], or the envelope
    /// is too small for the payload (`N < T::SIZE`).
    Length,
    /// The CRC-32 did not match; the record is corrupt or was torn by a reset.
    Checksum,
    /// The bytes checked out but [`Retainable::read`] rejected them.
    Payload,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PowerOn => write!(f, "record discarded after power-on reset"),
            Self::Brownout => write!(f, "record discarded after brownout reset"),
            Self::Magic => write!(f, "record magic mismatch; memory never held a record"),
            Self::Version => write!(f, "record schema version mismatch"),
            Self::Length => write!(f, "record length does not match the payload size"),
            Self::Checksum => write!(f, "record checksum mismatch"),
            Self::Payload => write!(f, "record bytes rejected by the payload decoder"),
        }
    }
}

impl core::error::Error for Invalid {}

/// Why [`Retained::store`] could not write a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// The envelope is too small for the payload (`N < T::SIZE`), or the size
    /// does not fit the 16-bit length field.
    Length,
    /// [`Retainable::write`] returned `None`; the record has been invalidated.
    Payload,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length => write!(f, "envelope too small for the payload"),
            Self::Payload => write!(f, "payload could not be serialised; record invalidated"),
        }
    }
}

impl core::error::Error for StoreError {}

/// A validity-checked retention record holding up to `N` payload bytes.
///
/// Built only from integers and a byte array so arbitrary bits after power
/// loss are a well-formed (if invalid) value.
/// `#[repr(C)]` keeps the layout stable across firmware builds; with this
/// field order the only padding is trailing.
///
/// `N >= T::SIZE` is checked at runtime by [`load`](Self::load) and
/// [`store`](Self::store) because stable Rust cannot express it in the type.
///
/// See the [module docs](self) for placement and encoding.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retained<const N: usize> {
    magic: u32,
    checksum: u32,
    version: u16,
    len: u16,
    bytes: [u8; N],
}

impl<const N: usize> Retained<N> {
    /// An all-zero record that fails [`load`](Self::load) with [`Invalid::Magic`].
    ///
    /// Use as the initialiser for `.rtc.data` placement; a `NOLOAD` section
    /// never applies it.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            magic: 0,
            checksum: 0,
            version: 0,
            len: 0,
            bytes: [0; N],
        }
    }

    /// Validate the record against `reset` and decode a `T` from it.
    ///
    /// Checks run in this order, and the first failure is returned:
    /// [`Invalid::PowerOn`] or [`Invalid::Brownout`] from `reset`,
    /// [`Invalid::Magic`], [`Invalid::Version`], [`Invalid::Length`],
    /// [`Invalid::Checksum`], [`Invalid::Payload`].
    /// [`ResetReason::DeepSleep`], [`ResetReason::Software`],
    /// [`ResetReason::Panic`], [`ResetReason::Watchdog`] and
    /// [`ResetReason::Other`] trust the checksum.
    ///
    /// # Errors
    ///
    /// Returns the [`Invalid`] reason the record was refused.
    pub fn load<T: Retainable>(&self, reset: ResetReason) -> Result<T, Invalid> {
        reset_gate(reset)?;
        if self.magic != MAGIC {
            return Err(Invalid::Magic);
        }
        if self.version != T::VERSION {
            return Err(Invalid::Version);
        }
        let len = usize::from(self.len);
        if N < T::SIZE || len != T::SIZE {
            return Err(Invalid::Length);
        }
        let payload = self.bytes.get(..len).ok_or(Invalid::Length)?;
        if self.checksum != checksum(self.magic, self.version, self.len, payload) {
            return Err(Invalid::Checksum);
        }
        T::read(payload).ok_or(Invalid::Payload)
    }

    /// Serialise `value` into the record and seal it with a fresh checksum.
    ///
    /// The magic is cleared first and written last, so a reset in the middle
    /// of `store` leaves a record that [`load`](Self::load) rejects with
    /// [`Invalid::Magic`] (or [`Invalid::Checksum`] if the compiler reordered
    /// the plain stores; [`RetainedCell::store`] pins the order with volatile
    /// writes).
    ///
    /// # Errors
    ///
    /// - [`StoreError::Length`] — `N < T::SIZE`, or the size does not fit `u16`;
    ///   the record is unchanged.
    /// - [`StoreError::Payload`] — [`Retainable::write`] returned `None`; the
    ///   record is invalidated so a half-written payload is never trusted.
    #[must_use = "a failed store leaves no valid record"]
    pub fn store<T: Retainable>(&mut self, value: &T) -> Result<(), StoreError> {
        let len = u16::try_from(T::SIZE).map_err(|_| StoreError::Length)?;
        if N < T::SIZE {
            return Err(StoreError::Length);
        }
        self.magic = 0;
        let payload = self.bytes.get_mut(..T::SIZE).ok_or(StoreError::Length)?;
        if value.write(payload).is_none() {
            self.magic = 0;
            return Err(StoreError::Payload);
        }
        self.version = T::VERSION;
        self.len = len;
        self.checksum = checksum(MAGIC, T::VERSION, len, payload);
        self.magic = MAGIC;
        Ok(())
    }
}

/// Refuse a load after the reset reasons that leave RTC memory untrustworthy.
fn reset_gate(reset: ResetReason) -> Result<(), Invalid> {
    match reset {
        ResetReason::PowerOn => Err(Invalid::PowerOn),
        ResetReason::Brownout => Err(Invalid::Brownout),
        ResetReason::DeepSleep
        | ResetReason::Software
        | ResetReason::Panic
        | ResetReason::Watchdog
        | ResetReason::Other => Ok(()),
    }
}

/// A [`Retained`] record behind a `static`, read and written through volatile
/// accesses.
///
/// This is the type to place in RTC memory: it is `Sync`, so it can be a
/// `static` without `static mut`, and it never hands out a reference to the
/// memory.
/// [`load`](Self::load) checks the [`ResetReason`] first and only then copies
/// the record out with a volatile read; [`store`](Self::store) clears the
/// magic, writes the record, and writes the magic last, each step volatile, so
/// in program order the memory holds either the previous valid record, an
/// invalid record, or the new valid record, and a memory fence after the last
/// write commits it before the caller goes on to sleep or reset.
///
/// # Concurrency
///
/// Every `load`, `store` and `invalidate` copies the record inside a
/// [`critical_section::with`] block, so two callers on the same cell (two
/// tasks, or `main` and an interrupt) are serialised and the safe `&self` API
/// is sound; riscv32imc has no compare-and-swap, so a critical section is the
/// portable lock.
/// The platform supplies the `critical-section` implementation: esp-hal with
/// its `rt` feature, esp-idf-hal with its `critical-section` feature, and the
/// crate's `std` feature on the host (a dev-dependency here).
/// The section lasts for one volatile copy of `N + 12` bytes; serialising the
/// payload happens outside it.
///
/// # Initialisation
///
/// [`new`](Self::new) is the only constructor and is zeroed; it is the right
/// initialiser for every placement, because a `NOLOAD` section ignores it and
/// every other section reads the zeroes as [`Invalid::Magic`].
/// [`Retained::empty`] is for a bare record in `.rtc.data` or on the host, not
/// for the cell.
///
/// # Examples
///
/// ```
/// use stoker::{ResetReason, RetainedCell};
///
/// static BOOTS: RetainedCell<4> = RetainedCell::new();
///
/// // In firmware, add the tier's RTC section attribute to the static.
/// let boots = BOOTS.load::<u32>(ResetReason::DeepSleep).unwrap_or(0);
/// BOOTS.store(&(boots + 1)).unwrap();
/// assert_eq!(BOOTS.load::<u32>(ResetReason::DeepSleep), Ok(boots + 1));
/// ```
pub struct RetainedCell<const N: usize> {
    record: UnsafeCell<MaybeUninit<Retained<N>>>,
}

// SAFETY: the memory is only ever accessed through `load`, `store` and
// `invalidate`, which copy through volatile reads and writes inside a critical
// section and never hand out a reference, so no two accesses to the cell can
// overlap from any task, thread or interrupt.
unsafe impl<const N: usize> Sync for RetainedCell<N> {}

impl<const N: usize> RetainedCell<N> {
    /// A zero-initialised cell, which [`load`](Self::load) reports as
    /// [`Invalid::Magic`].
    ///
    /// In a `NOLOAD` RTC section the initialiser is never applied and the
    /// memory keeps whatever it held before the reset; everywhere else (host
    /// tests, `.bss`, `.rtc.data`, the stack) the zeroes make every read well
    /// defined.
    /// Use this for every placement; there is no `empty()` on the cell.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            record: UnsafeCell::new(MaybeUninit::zeroed()),
        }
    }

    fn ptr(&self) -> *mut Retained<N> {
        self.record.get().cast::<Retained<N>>()
    }

    /// Validate the record in the cell against `reset` and decode a `T` from it.
    ///
    /// Returns [`Invalid::PowerOn`] or [`Invalid::Brownout`] before reading the
    /// memory; otherwise copies the record out and applies
    /// [`Retained::load`].
    ///
    /// # Errors
    ///
    /// Returns the [`Invalid`] reason the record was refused.
    pub fn load<T: Retainable>(&self, reset: ResetReason) -> Result<T, Invalid> {
        reset_gate(reset)?;
        // SAFETY: the pointer comes from `UnsafeCell::get` on a live `self`,
        // is aligned, and is valid for reads of `Retained<N>`. The initialiser
        // is zeroed, so the memory is initialised everywhere except a NOLOAD
        // section, which the abstract machine treats as uninitialised; that
        // read is accepted on the hardware argument, as for MMIO: the RAM
        // holds some bit pattern, and every bit pattern is a valid
        // `Retained<N>` (integers and a byte array only). The volatile read
        // keeps the compiler from substituting the initialiser it saw in
        // source. The critical section excludes every other `load`, `store`
        // and `invalidate` on this cell.
        let record = critical_section::with(|_| unsafe { ptr::read_volatile(self.ptr()) });
        record.load(reset)
    }

    /// Serialise `value` into the cell and seal it with a fresh checksum.
    ///
    /// The magic is cleared with a volatile write before anything else and
    /// written with a volatile write after everything else, so a torn update
    /// is never a valid record; a memory fence after the final write commits
    /// it before the caller sleeps or resets.
    /// `store` builds the record on the stack first, so a large `N` costs
    /// `N + 12` bytes of stack.
    ///
    /// # Errors
    ///
    /// - [`StoreError::Length`] — `N < T::SIZE`, or the size does not fit `u16`;
    ///   the record is unchanged.
    /// - [`StoreError::Payload`] — [`Retainable::write`] returned `None`; the
    ///   record is invalidated so a half-written payload is never trusted.
    #[must_use = "a failed store leaves no valid record"]
    pub fn store<T: Retainable>(&self, value: &T) -> Result<(), StoreError> {
        let mut record = Retained::<N>::empty();
        record.store(value).inspect_err(|err| {
            if *err == StoreError::Payload {
                self.invalidate();
            }
        })?;
        let magic = record.magic;
        record.magic = 0;
        critical_section::with(|_| {
            self.clear_magic();
            // SAFETY: see `load`; the pointer is valid for writes of
            // `Retained<N>` and the critical section excludes every other
            // access to this cell.
            unsafe { ptr::write_volatile(self.ptr(), record) };
            compiler_fence(Ordering::SeqCst);
            // SAFETY: `&raw mut` projects the field without creating a
            // reference; the field is aligned and inside the valid allocation.
            unsafe { ptr::write_volatile(&raw mut (*self.ptr()).magic, magic) };
            fence(Ordering::SeqCst);
        });
        Ok(())
    }

    /// Clear the magic so the next [`load`](Self::load) returns [`Invalid::Magic`].
    ///
    /// The payload bytes are left in place.
    pub fn invalidate(&self) {
        critical_section::with(|_| self.clear_magic());
    }

    /// Volatile write of a zero magic; the caller holds the critical section.
    fn clear_magic(&self) {
        // SAFETY: field projection through `&raw mut` creates no reference;
        // the field is aligned and inside the valid allocation.
        unsafe { ptr::write_volatile(&raw mut (*self.ptr()).magic, 0) };
        compiler_fence(Ordering::SeqCst);
    }
}

impl<const N: usize> Default for RetainedCell<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// CRC-32 (IEEE 802.3) over the little-endian header fields and the payload.
fn checksum(magic: u32, version: u16, len: u16, payload: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(&magic.to_le_bytes());
    crc.update(&version.to_le_bytes());
    crc.update(&len.to_le_bytes());
    crc.update(payload);
    crc.finish()
}

/// Table-less CRC-32 (IEEE 802.3, reflected polynomial `0xEDB88320`).
struct Crc32(u32);

impl Crc32 {
    const POLY: u32 = 0xEDB8_8320;

    fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u32::from(b);
            for _ in 0..8 {
                let mask = (self.0 & 1).wrapping_neg();
                self.0 = (self.0 >> 1) ^ (Self::POLY & mask);
            }
        }
    }

    fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::ToString;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Probe {
        a: u32,
        b: u16,
    }

    impl Retainable for Probe {
        const VERSION: u16 = 1;
        const SIZE: usize = 6;
        fn write(&self, buf: &mut [u8]) -> Option<()> {
            buf.get_mut(0..4)?.copy_from_slice(&self.a.to_le_bytes());
            buf.get_mut(4..6)?.copy_from_slice(&self.b.to_le_bytes());
            Some(())
        }
        fn read(buf: &[u8]) -> Option<Self> {
            let a = u32::from_le_bytes(buf.get(0..4)?.try_into().ok()?);
            let b = u16::from_le_bytes(buf.get(4..6)?.try_into().ok()?);
            Some(Self { a, b })
        }
    }

    /// Same layout as `Probe`, newer schema version.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct ProbeV2(Probe);

    impl Retainable for ProbeV2 {
        const VERSION: u16 = 2;
        const SIZE: usize = Probe::SIZE;
        fn write(&self, buf: &mut [u8]) -> Option<()> {
            self.0.write(buf)
        }
        fn read(buf: &[u8]) -> Option<Self> {
            Probe::read(buf).map(Self)
        }
    }

    /// A payload whose decoder rejects a byte value, and whose encoder can fail.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Flag {
        Off,
        On,
        Unwritable,
    }

    impl Retainable for Flag {
        const VERSION: u16 = 1;
        const SIZE: usize = 1;
        fn write(&self, buf: &mut [u8]) -> Option<()> {
            let byte = match self {
                Self::Off => 0,
                Self::On => 1,
                Self::Unwritable => return None,
            };
            *buf.first_mut()? = byte;
            Some(())
        }
        fn read(buf: &[u8]) -> Option<Self> {
            match buf.first()? {
                0 => Some(Self::Off),
                1 => Some(Self::On),
                _ => None,
            }
        }
    }

    const PROBE: Probe = Probe {
        a: 0xDEAD_BEEF,
        b: 0x1234,
    };

    fn stored() -> Retained<16> {
        let mut record = Retained::<16>::empty();
        record.store(&PROBE).unwrap();
        record
    }

    // --- acceptance tests from the feature request ---

    #[test]
    fn round_trip_after_deep_sleep() {
        let record = stored();
        assert_eq!(record.load::<Probe>(ResetReason::DeepSleep), Ok(PROBE));
    }

    #[test]
    fn bad_magic_is_invalid_magic() {
        let mut record = stored();
        record.magic ^= 1;
        assert_eq!(
            record.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn flipped_payload_bit_is_invalid_checksum() {
        let mut record = stored();
        record.bytes[2] ^= 0x10;
        assert_eq!(
            record.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Checksum)
        );
    }

    #[test]
    fn stale_checksum_is_invalid_checksum() {
        let mut record = stored();
        record.checksum ^= 1;
        assert_eq!(
            record.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Checksum)
        );
    }

    #[test]
    fn wrong_version_is_invalid_version() {
        let record = stored();
        assert_eq!(
            record.load::<ProbeV2>(ResetReason::DeepSleep),
            Err(Invalid::Version)
        );
    }

    #[test]
    fn short_buffer_rejects_store_and_load() {
        let mut record = Retained::<4>::empty();
        assert_eq!(record.store(&PROBE), Err(StoreError::Length));
        assert_eq!(record, Retained::<4>::empty());
        assert_eq!(
            record.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn length_mismatch_is_invalid_length() {
        let mut record = stored();
        record.store(&7u32).unwrap();
        assert_eq!(
            record.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Length)
        );
    }

    #[test]
    fn corrupt_len_is_invalid_length() {
        let mut record = stored();
        record.len = 0xFFFF;
        assert_eq!(
            record.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Length)
        );
    }

    #[test]
    fn envelope_smaller_than_payload_is_invalid_length() {
        let mut record = Retained::<4>::empty();
        record.store(&7u32).unwrap();
        assert_eq!(record.load::<u32>(ResetReason::DeepSleep), Ok(7));
        assert_eq!(
            record.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Length)
        );
    }

    #[test]
    fn power_on_is_invalid_power_on() {
        let record = stored();
        assert_eq!(
            record.load::<Probe>(ResetReason::PowerOn),
            Err(Invalid::PowerOn)
        );
    }

    #[test]
    fn brownout_is_invalid_brownout() {
        let record = stored();
        assert_eq!(
            record.load::<Probe>(ResetReason::Brownout),
            Err(Invalid::Brownout)
        );
    }

    #[test]
    fn watchdog_with_valid_checksum_loads() {
        let record = stored();
        assert_eq!(record.load::<Probe>(ResetReason::Watchdog), Ok(PROBE));
    }

    #[test]
    fn software_and_other_trust_the_checksum() {
        let record = stored();
        assert_eq!(record.load::<Probe>(ResetReason::Software), Ok(PROBE));
        assert_eq!(record.load::<Probe>(ResetReason::Other), Ok(PROBE));
    }

    // --- envelope behaviour ---

    #[test]
    fn empty_record_is_invalid_magic() {
        let record = Retained::<8>::empty();
        assert_eq!(
            record.load::<u32>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn all_ones_record_is_invalid_magic() {
        let record = Retained::<8> {
            magic: 0xFFFF_FFFF,
            checksum: 0xFFFF_FFFF,
            version: 0xFFFF,
            len: 0xFFFF,
            bytes: [0xFF; 8],
        };
        assert_eq!(
            record.load::<u32>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn reset_reason_is_checked_before_contents() {
        let record = Retained::<8>::empty();
        assert_eq!(
            record.load::<u32>(ResetReason::PowerOn),
            Err(Invalid::PowerOn)
        );
    }

    #[test]
    fn exact_fit_round_trips() {
        let mut record = Retained::<6>::empty();
        record.store(&PROBE).unwrap();
        assert_eq!(record.load::<Probe>(ResetReason::DeepSleep), Ok(PROBE));
    }

    #[test]
    fn store_overwrites_previous_record() {
        let mut record = stored();
        let next = Probe { a: 1, b: 2 };
        record.store(&next).unwrap();
        assert_eq!(record.load::<Probe>(ResetReason::DeepSleep), Ok(next));
    }

    #[test]
    fn trailing_bytes_are_not_covered_by_checksum() {
        let mut record = stored();
        record.bytes[15] ^= 0xFF;
        assert_eq!(record.load::<Probe>(ResetReason::DeepSleep), Ok(PROBE));
    }

    #[test]
    fn undecodable_payload_is_invalid_payload() {
        let mut record = Retained::<4>::empty();
        record.store(&Flag::On).unwrap();
        assert_eq!(record.load::<Flag>(ResetReason::DeepSleep), Ok(Flag::On));
        record.store(&3u8).unwrap();
        assert_eq!(
            record.load::<Flag>(ResetReason::DeepSleep),
            Err(Invalid::Payload)
        );
    }

    #[test]
    fn failed_write_invalidates_record() {
        let mut record = Retained::<4>::empty();
        record.store(&Flag::On).unwrap();
        assert_eq!(record.store(&Flag::Unwritable), Err(StoreError::Payload));
        assert_eq!(
            record.load::<Flag>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn integer_impls_round_trip() {
        let mut record = Retained::<8>::empty();
        record.store(&0xA5u8).unwrap();
        assert_eq!(record.load::<u8>(ResetReason::DeepSleep), Ok(0xA5));
        record.store(&-2i16).unwrap();
        assert_eq!(record.load::<i16>(ResetReason::DeepSleep), Ok(-2));
        record.store(&0xDEAD_BEEFu32).unwrap();
        assert_eq!(record.load::<u32>(ResetReason::DeepSleep), Ok(0xDEAD_BEEF));
        record.store(&i64::MIN).unwrap();
        assert_eq!(record.load::<i64>(ResetReason::DeepSleep), Ok(i64::MIN));
    }

    #[test]
    fn integer_payload_is_little_endian() {
        let mut record = Retained::<4>::empty();
        record.store(&0x0102_0304u32).unwrap();
        assert_eq!(record.bytes, [0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn empty_is_copy_and_eq() {
        let a = Retained::<8>::empty();
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn invalid_display_names_the_cause() {
        assert!(Invalid::Checksum.to_string().contains("checksum"));
        assert!(Invalid::PowerOn.to_string().contains("power-on"));
        assert!(StoreError::Length.to_string().contains("too small"));
    }

    #[test]
    fn panic_reset_trusts_the_checksum() {
        assert_eq!(stored().load::<Probe>(ResetReason::Panic), Ok(PROBE));
    }

    // --- torn stores, layout, size ---

    /// Writes one byte, then panics, like a reset in the middle of `write`.
    #[derive(Clone, Copy)]
    struct Torn;

    impl Retainable for Torn {
        const VERSION: u16 = 1;
        const SIZE: usize = 6;
        fn write(&self, buf: &mut [u8]) -> Option<()> {
            buf[0] = 0xAA;
            panic!("reset during write");
        }
        fn read(_: &[u8]) -> Option<Self> {
            Some(Self)
        }
    }

    #[test]
    fn torn_store_is_invalid_magic() {
        let mut record = stored();
        let torn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = record.store(&Torn);
        }));
        assert!(torn.is_err());
        assert_eq!(
            record.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn layout_is_header_then_payload_with_trailing_padding() {
        assert_eq!(core::mem::size_of::<Retained<16>>(), 12 + 16);
        assert_eq!(core::mem::size_of::<Retained<5>>(), 20);
        assert_eq!(core::mem::align_of::<Retained<5>>(), 4);
        assert_eq!(core::mem::size_of::<RetainedCell<16>>(), 12 + 16);
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Big([u8; 1000]);

    impl Retainable for Big {
        const VERSION: u16 = 1;
        const SIZE: usize = 1000;
        fn write(&self, buf: &mut [u8]) -> Option<()> {
            buf.get_mut(..1000)?.copy_from_slice(&self.0);
            Some(())
        }
        fn read(buf: &[u8]) -> Option<Self> {
            Some(Self(buf.get(..1000)?.try_into().ok()?))
        }
    }

    #[test]
    fn large_envelope_round_trips() {
        let mut bytes = [0u8; 1000];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let big = Big(bytes);
        let mut record = Retained::<1024>::empty();
        record.store(&big).unwrap();
        assert_eq!(record.load::<Big>(ResetReason::Watchdog), Ok(big));
    }

    #[test]
    fn random_garbage_never_loads_or_panics() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..10_000 {
            let mut bytes = [0u8; 16];
            for b in &mut bytes {
                *b = next() as u8;
            }
            let record = Retained::<16> {
                magic: next() as u32,
                checksum: next() as u32,
                version: next() as u16,
                len: next() as u16,
                bytes,
            };
            assert!(record.load::<Probe>(ResetReason::DeepSleep).is_err());
        }
    }

    #[test]
    fn random_garbage_with_valid_magic_never_loads_or_panics() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for i in 0..10_000 {
            let mut bytes = [0u8; 16];
            for b in &mut bytes {
                *b = next() as u8;
            }
            let record = Retained::<16> {
                magic: MAGIC,
                checksum: next() as u32,
                version: if i % 2 == 0 {
                    Probe::VERSION
                } else {
                    next() as u16
                },
                len: if i % 4 < 2 {
                    Probe::SIZE as u16
                } else {
                    next() as u16
                },
                bytes,
            };
            let err = record.load::<Probe>(ResetReason::DeepSleep).unwrap_err();
            assert!(matches!(
                err,
                Invalid::Version | Invalid::Length | Invalid::Checksum
            ));
        }
    }

    // --- RetainedCell ---

    #[test]
    fn fresh_cell_is_invalid_magic() {
        let cell = RetainedCell::<16>::default();
        assert_eq!(
            cell.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn padded_cell_round_trips() {
        let cell = RetainedCell::<5>::new();
        cell.store(&Flag::On).unwrap();
        assert_eq!(cell.load::<Flag>(ResetReason::DeepSleep), Ok(Flag::On));
        cell.store(&0x1122_3344u32).unwrap();
        assert_eq!(cell.load::<u32>(ResetReason::Panic), Ok(0x1122_3344));
    }

    static CELL: RetainedCell<16> = RetainedCell::new();

    #[test]
    fn cell_round_trips_and_overwrites() {
        CELL.store(&PROBE).unwrap();
        assert_eq!(CELL.load::<Probe>(ResetReason::DeepSleep), Ok(PROBE));
        CELL.store(&7u32).unwrap();
        assert_eq!(CELL.load::<u32>(ResetReason::Software), Ok(7));
        assert_eq!(
            CELL.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Length)
        );
    }

    #[test]
    fn cell_refuses_power_on_and_brownout_before_reading() {
        let cell = RetainedCell::<16>::new();
        assert_eq!(
            cell.load::<Probe>(ResetReason::PowerOn),
            Err(Invalid::PowerOn)
        );
        assert_eq!(
            cell.load::<Probe>(ResetReason::Brownout),
            Err(Invalid::Brownout)
        );
    }

    #[test]
    fn cell_invalidate_is_invalid_magic() {
        let cell = RetainedCell::<16>::new();
        cell.store(&PROBE).unwrap();
        cell.invalidate();
        assert_eq!(
            cell.load::<Probe>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn cell_failed_write_invalidates() {
        let cell = RetainedCell::<16>::new();
        cell.store(&Flag::On).unwrap();
        assert_eq!(cell.store(&Flag::Unwritable), Err(StoreError::Payload));
        assert_eq!(
            cell.load::<Flag>(ResetReason::DeepSleep),
            Err(Invalid::Magic)
        );
    }

    #[test]
    fn cell_too_small_leaves_record_unchanged() {
        let cell = RetainedCell::<4>::new();
        cell.store(&3u32).unwrap();
        assert_eq!(cell.store(&9u64), Err(StoreError::Length));
        assert_eq!(cell.load::<u32>(ResetReason::DeepSleep), Ok(3));
    }

    #[test]
    fn cell_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<RetainedCell<8>>();
    }

    // --- CRC-32 ---

    #[test]
    fn crc32_matches_ieee_check_value() {
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
    }

    #[test]
    fn crc32_of_empty_input_is_zero() {
        assert_eq!(Crc32::new().finish(), 0);
    }

    #[test]
    fn checksum_covers_header_fields() {
        let payload = [1u8, 2, 3];
        let base = checksum(MAGIC, 1, 3, &payload);
        assert_ne!(base, checksum(MAGIC ^ 1, 1, 3, &payload));
        assert_ne!(base, checksum(MAGIC, 2, 3, &payload));
        assert_ne!(base, checksum(MAGIC, 1, 4, &payload));
    }
}
