//! ESP32-C3-DevKitM-1 — RTC Retention Example (esp-hal)
//!
//! Keeps a small session record in RTC memory and prints one line per boot,
//! then ends the boot with a configurable action: deep sleep, restart, panic or watchdog reset.
//! It shows which resets keep the record and which discard it.
//! The record is an `RtcRetainedCell` placed in `.rtc_fast.persistent`, so it survives deep sleep and warm resets.
//! `EspHalResetReasonSource` tells `load` when RTC memory cannot be trusted.
//! It is the esp-hal twin of `idf_c3_rtc_retention` and prints the same log lines.
//!
//! ## Board
//!
//! - **Chip:** ESP32-C3 (RISC-V, single core)
//! - **Board:** Espressif ESP32-C3-DevKitM-1 (CP2102N bridge on UART0, GPIO20/21)
//! - No wiring is needed.
//!
//! ## Build-time action
//!
//! `C3_RET_ACTION` selects what the example does at the end of each boot.
//! An unknown value fails the build.
//!
//! | Value     | Action                                                                  |
//! |:----------|:------------------------------------------------------------------------|
//! | `sleep`   | Deep sleep with a timer wake (default); expect `reset=DeepSleep`.       |
//! | `restart` | `software_reset()`; expect `reset=Software`.                            |
//! | `panic`   | `panic!`, whose handler restarts; expect `reset=Software` on this tier. |
//! | `wdt`     | Spin with interrupts off until the armed TIMG0 watchdog fires; expect `reset=Watchdog`. |
//! | `cycle`   | Picks `sleep`, `restart`, `panic`, `wdt` by `boots % 4` (1, 2, 3, 0).   |
//!
//! Bare-metal esp-hal records no panic reset, so the panic handler's `software_reset()` reads as `Software`, never `Panic`.
//! `esp_hal::init` disables every watchdog, so the `wdt` action arms the TIMG0 main watchdog (500 ms, stage 0 resets the system) itself.
//! The timer wake defaults to 5000 ms; override it with `C3_SLEEP_TIMER_MS`.
//! The runtime zeroes `.rtc_fast.persistent` on a power-on reset, so a cold boot always starts a new session.
//!
//! ```shell
//! C3_RET_ACTION=cycle just build-example-hal-c3 hal_c3_rtc_retention
//! ```
//!
//! ## Run
//!
//! ```shell
//! just flash-hal-c3 hal_c3_rtc_retention
//! ```
//!
//! ## Expected output (serial monitor)
//!
//! Built with `C3_RET_ACTION=cycle`.
//! The `session` stays constant and `boots` increments across all four reset kinds.
//!
//! ```text
//! RET reset=PowerOn wake=PowerOn load=err:PowerOn boots=1 session=0x9c41e7a2
//! Action: Sleep
//! RET reset=DeepSleep wake=Timer load=ok boots=2 session=0x9c41e7a2
//! Previous boot began with reset code 0
//! Action: Restart
//! RET reset=Software wake=PowerOn load=ok boots=3 session=0x9c41e7a2
//! Previous boot began with reset code 2
//! Action: Panic
//! RET reset=Software wake=PowerOn load=ok boots=4 session=0x9c41e7a2
//! Previous boot began with reset code 3
//! Action: Wdt
//! RET reset=Watchdog wake=PowerOn load=ok boots=5 session=0x9c41e7a2
//! Previous boot began with reset code 3
//! Action: Sleep
//! ```
//!
//! Each boot also prints `RTC rtc_us=<n>` after init and just before the action; `n` keeps growing across every reset kind except a power-on.
//!
//! After a USB power cycle the record is refused and a new session starts:
//!
//! ```text
//! RET reset=PowerOn wake=PowerOn load=err:PowerOn boots=1 session=0x3b7d0c15
//! ```
//!
//! A brownout is refused the same way (`load=err:Brownout`), but triggering one needs a bench supply.

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use esp_hal::{
    delay::Delay,
    gpio::Input,
    rng::Rng,
    system::software_reset,
    time::Duration,
    timer::timg::{MwdtStage, TimerGroup},
};
use esp_println::println;
use rustyfarian_esp_hal_power::{
    EspHalResetReasonSource, EspHalRtcClock, EspHalSleepManager, EspHalWakeCauseSource, Invalid,
    ResetReason, ResetReasonSource, Retainable, RtcRetainedCell, SleepManager, WakeCause,
    WakeCauseSource, WakeSource, MAX_TIMER_MS,
};

/// The retained record.
///
/// `.rtc_fast.persistent` is a `NOLOAD` section, so the initialiser never runs and the bytes survive resets.
/// The runtime zeroes it on a power-on reset only.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static RETAINED: RtcRetainedCell<16> = RtcRetainedCell::new();

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

