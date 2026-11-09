//! CrowPanel 1.28" ESP32-S3 — Touch Deep-Sleep GPIO Wake Example (ESP-IDF)
//!
//! Deep-sleeps until the CST816S touch controller pulls INT (GPIO5) low or a 60 s timer fires,
//! prints the wake cause, and sleeps again.
//!
//! ## Board
//!
//! - **Board:** Elecrow CrowPanel 1.28" HMI ESP32-S3 Rotary Display.
//! - **Wake pin:** CST816S touch INT on GPIO5, active-low, an RTC pad (GPIO0-21).
//!   The encoder and button (GPIO45/42/41) are not RTC pads and cannot wake the chip.
//! - **Touch power:** the CST816S needs `LCD_3V3` (GPIO1) on and its reset (GPIO13) released;
//!   without them INT sits LOW and every sleep is rejected.
//!   Both pins are held HIGH through deep sleep so the controller keeps watching for touches.
//!
//! On the S3 the wake goes through EXT1, so a touch reports `WakeCause::Ext1` with bit 5 set.
//! A finger resting on the screen pulses INT, so it wakes the board again and again.
//! A level already LOW at sleep entry is rejected by the hardware; the example retries.
//!
//! The console is the S3's native USB-Serial-JTAG, which drops off the bus during deep sleep.
//! The example stays awake 3 s after each boot so the host can reopen the re-enumerated port.
//! Bench: `bench/s3/` (`sleep`, `touch-wake`, `r2`, `r3`, `r4`, `r4-guard`).
//!
//! ## Bench case R2 (build-time switches)
//!
//! `S3_R2=1` enables the R2 flow for the `try_sleep` post-rejection postcondition; unset, the example behaves as above.
//! `S3_SLEEP_TIMER_MS=<ms>` sets the wake timer (default 60000; the bench uses 5000).
//! `S3_REJECT_HOLD_MS=<ms>` sets the pin-read window after the forced rejection (default 8000).
//! With `S3_R2=1` it must be at least `S3_SLEEP_TIMER_MS` + 2000, or the build fails.
//! `S3_KEEP_AUTOHOLD=1` is only meaningful with `S3_R2=1`.
//! With it, forced cycles keep the global digital-pad hold effective instead of clearing it, so the library guard in `try_sleep` is what prevents the hang.
//! The `FORCED` line then reports `autohold=1`.
//!
//! A retained record in RTC memory (`.rtc_noinit`) holds a magic word, the cycle counter, and the RTC time of the last sleep entry.
//! Odd cycles and every `Ext1` wake boot are forced cycles; the others are plain timer cycles.
//! A forced cycle cuts `LCD_3V3` (GPIO1) and holds the touch reset (GPIO13) LOW so INT stays LOW and the S3 hardware rejects the sleep.
//! It disables the global digital-pad hold first, unless `S3_KEEP_AUTOHOLD=1`: with it enabled and no library guard, a rejected deep sleep hangs the S3 (`docs/project-lore.md`).
//! It then reads INT through the existing driver, re-reads the wake cause, and retries with a changed mask (GPIO0 + GPIO5).
//! Two seconds into the window it restores the rail, pulses the touch reset and re-arms the pad holds, which gives an `L->H` edge on INT.
//! The window then runs to its end and the normal retry loop resumes.
//!
//! Lines, all through `esp_rom_printf`; `t_ms` is ms since boot:
//!
//! - `S3BENCH tier=idf cycle=<n> reset=<r> cause=<c> mask=0x<m>` once per boot; the boot lines repeat after the 3 s awake time, because the host reopens the port too late for the first copy.
//! - `LASTSTAGE tier=idf stage=<n>`: the last forced-cycle stage the previous boot reached (0 = none), kept in RTC memory so a hang leaves a trace.
//! - `FORCED tier=idf int_before=<H|L> autohold=<0|1>`: INT right before the forced attempt, and whether the global digital-pad hold is effective.
//! - `SLEPT tier=idf ms=<n> forced=<0|1>` on a `Timer` wake: RTC-clock ms from the last sleep entry to this boot, and whether the cycle that slept was forced.
//! - `ATTEMPT tier=idf n=<k> t_ms=<n>` right before every `try_sleep`; the last one before a sleep is the origin of the recount.
//! - `PINREAD start|edge|end tier=idf pin=5 ...` as in the C3 R2 bench: `level=<H|L>`, `<H->L|L->H>`, and `edges=<n> high=<0|1> low=<0|1>`.
//! - `WAKECAUSE tier=idf boot=<cause> after_reject=<cause>` re-read after the rejection (`wake-cause-survives-rejection`).
//! - `MASKRETRY tier=idf result=<err:Rejected|err:other|ok>` for the changed-mask retry while INT is still LOW.
//!
//! ## Bench case R4 (non-wake pad postcondition)
//!
//! `S3_R4=1` adds the R4 checks on top of R2; it is only meaningful with `S3_R2=1` and is ignored without it.
//! Unset, the example behaves exactly as without R4.
//! It checks that a rejected `try_sleep` leaves every non-wake pad and every user pad hold as the caller left it.
//! Three probe pads are configured once per boot, before the forced attempt.
//! They are ESP32-S3 digital-only pads (GPIO26-48), believed unwired on the CrowPanel 1.28"; the schematic is not verified, so abort the bench if anything on the board reacts.
//!
//! - GPIO38: output HIGH, input enabled, user hold (`gpio_hold_en`; bit `38 - 21` of `RTC_CNTL_DIG_PAD_HOLD_REG`).
//! - GPIO39: output HIGH, input enabled, unheld.
//! - GPIO47: input with pull-up, unheld.
//!
//! GPIO38 keeps its hold across the real sleep that follows, because a user hold on a digital pad does not change the sleep decision.
//! The next boot releases and re-arms it.
//! In R4 forced cycles the GPIO1 and GPIO13 RTC holds stay enabled through the attempt: each pin goes `gpio_hold_dis`, LOW, `gpio_hold_en`.
//! INT still sits LOW, so the hardware (or the library guard with `S3_KEEP_AUTOHOLD=1`) rejects the sleep while the holds stay on.
//! The restore at 2 s is unchanged.
//!
//! Snapshot S0 is taken right before the first forced `try_sleep`, snapshot S1 right after it returns `Rejected`.
//! S1 follows the `PINREAD start` level sample, which stays the first read after the return; both are read-only.
//! Each snapshot holds, per pad (38, 39, 47, 1, 13), the IO_MUX word, the `FUNCn_OUT_SEL_CFG` word, the `GPIO_ENABLE` bit and the `GPIO_IN` bit.
//! It also holds `RTC_CNTL_DIG_PAD_HOLD_REG`, `RTC_CNTL_PAD_HOLD_REG` and `RTC_CNTL_DIG_ISO_REG`.
//! After S1 a hold test writes GPIO38 LOW while held (its `GPIO_IN` bit must stay HIGH) and GPIO1 HIGH while held (must stay LOW), then restores the commanded levels.
//! GPIO1 is an output without input enable, so the test sets `FUN_IE` in its IO_MUX word for the read and restores the word afterwards.
//!
//! - `NONWAKE tier=idf path=<hw|guard> same=<0|1> holdok=<0|1> diff=<pin:field,...|->` once per forced cycle: `path=guard` with `S3_KEEP_AUTOHOLD=1`, else `hw`; `same=1` when S0 equals S1; `diff` names the differing pads and fields (`mux`, `osel`, `en`, `in`) or registers (`dighold`, `padhold`, `iso`).
//! - `NONWAKE selftest tier=idf detect=<0|1> hold38=<0|1>` once per boot: the comparator must report a difference after GPIO39 `FUN_IE` is cleared; `hold38` is the GPIO38 hold bit after setup.
//!
//! ## Run
//!
//! ```shell
//! just run idf_s3_deep_sleep_gpio
//! ```
//!
//! For the unattended bench, build with the switches and capture with `just serial-capture`:
//!
//! ```shell
//! S3_R2=1 S3_SLEEP_TIMER_MS=5000 just run idf_s3_deep_sleep_gpio
//! ```

