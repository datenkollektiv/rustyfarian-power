//! ESP32-C3-MINI-1 — Deep-Sleep Bench Firmware (esp-hal tier)
//!
//! One firmware, one case per build: `BENCH_CASE` selects the case at build time and an unset or
//! unknown value fails the build.
//! The cases and pass criteria are in `docs/bench/c3-devkitm1-sleep-bench-plan-v1.md`.
//! Needs the `bench` feature, which drives a marker pad around the arm-to-sleep path.
//!
//! ```shell
//! BENCH_CASE=H1 just build-example-hal-c3 hal_c3_sleep_bench esp32c3,unstable,examples,bench
//! BENCH_CASE=H1 just run-hal-c3 hal_c3_sleep_bench esp32c3,unstable,examples,bench
//! ```
//!
//! ## Cases
//!
//! | Case | What the firmware does                                                                    |
//! |:-----|:------------------------------------------------------------------------------------------|
//! | W1   | GPIO4 `AnyHigh` + 60 s timer, retry while rejected                                        |
//! | W2   | GPIO4 `AnyHigh` + 5 s timer, retry while rejected                                         |
//! | W3   | as W2; the host pulses GPIO4 at a random 0-6 s                                            |
//! | H1   | one `try_sleep` with GPIO4 `AnyHigh` + 60 s timer; reports the outcome, then a 2 s sleep  |
//! | H2   | boot wake-cause reads, one `try_sleep`, then the same reads again                         |
//! | H3   | custom GPIO handler counts calls and disables the pin trigger; one `try_sleep`            |
//! | H4   | retry loop, one `BENCH` line per rejected attempt                                         |
//! | H5   | short-timer sweep, knobs below; the H3 handler is active                                  |
//! | H6   | timer range limits: 0 ms and `MAX_TIMER_MS + 1`                                           |
//! | H7   | UART1 with CTS held inactive and 16 bytes queued, then `try_sleep`                        |
//! | H8   | GPIO5 armed for wake and listening, `try_sleep` binds GPIO4 only                          |
//! | R1   | as W2, with the RTC-fast boot counter as `cycle`                                          |
//!
//! Cases that do not end in a real sleep (H1-H3, H5-H7) finish with a 2 s timer-only sleep so the
//! host sees one boot per cycle; H7 restarts instead, because its stuck UART1 FIFO would make that
//! sleep fail too.
//!
//! ## H5 knobs
//!
//! | Variable          | Values            | Default | Meaning                                         |
//! |:------------------|:------------------|:--------|:------------------------------------------------|
//! | `BENCH_TIMER_MS`  | decimal ms        | 5       | timer duration                                  |
//! | `BENCH_METHOD`    | `try` or `sleep`  | `try`   | `try_sleep` or `sleep`                          |
//! | `BENCH_UART_Q`    | `0` or `1`        | 0       | 1 queues a 128-byte line on UART0 at the call   |
//! | `BENCH_GPIO`      | `0` or `1`        | 1       | 1 adds GPIO4 `AnyHigh` to the timer             |
//!
//! ## Wiring
//!
//! | Signal       | Pad    | Connection                                                  |
//! |:-------------|:-------|:------------------------------------------------------------|
//! | Wake (PIR)   | GPIO4  | PIR or stimulus via 1 kΩ; 10 kΩ pull-down to GND            |
//! | Foreign pad  | GPIO5  | H8 only: stimulus via 1 kΩ; 10 kΩ pull-down to GND          |
//! | Guard marker | GPIO10 | logic analyser and PPK2 D0; output, low at arm, high after  |
//! | Console      | GPIO20/21 | CH340 on UART0                                           |
//!
//! H7 needs no pad: UART1's CTS input is tied to a constant HIGH (inactive) inside the GPIO matrix
//! and no TX pad is connected, so nothing is wired and no pad is claimed.
//! Avoid GPIO2, GPIO8 and GPIO9 (strapping), GPIO18/19 (USB-Serial-JTAG) and GPIO20/21 (console).
//!
//! ## Log lines
//!
//! One `BENCH` line per boot (`result=boot`) and per sleep attempt that returns:
//!
//! ```text
//! BENCH case=H1 tier=hal cycle=0 reset=DeepSleep cause=Gpio mask=0x10 result=err:Rejected guard_us=48 drain_us=3
//! ```
//!
//! `result` carries the error variant name only; payloads and the `Display` text go on a `NOTE` line.
//! `guard_us` and `drain_us` come from the library's `bench::last()`.
//! `guard_us` is only measured on a rejected `try_sleep`.
//! `drain_us` is stored inside `arm()` and so is also valid on a rejected attempt only: a successful
//! sleep never returns to print it.
//!
//! ## Cycle counter
//!
//! `cycle` counts boots in an RTC-fast persistent static for R1 and H5, and is 0 for every other case.
//! The counter restarts at 0 on any boot that is not a deep-sleep wake (power-on, flash reset,
//! software reset, panic), so the first boot of a run reads `cycle=0`.

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use core::fmt::Display;
use core::sync::atomic::{AtomicU32, Ordering};

