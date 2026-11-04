//! Heltec WiFi LoRa 32 V3 (ESP32-S3) — Battery Monitor Example (esp-hal)
//!
//! Reads the battery voltage once a second and prints the averaged status.
//! The esp-hal counterpart of `idf_esp32s3_battery`.
//!
//! ## Board
//!
//! - **Board:** Heltec WiFi LoRa 32 V3.
//! - **Pin:** GPIO1 (ADC1_CH0) behind the always-on onboard divider (no GPIO37 handling needed); `BatteryConfig::heltec_v3()` carries the divider ratio.
//!
//! With USB connected and no battery the always-on divider reads the charger rail, so a false ~80 % can appear; see `docs/key-insights.md`.
//!
//! ## Run
//!
//! ```shell
//! just run-hal-s3 hal_s3_battery
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use esp_hal::delay::Delay;
use esp_println::println;
use rustyfarian_esp_hal_power::{BatteryConfig, BatteryMonitor, EspHalAdcBatteryMonitor};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    loop {}
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    let mut battery = EspHalAdcBatteryMonitor::new(
        peripherals.ADC1,
        peripherals.GPIO1,
        BatteryConfig::heltec_v3(),
    );

    loop {
        println!("Battery: {}", battery.read());
        delay.delay_millis(1_000);
    }
}