use std::ffi::CString;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{Input, Level, Output, PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::sys;
use rustyfarian_esp_idf_power::{
    Error, EspResetReasonSource, EspRtcClock, EspSleepManager, EspWakeCauseSource, GpioWakeLevel,
    ResetReasonSource, SleepErrorKind, SleepManager, WakeCause, WakeCauseSource, WakeSource,
    MAX_TIMER_MS,
};
use stoker::bench::{
    write_attempt, write_nonwake, write_pinread_edge, write_pinread_end, write_pinread_start,
    LineBuf, NonWakePath, PinWindow,
};

/// Tier tag in the shared `ATTEMPT` and `PINREAD` bench lines.
const TIER: &str = "idf";

/// CST816S touch INT pin; bit position in the wake mask.
const TOUCH_INT_PIN: u8 = 5;

/// `LCD_3V3` rail enable; the CST816S is powered from it.
const LCD_RAIL_PIN: i32 = 1;

/// CST816S reset, active low.
const TOUCH_RST_PIN: i32 = 13;

/// Time awake after each boot; the USB-Serial-JTAG port needs ~0.5-2 s to re-enumerate.
const AWAKE_MS: u32 = 3_000;

/// Second wake pin of the changed-mask retry: GPIO0 (BOOT button, pulled up on the board).
///
/// It is an RTC pad (0-21), so the mask validates; it is unused by the CrowPanel peripherals.
const MASKRETRY_PIN: u8 = 0;

/// Bench variant (`just bench-build NAME+VARIANT`, `bench/s3/`): `r2` is the R2 flow, `r3` keeps
/// the global pad hold on (the guard path), `r4` adds the non-wake pad probes, `r4-guard` is R4
/// with the hold kept on. Each uses the 5 s wake timer; the `S3_*` variables below still work.
const VARIANT: Option<usize> = variant_index(
    option_env!("BENCH_VARIANT"),
    &["r2", "r3", "r4", "r4-guard"],
);

/// Bench case R2 (`S3_R2=1` at build time, or any bench variant).
const R2: bool = VARIANT.is_some()
    || match option_env!("S3_R2") {
        Some(s) => s.len() == 1 && s.as_bytes()[0] == b'1',
        None => false,
    };

/// Bench case R3 (`S3_KEEP_AUTOHOLD=1` at build time, or the `r3` / `r4-guard` variants; only
/// meaningful with R2).
const KEEP_AUTOHOLD: bool = matches!(VARIANT, Some(1 | 3))
    || match option_env!("S3_KEEP_AUTOHOLD") {
        Some(s) => s.len() == 1 && s.as_bytes()[0] == b'1',
        None => false,
    };

/// Bench case R4 (`S3_R4=1` at build time, or the `r4` / `r4-guard` variants); only active together with R2.
const R4: bool = R2
    && (matches!(VARIANT, Some(2 | 3))
        || match option_env!("S3_R4") {
            Some(s) => s.len() == 1 && s.as_bytes()[0] == b'1',
            None => false,
        });

/// Wake timer in ms: `S3_SLEEP_TIMER_MS` at build time, else 5 s for a bench variant and 60 s otherwise.
const TIMER_MS: u64 = match option_env!("S3_SLEEP_TIMER_MS") {
    Some(s) => parse_u64(s),
    None if VARIANT.is_some() => 5_000,
    None => 60_000,
};

// An out-of-range override would fail validation on every boot; fail the build instead.
const _: () = assert!(
    TIMER_MS >= 1 && TIMER_MS <= MAX_TIMER_MS,
    "S3_SLEEP_TIMER_MS must be between 1 and MAX_TIMER_MS"
);

/// Pin-read window after the forced rejection in ms: `S3_REJECT_HOLD_MS` at build time, else 8 s.
const REJECT_HOLD_MS: u64 = match option_env!("S3_REJECT_HOLD_MS") {
    Some(s) => parse_u64(s),
    None => 8_000,
};

// The window must outlast the wake timer so the retry that follows it can show the timer recount.
// Gated on R2: the defaults (60 s timer, 8 s window) would violate it.
const _: () = assert!(
    !R2 || REJECT_HOLD_MS >= TIMER_MS + 2000,
    "S3_REJECT_HOLD_MS must be at least S3_SLEEP_TIMER_MS + 2000 when S3_R2=1"
);

/// Poll period of the rejection-phase pin read.
const PINREAD_PERIOD_MS: u32 = 10;

/// Time into the window at which the rail, reset and pad holds are restored.
const RESTORE_AT_MS: u64 = 2_000;

/// Delay between rejected `try_sleep` attempts.
const RETRY_DELAY_MS: u32 = 50;

/// Marks the retained record as written by this firmware (power-on RTC memory is arbitrary).
const R2_MAGIC: u32 = 0x5233_5333;

/// Bench record kept across deep sleep; plain atomics, so no `critical-section` provider is needed.
///
/// `.rtc_noinit` is a `NOLOAD` section: the initialiser never runs and the bytes survive resets.
/// On the S3 it lands in RTC slow memory, which ESP-IDF keeps powered in deep sleep.
struct R2Record {
    magic: AtomicU32,
    cycle: AtomicU32,
    /// RTC clock in ms (wrapping) right before the last `try_sleep`.
    entry_ms: AtomicU32,
    entry_valid: AtomicU32,
    /// Whether the cycle that stored `entry_ms` was forced.
    entry_forced: AtomicU32,
    /// Debug breadcrumb: last forced-cycle stage reached; printed and cleared at boot.
    stage: AtomicU32,
}

#[link_section = ".rtc_noinit"]
static R2_RECORD: R2Record = R2Record {
    magic: AtomicU32::new(0),
    cycle: AtomicU32::new(0),
    entry_ms: AtomicU32::new(0),
    entry_valid: AtomicU32::new(0),
    entry_forced: AtomicU32::new(0),
    stage: AtomicU32::new(0),
};

/// Records the forced-cycle stage in RTC memory so a hang that resets the chip leaves a trace.
fn stage(n: u32) {
    R2_RECORD.stage.store(n, Ordering::Relaxed);
}

/// RTC clock in ms, wrapping at 2^32 (~49 days), which is fine for differences.
fn rtc_ms() -> u32 {
    (EspRtcClock::now_us() / 1000) as u32
}

use stoker::bench::{parse_u64, variant_index};

/// Milliseconds since boot.
fn now_ms() -> u64 {
    // SAFETY: esp_timer_get_time has no preconditions once the system is up.
    (unsafe { sys::esp_timer_get_time() } / 1000) as u64
}

fn level_char(level: Level) -> char {
    match level {
        Level::High => 'H',
        Level::Low => 'L',
    }
}

fn cause_name(cause: WakeCause) -> (&'static str, u64) {
    match cause {
        WakeCause::PowerOn => ("PowerOn", 0),
        WakeCause::Timer => ("Timer", 0),
        WakeCause::Gpio(mask) => ("Gpio", mask.0),
        WakeCause::Ext1(mask) => ("Ext1", mask.0),
        WakeCause::Ext0 => ("Ext0", 0),
        WakeCause::Touch => ("Touch", 0),
        _ => ("Other", 0),
    }
}

/// Prints `ATTEMPT` and stores the sleep-entry time; call right before every bench `try_sleep`.
fn mark_attempt(attempt: u32, forced: bool) {
    let mut line = LineBuf::<64>::new();
    let _ = write_attempt(&mut line, TIER, attempt, now_ms());
    say_line(&line);
    R2_RECORD
        .entry_forced
        .store(forced as u32, Ordering::Relaxed);
    R2_RECORD.entry_ms.store(rtc_ms(), Ordering::Relaxed);
    R2_RECORD.entry_valid.store(1, Ordering::Relaxed);
}

/// Probe pad: digital-only, output HIGH, input enabled, user hold.
const PROBE_HELD_PIN: u8 = 38;

/// Probe pad: digital-only, output HIGH, input enabled, unheld.
const PROBE_OUT_PIN: u8 = 39;

/// Probe pad: digital-only, input with pull-up, unheld.
const PROBE_IN_PIN: u8 = 47;

/// Pads in a snapshot: the three probes, then the two RTC-held touch pins.
const SNAP_PINS: [u8; 5] = [
    PROBE_HELD_PIN,
    PROBE_OUT_PIN,
    PROBE_IN_PIN,
    LCD_RAIL_PIN as u8,
    TOUCH_RST_PIN as u8,
];

/// Settle time between a write and the `GPIO_IN` read-back of the hold test.
const HOLD_TEST_SETTLE_MS: u32 = 1;

/// `DR_REG_GPIO_BASE` on the ESP32-S3 (`soc/esp32s3/include/soc/reg_base.h`).
const GPIO_BASE: usize = 0x6000_4000;
/// `GPIO_ENABLE_REG` (pads 0-31) and `GPIO_ENABLE1_REG` (pads 32-48), `gpio_reg.h`.
const GPIO_ENABLE_REG: usize = GPIO_BASE + 0x20;
const GPIO_ENABLE1_REG: usize = GPIO_BASE + 0x2C;
/// `GPIO_IN_REG` (pads 0-31) and `GPIO_IN1_REG` (pads 32-48), `gpio_reg.h`.
const GPIO_IN_REG: usize = GPIO_BASE + 0x3C;
const GPIO_IN1_REG: usize = GPIO_BASE + 0x40;
/// `GPIO_FUNC0_OUT_SEL_CFG_REG`; `GPIO_FUNCn_OUT_SEL_CFG_REG` is this plus 4 * n, `gpio_reg.h`.
const GPIO_FUNC0_OUT_SEL_CFG_REG: usize = GPIO_BASE + 0x554;
/// `DR_REG_IO_MUX_BASE`; the word of GPIOn sits at this plus 4 * (n + 1), `io_mux_reg.h`.
const IO_MUX_BASE: usize = 0x6000_9000;
/// `FUN_IE`: input enable in an IO_MUX word (`io_mux_reg.h`).
const IO_MUX_FUN_IE: u32 = 1 << 9;
/// `DR_REG_RTCCNTL_BASE` (`reg_base.h`).
const RTC_CNTL_BASE: usize = 0x6000_8000;
/// `RTC_CNTL_DIG_ISO_REG`, `RTC_CNTL_PAD_HOLD_REG` and `RTC_CNTL_DIG_PAD_HOLD_REG` (`rtc_cntl_reg.h`).
const RTC_CNTL_DIG_ISO_REG: usize = RTC_CNTL_BASE + 0x94;
/// `RTC_CNTL_WIFI_FORCE_ISO` (bit 28 of `RTC_CNTL_DIG_ISO_REG`): a WiFi/BLE power-domain bit, not
/// pad state; esp-hal sleep entry clears it and a rejection does not restore it, so the R4 comparison
/// masks it out, as ADR-003 point 1 excludes power-domain configuration.
const DIG_ISO_WIFI_FORCE_ISO: u32 = 1 << 28;
const RTC_CNTL_PAD_HOLD_REG: usize = RTC_CNTL_BASE + 0xD8;
const RTC_CNTL_DIG_PAD_HOLD_REG: usize = RTC_CNTL_BASE + 0xDC;
/// The digital-pad hold register starts at GPIO21 (`gpio_ll_is_digital_io_hold`: `BIT(gpio - 21)`).
const DIG_PAD_HOLD_FIRST_GPIO: u8 = 21;

/// Reads a memory-mapped register.
fn reg_read(addr: usize) -> u32 {
    // SAFETY: only called with the always-mapped GPIO, IO_MUX and RTC_CNTL register addresses above; a volatile 32-bit read has no side effects.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

/// Writes a memory-mapped register.
fn reg_write(addr: usize, value: u32) {
    // SAFETY: only called with IO_MUX words of the pads this bench owns; a volatile 32-bit write of a word read just before.
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) };
}