use esp_hal::{
    delay::Delay,
    gpio::{Event, Input, InputConfig, Io, Level, Output, OutputConfig, Pull, WakeupConfig},
    handler,
    peripherals::{GPIO, GPIO5, IO_MUX, UART1},
    rtc_cntl::{reset_reason, SocResetReason},
    system::{software_reset, Cpu},
    uart::{Config as UartConfig, CtsConfig, HwFlowControl, RtsConfig, Uart},
};
use esp_println::println;
use rustyfarian_esp_hal_power::{
    sleep::bench::{self, BenchTimings},
    Error, EspHalSleepManager, EspHalWakeCauseSource, GpioWakeLevel, SleepError, SleepErrorKind,
    SleepManager, WakeCause, WakeCauseSource, WakeSource, MAX_TIMER_MS,
};

/// PIR / stimulus pin; bit position in the wake mask.
const PIR_PIN: u8 = 4;

/// Guard marker pad (not a low-power pad, not a strapping pad).
const MARKER_PIN: u8 = 10;

/// Delay between rejected `try_sleep` attempts.
const RETRY_DELAY_MS: u32 = 50;

/// Log a compact count line every this many rejections.
const LOG_EVERY: u32 = 20;

/// Timer of the 2 s timer-only sleep that starts the next cycle.
const NEXT_CYCLE_MS: u64 = 2_000;

/// Timer fallback of the wake-path cases that wait for the PIR.
const LONG_TIMER_MS: u64 = 60_000;

/// Timer of W2, W3 and R1.
const SHORT_TIMER_MS: u64 = 5_000;

/// Bytes queued on UART1 for H7.
const H7_QUEUED: [u8; 16] = *b"0123456789abcdef";

/// A 127-character line, so the newline fills the 128-byte UART0 TX FIFO (H5 `BENCH_UART_Q`).
const FILLER: &str = "QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ";

const _: () = assert!(FILLER.len() == 127, "FILLER must be 127 bytes");

// ── Build-time case selection ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Case {
    W1,
    W2,
    W3,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    H7,
    H8,
    R1,
}

impl Case {
    const fn name(self) -> &'static str {
        match self {
            Case::W1 => "W1",
            Case::W2 => "W2",
            Case::W3 => "W3",
            Case::H1 => "H1",
            Case::H2 => "H2",
            Case::H3 => "H3",
            Case::H4 => "H4",
            Case::H5 => "H5",
            Case::H6 => "H6",
            Case::H7 => "H7",
            Case::H8 => "H8",
            Case::R1 => "R1",
        }
    }

    /// Cases whose `cycle` field comes from the RTC counter.
    const fn counts_cycles(self) -> bool {
        matches!(self, Case::R1 | Case::H5)
    }
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
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

const CASES: [Case; 12] = [
    Case::W1,
    Case::W2,
    Case::W3,
    Case::H1,
    Case::H2,
    Case::H3,
    Case::H4,
    Case::H5,
    Case::H6,
    Case::H7,
    Case::H8,
    Case::R1,
];

/// Case lookup usable in a const context; an unknown name fails the build.
const fn parse_case(s: &str) -> Case {
    let mut i = 0;
    while i < CASES.len() {
        if str_eq(s, CASES[i].name()) {
            return CASES[i];
        }
        i += 1;
    }
    panic!("BENCH_CASE must be one of W1 W2 W3 H1 H2 H3 H4 H5 H6 H7 H8 R1")
}

const CASE: Case = match option_env!("BENCH_CASE") {
    Some(s) => parse_case(s),
    None => panic!("BENCH_CASE is not set; use one of W1 W2 W3 H1 H2 H3 H4 H5 H6 H7 H8 R1"),
};

