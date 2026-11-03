//! CrowPanel 1.28" ESP32-S3 — Touch Deep-Sleep GPIO Wake Example (esp-hal)
//!
//! Deep-sleeps until the CST816S touch controller pulls INT (GPIO5) low or a 60 s timer fires,
//! prints the wake cause, and sleeps again.
//!
//! ## Board
//!
//! - **Board:** Elecrow CrowPanel 1.28" HMI ESP32-S3 Rotary Display.
//! - **Wake pin:** CST816S touch INT on GPIO5, active-low, a low-power pad (GPIO0-21).
//!   The encoder and button (GPIO45/42/41) are not low-power pads and cannot wake the chip.
//! - **Touch power:** the CST816S needs `LCD_3V3` (GPIO1) on and its reset (GPIO13) released;
//!   without them INT sits LOW and every sleep is rejected.
//!   Both pins are held HIGH through deep sleep so the controller keeps watching for touches.
//!
//! On the S3 esp-hal arms the pin through EXT1, so a touch reports `WakeCause::Ext1` with bit 5
//! set. A finger resting on the screen pulses INT, so it wakes the board again and again.
//! A level already LOW at sleep entry is rejected by the hardware; the example retries.
//!
//! The console is the S3's native USB-Serial-JTAG (`esp-println`, feature `auto`), which drops
//! off the bus during deep sleep.
//! The example stays awake 3 s after each boot so the host can reopen the re-enumerated port.
//! Bench results: `docs/bench/s3-crowpanel-touch-sleep-bench-steps-v1.md`.
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
//! A retained record in RTC memory (`.rtc_fast.persistent`) holds a magic word, the cycle counter, and the RTC time of the last sleep entry.
//! Odd cycles and every `Ext1` wake boot are forced cycles; the others are plain timer cycles.
//! A forced cycle cuts `LCD_3V3` (GPIO1) and holds the touch reset (GPIO13) LOW so INT stays LOW and the S3 hardware rejects the sleep.
//! It clears the global digital-pad hold first, which an earlier ESP-IDF firmware can leave set, unless `S3_KEEP_AUTOHOLD=1`: with it enabled and no library guard, a rejected deep sleep hangs the S3 (`docs/project-lore.md`).
//! It then reads INT through the existing driver and re-reads the wake cause.
//! Two seconds into the window it restores the rail, pulses the touch reset and re-arms the pad holds, which gives an `L->H` edge on INT.
//! The window then runs to its end and the normal retry loop resumes.
//!
//! Lines, all through `println!`; `t_ms` is ms since boot:
//!
//! - `S3BENCH tier=hal cycle=<n> reset=<r> cause=<c> mask=0x<m>` once per boot; the boot lines repeat after the 3 s awake time, because the host reopens the port too late for the first copy.
//! - `DIGISO tier=hal autohold_en=<0|1> force_unhold=<0|1>`: the global digital-pad hold state at boot.
//! - `FORCED tier=hal int_before=<H|L> autohold=<0|1>`: INT right before the forced attempt, and whether the global digital-pad hold is effective.
//! - `SLEPT tier=hal ms=<n> forced=<0|1>` on a `Timer` wake: RTC-clock ms from the last sleep entry to this boot, and whether the cycle that slept was forced.
//! - `ATTEMPT tier=hal n=<k> t_ms=<n>` right before every `try_sleep`; the last one before a sleep is the origin of the recount.
//! - `PINREAD start|edge|end tier=hal pin=5 ...` as in the C3 R2 bench: `level=<H|L>`, `<H->L|L->H>`, and `edges=<n> high=<0|1> low=<0|1>`.
//! - `WAKECAUSE tier=hal boot=<cause> after_reject=<cause>` re-read after the rejection (`wake-cause-survives-rejection`).
//!
//! ## Run
//!
//! ```shell
//! just run-hal-s3 hal_s3_deep_sleep_gpio
//! ```
//!
//! `run-hal-s3` attaches `espflash monitor`, which cannot follow the USB-Serial-JTAG port across deep sleep.
//! For the unattended bench, flash without the monitor and capture with `just serial-capture`:
//!
//! ```shell
//! S3_R2=1 S3_SLEEP_TIMER_MS=5000 just flash-hal-s3 hal_s3_deep_sleep_gpio
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use core::sync::atomic::{AtomicU32, Ordering};

use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    rtc_cntl::{reset_reason, SocResetReason},
    system::Cpu,
    time::Instant,
};
use esp_println::println;
use rustyfarian_esp_hal_power::{
    EspHalRtcClock, EspHalSleepManager, EspHalWakeCauseSource, GpioWakeLevel, SleepError,
    SleepManager, WakeCause, WakeCauseSource, WakeSource, MAX_TIMER_MS,
};