/// IO_MUX word address of a pad.
const fn io_mux_reg(pin: u8) -> usize {
    IO_MUX_BASE + 4 * (pin as usize + 1)
}

/// `GPIO_FUNCn_OUT_SEL_CFG_REG` address of a pad.
const fn out_sel_reg(pin: u8) -> usize {
    GPIO_FUNC0_OUT_SEL_CFG_REG + 4 * pin as usize
}

/// Bit of a pad in a bank register, and the bank register to use (pads 0-31 / 32-48).
fn bank(pin: u8, low: usize, high: usize) -> (usize, u32) {
    if pin < 32 {
        (low, 1 << pin)
    } else {
        (high, 1 << (pin - 32))
    }
}

/// `GPIO_IN` level of a pad (reads LOW while the pad's input is not enabled).
fn gpio_in(pin: u8) -> bool {
    let (reg, bit) = bank(pin, GPIO_IN_REG, GPIO_IN1_REG);
    reg_read(reg) & bit != 0
}

/// `GPIO_ENABLE` (output enable) of a pad.
fn gpio_enabled(pin: u8) -> bool {
    let (reg, bit) = bank(pin, GPIO_ENABLE_REG, GPIO_ENABLE1_REG);
    reg_read(reg) & bit != 0
}

/// Sets or clears `FUN_IE` in the IO_MUX word of a pad and returns the previous word.
fn set_input_enable(pin: u8, on: bool) -> u32 {
    let word = reg_read(io_mux_reg(pin));
    let new = if on {
        word | IO_MUX_FUN_IE
    } else {
        word & !IO_MUX_FUN_IE
    };
    reg_write(io_mux_reg(pin), new);
    word
}

