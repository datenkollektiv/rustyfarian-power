//! CrowPanel 1.28" ESP32-S3 — Touch Deep-Sleep GPIO Wake Example (ESP-IDF)
//!
//! Deep-sleeps until the CST816S touch controller pulls INT (GPIO5) low or a 60 s timer fires,
//! prints the wake cause, and sleeps again.
//!
//! ## Board
//!
//! - **Board:** Elecrow CrowPanel 1.28" HMI ESP32-S3 Rotary Display.
//! - **Wake pin:** CST816S touch INT on GPIO5, active-low, an RTC pad (GPIO0-21).
//!   The encoder and button (GPIO45/42/41) are not RTC pads and cannot wake the chip.
//! - **Touch power:** the CST816S needs `LCD_3V3` (GPIO1) on and its reset (GPIO13) released;
//!   without them INT sits LOW and every sleep is rejected.
//!   Both pins are held HIGH through deep sleep so the controller keeps watching for touches.
//!
//! On the S3 the wake goes through EXT1, so a touch reports `WakeCause::Ext1` with bit 5 set.
//! A finger resting on the screen pulses INT, so it wakes the board again and again.
//! A level already LOW at sleep entry is rejected by the hardware; the example retries.
//!
//! The console is the S3's native USB-Serial-JTAG, which drops off the bus during deep sleep.
//! The example stays awake 3 s after each boot so the host can reopen the re-enumerated port.
//! Bench results: `docs/bench/s3-crowpanel-touch-sleep-bench-steps-v1.md`.
//!
//! ## Run
//!
//! ```shell
//! just run idf_s3_deep_sleep_gpio
//! ```

use std::ffi::CString;

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::sys;
use rustyfarian_esp_idf_power::{
    Error, EspSleepManager, EspWakeCauseSource, GpioWakeLevel, SleepErrorKind, SleepManager,
    WakeCauseSource, WakeSource,
};

/// CST816S touch INT pin; bit position in the wake mask.
const TOUCH_INT_PIN: u8 = 5;

/// `LCD_3V3` rail enable; the CST816S is powered from it.
const LCD_RAIL_PIN: i32 = 1;

/// CST816S reset, active low.
const TOUCH_RST_PIN: i32 = 13;

/// Time awake after each boot; the USB-Serial-JTAG port needs ~0.5-2 s to re-enumerate.
const AWAKE_MS: u32 = 3_000;

/// Print a line via `esp_rom_printf`; Rust `println!` does not reach the S3 console
/// (`docs/project-lore.md` → ESP-IDF / Rust on ESP32-S3).
fn say(msg: &str) {
    let Ok(c) = CString::new(msg) else { return };
    // SAFETY: `c` is a NUL-terminated string alive for the call, `c"%s\n"` a static format.
    unsafe { sys::esp_rom_printf(c"%s\n".as_ptr(), c.as_ptr()) };
}

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches must run before any ESP-IDF API call.
    sys::link_patches();

    // Read the wake cause first: a later sleep attempt can clear the hardware wake status.
    let wake_cause = EspWakeCauseSource.last_wake_cause();
    say(&format!("Wake cause: {wake_cause:?}"));

    let peripherals = Peripherals::take()?;

    // Touch power: drive the levels before releasing the hold so the rail does not glitch
    // after a wake; only a cold boot gets the reset pulse. The drivers live until sleep.
    let mut lcd_rail = PinDriver::output(peripherals.pins.gpio1)?;
    let mut touch_rst = PinDriver::output(peripherals.pins.gpio13)?;
    lcd_rail.set_high()?;
    touch_rst.set_high()?;
    // SAFETY: gpio_hold_dis only clears the hold latch of a valid GPIO number.
    unsafe {
        sys::gpio_hold_dis(LCD_RAIL_PIN);
        sys::gpio_hold_dis(TOUCH_RST_PIN);
    }
    // SAFETY: esp_reset_reason only reads a value latched at boot.
    if unsafe { sys::esp_reset_reason() } != sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP {
        FreeRtos::delay_ms(20);
        touch_rst.set_low()?;
        FreeRtos::delay_ms(10);
        touch_rst.set_high()?;
        FreeRtos::delay_ms(50);
    }
    // SAFETY: gpio_hold_en latches the current output level of a valid RTC GPIO; with
    // gpio_deep_sleep_hold_en the latch survives deep sleep.
    unsafe {
        sys::gpio_hold_en(LCD_RAIL_PIN);
        sys::gpio_hold_en(TOUCH_RST_PIN);
        sys::gpio_deep_sleep_hold_en();
    }

    // The CST816S drives INT; the knob reads it without a pull, and so does this example.
    let _touch_int = PinDriver::input(peripherals.pins.gpio5, Pull::Floating)?;

    // GPIO wake with isolation is refused (IsolationConflict).
    let mut manager = EspSleepManager {
        isolate_gpio: false,
    };
    let sources = [
        WakeSource::GpioLevel {
            pin_mask: 1 << TOUCH_INT_PIN,
            level: GpioWakeLevel::AnyLow,
        },
        WakeSource::Timer {
            duration_ms: 60_000,
        },
    ];

    FreeRtos::delay_ms(AWAKE_MS);
    loop {
        // On success the chip is asleep and this call does not return.
        match manager.try_sleep(&sources) {
            // INT is still LOW; try again shortly.
            Err(e) if e.kind() == SleepErrorKind::Rejected => FreeRtos::delay_ms(50),
            Err(e) => anyhow::bail!("try_sleep failed: {e}"),
            Ok(()) => anyhow::bail!("try_sleep returned without sleeping"),
        }
    }
}