/// Decimal parser usable in a const context; fails the build on anything but digits.
const fn parse_u64(s: &str) -> u64 {
    let b = s.as_bytes();
    assert!(!b.is_empty(), "BENCH_TIMER_MS must not be empty");
    let mut i = 0;
    let mut v: u64 = 0;
    while i < b.len() {
        assert!(
            b[i].is_ascii_digit(),
            "BENCH_TIMER_MS must be decimal digits"
        );
        v = v * 10 + (b[i] - b'0') as u64;
        i += 1;
    }
    v
}

/// `0` or `1`; anything else fails the build.
const fn parse_flag(s: &str) -> bool {
    if str_eq(s, "1") {
        true
    } else if str_eq(s, "0") {
        false
    } else {
        panic!("BENCH_UART_Q and BENCH_GPIO must be 0 or 1")
    }
}

#[derive(Clone, Copy)]
enum Method {
    Try,
    Sleep,
}

const fn parse_method(s: &str) -> Method {
    if str_eq(s, "try") {
        Method::Try
    } else if str_eq(s, "sleep") {
        Method::Sleep
    } else {
        panic!("BENCH_METHOD must be try or sleep")
    }
}

const H5_TIMER_MS: u64 = match option_env!("BENCH_TIMER_MS") {
    Some(s) => parse_u64(s),
    None => 5,
};

const H5_METHOD: Method = match option_env!("BENCH_METHOD") {
    Some(s) => parse_method(s),
    None => Method::Try,
};

const H5_UART_Q: bool = match option_env!("BENCH_UART_Q") {
    Some(s) => parse_flag(s),
    None => false,
};

const H5_GPIO: bool = match option_env!("BENCH_GPIO") {
    Some(s) => parse_flag(s),
    None => true,
};

// ── Shared state ──────────────────────────────────────────────────────────────────────────────

type Manager = EspHalSleepManager<'static>;
type Pir = Input<'static>;

/// Boot counter that survives deep sleep (RTC fast memory); see `next_cycle`.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut RTC_CYCLE: u32 = 0;

/// Calls of the custom GPIO handler (H3, H5). Only the handler writes it, so load/store suffices
/// (the C3 has no atomic read-modify-write).
static HANDLER_CALLS: AtomicU32 = AtomicU32::new(0);

/// Advances the RTC counter: +1 on a deep-sleep wake, back to 0 on any other boot.
fn next_cycle(woke_from_deep_sleep: bool) -> u32 {
    // SAFETY: single core; called once from `main` before any interrupt handler is installed,
    // and no handler touches the counter.
    unsafe {
        let counter = &raw mut RTC_CYCLE;
        let next = if woke_from_deep_sleep {
            counter.read_volatile().wrapping_add(1)
        } else {
            0
        };
        counter.write_volatile(next);
        next
    }
}

/// Identity of this boot, printed on every `BENCH` line.
struct Ctx {
    reset: &'static str,
    cause: &'static str,
    mask: u64,
    cycle: u32,
    wake: WakeCause,
}

impl Ctx {
    fn line(&self, result: impl Display, t: BenchTimings) {
        println!(
            "BENCH case={} tier=hal cycle={} reset={} cause={} mask=0x{:x} result={} guard_us={} drain_us={}",
            CASE.name(),
            self.cycle,
            self.reset,
            self.cause,
            self.mask,
            result,
            t.guard_us,
            t.drain_us
        );
    }

    /// Prints the `BENCH` line for a sleep return, and the details on a `NOTE` line.
    fn report(&self, result: &Result<(), SleepError>) {
        match result {
            Ok(()) => self.line("ok", bench::last()),
            Err(e) => {
                self.line(format_args!("err:{}", err_name(e)), bench::last());
                println!("NOTE {e} / {e:?}");
            }
        }
    }
}

/// Variant name of a `SleepError`, without its payload.
fn err_name(e: &SleepError) -> &'static str {
    match e {
        SleepError::GpioMaskZero => "GpioMaskZero",
        SleepError::GpioMaskOutOfRange { .. } => "GpioMaskOutOfRange",
        SleepError::MultipleTimers { .. } => "MultipleTimers",
        SleepError::MultipleGpioLevel => "MultipleGpioLevel",
        SleepError::NoWakeSource => "NoWakeSource",
        SleepError::Rejected => "Rejected",
        SleepError::TimerTooShort { .. } => "TimerTooShort",
        SleepError::TimerTooLong { .. } => "TimerTooLong",
        SleepError::PinNotBound { .. } => "PinNotBound",
        SleepError::ForeignWakePin { .. } => "ForeignWakePin",
        SleepError::UartBusy { .. } => "UartBusy",
        SleepError::IsolationConflict => "IsolationConflict",
        _ => "Other",
    }
}