/// Register state of one snapshotted pad.
#[derive(Clone, Copy)]
struct PinSnap {
    iomux: u32,
    out_sel: u32,
    enabled: bool,
    level: bool,
}

/// Register state of the probe and touch pads plus the global hold and isolation registers.
#[derive(Clone, Copy)]
struct Snap {
    pins: [PinSnap; SNAP_PINS.len()],
    dig_pad_hold: u32,
    pad_hold: u32,
    dig_iso: u32,
}

impl Snap {
    /// Reads every snapshot register; read-only.
    fn take() -> Self {
        Snap {
            pins: SNAP_PINS.map(|pin| PinSnap {
                iomux: reg_read(io_mux_reg(pin)),
                out_sel: reg_read(out_sel_reg(pin)),
                enabled: gpio_enabled(pin),
                level: gpio_in(pin),
            }),
            dig_pad_hold: reg_read(RTC_CNTL_DIG_PAD_HOLD_REG),
            pad_hold: reg_read(RTC_CNTL_PAD_HOLD_REG),
            dig_iso: reg_read(RTC_CNTL_DIG_ISO_REG),
        }
    }

    /// Writes the differing fields to `out` as `pin:field,...`; returns whether anything differs.
    fn diff(&self, other: &Snap, out: &mut LineBuf<96>) -> bool {
        let mut push = |name: core::fmt::Arguments<'_>| {
            if !out.as_str().is_empty() {
                let _ = out.write_str(",");
            }
            let _ = out.write_fmt(name);
        };
        for (i, pin) in SNAP_PINS.iter().enumerate() {
            let (a, b) = (&self.pins[i], &other.pins[i]);
            if a.iomux != b.iomux {
                push(format_args!("{pin}:mux"));
            }
            if a.out_sel != b.out_sel {
                push(format_args!("{pin}:osel"));
            }
            if a.enabled != b.enabled {
                push(format_args!("{pin}:en"));
            }
            if a.level != b.level {
                push(format_args!("{pin}:in"));
            }
        }
        if self.dig_pad_hold != other.dig_pad_hold {
            push(format_args!("dighold"));
        }
        if self.pad_hold != other.pad_hold {
            push(format_args!("padhold"));
        }
        if (self.dig_iso ^ other.dig_iso) & !DIG_ISO_WIFI_FORCE_ISO != 0 {
            push(format_args!("iso"));
        }
        !out.as_str().is_empty()
    }
}

