//! Heltec WiFi LoRa 32 V3 — RTC Retention Example (ESP-IDF, ESP32-S3)
//!
//! S3 twin of `idf_c3_rtc_retention`: same payload, same `RET` line, same action matrix.
//! Keeps a small session record in RTC memory and prints one line per boot,
//! then ends the boot with a configurable action: deep sleep, restart, panic or watchdog reset.
//! It shows which resets keep the record and which discard it.
//! The record is a `RetainedCell` placed in `.rtc_noinit`, so it survives deep sleep and warm resets.
//! `EspResetReasonSource` tells `load` when RTC memory cannot be trusted.
//!
//! On the S3, ESP-IDF v5.3.3 puts `.rtc_noinit` in RTC **slow** memory (0x50000000),
//! not RTC fast memory as on the C3, so this example checks a different power domain.
//!
//! ## Board
//!
//! - **Chip:** ESP32-S3 (Xtensa, dual core)
//! - **Board:** Heltec WiFi LoRa 32 V3 (CP2102 bridge on UART0)
//! - No wiring is needed.
//!
//! Output goes through `esp_rom_printf` because Rust `println!` does not reach the S3 console
//! (`docs/project-lore.md` → ESP-IDF / Rust on ESP32-S3).
//!
//! ## Build-time action
//!
//! `S3_RET_ACTION` selects what the example does at the end of each boot.
//! An unknown value fails the build.
//!
//! | Value     | Action                                                                  |
//! |:----------|:------------------------------------------------------------------------|
//! | `sleep`   | Deep sleep with a timer wake (default); expect `reset=DeepSleep`.       |
//! | `restart` | `esp_restart()`; expect `reset=Software`.                               |
//! | `panic`   | `panic!`, which reboots in this sdkconfig; expect `reset=Panic`.        |
//! | `wdt`     | Spin with interrupts off until the interrupt watchdog fires; expect `reset=Watchdog`. |
//! | `cycle`   | Picks `sleep`, `restart`, `panic`, `wdt` by `boots % 4` (1, 2, 3, 0).   |
//!
//! The task watchdog only warns in this sdkconfig (`CONFIG_ESP_TASK_WDT_PANIC` is unset),
//! so the `wdt` action trips the 300 ms interrupt watchdog instead.
//! The timer wake defaults to 5000 ms; override it with `S3_SLEEP_TIMER_MS`,
//! for example `900000` for a 15-minute field interval.
//!
//! ```shell
//! S3_RET_ACTION=cycle just build-example idf_s3_rtc_retention
//! ```
//!
//! ## Run
//!
//! ```shell
//! just run idf_s3_rtc_retention
//! ```
//!
//! ## Expected output (serial monitor)
//!
//! Built with `S3_RET_ACTION=cycle`.
//! The `session` stays constant and `boots` increments across all four reset kinds.
//!
//! Every boot also prints `RTC rtc_us=<n>`, the RTC timer in microseconds since the last power-on;
//! it keeps counting through deep sleep and warm resets, so consecutive values differ by the sleep
//! plus about one second of delay and boot time.
//!
//! ```text
//! RET reset=PowerOn wake=PowerOn load=err:PowerOn boots=1 session=0x9c41e7a2
//! RTC rtc_us=312455
//! Action: Sleep
//! RET reset=DeepSleep wake=Timer load=ok boots=2 session=0x9c41e7a2
//! Previous boot began with reset code 0
//! RTC rtc_us=6713802
//! Action: Restart
//! RET reset=Software wake=PowerOn load=ok boots=3 session=0x9c41e7a2
//! Previous boot began with reset code 2
//! Action: Panic
//! RET reset=Panic wake=PowerOn load=ok boots=4 session=0x9c41e7a2
//! Previous boot began with reset code 3
//! Action: Wdt
//! RET reset=Watchdog wake=PowerOn load=ok boots=5 session=0x9c41e7a2
//! Previous boot began with reset code 4
//! Action: Sleep
//! ```
//!
//! After a USB power cycle the record is refused and a new session starts:
//!
//! ```text
//! RET reset=PowerOn wake=PowerOn load=err:PowerOn boots=1 session=0x3b7d0c15
//! ```
//!
//! A brownout is refused the same way (`load=err:Brownout`), but triggering one needs a bench supply.