/// CST816S touch INT pin; bit position in the wake mask.
const TOUCH_INT_PIN: u8 = 5;

/// Time awake after each boot; the USB-Serial-JTAG port needs ~0.5-2 s to re-enumerate.
const AWAKE_MS: u32 = 3_000;

/// Bench case R2 (`S3_R2=1` at build time).
const R2: bool = match option_env!("S3_R2") {
    Some(s) => s.len() == 1 && s.as_bytes()[0] == b'1',
    None => false,
};

/// Bench case R3 (`S3_KEEP_AUTOHOLD=1` at build time, only meaningful with `S3_R2=1`).
const KEEP_AUTOHOLD: bool = match option_env!("S3_KEEP_AUTOHOLD") {
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
/// `.rtc_fast.persistent` is a `NOLOAD` section: the initialiser never runs and the bytes survive resets.
struct R2Record {
    magic: AtomicU32,
    cycle: AtomicU32,
    /// RTC clock in ms (wrapping) right before the last `try_sleep`.
    entry_ms: AtomicU32,
    entry_valid: AtomicU32,
    /// Whether the cycle that stored `entry_ms` was forced.
    entry_forced: AtomicU32,
}

#[link_section = ".rtc_fast.persistent"]
static R2_RECORD: R2Record = R2Record {
    magic: AtomicU32::new(0),
    cycle: AtomicU32::new(0),
    entry_ms: AtomicU32::new(0),
    entry_valid: AtomicU32::new(0),
    entry_forced: AtomicU32::new(0),
};

/// `RTC_CNTL_DIG_ISO_REG` on the ESP32-S3 (`soc/esp32s3/include/soc/rtc_cntl_reg.h`).
const RTC_CNTL_DIG_ISO_REG: usize = 0x6000_8094;
/// `RTC_CNTL_DG_PAD_AUTOHOLD_EN`: the global digital-pad hold that ESP-IDF's `gpio_deep_sleep_hold_en` sets.
const DG_PAD_AUTOHOLD_EN: u32 = 1 << 11;
/// `RTC_CNTL_DG_PAD_FORCE_UNHOLD`.
const DG_PAD_FORCE_UNHOLD: u32 = 1 << 14;

/// Reads `RTC_CNTL_DIG_ISO_REG`.
fn dig_iso() -> u32 {
    // SAFETY: an always-mapped RTC_CNTL register; a volatile 32-bit read has no side effects.
    unsafe { core::ptr::read_volatile(RTC_CNTL_DIG_ISO_REG as *const u32) }
}

/// RTC clock in ms, wrapping at 2^32 (~49 days), which is fine for differences.
fn rtc_ms() -> u32 {
    (EspHalRtcClock::now_us() / 1000) as u32
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
    Instant::now().duration_since_epoch().as_millis()
}

fn level_char(level: Level) -> char {
    match level {
        Level::High => 'H',
        Level::Low => 'L',
    }
}

/// Short name of the reset reason for the `S3BENCH` line.
fn reset_name(reason: Option<SocResetReason>) -> &'static str {
    match reason {
        Some(SocResetReason::ChipPowerOn) => "PowerOn",
        Some(SocResetReason::CoreSw | SocResetReason::CpuSw) => "Software",
        Some(SocResetReason::CoreDeepSleep) => "DeepSleep",
        Some(SocResetReason::SysBrownOut) => "Brownout",
        Some(
            SocResetReason::CoreMwdt0
            | SocResetReason::CoreMwdt1
            | SocResetReason::CoreRtcWdt
            | SocResetReason::CpuMwdt0
            | SocResetReason::CpuMwdt1
            | SocResetReason::CpuRtcWdt
            | SocResetReason::SysRtcWdt
            | SocResetReason::SysSuperWdt,
        ) => "Watchdog",
        _ => "Other",
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
    println!("ATTEMPT tier=hal n={} t_ms={}", attempt, now_ms());
    R2_RECORD
        .entry_forced
        .store(forced as u32, Ordering::Relaxed);
    R2_RECORD.entry_ms.store(rtc_ms(), Ordering::Relaxed);
    R2_RECORD.entry_valid.store(1, Ordering::Relaxed);
}

/// Restores the touch power: rail high, reset pulse, pad holds re-armed (the cold-boot sequence).
fn restore_touch_power(lcd_rail: &mut Output<'_>, touch_rst: &mut Output<'_>, delay: &Delay) {
    lcd_rail.set_pad_hold(false);
    touch_rst.set_pad_hold(false);
    lcd_rail.set_high();
    delay.delay_millis(20);
    touch_rst.set_low();
    delay.delay_millis(10);
    touch_rst.set_high();
    delay.delay_millis(50);
    lcd_rail.set_pad_hold(true);
    touch_rst.set_pad_hold(true);
}

/// Forced-cycle window after the first `Rejected`: pin read and wake-cause re-read.
///
/// Runs `REJECT_HOLD_MS` from the rejection and restores the touch power `RESTORE_AT_MS` in.
fn rejection_window(
    touch_int: &Input<'_>,
    lcd_rail: &mut Output<'_>,
    touch_rst: &mut Output<'_>,
    delay: &Delay,
    boot_cause: WakeCause,
) {
    let start_level = touch_int.level();
    let start_ms = now_ms();
    println!(
        "PINREAD start tier=hal pin={} level={} t_ms={}",
        TOUCH_INT_PIN,
        level_char(start_level),
        start_ms
    );

    let after_reject = EspHalWakeCauseSource::capture(&[touch_int]).last_wake_cause();
    println!(
        "WAKECAUSE tier=hal boot={:?} after_reject={:?}",
        boot_cause, after_reject
    );

    let mut last = start_level;
    let mut edges: u32 = 0;
    let mut high = start_level == Level::High;
    let mut low = start_level == Level::Low;
    let mut restored = false;
    while now_ms() - start_ms < REJECT_HOLD_MS {
        delay.delay_millis(PINREAD_PERIOD_MS);
        if !restored && now_ms() - start_ms >= RESTORE_AT_MS {
            restore_touch_power(lcd_rail, touch_rst, delay);
            restored = true;
        }
        let level = touch_int.level();
        high |= level == Level::High;
        low |= level == Level::Low;
        if level != last {
            edges += 1;
            println!(
                "PINREAD edge tier=hal pin={} {}->{} t_ms={}",
                TOUCH_INT_PIN,
                level_char(last),
                level_char(level),
                now_ms()
            );
            last = level;
        }
    }
    println!(
        "PINREAD end tier=hal pin={} edges={} high={} low={} t_ms={}",
        TOUCH_INT_PIN,
        edges,
        high as u8,
        low as u8,
        now_ms()
    );
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    loop {}
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // Touch power: `Output::new` releases any pad hold from the previous run and then drives the
    // level, so the pads are briefly undriven on a wake boot. Only a cold boot gets the reset
    // pulse. The hold is re-armed once the pulse is done; the drivers live until sleep.
    let mut lcd_rail = Output::new(peripherals.GPIO1, Level::High, OutputConfig::default());
    let mut touch_rst = Output::new(peripherals.GPIO13, Level::High, OutputConfig::default());
    if reset_reason(Cpu::ProCpu) != Some(SocResetReason::CoreDeepSleep) {
        delay.delay_millis(20);
        touch_rst.set_low();
        delay.delay_millis(10);
        touch_rst.set_high();
        delay.delay_millis(50);
    }
    lcd_rail.set_pad_hold(true);
    touch_rst.set_pad_hold(true);

    // The CST816S drives INT; the knob reads it without a pull, and so does this example.
    let mut touch_int = Input::new(
        peripherals.GPIO5,
        InputConfig::default().with_pull(Pull::None),
    );

    // Capture the wake cause before the first sleep attempt: a rejected sleep overwrites the
    // per-pin wake record.
    let wake_cause = EspHalWakeCauseSource::capture(&[&touch_int]).last_wake_cause();
    println!("Wake cause: {:?}", wake_cause);

    // Bench R2: load or reset the retained record, then report the boot and the last sleep.
    let mut forced = false;
    // Boot lines, printed again after the awake delay: the host reopens the
    // re-enumerated USB-Serial-JTAG port too late to see the first copy.
    let mut boot_line: Option<(u32, &str, &str, u64, Option<(u32, u32)>)> = None;
    let print_boot = |line: &Option<(u32, &str, &str, u64, Option<(u32, u32)>)>| {
        if let Some((cycle, reset_str, cause_str, mask, slept)) = line {
            println!(
                "S3BENCH tier=hal cycle={} reset={} cause={} mask=0x{:x}",
                cycle, reset_str, cause_str, mask
            );
            if let Some((ms, was_forced)) = slept {
                println!("SLEPT tier=hal ms={} forced={}", ms, was_forced);
            }
        }
    };
    if R2 {
        let record = &R2_RECORD;
        if record.magic.load(Ordering::Relaxed) == R2_MAGIC {
            record.cycle.fetch_add(1, Ordering::Relaxed);
        } else {
            record.magic.store(R2_MAGIC, Ordering::Relaxed);
            record.cycle.store(0, Ordering::Relaxed);
            record.entry_valid.store(0, Ordering::Relaxed);
        }
        let cycle = record.cycle.load(Ordering::Relaxed);
        let reset_str = reset_name(reset_reason(Cpu::ProCpu));
        let (cause_str, mask) = cause_name(wake_cause);
        let slept =
            (cause_str == "Timer" && record.entry_valid.load(Ordering::Relaxed) == 1).then(|| {
                (
                    rtc_ms().wrapping_sub(record.entry_ms.load(Ordering::Relaxed)),
                    record.entry_forced.load(Ordering::Relaxed),
                )
            });
        boot_line = Some((cycle, reset_str, cause_str, mask, slept));
        print_boot(&boot_line);
        record.entry_valid.store(0, Ordering::Relaxed);
        forced = cycle % 2 == 1 || cause_str == "Ext1";
    }
    let iso = dig_iso();
    let print_iso = || {
        if R2 {
            println!(
                "DIGISO tier=hal autohold_en={} force_unhold={}",
                (iso & DG_PAD_AUTOHOLD_EN != 0) as u8,
                (iso & DG_PAD_FORCE_UNHOLD != 0) as u8
            );
        }
    };
    print_iso();
    delay.delay_millis(AWAKE_MS);
    print_boot(&boot_line);
    print_iso();

    let mut manager = EspHalSleepManager::new(peripherals.LPWR);
    let sources = [
        WakeSource::GpioLevel {
            pin_mask: 1 << TOUCH_INT_PIN,
            level: GpioWakeLevel::AnyLow,
        },
        WakeSource::Timer {
            duration_ms: TIMER_MS,
        },
    ];

    // Forced cycle: cut the touch rail so INT stays LOW and the hardware rejects the sleep.
    if R2 && forced {
        lcd_rail.set_pad_hold(false);
        touch_rst.set_pad_hold(false);
        // A rejected deep sleep hangs the S3 while the global digital-pad hold is enabled: sleep
        // entry isolates every unheld digital pad, the flash pads included, and the rejection
        // does not restore them. An earlier ESP-IDF firmware can leave the bit set, and esp-hal
        // never clears it (docs/project-lore.md).
        // S3_KEEP_AUTOHOLD=1 instead makes the hold effective (bit 11 set, bit 14 clear) so the
        // library guard has to prevent the hang.
        // SAFETY: read-modify-write of the always-mapped RTC_CNTL_DIG_ISO_REG, touching only
        // the global digital-pad hold enable and force-unhold bits; clearing the enable is what
        // ESP-IDF's gpio_deep_sleep_hold_dis does.
        let dig_iso_value = if KEEP_AUTOHOLD {
            (dig_iso() | DG_PAD_AUTOHOLD_EN) & !DG_PAD_FORCE_UNHOLD
        } else {
            dig_iso() & !DG_PAD_AUTOHOLD_EN
        };
        unsafe { core::ptr::write_volatile(RTC_CNTL_DIG_ISO_REG as *mut u32, dig_iso_value) };
        // Reset LOW too, so the unpowered controller is not back-fed through its reset pin.
        touch_rst.set_low();
        lcd_rail.set_low();
        delay.delay_millis(100);
        println!(
            "FORCED tier=hal int_before={} autohold={}",
            level_char(touch_int.level()),
            u8::from(KEEP_AUTOHOLD)
        );
    }
    let mut window_pending = R2 && forced;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        if R2 {
            mark_attempt(attempt, forced);
        }
        // On success the chip is asleep and this call does not return.
        // The BoundSleep borrow ends with this statement, so the pin is readable again below.
        let res = manager
            .bind_wake_pins(&mut [&mut touch_int])
            .try_sleep(&sources);
        match res {
            // INT is still LOW; try again shortly.
            Err(SleepError::Rejected) => {
                if window_pending {
                    window_pending = false;
                    rejection_window(
                        &touch_int,
                        &mut lcd_rail,
                        &mut touch_rst,
                        &delay,
                        wake_cause,
                    );
                }
                delay.delay_millis(RETRY_DELAY_MS)
            }
            Err(e) => panic!("try_sleep failed: {}", e),
            Ok(()) => panic!("try_sleep returned without sleeping"),
        }
    }
}