/// The three R4 probe drivers; they live until the next sleep, which keeps the pads configured.
struct R4Probes {
    held: PinDriver<'static, Output>,
    _out: PinDriver<'static, Output>,
    _input: PinDriver<'static, Input>,
}

/// Configures the probe pads once per boot and runs the comparator self-test.
fn r4_setup(
    held: PinDriver<'static, Output>,
    out: PinDriver<'static, Output>,
    input: PinDriver<'static, Input>,
) -> anyhow::Result<R4Probes> {
    let mut probes = R4Probes {
        held,
        _out: out,
        _input: input,
    };
    let held_pin = i32::from(PROBE_HELD_PIN);
    // SAFETY: gpio_hold_dis/gpio_hold_en only change the hold latch of a valid output-capable GPIO.
    unsafe { sys::gpio_hold_dis(held_pin) };
    probes.held.set_high()?;
    probes._out.set_high()?;
    set_input_enable(PROBE_HELD_PIN, true);
    set_input_enable(PROBE_OUT_PIN, true);
    // SAFETY: as above; the pad is digital-only, so this sets its bit in RTC_CNTL_DIG_PAD_HOLD_REG.
    unsafe { sys::gpio_hold_en(held_pin) };
    FreeRtos::delay_ms(HOLD_TEST_SETTLE_MS);

    let before = Snap::take();
    let original = set_input_enable(PROBE_OUT_PIN, false);
    let after = Snap::take();
    reg_write(io_mux_reg(PROBE_OUT_PIN), original);
    let mut diff = LineBuf::<96>::new();
    let detect = before.diff(&after, &mut diff);
    let hold_bit = 1u32 << (PROBE_HELD_PIN - DIG_PAD_HOLD_FIRST_GPIO);
    let mut line = LineBuf::<96>::new();
    let _ = write!(
        line,
        "NONWAKE selftest tier={TIER} detect={} hold38={}",
        u8::from(detect),
        u8::from(before.dig_pad_hold & hold_bit != 0)
    );
    say_line(&line);
    Ok(probes)
}