use std::ffi::CString;

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::sys;
use rustyfarian_esp_idf_power::{
    EspResetReasonSource, EspRtcClock, EspSleepManager, EspWakeCauseSource, Invalid, ResetReason,
    ResetReasonSource, Retainable, RetainedCell, SleepManager, WakeCause, WakeCauseSource,
    WakeSource, MAX_TIMER_MS,
};

/// The retained record.
///
/// `.rtc_noinit` is a `NOLOAD` section, so the initialiser never runs and the bytes survive resets.
/// On the S3 it lands in RTC slow memory at 0x50000000, which ESP-IDF keeps powered in deep sleep.
#[link_section = ".rtc_noinit"]
static RETAINED: RetainedCell<16> = RetainedCell::new();

/// Payload kept across resets: a session id fixed at the first boot, a boot counter and the reset code the previous boot started with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Session {
    id: u32,
    boots: u32,
    last_reset: u8,
}

impl Retainable for Session {
    const VERSION: u16 = 1;
    const SIZE: usize = 9;

    fn write(&self, buf: &mut [u8]) -> Option<()> {
        buf.get_mut(0..4)?.copy_from_slice(&self.id.to_le_bytes());
        buf.get_mut(4..8)?
            .copy_from_slice(&self.boots.to_le_bytes());
        *buf.get_mut(8)? = self.last_reset;
        Some(())
    }

    fn read(buf: &[u8]) -> Option<Self> {
        let id = u32::from_le_bytes(buf.get(0..4)?.try_into().ok()?);
        let boots = u32::from_le_bytes(buf.get(4..8)?.try_into().ok()?);
        let last_reset = *buf.get(8)?;
        Some(Self {
            id,
            boots,
            last_reset,
        })
    }
}

/// What a boot does after printing its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Sleep,
    Restart,
    Panic,
    Wdt,
    Cycle,
}

/// Byte-string equality usable in a const context.
const fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Parse `S3_RET_ACTION`; an unknown value fails the build.
const fn parse_action(value: Option<&str>) -> Action {
    let Some(s) = value else {
        return Action::Sleep;
    };
    let b = s.as_bytes();
    if bytes_eq(b, b"sleep") {
        Action::Sleep
    } else if bytes_eq(b, b"restart") {
        Action::Restart
    } else if bytes_eq(b, b"panic") {
        Action::Panic
    } else if bytes_eq(b, b"wdt") {
        Action::Wdt
    } else if bytes_eq(b, b"cycle") {
        Action::Cycle
    } else {
        panic!("S3_RET_ACTION must be one of: sleep, restart, panic, wdt, cycle")
    }
}

/// Action at the end of each boot: `S3_RET_ACTION` at build time, else `sleep`.
const ACTION: Action = parse_action(option_env!("S3_RET_ACTION"));

/// Timer wake in ms: `S3_SLEEP_TIMER_MS` at build time, else 5 s.
const TIMER_MS: u64 = match option_env!("S3_SLEEP_TIMER_MS") {
    Some(s) => parse_u64(s),
    None => 5_000,
};

// An out-of-range override would fail validation on every boot; fail the build instead.
const _: () = assert!(
    TIMER_MS >= 1 && TIMER_MS <= MAX_TIMER_MS,
    "S3_SLEEP_TIMER_MS must be between 1 and MAX_TIMER_MS"
);

/// Decimal parser usable in a const context; fails the build on anything but digits.
const fn parse_u64(s: &str) -> u64 {
    let b = s.as_bytes();
    assert!(!b.is_empty(), "S3_SLEEP_TIMER_MS must not be empty");
    let mut i = 0;
    let mut v: u64 = 0;
    while i < b.len() {
        assert!(
            b[i].is_ascii_digit(),
            "S3_SLEEP_TIMER_MS must be decimal digits"
        );
        v = v * 10 + (b[i] - b'0') as u64;
        i += 1;
    }
    v
}

/// Resolve `Cycle` to the concrete action for this boot.
fn resolve(action: Action, boots: u32) -> Action {
    match action {
        Action::Cycle => match boots % 4 {
            1 => Action::Sleep,
            2 => Action::Restart,
            3 => Action::Panic,
            _ => Action::Wdt,
        },
        other => other,
    }
}

