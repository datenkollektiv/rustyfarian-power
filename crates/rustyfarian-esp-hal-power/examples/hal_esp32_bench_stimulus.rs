//! ESP32-WROOM — Bench Stimulus for the ESP32-C3 Sleep Bench (esp-hal)
//!
//! A host-controlled signal generator for the C3 deep-sleep bench (`hal_c3_sleep_bench`).
//! The host (`scripts/c3-bench.py`) sends ASCII commands over UART0 and this board drives or
//! tri-states two output pins that feed the C3 wake pins through series resistors.
//! It replaces a PIR sensor: the host decides exactly when, and for how long, the C3 wake pin is HIGH.
//! The firmware is bench-only and is excluded from the published package.
//!
//! ## Board
//!
//! - **Chip:** ESP32 (Xtensa LX6), ESP32-WROOM-32 module on a USB-UART dev board.
//! - **Console / control:** UART0 (GPIO1 TX, GPIO3 RX) at 115200 8N1, the USB-UART bridge of the board.
//!   Nothing else prints on UART0, so replies never interleave with logs.
//! - **ch1:** GPIO25, wired to the C3 wake pin GPIO4.
//! - **ch2:** GPIO26, wired to the C3 foreign pad GPIO5 (case H8).
//!
//! Both channels are high-Z through reset and until the first command.
//! A tri-stated channel has output, input and pulls disabled, so the C3-side pull-down owns the line.
//!
//! ## Wiring
//!
//! | Signal  | WROOM  | Connection                                          |
//! |:--------|:-------|:----------------------------------------------------|
//! | ch1     | GPIO25 | 1 kΩ in series to C3 GPIO4                          |
//! | ch2     | GPIO26 | 1 kΩ in series to C3 GPIO5                          |
//! | Pull-down | C3 side | 10 kΩ from C3 GPIO4 to GND, and from GPIO5 to GND |
//! | Ground  | GND    | common with the C3, the logic analyser and the PPK2 |
//!
//! The series resistor limits current if both ends ever drive against each other.
//! Send `ZZ` before unplugging the C3: a driven channel would back-power it through the 1 kΩ.
//! Keep the stimulus tri-stated (`ZZ`, or simply reset) during PPK2 current measurements.
//!
//! ## Protocol
//!
//! ASCII lines terminated by `\n`; `\r` is ignored, commands are case-insensitive.
//! Channels are `1` or `2`; times are decimal microseconds (`u32`).
//! Lines are capped at 64 bytes.
//! The boot banner is `STIM ready v1`.
//!
//! | Command                                       | Meaning                                                    | Reply                                                          |
//! |:----------------------------------------------|:-----------------------------------------------------------|:---------------------------------------------------------------|
//! | `PING`                                        | liveness                                                   | `PONG hal_esp32_bench_stimulus v1`                             |
//! | `VER`                                         | version and pins                                           | `VER 1 ch1=GPIO25 ch2=GPIO26`                                  |
//! | `T`                                           | device time in microseconds since boot                     | `T <now_us>`                                                   |
//! | `H <ch>` / `L <ch>` / `Z <ch>`                | drive high / drive low / tri-state now, cancels the channel's schedule | `OK H <ch>`, `OK L <ch>`, `OK Z <ch>`              |
//! | `ZZ`                                          | tri-state both channels, cancel both schedules             | `OK ZZ`                                                        |
//! | `P <ch> <H\|L> <width_us> [<delay_us> [<end H\|L\|Z>]]` | after the delay drive the level for the width, then the end state (default `Z`); replaces a pending schedule | `OK P <ch> armed`, later `DONE P <ch> t_start_us=<n> t_end_us=<n>` |
//! | `X <ch>`                                      | cancel a pending pulse, leave the pin as it is             | `OK X <ch>`                                                    |
//! | anything else                                 | -                                                          | `ERR unknown`, `ERR badarg`, `ERR badch` or `ERR overflow`     |
//!
//! `DONE` carries the device-clock times (`T` units) at which the edges were actually applied.
//! `X` and a replacing `P` leave a pulse that is already driving at its level; send `Z <ch>` to stop it.
//! The main loop polls `esp_hal::time::Instant` (1 us resolution on the ESP32), so edge jitter is
//! a few microseconds plus the time to write a pending reply.
//!
//! ## Run
//!
//! ```shell
//! just flash-hal-esp32 hal_esp32_bench_stimulus
//! ```
//!
//! The WROOM may reset when the host opens its serial port; wait for `STIM ready v1`, then `PING`.
//! A panic resets the chip, which leaves both channels tri-stated.

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use core::fmt::Write as _;

