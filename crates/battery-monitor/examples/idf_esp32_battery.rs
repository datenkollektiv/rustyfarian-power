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
//! The Feather V2 routes the LiPo battery through an onboard 100 kΩ + 100 kΩ
//! voltage divider to **GPIO35** (ADC1_CH7).
//!
//! ```text
//! LiPo+ ──[100 kΩ]──┬──[100 kΩ]── GND
//!                   └──► GPIO35 (ADC1_CH7)
//! ```
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
//! Charging: Charging (USB)
//! Entering deep sleep for 60 s
//! ```
//!
//! The device then enters deep sleep for 60 seconds, reboots, and repeats.
//! When the battery is full the charging line will read `Full`.
//! When running on battery only (no USB) it will read `No battery` for the charging line.

use battery_monitor::{
    BatteryConfig, BatteryMonitor, ChargingMonitor, ChargingSource, EspAdcBatteryMonitor,
    EspChargingMonitor, EspWakeCauseSource, WakeCause, WakeCauseSource,
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
    // The onboard 100 kΩ + 100 kΩ divider halves the battery voltage before
    // the ADC pin; BatteryConfig::adafruit_feather_v2() captures divider_ratio: 2.0.
    let mut battery = EspAdcBatteryMonitor::new(
        peripherals.adc1,
        peripherals.pins.gpio35,
        BatteryConfig::adafruit_feather_v2(),
    )?;

    // GPIO34 is RTC_GPIO4 and may be left in the RTC domain after the boot ROM samples
    // the strapping pins. Release it to the digital domain before configuring it as an input.
    // SAFETY: gpio_num_t 34 is valid on the Adafruit ESP32 Feather V2.
    // rtc_gpio_deinit is safe to call whether or not the pin is currently in RTC mode.
    unsafe {
        sys::rtc_gpio_deinit(34);
    }

    // GPIO13 — MCP73831 STAT pin (open-drain, board has external 4.7 kΩ pull-up).
    // GPIO34 — USB VBUS detect (100 kΩ + 100 kΩ divider; input-only pin on ESP32).
    let mut charging = EspChargingMonitor::new(
        peripherals.pins.gpio13,
        peripherals.pins.gpio34,
        ChargingSource::Usb,
    )?;

    // Allow the MCP73831 STAT pin and voltage rails to settle after waking from deep sleep.
    // The STAT pin can glitch LOW briefly during rail ramp-up, producing a spurious
    // ChargingState::Unknown reading if sampled too early.
    std::thread::sleep(std::time::Duration::from_millis(20));

    let status = battery.read();
    let charging_state = charging.read_charging_state();
    println!("Battery: {}", status);
    println!("Charging: {}", charging_state);

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