/// Parse `C3_RET_ACTION`; an unknown value fails the build.
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
        panic!("C3_RET_ACTION must be one of: sleep, restart, panic, wdt, cycle")
    }
}

/// Action at the end of each boot: `C3_RET_ACTION` at build time, else `sleep`.
const ACTION: Action = parse_action(option_env!("C3_RET_ACTION"));

/// Timer wake in ms: `C3_SLEEP_TIMER_MS` at build time, else 5 s.
const TIMER_MS: u64 = match option_env!("C3_SLEEP_TIMER_MS") {
    Some(s) => parse_u64(s),
    None => 5_000,
};

// An out-of-range override would fail validation on every boot; fail the build instead.
const _: () = assert!(
    TIMER_MS >= 1 && TIMER_MS <= MAX_TIMER_MS,
    "C3_SLEEP_TIMER_MS must be between 1 and MAX_TIMER_MS"
);

/// Decimal parser usable in a const context; fails the build on anything but digits.
const fn parse_u64(s: &str) -> u64 {
    let b = s.as_bytes();
    assert!(!b.is_empty(), "C3_SLEEP_TIMER_MS must not be empty");
    let mut i = 0;
    let mut v: u64 = 0;
    while i < b.len() {
        assert!(
            b[i].is_ascii_digit(),
            "C3_SLEEP_TIMER_MS must be decimal digits"
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

/// Prints the message and restarts: unattended hardware must recover, not hang.
///
/// The restart reads as `reset=Software` on the next boot; esp-hal records no panic reset.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    Delay::new().delay_millis(50);
    software_reset()
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // Read the reset reason before touching retained state: load needs it.
    // Capture the wake cause first too (ADR-001): a later sleep attempt overwrites the per-pin record.
    let wake = EspHalWakeCauseSource::capture(&[]).last_wake_cause();
    let reset = EspHalResetReasonSource.last_reset_reason();
    let loaded: Result<Session, Invalid> = RETAINED.load::<Session>(reset);

    let mut session = match loaded {
        Ok(s) => s,
        Err(_) => Session {
            // The RNG is only weakly random without the radio, which is fine for a demo id.
            id: Rng::new().random(),
            boots: 0,
            last_reset: 0,
        },
    };
    let previous_reset = loaded.ok().map(|s| s.last_reset);

    session.boots = session.boots.wrapping_add(1);
    session.last_reset = reset_code(reset);
    if let Err(e) = RETAINED.store(&session) {
        println!("store failed: {e:?} — continuing without retention");
    }

    match loaded {
        Ok(_) => println!(
            "RET reset={reset:?} wake={} load=ok boots={} session=0x{:08x}",
            wake_name(wake),
            session.boots,
            session.id
        ),
        Err(e) => println!(
            "RET reset={reset:?} wake={} load=err:{e:?} boots={} session=0x{:08x}",
            wake_name(wake),
            session.boots,
            session.id
        ),
    }
    if let Some(code) = previous_reset {
        println!("Previous boot began with reset code {code}");
    }

    println!("RTC rtc_us={}", EspHalRtcClock::now_us());

    let action = resolve(ACTION, session.boots);
    println!("Action: {action:?}");

    // Let the UART drain so the lines above are readable before the reset.
    delay.delay_millis(1_000);
    println!("RTC rtc_us={}", EspHalRtcClock::now_us());

    match action {
        Action::Sleep | Action::Cycle => {
            let mut manager = EspHalSleepManager::new(peripherals.LPWR);
            let mut none: [&mut Input<'_>; 0] = [];
            // On success the chip is asleep and this call does not return.
            if let Err(e) = manager
                .bind_wake_pins(&mut none)
                .sleep(&[WakeSource::Timer {
                    duration_ms: TIMER_MS,
                }])
            {
                println!("sleep failed: {e} — restarting");
                delay.delay_millis(50);
                software_reset();
            }
        }
        Action::Restart => software_reset(),
        Action::Panic => {
            // The panic handler above restarts; the next boot reads reset=Software.
            panic!("C3_RET_ACTION=panic");
        }
        Action::Wdt => {
            // esp_hal::init disables every watchdog, so arm the TIMG0 main watchdog:
            // stage 0 resets the system after 500 ms, reported as CoreMwdt0 -> Watchdog.
            let mut wdt = TimerGroup::new(peripherals.TIMG0).wdt;
            wdt.set_timeout(MwdtStage::Stage0, Duration::from_millis(500));
            wdt.enable();
            // Never feed it; mask interrupts so nothing else can run.
            critical_section::with(|_| loop {
                core::hint::spin_loop();
            });
        }
    }

    // Only reachable if sleep returned without sleeping; do not stay awake on battery.
    delay.delay_millis(50);
    software_reset()
}