use esp_hal::{
    gpio::{Flex, InputConfig, Level, Pull},
    time::Instant,
    uart::{Config as UartConfig, Uart},
};

/// Maximum accepted line length in bytes (without the terminator).
const LINE_CAP: usize = 64;

/// Capacity of the reply buffer; the longest reply is a `DONE` line.
const REPLY_CAP: usize = 96;

/// Output state of a channel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    High,
    Low,
    Z,
}

impl State {
    fn letter(self) -> char {
        match self {
            State::High => 'H',
            State::Low => 'L',
            State::Z => 'Z',
        }
    }
}

/// One scheduled pulse on a channel.
#[derive(Clone, Copy)]
struct Slot {
    start_at: u64,
    end_at: u64,
    level: State,
    end_state: State,
    /// Device time the level was applied; `None` while the pulse is pending.
    t_start: Option<u64>,
}

/// A stimulus channel: a GPIO and its optional pulse schedule.
struct Channel<'d> {
    pin: Flex<'d>,
    slot: Option<Slot>,
}

impl<'d> Channel<'d> {
    /// Takes the pin and leaves it tri-stated.
    fn new(pin: Flex<'d>) -> Self {
        let mut ch = Self { pin, slot: None };
        ch.pin
            .apply_input_config(&InputConfig::default().with_pull(Pull::None));
        ch.set(State::Z);
        ch
    }

    /// Applies a state: level first, then the output enable, so no glitch appears on enable.
    fn set(&mut self, state: State) {
        match state {
            State::High => {
                self.pin.set_level(Level::High);
                self.pin.set_output_enable(true);
            }
            State::Low => {
                self.pin.set_level(Level::Low);
                self.pin.set_output_enable(true);
            }
            State::Z => {
                self.pin.set_output_enable(false);
                self.pin.set_input_enable(false);
            }
        }
    }
}

/// Fixed-capacity text buffer; output beyond the capacity is dropped.
struct Text<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Text<N> {
    const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> core::fmt::Write for Text<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(N - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// Microseconds since boot.
fn now_us() -> u64 {
    Instant::now().duration_since_epoch().as_micros()
}

/// Writes all of `data` to the UART (blocks only while the TX FIFO drains).
fn send(uart: &mut Uart<'_, esp_hal::Blocking>, data: &[u8]) {
    let mut rest = data;
    while !rest.is_empty() {
        match uart.write(rest) {
            Ok(n) => rest = &rest[n..],
            Err(_) => return,
        }
    }
}

/// Formats and sends one reply line.
fn reply(uart: &mut Uart<'_, esp_hal::Blocking>, args: core::fmt::Arguments<'_>) {
    let mut t = Text::<REPLY_CAP>::new();
    let _ = t.write_fmt(args);
    let _ = t.write_str("\n");
    send(uart, t.as_bytes());
}

fn parse_channel(tok: Option<&str>) -> Result<usize, &'static str> {
    match tok {
        Some("1") => Ok(0),
        Some("2") => Ok(1),
        Some(_) => Err("badch"),
        None => Err("badarg"),
    }
}

fn parse_state(tok: Option<&str>, allow_z: bool) -> Option<State> {
    match tok {
        Some("H") => Some(State::High),
        Some("L") => Some(State::Low),
        Some("Z") if allow_z => Some(State::Z),
        _ => None,
    }
}

fn parse_u32(tok: Option<&str>) -> Option<u32> {
    tok?.parse().ok()
}

/// Executes one complete command line and sends its reply.
fn handle(line: &[u8], uart: &mut Uart<'_, esp_hal::Blocking>, chans: &mut [Channel<'_>; 2]) {
    let Ok(text) = core::str::from_utf8(line) else {
        reply(uart, format_args!("ERR unknown"));
        return;
    };
    let mut it = text.split_ascii_whitespace();
    let Some(cmd) = it.next() else {
        return;
    };
    let result = match cmd {
        "PING" => {
            reply(uart, format_args!("PONG hal_esp32_bench_stimulus v1"));
            Ok(())
        }
        "VER" => {
            reply(uart, format_args!("VER 1 ch1=GPIO25 ch2=GPIO26"));
            Ok(())
        }
        "T" => {
            reply(uart, format_args!("T {}", now_us()));
            Ok(())
        }
        "H" | "L" | "Z" => parse_channel(it.next()).and_then(|i| {
            if it.next().is_some() {
                return Err("badarg");
            }
            let state = match cmd {
                "H" => State::High,
                "L" => State::Low,
                _ => State::Z,
            };
            chans[i].slot = None;
            chans[i].set(state);
            reply(uart, format_args!("OK {} {}", state.letter(), i + 1));
            Ok(())
        }),
        "ZZ" => {
            for ch in chans.iter_mut() {
                ch.slot = None;
                ch.set(State::Z);
            }
            reply(uart, format_args!("OK ZZ"));
            Ok(())
        }
        "X" => parse_channel(it.next()).map(|i| {
            chans[i].slot = None;
            reply(uart, format_args!("OK X {}", i + 1));
        }),
        "P" => parse_channel(it.next()).and_then(|i| {
            let level = parse_state(it.next(), false).ok_or("badarg")?;
            let width = u64::from(parse_u32(it.next()).ok_or("badarg")?);
            let delay = match it.next() {
                Some(tok) => u64::from(parse_u32(Some(tok)).ok_or("badarg")?),
                None => 0,
            };
            let end_state = match it.next() {
                Some(tok) => parse_state(Some(tok), true).ok_or("badarg")?,
                None => State::Z,
            };
            if it.next().is_some() {
                return Err("badarg");
            }
            let start_at = now_us() + delay;
            chans[i].slot = Some(Slot {
                start_at,
                end_at: start_at + width,
                level,
                end_state,
                t_start: None,
            });
            reply(uart, format_args!("OK P {} armed", i + 1));
            Ok(())
        }),
        _ => Err("unknown"),
    };
    if let Err(kind) = result {
        reply(uart, format_args!("ERR {kind}"));
    }
}

