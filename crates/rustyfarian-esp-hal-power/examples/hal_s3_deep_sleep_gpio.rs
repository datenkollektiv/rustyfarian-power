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
//! Bench results: `docs/bench/s3-crowpanel-touch-sleep-bench-steps-v1.md`.
//!
//! ## Run
//!
//! ```shell
//! just run-hal-s3 hal_s3_deep_sleep_gpio
//! ```

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    peripherals::LPWR,
    rtc_cntl::{reset_reason, SocResetReason},
    system::Cpu,
};
use esp_println::println;
use rustyfarian_esp_hal_power::{
    EspHalSleepManager, EspHalWakeCauseSource, GpioWakeLevel, SleepError, SleepManager,
    WakeCauseSource, WakeSource,
};

/// CST816S touch INT pin; bit position in the wake mask.
const TOUCH_INT_PIN: u8 = 5;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    loop {}
}

/// Latches GPIO1 (`LCD_3V3` enable, touch pad 1) and GPIO13 (CST816S reset, touch pad 13) at
/// their current level, through deep sleep.
///
/// esp-hal 1.2.2 has no public pad hold, so this writes `RTC_CNTL_PAD_HOLD`, the register esp-hal
/// itself uses to hold wake pads.
fn set_touch_pads_held(held: bool) {
    LPWR::regs()
        .pad_hold()
        .modify(|_, w| w.touch_pad1().bit(held).touch_pad13().bit(held));
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // Touch power: drive the levels before releasing the hold so the rail does not glitch
    // after a wake; only a cold boot gets the reset pulse. The drivers live until sleep.
    let _lcd_rail = Output::new(peripherals.GPIO1, Level::High, OutputConfig::default());
    let mut touch_rst = Output::new(peripherals.GPIO13, Level::High, OutputConfig::default());
    set_touch_pads_held(false);
    if reset_reason(Cpu::ProCpu) != Some(SocResetReason::CoreDeepSleep) {
        delay.delay_millis(20);
        touch_rst.set_low();
        delay.delay_millis(10);
        touch_rst.set_high();
        delay.delay_millis(50);
    }
    set_touch_pads_held(true);

    // The CST816S drives INT; the knob reads it without a pull, and so does this example.
    let mut touch_int = Input::new(
        peripherals.GPIO5,
        InputConfig::default().with_pull(Pull::None),
    );

    // Capture the wake cause before the first sleep attempt: a rejected sleep overwrites the
    // per-pin wake record.
    let wake_cause = EspHalWakeCauseSource::capture(&[&touch_int]).last_wake_cause();
    println!("Wake cause: {:?}", wake_cause);

    let mut manager = EspHalSleepManager::new(peripherals.LPWR);
    let sources = [
        WakeSource::GpioLevel {
            pin_mask: 1 << TOUCH_INT_PIN,
            level: GpioWakeLevel::AnyLow,
        },
        WakeSource::Timer {
            duration_ms: 60_000,
        },
    ];

    loop {
        // On success the chip is asleep and this call does not return.
        match manager
            .bind_wake_pins(&mut [&mut touch_int])
            .try_sleep(&sources)
        {
            // INT is still LOW; try again shortly.
            Err(SleepError::Rejected) => delay.delay_millis(50),
            Err(e) => panic!("try_sleep failed: {}", e),
            Ok(()) => panic!("try_sleep returned without sleeping"),
        }
    }
}