/// Hold test: a write to a held pad must not reach it; restores the commanded levels afterwards.
///
/// GPIO38 is written LOW and must keep reading HIGH; GPIO1 is written HIGH and must keep reading LOW.
/// GPIO38 has input enabled; GPIO1 does not, so its `FUN_IE` is set for the read and restored.
fn hold_test(
    probes: &mut R4Probes,
    lcd_rail: &mut PinDriver<'static, Output>,
) -> anyhow::Result<bool> {
    probes.held.set_low()?;
    FreeRtos::delay_ms(HOLD_TEST_SETTLE_MS);
    let held_kept = gpio_in(PROBE_HELD_PIN);
    probes.held.set_high()?;

    let rail_pin = LCD_RAIL_PIN as u8;
    let original = set_input_enable(rail_pin, true);
    lcd_rail.set_high()?;
    FreeRtos::delay_ms(HOLD_TEST_SETTLE_MS);
    let rail_kept = !gpio_in(rail_pin);
    lcd_rail.set_low()?;
    reg_write(io_mux_reg(rail_pin), original);
    Ok(held_kept && rail_kept)
}

/// Re-latches a held pin LOW without ever leaving it unheld: `gpio_hold_dis`, LOW, `gpio_hold_en`.
fn relatch_low(pin: &mut PinDriver<'static, Output>, gpio: i32) -> anyhow::Result<()> {
    // SAFETY: gpio_hold_dis/gpio_hold_en only change the hold latch of a valid RTC GPIO.
    unsafe { sys::gpio_hold_dis(gpio) };
    pin.set_low()?;
    // SAFETY: as above.
    unsafe { sys::gpio_hold_en(gpio) };
    Ok(())
}

/// Restores the touch power: rail high, reset pulse, pad holds re-armed (the cold-boot sequence).
fn restore_touch_power(
    lcd_rail: &mut PinDriver<'static, Output>,
    touch_rst: &mut PinDriver<'static, Output>,
) -> anyhow::Result<()> {
    // SAFETY: gpio_hold_dis only clears the hold latch of a valid GPIO number.
    unsafe {
        sys::gpio_hold_dis(LCD_RAIL_PIN);
        sys::gpio_hold_dis(TOUCH_RST_PIN);
    }
    lcd_rail.set_high()?;
    FreeRtos::delay_ms(20);
    touch_rst.set_low()?;
    FreeRtos::delay_ms(10);
    touch_rst.set_high()?;
    FreeRtos::delay_ms(50);
    // SAFETY: as at boot, gpio_hold_en latches the current output level of a valid RTC GPIO.
    unsafe {
        sys::gpio_hold_en(LCD_RAIL_PIN);
        sys::gpio_hold_en(TOUCH_RST_PIN);
        sys::gpio_deep_sleep_hold_en();
    }
    Ok(())
}

