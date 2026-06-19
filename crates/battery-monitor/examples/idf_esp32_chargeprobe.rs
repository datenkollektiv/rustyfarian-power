//! Adafruit ESP32 Feather V2 — Charge Pin Diagnostic Probe (ESP-IDF)
//!
//! **TEMPORARY DIAGNOSTIC — do not ship in production firmware.**
//! This probe reads three pins as analog voltages to verify the actual hardware
//! wiring on the Adafruit ESP32 Feather V2 before committing to a charging-monitor
//! implementation.
//!
//! ## Hypothesis under test
//!
//! The existing `idf_esp32_battery` example assumes:
//! - GPIO34 carries a USB-VBUS voltage-divider signal for VBUS detection.
//! - GPIO13 carries the MCP73831 STAT open-drain output (pull-up to 3.3 V).
//!
//! Online documentation (Adafruit CircuitPython pin map and learn guides) suggests this
//! may be incorrect: GPIO34 could be an unconnected analog input and GPIO13 could be
//! the user LED with no STAT wiring.
//! This probe reads all three pins as raw millivolts so the real hardware answers the
//! question definitively.
//!
//! ## Pin assignments
//!
//! | GPIO | ADC unit | Channel | Hypothesis |
//! |------|----------|---------|------------|
//! | 35   | ADC1     | CH7     | Battery voltage sense (2×200 kΩ ÷2 divider) — reference/control |
//! | 34   | ADC1     | CH6     | Suspected VBUS detect (100 kΩ + 100 kΩ divider from USB 5 V) |
//! | 13   | ADC2     | CH4     | Suspected MCP73831 STAT (open-drain, 4.7 kΩ pull-up to 3.3 V) |
//!
//! ## How to interpret the output
//!
//! **GPIO35 (batt sense)** should read approximately half the battery voltage at all
//! times (e.g. ~1960 mV for a 3.9 V cell).
//! A reading near 0 mV indicates the battery is disconnected.
//! This pin is the control: if it reads correctly, ADC1 and its calibration are working.
//!
//! **GPIO34 (VBUS?)** — plug and unplug USB while watching this pin.
//! If it swings between ~2500 mV (USB connected) and ~0 mV (USB disconnected), a real
//! VBUS voltage divider exists and the hypothesis is confirmed.
//! If it stays near 0 mV or reads noise regardless of USB state, nothing is connected
//! to GPIO34 and the VBUS detection idea must be abandoned.
//!
//! **GPIO13 (STAT?)** — with USB connected and battery not yet full, the MCP73831 pulls
//! STAT LOW (expect ~0 mV).
//! With USB connected and battery full, STAT goes Hi-Z and the pull-up holds it HIGH
//! (expect ~3300 mV).
//! If the reading is unrelated to charging state and stays stuck (e.g., always ~3300 mV
//! with some ADC noise, or tracks power-rail noise), GPIO13 is probably the user LED
//! output, not the STAT pin.
//!
//! ## Run
//!
//! ```shell
//! just run idf_esp32_chargeprobe
//! ```
//!
//! Plug and unplug USB while the probe is running and watch which readings change.
//! Let the battery charge fully and observe whether GPIO13 transitions.
//!
//! ## Expected output format
//!
//! ```text
//! GPIO35 (batt sense): 1960 mV | GPIO34 (VBUS?):    5 mV | GPIO13 (STAT?): 3290 mV
//! GPIO35 (batt sense): 1963 mV | GPIO34 (VBUS?):    4 mV | GPIO13 (STAT?): 3291 mV
//! ```

use esp_idf_hal::adc::attenuation::DB_12;
use esp_idf_hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
use esp_idf_hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_hal::adc::AdcChannel;
use esp_idf_hal::peripherals::Peripherals;

const NUM_SAMPLES: u32 = 16;

/// Read `NUM_SAMPLES` averaged millivolts from an [`AdcChannelDriver`].
///
/// Any failed sample is skipped; if all samples fail, returns 0.
fn read_averaged_mv<'d, C, M>(channel: &mut AdcChannelDriver<'d, C, M>) -> u16
where
    C: AdcChannel,
    M: core::borrow::Borrow<AdcDriver<'d, C::AdcUnit>>,
{
    let mut sum: u32 = 0;
    let mut valid: u32 = 0;

    for _ in 0..NUM_SAMPLES {
        match channel.read() {
            Ok(mv) => {
                sum += mv as u32;
                valid += 1;
            }
            Err(e) => {
                log::warn!("ADC read failed: {:?}", e);
            }
        }
    }

    if valid == 0 {
        log::error!("All ADC reads failed for channel");
        return 0;
    }

    (sum / valid) as u16
}

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches satisfies ESP-IDF's requirement that ROM function
    // stubs are patched before any ESP-IDF API is called.
    // Must be the very first call in main().
    esp_idf_hal::sys::link_patches();

    let peripherals = Peripherals::take()?;

    // Channel config shared by all three pins.
    // 12 dB attenuation covers 0–2450 mV on the original ESP32, which is sufficient
    // for all three signals (battery ÷2 ≈ 2100 mV max; VBUS ÷2 ≈ 2500 mV).
    // Line-fitting calibration corrects the ESP32 ADC's significant non-linearity at
    // this attenuation level; without it, readings can be hundreds of mV low.
    let channel_config = AdcChannelConfig {
        attenuation: DB_12,
        calibration: Calibration::Line,
        ..Default::default()
    };

    // ADC1 — drives GPIO35 (battery sense, CH7) and GPIO34 (VBUS?, CH6).
    // Both channels share the same AdcDriver instance via shared references.
    let adc1 = AdcDriver::new(peripherals.adc1)?;

    // GPIO35 — ADC1_CH7 — battery voltage sense (2×200 kΩ divider).
    // Control pin: confirms ADC1 + Calibration::Line are working correctly.
    let mut batt = AdcChannelDriver::new(&adc1, peripherals.pins.gpio35, &channel_config)?;

    // GPIO34 — ADC1_CH6 — suspected VBUS detect.
    // GPIO34 is an input-only RTC GPIO (RTC_GPIO4) on the original ESP32.
    // It may be left in the RTC domain after the boot ROM samples strapping pins;
    // rtc_gpio_deinit releases it to the digital ADC domain before first use.
    // SAFETY: gpio_num_t 34 is valid on the Adafruit ESP32 Feather V2.
    // rtc_gpio_deinit is safe to call whether or not the pin is currently in RTC mode.
    unsafe {
        esp_idf_hal::sys::rtc_gpio_deinit(34);
    }
    let mut vbus = AdcChannelDriver::new(&adc1, peripherals.pins.gpio34, &channel_config)?;

    // ADC2 — drives GPIO13 (STAT?, CH4).
    // A separate AdcDriver is required because GPIO13 is on ADC2, not ADC1.
    let adc2 = AdcDriver::new(peripherals.adc2)?;
    let mut stat = AdcChannelDriver::new(&adc2, peripherals.pins.gpio13, &channel_config)?;

    loop {
        let batt_mv = read_averaged_mv(&mut batt);
        let vbus_mv = read_averaged_mv(&mut vbus);
        let stat_mv = read_averaged_mv(&mut stat);

        println!(
            "GPIO35 (batt sense): {:4} mV | GPIO34 (VBUS?): {:4} mV | GPIO13 (STAT?): {:4} mV",
            batt_mv, vbus_mv, stat_mv
        );

        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
}