/// Small stable code for a reset reason, stored in the record.
fn reset_code(reason: ResetReason) -> u8 {
    match reason {
        ResetReason::PowerOn => 0,
        ResetReason::Brownout => 1,
        ResetReason::DeepSleep => 2,
        ResetReason::Software => 3,
        ResetReason::Panic => 4,
        ResetReason::Watchdog => 5,
        _ => 255,
    }
}

/// Variant name of a wake cause, without the fired-pin mask.
fn wake_name(cause: WakeCause) -> &'static str {
    match cause {
        WakeCause::PowerOn => "PowerOn",
        WakeCause::Timer => "Timer",
        WakeCause::Gpio(_) => "Gpio",
        WakeCause::Ext1(_) => "Ext1",
        WakeCause::Ext0 => "Ext0",
        WakeCause::Touch => "Touch",
        _ => "Other",
    }
}

/// Print a line via `esp_rom_printf`; Rust `println!` does not reach the S3 console.
fn say(msg: &str) {
    let Ok(c) = CString::new(msg) else { return };
    // SAFETY: `c` is a NUL-terminated string alive for the call, `c"%s\n"` a static format.
    unsafe { sys::esp_rom_printf(c"%s\n".as_ptr(), c.as_ptr()) };
}

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches satisfies ESP-IDF's requirement that ROM function
    // stubs are patched before any ESP-IDF API is called. Must be the very
    // first call in main().
    esp_idf_hal::sys::link_patches();

    // Read the reset reason before touching retained state: load needs it.
    let reset = EspResetReasonSource.last_reset_reason();
    let wake = EspWakeCauseSource.last_wake_cause();
    let loaded: Result<Session, Invalid> = RETAINED.load::<Session>(reset);

    let mut session = match loaded {
        Ok(s) => s,
        Err(_) => Session {
            // SAFETY: esp_random reads the hardware RNG and changes no state.
            id: unsafe { sys::esp_random() },
            boots: 0,
            last_reset: 0,
        },
    };
    let previous_reset = loaded.ok().map(|s| s.last_reset);

    session.boots = session.boots.wrapping_add(1);
    session.last_reset = reset_code(reset);
    if let Err(e) = RETAINED.store(&session) {
        say(&format!(
            "store failed: {e:?} — continuing without retention"
        ));
    }

    let load = match loaded {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("err:{e:?}"),
    };
    say(&format!(
        "RET reset={reset:?} wake={} load={load} boots={} session=0x{:08x}",
        wake_name(wake),
        session.boots,
        session.id
    ));
    if let Some(code) = previous_reset {
        say(&format!("Previous boot began with reset code {code}"));
    }
    // The RTC timer keeps counting through deep sleep and warm resets; the delta between two
    // boots' values is the slept time plus the boot time, which consumers use to age a session.
    say(&format!("RTC rtc_us={}", EspRtcClock::now_us()));

    let action = resolve(ACTION, session.boots);
    say(&format!("Action: {action:?}"));

    // Let the UART drain so the lines above are readable before the reset.
    FreeRtos::delay_ms(1_000);

    match action {
        Action::Sleep | Action::Cycle => {
            let mut manager = EspSleepManager::default();
            // On success the chip is asleep and this call does not return.
            if let Err(e) = manager.sleep(&[WakeSource::Timer {
                duration_ms: TIMER_MS,
            }]) {
                say(&format!("sleep failed: {e} — restarting"));
                // SAFETY: esp_restart never returns.
                unsafe { sys::esp_restart() };
            }
        }
        Action::Restart => {
            // SAFETY: esp_restart never returns.
            unsafe { sys::esp_restart() };
        }
        Action::Panic => {
            // CONFIG_ESP_SYSTEM_PANIC_PRINT_REBOOT=y: the panic handler reboots
            // with ESP_RST_PANIC.
            panic!("S3_RET_ACTION=panic");
        }
        Action::Wdt => {
            // CONFIG_ESP_TASK_WDT_PANIC is unset, so a starved task only warns.
            // With interrupts off the 300 ms interrupt watchdog fires instead
            // and resets with ESP_RST_INT_WDT, reported as Watchdog.
            esp_idf_hal::interrupt::free(|| loop {
                core::hint::spin_loop();
            });
        }
    }

    // Only reachable if sleep returned without sleeping; do not stay awake on battery.
    // SAFETY: esp_restart never returns.
    unsafe { sys::esp_restart() };
    #[allow(unreachable_code)]
    Ok(())
}