/// Forced-cycle window after the first `Rejected`: pin read, wake-cause re-read, changed-mask retry.
///
/// Runs `REJECT_HOLD_MS` from the rejection and restores the touch power `RESTORE_AT_MS` in.
fn rejection_window(
    manager: &mut EspSleepManager,
    touch_int: &PinDriver<'static, Input>,
    lcd_rail: &mut PinDriver<'static, Output>,
    touch_rst: &mut PinDriver<'static, Output>,
    boot_cause: WakeCause,
    attempt: &mut u32,
    r4: Option<(&mut R4Probes, &Snap)>,
) -> anyhow::Result<()> {
    let start_level = touch_int.get_level();
    stage(5);
    let start_ms = now_ms();
    let mut line = LineBuf::<192>::new();
    let mut window = PinWindow::start(start_level == Level::High, start_ms);
    let _ = write_pinread_start(
        &mut line,
        TIER,
        TOUCH_INT_PIN,
        start_level == Level::High,
        start_ms,
    );
    say_line(&line);

    // R4: S1 is a read-only snapshot taken after the PINREAD start sample, so that sample stays the first read after the return.
    if let Some((probes, s0)) = r4 {
        let s1 = Snap::take();
        let holdok = hold_test(probes, lcd_rail)?;
        let mut diff = LineBuf::<96>::new();
        let differs = s0.diff(&s1, &mut diff);
        let path = if KEEP_AUTOHOLD {
            NonWakePath::Guard
        } else {
            NonWakePath::Hw
        };
        line.clear();
        let _ = write_nonwake(
            &mut line,
            TIER,
            path,
            !differs,
            holdok,
            if differs { diff.as_str() } else { "-" },
        );
        say_line(&line);
    }

    let after_reject = EspWakeCauseSource.last_wake_cause();
    stage(6);
    say(&format!(
        "WAKECAUSE tier=idf boot={boot_cause:?} after_reject={after_reject:?}"
    ));

    let retry_sources = [
        WakeSource::GpioLevel {
            pin_mask: (1 << TOUCH_INT_PIN) | (1 << MASKRETRY_PIN),
            level: GpioWakeLevel::AnyLow,
        },
        WakeSource::Timer {
            duration_ms: TIMER_MS,
        },
    ];
    *attempt += 1;
    mark_attempt(*attempt, true);
    stage(7);
    let result = match manager.try_sleep(&retry_sources) {
        Err(e) if e.kind() == SleepErrorKind::Rejected => "err:Rejected",
        Err(_) => "err:other",
        Ok(()) => "ok",
    };
    stage(8);
    say(&format!("MASKRETRY tier=idf result={result}"));

    let mut restored = false;
    while now_ms() - start_ms < REJECT_HOLD_MS {
        FreeRtos::delay_ms(PINREAD_PERIOD_MS);
        if !restored && now_ms() - start_ms >= RESTORE_AT_MS {
            stage(9);
            restore_touch_power(lcd_rail, touch_rst)?;
            stage(10);
            restored = true;
        }
        let level = touch_int.get_level() == Level::High;
        if let Some(edge) = window.sample(level, now_ms()) {
            line.clear();
            let _ = write_pinread_edge(&mut line, TIER, TOUCH_INT_PIN, &edge);
            say_line(&line);
        }
    }
    line.clear();
    let _ = write_pinread_end(&mut line, TIER, TOUCH_INT_PIN, &window.finish(), now_ms());
    say_line(&line);
    Ok(())
}

/// Print a line via `esp_rom_printf`; Rust `println!` does not reach the S3 console
/// (`docs/project-lore.md` → ESP-IDF / Rust on ESP32-S3).
fn say(msg: &str) {
    let Ok(c) = CString::new(msg) else { return };
    // SAFETY: `c` is a NUL-terminated string alive for the call, `c"%s\n"` a static format.
    unsafe { sys::esp_rom_printf(c"%s\n".as_ptr(), c.as_ptr()) };
}

