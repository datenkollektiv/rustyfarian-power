//! Heltec WiFi LoRa 32 V3 (ESP32-S3) — Vext Power Rail Gate Example (esp-hal)
//!
//! Powers the Vext rail at boot, holds it for 10 s, then deep-sleeps for 30 s with the rail off.
//! Built as `hal_s3_power_rail_gate--meter` (bench variant) it holds the rail for 90 s and sleeps for 120 s, so a meter can be read in each state.
//! On wake the cycle repeats: the rail is switched on again by the firmware, because deep sleep never returns.
//!
//! ## Board
//!
//! - **Board:** Heltec WiFi LoRa 32 V3.
//! - **Rail:** `Vext_Ctrl` on GPIO36, active-low, pulled up (P-MOSFET). Vext feeds the OLED and the `Ve` header pins.
//! - **SX1262:** on the always-on 3V3 rail; it cannot be gated, so the bench measures the Vext load only.
//!
//! `GatedSleep` powers the rail off, then sleeps; if the sleep fails it restores the rail.
//! No settle delay is modelled; the example waits 50 ms after power-on.
//!
//! ## Output
//!
//! ```text
//! BENCH case=rail tier=hal event=boot reset=DeepSleep powered=true
//! BENCH case=rail tier=hal event=sleep powered=false
//! ```
//!
//! ## Run
//!
//! ```shell
//! just run-hal-s3 hal_s3_power_rail_gate
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use esp_hal::{
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
};
use esp_println::println;
use rustyfarian_esp_hal_power::{
    EspHalSleepManager, GatedSleep, GpioRailGate, HeltecV3Vext, PowerRailGate, SleepManager,
    WakeSource,
};

use stoker::bench::variant_index;

/// `BENCH_VARIANT` at build time (`just bench-build NAME--VARIANT`): `meter` stretches the
/// phases so an operator can read a meter on `Ve` in each state (90 s awake with the rail
/// on, 120 s asleep with the rail off; see `bench/s3-heltec/rail-gate-ve.toml`). Unknown values fail the build.
const METER: bool = variant_index(option_env!("BENCH_VARIANT"), &["meter"]).is_some();

const AWAKE_MS: u32 = if METER { 90_000 } else { 10_000 };
const SLEEP_MS: u64 = if METER { 120_000 } else { 30_000 };
const SETTLE_MS: u32 = 50;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    loop {}
}

use rustyfarian_esp_hal_power::{EspHalResetReasonSource, ResetReasonSource};

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    let vext = Output::new(peripherals.GPIO36, Level::High, OutputConfig::default());
    let mut gate = match GpioRailGate::heltec_v3_vext(vext) {
        Ok(gate) => gate,
        Err(e) => panic!("gate init failed: {}", e),
    };
    if let Err(e) = gate.power_on() {
        panic!("power_on failed: {}", e);
    }
    delay.delay_millis(SETTLE_MS);
    println!(
        "BENCH case=rail tier=hal event=boot reset={} powered={}",
        EspHalResetReasonSource.last_reset_reason().name(),
        gate.is_powered()
    );

    delay.delay_millis(AWAKE_MS);

    let mut manager = EspHalSleepManager::new(peripherals.LPWR);
    let sources = [WakeSource::Timer {
        duration_ms: SLEEP_MS,
    }];

    println!("BENCH case=rail tier=hal event=sleep powered=false");
    let mut gated = GatedSleep::new(gate, manager.bind_wake_pins(&mut []));
    // On success the chip is asleep and this call does not return.
    match gated.sleep(&sources) {
        Err(e) => panic!("sleep failed: {:?}", e),
        Ok(()) => panic!("sleep returned without sleeping"),
    }
}
