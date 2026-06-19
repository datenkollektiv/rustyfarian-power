//! Adafruit ESP32 Feather V2 — Battery Monitor Example (ESP-IDF)
//!
//! Reads the battery voltage via GPIO35 on an Adafruit ESP32 Feather V2,
//! logs the wake cause and battery status, then enters deep sleep for 60 seconds.
//! On the next wake the cycle repeats, making this a minimal duty-cycle sensor loop.
//!
//! ## Board
//!
//! - **Chip:** ESP32 (original, Xtensa LX6 dual-core)
//! - **Board:** Adafruit ESP32 Feather V2 (https://www.adafruit.com/product/5400)
//!
//! ## Wiring
//!
//! No external wiring is needed.
//! The Feather V2 routes the LiPo battery through an onboard 200 kΩ + 200 kΩ
//! voltage divider to **GPIO35** (ADC1_CH7).
//!
//! ```text
//! LiPo+ ──[200 kΩ]──┬──[200 kΩ]── GND
//!                   └──► GPIO35 (ADC1_CH7)
//! ```
//!
//! ## No charge-status pin (and what that means for "no battery")
//!
//! The stock Feather V2 does **not** expose the MCP73831 charge status (STAT) or a
//! USB-VBUS-detect signal on any readable GPIO. Confirmed against the board's EagleCAD
//! schematic (rev F): STAT runs `VBUS → CHG LED → 5.1 kΩ → STAT` and connects to nothing
//! else — the charger drives only the orange CHG LED, with no trace to the ESP32. GPIO13
//! is the user LED. Charging is therefore not reported here.
//!
//! A direct consequence: **with USB connected, a missing battery cannot be detected in
//! software.** The charger holds the BAT net at its ~4.16 V regulation point whether or
//! not a cell is present, so GPIO35 reads ~2080 mV → the library reports a (false)
//! ~4160 mV / 96 % "battery". A real cell at that voltage reads identically, and the only
//! signal that distinguishes them (STAT / the CHG LED) is not on a GPIO.
//!
//! In actual battery-powered deployment (no USB) this ambiguity disappears: with no cell
//! the BAT net sinks to ~0 V, so GPIO35 reads near 0 and the status is correctly
//! `No battery`. The false-full reading is a bench artifact of USB + no battery.
//!
//! To get real charge/presence detection, wire the STAT side of the charge LED's 5.1 kΩ
//! resistor to the otherwise-unconnected **GPIO34 (A2)** and use `EspChargingMonitor`.
//!
//! ## Run
//!
//! ```shell
//! just run idf_esp32_battery
//! ```
//!
//! ## Expected output (serial monitor)
//!
//! ```text
//! Wake cause: PowerOn
//! Battery: 3842mV (70%)
//! Entering deep sleep for 60 s
//! ```
//!
//! The device then enters deep sleep for 60 seconds, reboots, and repeats.

use battery_monitor::{
    BatteryConfig, BatteryMonitor, EspAdcBatteryMonitor, EspWakeCauseSource, WakeCause,
    WakeCauseSource,
};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::sys;

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches satisfies ESP-IDF's requirement that ROM function
    // stubs are patched before any ESP-IDF API is called. Must be the very
    // first call in main().
    esp_idf_hal::sys::link_patches();

    // Read the wake cause before any peripheral initialisation.
    // The EXT1 status register is hardware-preserved until the next sleep entry;
    // reading early is the Espressif-recommended practice.
    //
    // EspWakeCauseSource is a unit struct — the type name is also a value expression in Rust,
    // so `EspWakeCauseSource.last_wake_cause()` constructs the value and calls the trait method
    // in a single expression.  Equivalent to `let s = EspWakeCauseSource; s.last_wake_cause()`.
    let wake_cause = EspWakeCauseSource.last_wake_cause();
    match wake_cause {
        WakeCause::PowerOn => println!("Wake cause: PowerOn"),
        WakeCause::Timer => println!("Wake cause: Timer"),
        WakeCause::Ext1(mask) => println!("Wake cause: Ext1 (GPIO) — fired pins: 0x{:x}", mask.0),
        WakeCause::Ext0 => println!("Wake cause: Ext0 (GPIO)"),
        WakeCause::Gpio => println!("Wake cause: Gpio (deep-sleep GPIO wake)"),
        WakeCause::Touch => println!("Wake cause: Touch"),
        WakeCause::Other => println!("Wake cause: Other"),
    }

    let peripherals = Peripherals::take()?;

    // GPIO35 is ADC1_CH7 on the Adafruit ESP32 Feather V2.
    // The onboard 200 kΩ + 200 kΩ divider halves the battery voltage before
    // the ADC pin; BatteryConfig::adafruit_feather_v2() captures divider_ratio: 2.0.
    let mut battery = EspAdcBatteryMonitor::new(
        peripherals.adc1,
        peripherals.pins.gpio35,
        BatteryConfig::adafruit_feather_v2(),
    )?;

    let status = battery.read();
    println!("Battery: {}", status);

    println!("Entering deep sleep for 60 s");

    // NOTE: EspSleepManager (esp_sleep.rs) was written targeting ESP32-S3.
    // It uses `esp_sleep_ext1_wakeup_mode_t_ESP_EXT1_WAKEUP_ANY_HIGH` and
    // `esp_sleep_config_gpio_isolate()`, which may not be present on the
    // original ESP32 target. Rather than risk a compile failure that blocks
    // the example, we call the ESP-IDF timer sleep API directly here.
    // If esp_sleep.rs is confirmed to compile for xtensa-esp32-espidf, replace
    // this block with:
    //   EspSleepManager::default()
    //       .sleep(&[WakeSource::Timer { duration_ms: 60_000 }])
    //       .expect("sleep configuration failed");
    const SLEEP_MS: u64 = 60_000;
    const US_PER_MS: u64 = 1_000;
    // SAFETY: esp_sleep_enable_timer_wakeup configures the RTC timer wake source.
    // Safe to call before deep sleep entry. The duration is a compile-time constant
    // and cannot overflow the u64 µs argument.
    unsafe {
        let err = sys::esp_sleep_enable_timer_wakeup(SLEEP_MS * US_PER_MS);
        if err != sys::ESP_OK {
            anyhow::bail!("esp_sleep_enable_timer_wakeup failed: 0x{:08x}", err);
        }
    }

    // SAFETY: All wake sources are configured above. esp_deep_sleep_start()
    // never returns — the next execution begins at the firmware entry point
    // when the RTC timer fires after 60 seconds.
    unsafe {
        sys::esp_deep_sleep_start();
    }

    #[allow(unreachable_code)]
    Ok(())
}
