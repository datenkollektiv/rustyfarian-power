//! Adafruit ESP32 Feather V2 (ESP32) — Battery Monitor Example (esp-hal)
//!
//! Reads the battery voltage once a second and prints the averaged status.
//! The esp-hal counterpart of `idf_esp32_battery`.
//!
//! ## Board
//!
//! - **Board:** Adafruit ESP32 Feather V2.
//! - **Pin:** GPIO35 (ADC1_CH7) behind the onboard 200 kΩ + 200 kΩ divider; `BatteryConfig::adafruit_feather_v2()` carries the divider ratio.
//!
//! With USB connected and no battery the charger holds the BAT net at about 4.16 V, so a false ~96 % can appear; see `docs/key-insights.md`.
//!
//! ## Run
//!
//! ```shell
//! just run-hal-esp32 hal_esp32_battery
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
        peripherals.GPIO35,
        BatteryConfig::adafruit_feather_v2(),
    );

    loop {
        println!("Battery: {}", battery.read());
        delay.delay_millis(1_000);
    }
}