/// Applies due edges of both schedules; non-blocking.
fn service(uart: &mut Uart<'_, esp_hal::Blocking>, chans: &mut [Channel<'_>; 2]) {
    for (i, ch) in chans.iter_mut().enumerate() {
        let Some(mut slot) = ch.slot else {
            continue;
        };
        let now = now_us();
        if slot.t_start.is_none() && now >= slot.start_at {
            ch.set(slot.level);
            slot.t_start = Some(now_us());
            ch.slot = Some(slot);
        }
        if let Some(t_start) = slot.t_start {
            if now_us() >= slot.end_at {
                ch.set(slot.end_state);
                let t_end = now_us();
                ch.slot = None;
                reply(
                    uart,
                    format_args!("DONE P {} t_start_us={t_start} t_end_us={t_end}", i + 1),
                );
            }
        }
    }
}

/// Resets the chip: unattended hardware must recover, and a reset tri-states both channels.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    esp_hal::system::software_reset()
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let mut uart = Uart::new(peripherals.UART0, UartConfig::default())
        .expect("UART0 config")
        .with_rx(peripherals.GPIO3)
        .with_tx(peripherals.GPIO1);

    let mut chans = [
        Channel::new(Flex::new(peripherals.GPIO25)),
        Channel::new(Flex::new(peripherals.GPIO26)),
    ];

    send(&mut uart, b"STIM ready v1\n");

    let mut line = [0u8; LINE_CAP];
    let mut len = 0usize;
    let mut overflow = false;
    let mut chunk = [0u8; 32];

    loop {
        while uart.read_ready() {
            let Ok(n) = uart.read(&mut chunk) else {
                break;
            };
            if n == 0 {
                break;
            }
            for &b in &chunk[..n] {
                match b {
                    b'\r' => {}
                    b'\n' => {
                        if overflow {
                            reply(&mut uart, format_args!("ERR overflow"));
                        } else {
                            handle(&line[..len], &mut uart, &mut chans);
                        }
                        len = 0;
                        overflow = false;
                    }
                    _ if overflow => {}
                    _ if len == LINE_CAP => overflow = true,
                    _ => {
                        line[len] = b.to_ascii_uppercase();
                        len += 1;
                    }
                }
            }
        }
        service(&mut uart, &mut chans);
    }
}
