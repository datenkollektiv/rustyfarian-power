//! ESP32-C3-DevKitM-1 — PIR Deep-Sleep GPIO Wake Example (ESP-IDF)
//!
//! Sleeps in deep sleep until an HC-SR501 PIR on GPIO4 reports motion or a timer
//! (60 s by default) fires, then logs the wake cause and sleeps again.
//! It exercises `try_sleep`: right after a PIR wake the sensor output is still HIGH
//! for its hold time, so the first `try_sleep` is rejected and the example retries
//! every 50 ms until the PIR releases.
//!
//! ## Board
//!
//! - **Chip:** ESP32-C3 (RISC-V, single core)
//! - **Board:** Espressif ESP32-C3-DevKitM-1 (CP2102N bridge on UART0, GPIO20/21)
//! - The onboard SSD1306 (GPIO6/7) and WS2812 (GPIO8) are present but not used.
//!   GPIO8 is a strapping pin and is left alone.
//!
//! ## Wiring
//!
//! | Signal      | Connection                                       |
//! |:------------|:-------------------------------------------------|
//! | PIR OUT     | GPIO4                                            |
//! | PIR VCC     | 5V                                               |
//! | PIR GND     | GND                                              |
//! | Pull-down   | external 10 kΩ from GPIO4 to GND (recommended)   |
//!
//! The pull-down is the pull *against* the wake level: the wake level is HIGH, so the pin rests LOW.
//! The HC-SR501 output swings to about 3.3 V, so no level shifter is needed.
//! Meter the OUT pin first anyway: the C3 pads are not 5 V tolerant.
//! The IDF on C2/C3 also enables the internal pull against the wake level at sleep entry
//! (`CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS=y`, the default).
//!
//! ## GPIO4 is a JTAG pad
//!
//! GPIO4 is MTMS (JTAG TMS), and GPIO4-7 form the JTAG pad group.
//! A PIR on GPIO4 is fine for firmware, but it rules out pad-JTAG debugging while it is wired.
//! On the default eFuse settings the C3 uses its built-in USB-JTAG instead (bench plan, setup risks),
//! so the pads are free; make sure no external probe is attached.
//!
//! ## HC-SR501 settings
//!
//! Put the jumper in "H" (retrigger) mode.
//! The time-delay pot (minimum about a few seconds) decides how long the output stays HIGH,
//! and therefore how long the rejection loop runs.
//! In retrigger mode, motion while the output is HIGH extends the loop.
//!
//! While awake the example configures GPIO4 as an input with the internal pull-down,
//! so it does not depend on the pad's reset (JTAG) defaults.
//!
//! ## Wake timer
//!
//! The timer fallback defaults to 60 000 ms.
//! Override it at build time, for example for bench case W2:
//!
//! ```shell
//! C3_SLEEP_TIMER_MS=5000 just build-example idf_c3_deep_sleep_gpio
//! ```
//!
//! The boot `BENCH` line is always printed; set `C3_BENCH_LOG=1` at build time to also print one
//! per `try_sleep` return (bench cases H1 and H4), which is 20 lines per second while the PIR is held.
//! Set `C3_AWAKE_PROBE_MS=30000` at build time to poll and print the GPIO0-5 levels for 30 s before
//! the first sleep (awake wiring check); the chip also stays visible on a USB-Serial-JTAG console that long.
//! `C3_PROBE_PULL=up` probes with pull-ups instead: an open pin then reads High and a connected
//! push-pull sensor output reads Low at rest, which tells a missing wire from a dead sensor.
//! `C3_PIR_PIN=<0-5>` moves the PIR pin (default GPIO4), for example after the probe shows the
//! signal on a neighbouring header pin.
//!
//! ## Run
//!
//! ```shell
//! just run idf_c3_deep_sleep_gpio
//! ```
//!
//! ## Expected output (serial monitor)
//!
//! Built with `C3_BENCH_LOG=1`; without it the per-attempt `BENCH ... result=err:Rejected` lines are absent.
//!
//! ```text
//! Reset reason: DeepSleep
//! Wake cause: Gpio — fired pins: 0x10, GPIO4 fired: true
//! BENCH case=demo tier=idf cycle=0 reset=DeepSleep cause=Gpio mask=0x10 result=boot guard_us=0 drain_us=0
//! Sleeping: GPIO4 AnyHigh + 60000 ms timer
//! try_sleep rejected (first): <error>, kind=Rejected
//! BENCH case=demo tier=idf cycle=0 reset=DeepSleep cause=Gpio mask=0x10 result=err:Rejected guard_us=0 drain_us=0
//! try_sleep rejected: 20 so far
//! ```
//!
//! When the PIR releases, `try_sleep` succeeds and the chip sleeps again; the next boot prints the lines above.
//! After a power-on the first lines read `Wake cause: PowerOn` and `cause=PowerOn mask=0x0`.
//! The `BENCH` line format is the one in `docs/bench/c3-devkitm1-sleep-bench-plan-v1.md`;
//! `cycle` stays 0 because the host counts boots.

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{AnyIOPin, Input, Level, PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::sys;
use rustyfarian_esp_idf_power::{
    Error, EspResetReasonSource, EspSleepManager, EspWakeCauseSource, GpioWakeLevel,
    ResetReasonSource, SleepErrorKind, SleepManager, WakeCause, WakeCauseSource, WakeSource,
    MAX_TIMER_MS,
};

/// PIR output pin; bit position in the wake mask: `C3_PIR_PIN` at build time (0-5), else GPIO4.
const PIR_PIN: u8 = match option_env!("C3_PIR_PIN") {
    Some(s) => {
        // Only GPIO0-5 can wake the C3 from deep sleep; range-check before the cast truncates.
        let pin = parse_u64(s);
        assert!(pin <= 5, "C3_PIR_PIN must be between 0 and 5");
        pin as u8
    }
    None => 4,
};

/// Number of deep-sleep wake-capable pins on the C3 (GPIO0-5), all polled by the awake probe.
const WAKE_PIN_COUNT: u8 = 6;

/// Delay between rejected `try_sleep` attempts.
const RETRY_DELAY_MS: u32 = 50;

/// Log a compact count line every this many rejections.
const LOG_EVERY: u32 = 20;

/// Timer fallback in ms: `C3_SLEEP_TIMER_MS` at build time, else 60 s.
const TIMER_MS: u64 = match option_env!("C3_SLEEP_TIMER_MS") {
    Some(s) => parse_u64(s),
    None => 60_000,
};

// An out-of-range override would fail validation on every boot; fail the build instead.
const _: () = assert!(
    TIMER_MS >= 1 && TIMER_MS <= MAX_TIMER_MS,
    "C3_SLEEP_TIMER_MS must be between 1 and MAX_TIMER_MS"
);

/// Awake probe before the first sleep, in ms: `C3_AWAKE_PROBE_MS` at build time, else off.
///
/// Polls GPIO0-5 every 100 ms for that long and prints every level change plus one line per
/// second, so the wiring can be checked while waving (bench step "awake sanity").
const AWAKE_PROBE_MS: u64 = match option_env!("C3_AWAKE_PROBE_MS") {
    Some(s) => parse_u64(s),
    None => 0,
};

/// Pull applied to GPIO0-5 during the awake probe: `C3_PROBE_PULL=up|down` at build time, else down.
///
/// With `up`, an open pin reads High and a connected push-pull sensor output pulls it Low at
/// rest, so a missing wire or a wrong header pin shows up without a meter.
const PROBE_PULL_UP: bool = match option_env!("C3_PROBE_PULL") {
    Some(s) => parse_pull_up(s),
    None => false,
};

/// Poll period of the awake probe.
const PROBE_PERIOD_MS: u64 = 100;

/// Print a `BENCH` line for every `try_sleep` return (`C3_BENCH_LOG` set at build time).
///
/// Off by default: while the PIR is held the retry loop would print 20 lines per second.
const BENCH_LOG: bool = option_env!("C3_BENCH_LOG").is_some();

/// Decimal parser usable in a const context; fails the build on anything but digits.
const fn parse_u64(s: &str) -> u64 {
    let b = s.as_bytes();
    assert!(!b.is_empty(), "build-time ms variable must not be empty");
    let mut i = 0;
    let mut v: u64 = 0;
    while i < b.len() {
        assert!(
            b[i].is_ascii_digit(),
            "build-time ms variable must be decimal digits"
        );
        v = v * 10 + (b[i] - b'0') as u64;
        i += 1;
    }
    v
}

/// `up` or `down`; fails the build on anything else.
const fn parse_pull_up(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() == 2 && b[0] == b'u' && b[1] == b'p' {
        return true;
    }
    assert!(
        b.len() == 4 && b[0] == b'd' && b[1] == b'o' && b[2] == b'w' && b[3] == b'n',
        "C3_PROBE_PULL must be up or down"
    );
    false
}

fn level_char(level: Level) -> char {
    match level {
        Level::High => 'H',
        Level::Low => 'L',
    }
}

fn levels_line(levels: &[Level]) -> String {
    levels
        .iter()
        .enumerate()
        .map(|(pin, level)| format!("GPIO{}={}", pin, level_char(*level)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Polls GPIO0-5 with the probe pull for `AWAKE_PROBE_MS`, printing changes and one line per second.
///
/// The drivers are dropped at the end, which resets the pins; the caller configures the PIR
/// pin afterwards.
fn awake_probe() -> anyhow::Result<()> {
    let pull = if PROBE_PULL_UP { Pull::Up } else { Pull::Down };
    let mut pins: Vec<PinDriver<'static, Input>> = Vec::new();
    for pin in 0..WAKE_PIN_COUNT {
        // SAFETY: GPIO0-5 are valid C3 pins, each is stolen exactly once here, and the
        // drivers are dropped before the PIR pin driver is created, so no pin has two
        // live instances.
        pins.push(PinDriver::input(unsafe { AnyIOPin::steal(pin) }, pull)?);
    }
    let read = |pins: &[PinDriver<'static, Input>]| -> Vec<Level> {
        pins.iter().map(|p| p.get_level()).collect()
    };
    println!(
        "PROBE start: polling GPIO0-{} for {} ms with pull-{}",
        WAKE_PIN_COUNT - 1,
        AWAKE_PROBE_MS,
        if PROBE_PULL_UP { "up" } else { "down" }
    );
    let mut last = read(&pins);
    println!("PROBE t=0 ms {}", levels_line(&last));
    let mut elapsed: u64 = 0;
    while elapsed < AWAKE_PROBE_MS {
        FreeRtos::delay_ms(PROBE_PERIOD_MS as u32);
        elapsed += PROBE_PERIOD_MS;
        let now = read(&pins);
        if now != last {
            for (pin, (before, after)) in last.iter().zip(now.iter()).enumerate() {
                if before != after {
                    println!("PROBE t={} ms GPIO{} changed -> {:?}", elapsed, pin, after);
                }
            }
        } else if elapsed % 1000 == 0 {
            println!("PROBE t={} ms {}", elapsed, levels_line(&now));
        }
        last = now;
    }
    println!("PROBE end: {}", levels_line(&read(&pins)));
    Ok(())
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

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches satisfies ESP-IDF's requirement that ROM function
    // stubs are patched before any ESP-IDF API is called. Must be the very
    // first call in main().
    esp_idf_hal::sys::link_patches();

    // Capture the wake cause first (ADR-001): a later sleep attempt, even a
    // rejected one, can clear the hardware wake status.
    let wake_cause = EspWakeCauseSource.last_wake_cause();
    let reset = EspResetReasonSource.last_reset_reason();
    let reset_str = format!("{reset:?}");
    let (cause_str, mask) = cause_name(wake_cause);

    println!("Reset reason: {reset:?}");
    match wake_cause {
        WakeCause::Gpio(m) => println!(
            "Wake cause: Gpio — fired pins: 0x{:x}, GPIO{} fired: {}",
            m.0,
            PIR_PIN,
            m.contains_pin(PIR_PIN)
        ),
        other => println!("Wake cause: {:?}", other),
    }
    println!(
        "BENCH case=demo tier=idf cycle=0 reset={} cause={} mask=0x{:x} result=boot guard_us=0 drain_us=0",
        reset_str, cause_str, mask
    );

    // Configure GPIO4 as an input with the pull against the wake level instead of
    // relying on the reset/JTAG pad defaults; the arming call only sets wake registers.
    // The driver lives until the chip sleeps or restarts, as the loop never exits.
    // Taken once so no other driver can own the pins; the PIR pin is addressed by number.
    let _peripherals = Peripherals::take()?;

    if AWAKE_PROBE_MS > 0 {
        awake_probe()?;
    }

    // SAFETY: PIR_PIN is 0-5 (const-asserted), the probe's drivers are dropped, and this is
    // the only live instance of the pin until the chip sleeps or restarts.
    let pir = PinDriver::input(unsafe { AnyIOPin::steal(PIR_PIN) }, Pull::Down)?;
    println!("GPIO{} level before sleep: {:?}", PIR_PIN, pir.get_level());

    // GPIO wake with isolation is refused (IsolationConflict) until bench case I4.
    let mut manager = EspSleepManager {
        isolate_gpio: false,
    };
    let sources = [
        WakeSource::GpioLevel {
            pin_mask: 1 << PIR_PIN,
            level: GpioWakeLevel::AnyHigh,
        },
        WakeSource::Timer {
            duration_ms: TIMER_MS,
        },
    ];

    println!("Sleeping: GPIO{} AnyHigh + {} ms timer", PIR_PIN, TIMER_MS);

    let mut rejected: u32 = 0;
    loop {
        // On success the chip is asleep and this call does not return.
        match manager.try_sleep(&sources) {
            Ok(()) => {
                // Only reachable if the platform returned without sleeping.
                println!(
                    "BENCH case=demo tier=idf cycle=0 reset={} cause={} mask=0x{:x} result=ok guard_us=0 drain_us=0",
                    reset_str, cause_str, mask
                );
                break;
            }
            Err(e) if e.kind() == SleepErrorKind::Rejected => {
                rejected += 1;
                if rejected == 1 {
                    println!("try_sleep rejected (first): {e}, kind={:?}", e.kind());
                } else if rejected % LOG_EVERY == 0 {
                    println!("try_sleep rejected: {} so far", rejected);
                }
                if BENCH_LOG {
                    println!(
                    "BENCH case=demo tier=idf cycle=0 reset={} cause={} mask=0x{:x} result=err:Rejected guard_us=0 drain_us=0",
                        reset_str, cause_str, mask
                    );
                }
                FreeRtos::delay_ms(RETRY_DELAY_MS);
            }
            Err(e) => {
                println!(
                    "BENCH case=demo tier=idf cycle=0 reset={} cause={} mask=0x{:x} result=err:{:?} guard_us=0 drain_us=0",
                    reset_str,
                    cause_str,
                    mask,
                    e.kind()
                );
                println!("try_sleep failed: {e} — falling back to a timer-only sleep");
                // A restart would retry the same failing configuration in a loop, so sleep
                // on the timer alone; sleep() does not return unless that fails too.
                if let Err(e) = manager.sleep(&[WakeSource::Timer {
                    duration_ms: TIMER_MS,
                }]) {
                    println!("timer-only sleep failed: {e} — restarting");
                    // SAFETY: esp_restart never returns; staying awake would drain the battery.
                    unsafe { sys::esp_restart() };
                }
            }
        }
    }

    // try_sleep returned Ok without sleeping; do not stay awake on battery.
    // SAFETY: esp_restart never returns.
    unsafe { sys::esp_restart() };
    #[allow(unreachable_code)]
    Ok(())
}
