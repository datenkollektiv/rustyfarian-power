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
//! ## Bench case R4 (non-wake pad postcondition)
//!
//! `S3_R4=1` adds the R4 checks on top of R2; it is only meaningful with `S3_R2=1` and is ignored without it.
//! Unset, the example behaves exactly as without R4.
//! It checks that a rejected `try_sleep` leaves every non-wake pad and every user pad hold as the caller left it.
//! Three probe pads are configured once per boot, before the forced attempt.
//! They are ESP32-S3 digital-only pads (GPIO26-48), believed unwired on the CrowPanel 1.28"; the schematic is not verified, so abort the bench if anything on the board reacts.
//!
//! - GPIO38: output HIGH, input enabled, user hold (`Output::set_pad_hold`, which sets bit `38 - 21` of `RTC_CNTL_DIG_PAD_HOLD_REG` for a digital-only pad).
//! - GPIO39: output HIGH, input enabled, unheld.
//! - GPIO47: input with pull-up, unheld.
//!
//! GPIO38 keeps its hold across the real sleep that follows, because a user hold on a digital pad does not change the sleep decision.
//! The next boot releases and re-arms it.
//! In R4 forced cycles the GPIO1 and GPIO13 RTC holds stay enabled through the attempt: each pin goes `set_pad_hold(false)`, LOW, `set_pad_hold(true)`.
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
//! - `NONWAKE tier=hal path=<hw|guard> same=<0|1> holdok=<0|1> diff=<pin:field,...|->` once per forced cycle: `path=guard` with `S3_KEEP_AUTOHOLD=1`, else `hw`; `same=1` when S0 equals S1; `diff` names the differing pads and fields (`mux`, `osel`, `en`, `in`) or registers (`dighold`, `padhold`, `iso`).
//! - `NONWAKE selftest tier=hal detect=<0|1> hold38=<0|1>` once per boot: the comparator must report a difference after GPIO39 `FUN_IE` is cleared; `hold38` is the GPIO38 hold bit after setup.
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

use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, Ordering};

use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    rtc_cntl::{reset_reason, SocResetReason},
    system::Cpu,
    time::Instant,
};
use esp_println::println;
use rustyfarian_esp_hal_power::bench::{
    write_attempt, write_nonwake, write_pinread_edge, write_pinread_end, write_pinread_start,
    LineBuf, NonWakePath, PinWindow,
};
use rustyfarian_esp_hal_power::{
    EspHalRtcClock, EspHalSleepManager, EspHalWakeCauseSource, GpioWakeLevel, SleepError,
    SleepManager, WakeCause, WakeCauseSource, WakeSource, MAX_TIMER_MS,
};

/// Tier tag in the shared `ATTEMPT` and `PINREAD` bench lines.
const TIER: &str = "hal";

/// CST816S touch INT pin; bit position in the wake mask.
const TOUCH_INT_PIN: u8 = 5;

/// `LCD_3V3` rail enable; the CST816S is powered from it.
const LCD_RAIL_PIN: u8 = 1;

/// CST816S reset, active low.
const TOUCH_RST_PIN: u8 = 13;

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

/// Bench case R4 (`S3_R4=1` at build time); only active together with R2.
const R4: bool = R2
    && match option_env!("S3_R4") {
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
/// `RTC_CNTL_WIFI_FORCE_ISO` (bit 28 of `RTC_CNTL_DIG_ISO_REG`): a WiFi/BLE power-domain bit, not
/// pad state; esp-hal sleep entry clears it and a rejection does not restore it, so the R4 comparison
/// masks it out, as ADR-003 point 1 excludes power-domain configuration.
const DIG_ISO_WIFI_FORCE_ISO: u32 = 1 << 28;
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
    let mut line = LineBuf::<64>::new();
    let _ = write_attempt(&mut line, TIER, attempt, now_ms());
    println!("{}", line.as_str());
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
    LCD_RAIL_PIN,
    TOUCH_RST_PIN,
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
/// `RTC_CNTL_PAD_HOLD_REG` and `RTC_CNTL_DIG_PAD_HOLD_REG` (`rtc_cntl_reg.h`); `RTC_CNTL_DIG_ISO_REG` is defined above.
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
struct R4Probes<'d> {
    held: Output<'d>,
    _out: Output<'d>,
    _input: Input<'d>,
}

/// Configures the probe pads once per boot and runs the comparator self-test.
///
/// esp-hal's `set_pad_hold` also covers digital-only pads: it sets bit `gpio - 21` of `RTC_CNTL_DIG_PAD_HOLD_REG` for pads without a low-power number.
fn r4_setup<'d>(
    held: Output<'d>,
    out: Output<'d>,
    input: Input<'d>,
    delay: &Delay,
) -> R4Probes<'d> {
    let mut probes = R4Probes {
        held,
        _out: out,
        _input: input,
    };
    probes.held.set_pad_hold(false);
    probes.held.set_high();
    probes._out.set_high();
    set_input_enable(PROBE_HELD_PIN, true);
    set_input_enable(PROBE_OUT_PIN, true);
    probes.held.set_pad_hold(true);
    delay.delay_millis(HOLD_TEST_SETTLE_MS);

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
    println!("{}", line.as_str());
    probes
}

