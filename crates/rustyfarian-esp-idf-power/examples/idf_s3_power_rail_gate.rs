//! Heltec WiFi LoRa 32 V3 — Vext Power Rail Gate Example (ESP-IDF)
//!
//! Switches the `Vext` rail on GPIO36 ON for 10 s and OFF for 10 s, twice,
//! then deep-sleeps with the rail gated off for 30 s (timer wake) through `GatedSleep`.
//! After the wake the example logs the wake cause and powers the rail back on before the loop.
//!
//! ## Board
//!
//! - **Chip:** ESP32-S3 (Xtensa, dual core)
//! - **Board:** Heltec WiFi LoRa 32 V3
//! - **Rail:** `Vext_Ctrl` on GPIO36, pulled up, gating a P-MOSFET, so the rail is on while the pin is LOW.
//!   It feeds the OLED and the `Ve` header pins.
//!   The SX1262 is on the always-on 3V3 rail and cannot be gated.
//!
//! ## Wiring
//!
//! | Signal           | Connection                                         |
//! |:-----------------|:---------------------------------------------------|
//! | Load (optional)  | `Ve` header pin to an LED with a series resistor   |
//! | Meter (optional) | `Ve` header pin to GND, reads 3.3 V while ON       |
//!
//! The onboard OLED also loses power while the rail is OFF.
//!
//! `GatedSleep` powers the rail off before the sleep call.
//! Deep sleep does not return on hardware, so the rail is restored by `power_on` at the next boot.
//! The gate drives the pin HIGH (rail off) on construction, so the rail is off between boot and that call.
//!
//! The console is the S3's native USB-Serial-JTAG, which drops off the bus during deep sleep.
//! Output goes through `esp_rom_printf` because Rust `println!` does not reach the S3 console
//! (`docs/project-lore.md` → ESP-IDF / Rust on ESP32-S3).
//!
//! ## Run
//!
//! ```shell
//! just run idf_s3_power_rail_gate
//! ```
//!
//! ## Expected output (serial monitor)
//!
//! ```text
//! BENCH case=rail tier=idf phase=boot cause=PowerOn mask=0x0 powered=false
//! BENCH case=rail tier=idf phase=on cycle=0 powered=true
//! BENCH case=rail tier=idf phase=off cycle=0 powered=false
//! BENCH case=rail tier=idf phase=on cycle=1 powered=true
//! BENCH case=rail tier=idf phase=off cycle=1 powered=false
//! BENCH case=rail tier=idf phase=sleep timer_ms=30000
//! ```
//!
//! After 30 s the board boots again and prints `cause=Timer` on the boot line.

use std::ffi::CString;

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::sys;
use rustyfarian_esp_idf_power::{
    EspSleepManager, EspWakeCauseSource, GatedSleep, GpioRailGate, PowerRailGate, SleepManager,
    WakeCause, WakeCauseSource, WakeSource,
};

/// Time the rail stays in each state during the demo loop.
const HOLD_MS: u32 = 10_000;

/// Number of ON/OFF cycles before sleeping.
const CYCLES: u32 = 2;

/// Timer wake after the gated deep sleep.
const SLEEP_MS: u64 = 30_000;

/// Print a line via `esp_rom_printf`; Rust `println!` does not reach the S3 console.
fn say(msg: &str) {
    let Ok(c) = CString::new(msg) else { return };
    // SAFETY: `c` is a NUL-terminated string alive for the call, `c"%s\n"` a static format.
    unsafe { sys::esp_rom_printf(c"%s\n".as_ptr(), c.as_ptr()) };
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
    // SAFETY: link_patches must run before any ESP-IDF API call.
    sys::link_patches();

    // Read the wake cause first: a later sleep attempt can clear the hardware wake status.
    let (cause, mask) = cause_name(EspWakeCauseSource.last_wake_cause());

    let peripherals = Peripherals::take()?;
    let mut gate = GpioRailGate::heltec_v3_vext(peripherals.pins.gpio36)?;
    say(&format!(
        "BENCH case=rail tier=idf phase=boot cause={cause} mask=0x{mask:x} powered={}",
        gate.is_powered()
    ));

    for cycle in 0..CYCLES {
        gate.power_on()?;
        say(&format!(
            "BENCH case=rail tier=idf phase=on cycle={cycle} powered={}",
            gate.is_powered()
        ));
        FreeRtos::delay_ms(HOLD_MS);

        gate.power_off()?;
        say(&format!(
            "BENCH case=rail tier=idf phase=off cycle={cycle} powered={}",
            gate.is_powered()
        ));
        FreeRtos::delay_ms(HOLD_MS);
    }

    say(&format!(
        "BENCH case=rail tier=idf phase=sleep timer_ms={SLEEP_MS}"
    ));
    let mut gated = GatedSleep::new(gate, EspSleepManager::default());
    // On success the chip is asleep and this call does not return; the rail is restored at boot.
    if let Err(e) = gated.sleep(&[WakeSource::Timer {
        duration_ms: SLEEP_MS,
    }]) {
        say(&format!("gated sleep failed: {e:?} — restarting"));
        // SAFETY: esp_restart never returns; staying awake would drain the battery.
        unsafe { sys::esp_restart() };
    }

    // sleep returned Ok without sleeping; do not stay awake on battery.
    // SAFETY: esp_restart never returns.
    unsafe { sys::esp_restart() };
    #[allow(unreachable_code)]
    Ok(())
}