/// Prints a [`LineBuf`] via `esp_rom_printf` without allocating.
fn say_line<const N: usize>(line: &LineBuf<N>) {
    // SAFETY: `as_c_bytes` is NUL-terminated and alive for the call, `c"%s\n"` a static format.
    unsafe { sys::esp_rom_printf(c"%s\n".as_ptr(), line.as_c_bytes().as_ptr()) };
}

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches must run before any ESP-IDF API call.
    sys::link_patches();

    // Read the wake cause first: a later sleep attempt can clear the hardware wake status.
    let wake_cause = EspWakeCauseSource.last_wake_cause();
    say(&format!("Wake cause: {wake_cause:?}"));

    // Bench R2: load or reset the retained record, then report the boot and the last sleep.
    let mut forced = false;
    // Boot lines, printed again after the awake delay: the host reopens the
    // re-enumerated USB-Serial-JTAG port too late to see the first copy.
    let mut boot_lines: Vec<String> = Vec::new();
    if R2 {
        let reset = EspResetReasonSource.last_reset_reason();
        let record = &R2_RECORD;
        if record.magic.load(Ordering::Relaxed) == R2_MAGIC {
            record.cycle.fetch_add(1, Ordering::Relaxed);
        } else {
            record.magic.store(R2_MAGIC, Ordering::Relaxed);
            record.cycle.store(0, Ordering::Relaxed);
            record.entry_valid.store(0, Ordering::Relaxed);
        }
        let cycle = record.cycle.load(Ordering::Relaxed);
        let (cause_str, mask) = cause_name(wake_cause);
        boot_lines.push(format!(
            "S3BENCH tier=idf cycle={cycle} reset={reset:?} cause={cause_str} mask=0x{mask:x}"
        ));
        if cause_str == "Timer" && record.entry_valid.load(Ordering::Relaxed) == 1 {
            let slept = rtc_ms().wrapping_sub(record.entry_ms.load(Ordering::Relaxed));
            boot_lines.push(format!(
                "SLEPT tier=idf ms={slept} forced={}",
                record.entry_forced.load(Ordering::Relaxed)
            ));
        }
        boot_lines.push(format!(
            "LASTSTAGE tier=idf stage={}",
            record.stage.load(Ordering::Relaxed)
        ));
        boot_lines.iter().for_each(|line| say(line));
        record.stage.store(0, Ordering::Relaxed);
        record.entry_valid.store(0, Ordering::Relaxed);
        forced = cycle % 2 == 1 || cause_str == "Ext1";
    }

    let peripherals = Peripherals::take()?;

    // Touch power: drive the levels before releasing the hold so the rail does not glitch
    // after a wake; only a cold boot gets the reset pulse. The drivers live until sleep.
    let mut lcd_rail = PinDriver::output(peripherals.pins.gpio1)?;
    let mut touch_rst = PinDriver::output(peripherals.pins.gpio13)?;
    lcd_rail.set_high()?;
    touch_rst.set_high()?;
    // SAFETY: gpio_hold_dis only clears the hold latch of a valid GPIO number.
    unsafe {
        sys::gpio_hold_dis(LCD_RAIL_PIN);
        sys::gpio_hold_dis(TOUCH_RST_PIN);
    }
    // SAFETY: esp_reset_reason only reads a value latched at boot.
    if unsafe { sys::esp_reset_reason() } != sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP {
        FreeRtos::delay_ms(20);
        touch_rst.set_low()?;
        FreeRtos::delay_ms(10);
        touch_rst.set_high()?;
        FreeRtos::delay_ms(50);
    }
    // SAFETY: gpio_hold_en latches the current output level of a valid RTC GPIO; with
    // gpio_deep_sleep_hold_en the latch survives deep sleep.
    unsafe {
        sys::gpio_hold_en(LCD_RAIL_PIN);
        sys::gpio_hold_en(TOUCH_RST_PIN);
        sys::gpio_deep_sleep_hold_en();
    }

    // The CST816S drives INT; the knob reads it without a pull, and so does this example.
    let touch_int = PinDriver::input(peripherals.pins.gpio5, Pull::Floating)?;

    // GPIO wake with isolation is refused (IsolationConflict).
    let mut manager = EspSleepManager {
        isolate_gpio: false,
    };
    let sources = [
        WakeSource::GpioLevel {
            pin_mask: 1 << TOUCH_INT_PIN,
            level: GpioWakeLevel::AnyLow,
        },
        WakeSource::Timer {
            duration_ms: TIMER_MS,
        },
    ];

    FreeRtos::delay_ms(AWAKE_MS);
    boot_lines.iter().for_each(|line| say(line));
    // R4: configure the probe pads once per boot, before the forced attempt.
    let mut probes = if R4 {
        Some(r4_setup(
            PinDriver::output(peripherals.pins.gpio38)?,
            PinDriver::output(peripherals.pins.gpio39)?,
            PinDriver::input(peripherals.pins.gpio47, Pull::Up)?,
        )?)
    } else {
        None
    };
    // Forced cycle: cut the touch rail so INT stays LOW and the hardware rejects the sleep.
    if R2 && forced {
        stage(1);
        // R4 keeps the RTC holds on through the attempt and re-latches the pins LOW instead.
        if !R4 {
            // SAFETY: gpio_hold_dis only clears the hold latch of a valid GPIO number.
            unsafe {
                sys::gpio_hold_dis(LCD_RAIL_PIN);
                sys::gpio_hold_dis(TOUCH_RST_PIN);
            }
        }
        // SAFETY: gpio_deep_sleep_hold_en/dis only toggle the global digital-pad hold enable.
        unsafe {
            // A rejected deep sleep hangs the S3 while the global digital-pad hold is enabled:
            // sleep entry isolates every unheld digital pad, the flash pads included, and the
            // rejection does not restore them (docs/project-lore.md). Re-enabled on restore.
            // S3_KEEP_AUTOHOLD=1 keeps it enabled so the library guard has to prevent the hang.
            if KEEP_AUTOHOLD {
                sys::gpio_deep_sleep_hold_en();
            } else {
                sys::gpio_deep_sleep_hold_dis();
            }
        }
        // Reset LOW too, so the unpowered controller is not back-fed through its reset pin.
        if R4 {
            relatch_low(&mut touch_rst, TOUCH_RST_PIN)?;
            relatch_low(&mut lcd_rail, LCD_RAIL_PIN)?;
        } else {
            touch_rst.set_low()?;
            lcd_rail.set_low()?;
        }
        FreeRtos::delay_ms(100);
        stage(2);
        say(&format!(
            "FORCED tier=idf int_before={} autohold={}",
            level_char(touch_int.get_level()),
            u8::from(KEEP_AUTOHOLD)
        ));
    }
    let mut window_pending = R2 && forced;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        if R2 {
            mark_attempt(attempt, forced);
        }
        if window_pending {
            stage(3);
        }
        // R4: S0 immediately before the first forced attempt.
        let s0 = (R4 && window_pending).then(Snap::take);
        // On success the chip is asleep and this call does not return.
        match manager.try_sleep(&sources) {
            // INT is still LOW; try again shortly.
            Err(e) if e.kind() == SleepErrorKind::Rejected => {
                if window_pending {
                    stage(4);
                    window_pending = false;
                    rejection_window(
                        &mut manager,
                        &touch_int,
                        &mut lcd_rail,
                        &mut touch_rst,
                        wake_cause,
                        &mut attempt,
                        probes.as_mut().zip(s0.as_ref()),
                    )?;
                }
                FreeRtos::delay_ms(RETRY_DELAY_MS)
            }
            Err(e) => anyhow::bail!("try_sleep failed: {e}"),
            Ok(()) => anyhow::bail!("try_sleep returned without sleeping"),
        }
    }
}