/// Short name of the reset reason for the BENCH line.
fn reset_name(reason: Option<SocResetReason>) -> &'static str {
    match reason {
        Some(SocResetReason::ChipPowerOn) => "PowerOn",
        Some(SocResetReason::CoreSw | SocResetReason::Cpu0Sw) => "Software",
        Some(SocResetReason::CoreDeepSleep) => "DeepSleep",
        Some(SocResetReason::SysBrownOut) => "Brownout",
        Some(
            SocResetReason::CoreMwdt0
            | SocResetReason::CoreMwdt1
            | SocResetReason::CoreRtcWdt
            | SocResetReason::Cpu0Mwdt0
            | SocResetReason::Cpu0Mwdt1
            | SocResetReason::Cpu0RtcWdt
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

/// Prints the message and restarts: a hung board on the bench is worse than a reboot.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    restart()
}

/// Gives the console a moment to drain, then resets.
fn restart() -> ! {
    Delay::new().delay_millis(50);
    software_reset()
}

/// Custom GPIO handler for H3 and H5: counts the call and disables the GPIO4 trigger, the effect
/// a handler's `unlisten()` has. It must never run during an attempt; the library masks
/// interrupts from arming to cleanup.
#[handler]
fn gpio_handler() {
    HANDLER_CALLS.store(HANDLER_CALLS.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
    GPIO::regs()
        .pin(PIR_PIN as usize)
        .modify(|_, w| unsafe { w.int_type().bits(0) });
}

// ── Sleep helpers ─────────────────────────────────────────────────────────────────────────────

fn gpio_source() -> WakeSource {
    WakeSource::GpioLevel {
        pin_mask: 1 << PIR_PIN,
        level: GpioWakeLevel::AnyHigh,
    }
}

fn timer_source(duration_ms: u64) -> WakeSource {
    WakeSource::Timer { duration_ms }
}

/// Binds GPIO4 and makes one attempt. On success the chip sleeps and this does not return.
fn attempt(
    manager: &mut Manager,
    pir: &mut Pir,
    sources: &[WakeSource],
    method: Method,
) -> Result<(), SleepError> {
    let mut pins = [pir];
    let mut bound = manager.bind_wake_pins(&mut pins);
    match method {
        Method::Try => bound.try_sleep(sources),
        Method::Sleep => bound.sleep(sources),
    }
}

/// Timer-only sleep that starts the next cycle; restarts if even that fails.
fn next_cycle_sleep(manager: &mut Manager) -> ! {
    let mut none: [&mut Pir; 0] = [];
    let result = manager
        .bind_wake_pins(&mut none)
        .sleep(&[timer_source(NEXT_CYCLE_MS)]);
    if let Err(e) = result {
        println!(
            "NOTE timer-only sleep failed: err:{} / {e} — restarting",
            err_name(&e)
        );
    }
    restart()
}

/// Wake-path cases (W1-W3, H4, R1): retry `try_sleep` while the PIR holds the pin.
///
/// `log_each` prints one `BENCH` line per rejected attempt (H4).
fn retry_loop(ctx: &Ctx, manager: &mut Manager, pir: &mut Pir, timer_ms: u64, log_each: bool) -> ! {
    let delay = Delay::new();
    let sources = [gpio_source(), timer_source(timer_ms)];
    println!("Sleeping: GPIO{} AnyHigh + {} ms timer", PIR_PIN, timer_ms);

    let mut rejected: u32 = 0;
    loop {
        match attempt(manager, pir, &sources, Method::Try) {
            Err(e) if e.kind() == SleepErrorKind::Rejected => {
                rejected += 1;
                if rejected == 1 {
                    println!("try_sleep rejected (first): {e}, kind={:?}", e.kind());
                } else if rejected.is_multiple_of(LOG_EVERY) {
                    println!("try_sleep rejected: {} so far", rejected);
                }
                if log_each {
                    ctx.line("err:Rejected", bench::last());
                }
                delay.delay_millis(RETRY_DELAY_MS);
            }
            other => {
                // Only reachable if the platform returned without sleeping, or on a config error.
                ctx.report(&other);
                println!("try_sleep did not sleep — falling back to a timer-only sleep");
                next_cycle_sleep(manager);
            }
        }
    }
}

/// Installs the custom GPIO handler (H3, H5).
fn install_handler(io_mux: IO_MUX<'static>) {
    let mut io = Io::new(io_mux);
    io.set_interrupt_handler(gpio_handler);
}

// ── Cases ─────────────────────────────────────────────────────────────────────────────────────

/// H1: one attempt with GPIO4 held; records whether it is rejected or sleeps and wakes at once.
fn h1(ctx: &Ctx, manager: &mut Manager, pir: &mut Pir) -> ! {
    let sources = [gpio_source(), timer_source(LONG_TIMER_MS)];
    println!(
        "Sleeping: GPIO{} AnyHigh + {} ms timer",
        PIR_PIN, LONG_TIMER_MS
    );
    let result = attempt(manager, pir, &sources, Method::Try);
    ctx.report(&result);
    next_cycle_sleep(manager)
}

/// H2: reads the wake cause at boot, makes one rejected attempt, reads it again.
fn h2(ctx: &Ctx, manager: &mut Manager, pir: &mut Pir) -> ! {
    println!(
        "NOTE h2 read1 caused_wakeup={} cause={:?}",
        pir.caused_wakeup(),
        ctx.wake
    );
    let sources = [gpio_source(), timer_source(LONG_TIMER_MS)];
    let result = attempt(manager, pir, &sources, Method::Try);
    ctx.report(&result);
    let again = EspHalWakeCauseSource::capture(&[&*pir]).last_wake_cause();
    println!(
        "NOTE h2 read2 caused_wakeup={} cause={:?}",
        pir.caused_wakeup(),
        again
    );
    next_cycle_sleep(manager)
}

/// H3: a custom GPIO handler is installed and must not run during a rejected attempt.
fn h3(ctx: &Ctx, manager: &mut Manager, pir: &mut Pir, io_mux: IO_MUX<'static>) -> ! {
    install_handler(io_mux);
    let sources = [gpio_source(), timer_source(LONG_TIMER_MS)];
    let result = attempt(manager, pir, &sources, Method::Try);
    ctx.report(&result);
    println!(
        "NOTE h3 handler_calls={}",
        HANDLER_CALLS.load(Ordering::Relaxed)
    );
    next_cycle_sleep(manager)
}

/// H5: short-timer sweep; the knobs are build-time variables.
fn h5(ctx: &Ctx, manager: &mut Manager, pir: &mut Pir, io_mux: IO_MUX<'static>) -> ! {
    install_handler(io_mux);
    println!(
        "NOTE h5 timer_ms={} method={} uart_q={} gpio={}",
        H5_TIMER_MS,
        match H5_METHOD {
            Method::Try => "try",
            Method::Sleep => "sleep",
        },
        u8::from(H5_UART_Q),
        u8::from(H5_GPIO)
    );
    let both = [gpio_source(), timer_source(H5_TIMER_MS)];
    let sources = if H5_GPIO { &both[..] } else { &both[1..] };

    if H5_UART_Q {
        // The line is still in the TX FIFO at the call, so the drain has work to do.
        println!("{}", FILLER);
    } else {
        // Let the notes above leave the FIFO first, so the drain starts idle.
        Delay::new().delay_millis(50);
    }

    let result = attempt(manager, pir, sources, H5_METHOD);
    ctx.report(&result);
    next_cycle_sleep(manager)
}

/// H6: timer range limits; validation must reject both before touching any hardware.
///
/// `MIN_TIMER_MS - 1` is 0 at the provisional floor of 1 ms, so only one low value is tried.
fn h6(ctx: &Ctx, manager: &mut Manager, pir: &mut Pir) -> ! {
    for timer_ms in [0, MAX_TIMER_MS + 1] {
        println!("NOTE h6 timer_ms={}", timer_ms);
        let sources = [gpio_source(), timer_source(timer_ms)];
        let result = attempt(manager, pir, &sources, Method::Try);
        ctx.report(&result);
    }
    next_cycle_sleep(manager)
}

/// H7: UART1 transmits only when CTS is active; with CTS held inactive the 16 queued bytes stay
/// in the FIFO and the drain times out with `UartBusy { uart: 1 }` after about 200 ms.
fn h7(ctx: &Ctx, manager: &mut Manager, pir: &mut Pir, uart1: UART1<'static>) -> ! {
    let config = UartConfig::default().with_hw_flow_ctrl(HwFlowControl {
        cts: CtsConfig::Enabled,
        rts: RtsConfig::Disabled,
    });
    // CTS is active low; a constant HIGH keeps it inactive. No TX pad is connected.
    let mut uart = Uart::new(uart1, config)
        .expect("UART1 config")
        .with_cts(Level::High);
    let queued = uart.write(&H7_QUEUED).expect("UART1 write");
    println!("NOTE h7 queued={}", queued);

    let sources = [gpio_source(), timer_source(LONG_TIMER_MS)];
    let result = attempt(manager, pir, &sources, Method::Try);
    ctx.report(&result);

    // The FIFO stays stuck, so a timer-only sleep would fail too; a reset clears it.
    Delay::new().delay_millis(NEXT_CYCLE_MS as u32);
    restart()
}

/// H8: GPIO5 is armed for low-power wake and listening while only GPIO4 is bound.
///
/// After the `ForeignWakePin` report GPIO5 is released and the board sleeps on GPIO4; the host
/// pulses GPIO5 (no wake expected), then GPIO4 (wake expected).
fn h8(ctx: &Ctx, manager: &mut Manager, pir: &mut Pir, gpio5: GPIO5<'static>) -> ! {
    let mut foreign = Input::new(gpio5, InputConfig::default().with_pull(Pull::Down));
    let low_power = WakeupConfig::default().with_low_power_path(true);
    foreign
        .apply_wakeup_config(&low_power)
        .expect("GPIO5 low-power wake");
    foreign.listen(Event::HighLevel);

    let sources = [gpio_source(), timer_source(LONG_TIMER_MS)];
    let result = attempt(manager, pir, &sources, Method::Try);
    ctx.report(&result);

    foreign.unlisten();
    foreign.clear_interrupt();
    let _ = foreign.apply_wakeup_config(&WakeupConfig::default());
    retry_loop(ctx, manager, pir, LONG_TIMER_MS, false)
}

// ── Entry ─────────────────────────────────────────────────────────────────────────────────────

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    // Configure GPIO4 with the pull against the wake level instead of the JTAG pad defaults.
    let mut pir = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Down),
    );

    // Capture the wake cause first: a later sleep attempt, even a rejected one, overwrites the
    // per-pin wake record (ADR-001).
    let wake = EspHalWakeCauseSource::capture(&[&pir]).last_wake_cause();
    let reset = reset_reason(Cpu::ProCpu);
    let counter = next_cycle(reset == Some(SocResetReason::CoreDeepSleep));
    let (cause, mask) = cause_name(wake);
    let ctx = Ctx {
        reset: reset_name(reset),
        cause,
        mask,
        cycle: if CASE.counts_cycles() { counter } else { 0 },
        wake,
    };

    // The marker pad stays an output for the whole run; the library writes its set/clear registers.
    let _marker = Output::new(peripherals.GPIO10, Level::High, OutputConfig::default());
    bench::set_marker_pin(MARKER_PIN);

    println!("Reset reason: {:?} ({})", reset, ctx.reset);
    match wake {
        WakeCause::Gpio(m) => println!(
            "Wake cause: Gpio — fired pins: 0x{:x}, GPIO{} fired: {}",
            m.0,
            PIR_PIN,
            m.contains_pin(PIR_PIN)
        ),
        other => println!("Wake cause: {:?}", other),
    }
    ctx.line(
        "boot",
        BenchTimings {
            drain_us: 0,
            guard_us: 0,
        },
    );
    if CASE.counts_cycles() {
        println!("NOTE rtc_counter={}", counter);
    }
    println!("GPIO{} level before sleep: {:?}", PIR_PIN, pir.level());

    let mut manager = EspHalSleepManager::new(peripherals.LPWR);

    match CASE {
        Case::W1 => retry_loop(&ctx, &mut manager, &mut pir, LONG_TIMER_MS, false),
        Case::W2 | Case::W3 | Case::R1 => {
            retry_loop(&ctx, &mut manager, &mut pir, SHORT_TIMER_MS, false)
        }
        Case::H1 => h1(&ctx, &mut manager, &mut pir),
        Case::H2 => h2(&ctx, &mut manager, &mut pir),
        Case::H3 => h3(&ctx, &mut manager, &mut pir, peripherals.IO_MUX),
        Case::H4 => retry_loop(&ctx, &mut manager, &mut pir, LONG_TIMER_MS, true),
        Case::H5 => h5(&ctx, &mut manager, &mut pir, peripherals.IO_MUX),
        Case::H6 => h6(&ctx, &mut manager, &mut pir),
        Case::H7 => h7(&ctx, &mut manager, &mut pir, peripherals.UART1),
        Case::H8 => h8(&ctx, &mut manager, &mut pir, peripherals.GPIO5),
    }
}