/// Hold test: a write to a held pad must not reach it; restores the commanded levels afterwards.
///
/// GPIO38 is written LOW and must keep reading HIGH; GPIO1 is written HIGH and must keep reading LOW.
/// GPIO38 has input enabled; GPIO1 does not, so its `FUN_IE` is set for the read and restored.
fn hold_test(probes: &mut R4Probes<'_>, lcd_rail: &mut Output<'_>, delay: &Delay) -> bool {
    probes.held.set_low();
    delay.delay_millis(HOLD_TEST_SETTLE_MS);
    let held_kept = gpio_in(PROBE_HELD_PIN);
    probes.held.set_high();

    let original = set_input_enable(LCD_RAIL_PIN, true);
    lcd_rail.set_high();
    delay.delay_millis(HOLD_TEST_SETTLE_MS);
    let rail_kept = !gpio_in(LCD_RAIL_PIN);
    lcd_rail.set_low();
    reg_write(io_mux_reg(LCD_RAIL_PIN), original);
    held_kept && rail_kept
}

/// Re-latches a held pin LOW without ever leaving it unheld: hold off, LOW, hold on.
fn relatch_low(pin: &mut Output<'_>) {
    pin.set_pad_hold(false);
    pin.set_low();
    pin.set_pad_hold(true);
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
    r4: Option<(&mut R4Probes<'_>, &Snap)>,
) {
    let start_level = touch_int.level();
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
    println!("{}", line.as_str());

    // R4: S1 is a read-only snapshot taken after the PINREAD start sample, so that sample stays the first read after the return.
    if let Some((probes, s0)) = r4 {
        let s1 = Snap::take();
        let holdok = hold_test(probes, lcd_rail, delay);
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
        println!("{}", line.as_str());
    }

    let after_reject = EspHalWakeCauseSource::capture(&[touch_int]).last_wake_cause();
    println!(
        "WAKECAUSE tier=hal boot={:?} after_reject={:?}",
        boot_cause, after_reject
    );

    let mut restored = false;
    while now_ms() - start_ms < REJECT_HOLD_MS {
        delay.delay_millis(PINREAD_PERIOD_MS);
        if !restored && now_ms() - start_ms >= RESTORE_AT_MS {
            restore_touch_power(lcd_rail, touch_rst, delay);
            restored = true;
        }
        let level = touch_int.level() == Level::High;
        if let Some(edge) = window.sample(level, now_ms()) {
            line.clear();
            let _ = write_pinread_edge(&mut line, TIER, TOUCH_INT_PIN, &edge);
            println!("{}", line.as_str());
        }
    }
    line.clear();
    let _ = write_pinread_end(&mut line, TIER, TOUCH_INT_PIN, &window.finish(), now_ms());
    println!("{}", line.as_str());
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    loop {}
}

/// The R2 boot lines, kept to print them again after the awake delay.
struct BootLine {
    cycle: u32,
    reset_str: &'static str,
    cause_str: &'static str,
    mask: u64,
    /// RTC-clock ms slept and whether the cycle that slept was forced.
    slept: Option<(u32, u32)>,
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
    let mut boot_line: Option<BootLine> = None;
    let print_boot = |line: &Option<BootLine>| {
        if let Some(BootLine {
            cycle,
            reset_str,
            cause_str,
            mask,
            slept,
        }) = line
        {
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
        boot_line = Some(BootLine {
            cycle,
            reset_str,
            cause_str,
            mask,
            slept,
        });
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

    // R4: configure the probe pads once per boot, before the forced attempt.
    let mut probes = if R4 {
        Some(r4_setup(
            Output::new(peripherals.GPIO38, Level::High, OutputConfig::default()),
            Output::new(peripherals.GPIO39, Level::High, OutputConfig::default()),
            Input::new(
                peripherals.GPIO47,
                InputConfig::default().with_pull(Pull::Up),
            ),
            &delay,
        ))
    } else {
        None
    };

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
        // R4 keeps the RTC holds on through the attempt and re-latches the pins LOW instead.
        if !R4 {
            lcd_rail.set_pad_hold(false);
            touch_rst.set_pad_hold(false);
        }
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
        if R4 {
            relatch_low(&mut touch_rst);
            relatch_low(&mut lcd_rail);
        } else {
            touch_rst.set_low();
            lcd_rail.set_low();
        }
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
        // R4: S0 immediately before the first forced attempt.
        let s0 = (R4 && window_pending).then(Snap::take);
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
                        probes.as_mut().zip(s0.as_ref()),
                    );
                }
                delay.delay_millis(RETRY_DELAY_MS)
            }
            Err(e) => panic!("try_sleep failed: {}", e),
            Ok(()) => panic!("try_sleep returned without sleeping"),
        }
    }
}
