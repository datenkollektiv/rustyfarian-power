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
//! Bench results: `docs/bench/s3-crowpanel-touch-sleep-bench-steps-v1.md`.
//!
//! ## Bench case R2 (build-time switches)
//!
//! `S3_R2=1` enables the R2 flow for the `try_sleep` post-rejection postcondition; unset, the example behaves as above.
//! `S3_SLEEP_TIMER_MS=<ms>` sets the wake timer (default 60000; the bench uses 5000).
//! `S3_REJECT_HOLD_MS=<ms>` sets the pin-read window after the forced rejection (default 8000).
//! With `S3_R2=1` it must be at least `S3_SLEEP_TIMER_MS` + 2000, or the build fails.
//!
//! A retained record in RTC memory (`.rtc_noinit`) holds a magic word, the cycle counter, and the RTC time of the last sleep entry.
//! Odd cycles and every `Ext1` wake boot are forced cycles; the others are plain timer cycles.
//! A forced cycle cuts `LCD_3V3` (GPIO1) and holds the touch reset (GPIO13) LOW so INT stays LOW and the S3 hardware rejects the sleep.
//! It disables the global digital-pad hold first: with it enabled, a rejected deep sleep hangs the S3 (`docs/project-lore.md`).
//! It then reads INT through the existing driver, re-reads the wake cause, and retries with a changed mask (GPIO0 + GPIO5).
//! Two seconds into the window it restores the rail, pulses the touch reset and re-arms the pad holds, which gives an `L->H` edge on INT.
//! The window then runs to its end and the normal retry loop resumes.
//!
//! Lines, all through `esp_rom_printf`; `t_ms` is ms since boot:
//!
//! - `S3BENCH tier=idf cycle=<n> reset=<r> cause=<c> mask=0x<m>` once per boot; the boot lines repeat after the 3 s awake time, because the host reopens the port too late for the first copy.
//! - `LASTSTAGE tier=idf stage=<n>`: the last forced-cycle stage the previous boot reached (0 = none), kept in RTC memory so a hang leaves a trace.
//! - `FORCED tier=idf int_before=<H|L>`: INT right before the forced attempt.
//! - `SLEPT tier=idf ms=<n> forced=<0|1>` on a `Timer` wake: RTC-clock ms from the last sleep entry to this boot, and whether the cycle that slept was forced.
//! - `ATTEMPT tier=idf n=<k> t_ms=<n>` right before every `try_sleep`; the last one before a sleep is the origin of the recount.
//! - `PINREAD start|edge|end tier=idf pin=5 ...` as in the C3 R2 bench: `level=<H|L>`, `<H->L|L->H>`, and `edges=<n> high=<0|1> low=<0|1>`.
//! - `WAKECAUSE tier=idf boot=<cause> after_reject=<cause>` re-read after the rejection (`wake-cause-survives-rejection`).
//! - `MASKRETRY tier=idf result=<err:Rejected|err:other|ok>` for the changed-mask retry while INT is still LOW.
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

/// Bench case R2 (`S3_R2=1` at build time).
const R2: bool = match option_env!("S3_R2") {
    Some(s) => s.len() == 1 && s.as_bytes()[0] == b'1',
    None => false,
};

/// Wake timer in ms: `S3_SLEEP_TIMER_MS` at build time, else 60 s.
const TIMER_MS: u64 = match option_env!("S3_SLEEP_TIMER_MS") {
    Some(s) => parse_u64(s),
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

/// Decimal parser usable in a const context; fails the build on anything but digits.
const fn parse_u64(s: &str) -> u64 {
    let b = s.as_bytes();
    assert!(!b.is_empty(), "S3_* build variables must not be empty");
    let mut i = 0;
    let mut v: u64 = 0;
    while i < b.len() {
        assert!(
            b[i].is_ascii_digit(),
            "S3_* build variables must be decimal digits"
        );
        v = v * 10 + (b[i] - b'0') as u64;
        i += 1;
    }
    v
}

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
    say(&format!("ATTEMPT tier=idf n={attempt} t_ms={}", now_ms()));
    R2_RECORD
        .entry_forced
        .store(forced as u32, Ordering::Relaxed);
    R2_RECORD.entry_ms.store(rtc_ms(), Ordering::Relaxed);
    R2_RECORD.entry_valid.store(1, Ordering::Relaxed);
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
) -> anyhow::Result<()> {
    let start_level = touch_int.get_level();
    stage(5);
    let start_ms = now_ms();
    say(&format!(
        "PINREAD start tier=idf pin={TOUCH_INT_PIN} level={} t_ms={start_ms}",
        level_char(start_level)
    ));

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

    let mut last = start_level;
    let mut edges: u32 = 0;
    let mut high = start_level == Level::High;
    let mut low = start_level == Level::Low;
    let mut restored = false;
    while now_ms() - start_ms < REJECT_HOLD_MS {
        FreeRtos::delay_ms(PINREAD_PERIOD_MS);
        if !restored && now_ms() - start_ms >= RESTORE_AT_MS {
            stage(9);
            restore_touch_power(lcd_rail, touch_rst)?;
            stage(10);
            restored = true;
        }
        let level = touch_int.get_level();
        high |= level == Level::High;
        low |= level == Level::Low;
        if level != last {
            edges += 1;
            say(&format!(
                "PINREAD edge tier=idf pin={TOUCH_INT_PIN} {}->{} t_ms={}",
                level_char(last),
                level_char(level),
                now_ms()
            ));
            last = level;
        }
    }
    say(&format!(
        "PINREAD end tier=idf pin={TOUCH_INT_PIN} edges={edges} high={} low={} t_ms={}",
        high as u8,
        low as u8,
        now_ms()
    ));
    Ok(())
}

/// Print a line via `esp_rom_printf`; Rust `println!` does not reach the S3 console
/// (`docs/project-lore.md` → ESP-IDF / Rust on ESP32-S3).
fn say(msg: &str) {
    let Ok(c) = CString::new(msg) else { return };
    // SAFETY: `c` is a NUL-terminated string alive for the call, `c"%s\n"` a static format.
    unsafe { sys::esp_rom_printf(c"%s\n".as_ptr(), c.as_ptr()) };
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
    // Forced cycle: cut the touch rail so INT stays LOW and the hardware rejects the sleep.
    if R2 && forced {
        stage(1);
        // SAFETY: gpio_hold_dis only clears the hold latch of a valid GPIO number.
        unsafe {
            sys::gpio_hold_dis(LCD_RAIL_PIN);
            sys::gpio_hold_dis(TOUCH_RST_PIN);
            // A rejected deep sleep hangs the S3 while the global digital-pad hold is enabled:
            // sleep entry isolates every unheld digital pad, the flash pads included, and the
            // rejection does not restore them (docs/project-lore.md). Re-enabled on restore.
            sys::gpio_deep_sleep_hold_dis();
        }
        // Reset LOW too, so the unpowered controller is not back-fed through its reset pin.
        touch_rst.set_low()?;
        lcd_rail.set_low()?;
        FreeRtos::delay_ms(100);
        stage(2);
        say(&format!(
            "FORCED tier=idf int_before={}",
            level_char(touch_int.get_level())
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
                    )?;
                }
                FreeRtos::delay_ms(RETRY_DELAY_MS)
            }
            Err(e) => anyhow::bail!("try_sleep failed: {e}"),
            Ok(()) => anyhow::bail!("try_sleep returned without sleeping"),
        }
    }
}
