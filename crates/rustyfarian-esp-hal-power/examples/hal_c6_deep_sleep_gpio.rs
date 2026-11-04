//! ESP32-C6-DevKitC-1 — PIR Deep-Sleep EXT1 Wake Example (esp-hal)
//!
//! Sleeps in deep sleep until an HC-SR501 PIR on GPIO2 reports motion or a timer (60 s) fires,
//! then logs the reset reason and wake cause and sleeps again.
//! It exercises `try_sleep`: right after a PIR wake the sensor output is still HIGH for its hold
//! time, so the first `try_sleep` may be rejected and the example retries every 50 ms until the
//! PIR releases.
//! It is the esp-hal twin of `idf_c6_deep_sleep_gpio`.
//!
//! ## Board
//!
//! - **Chip:** ESP32-C6 (RISC-V, single core, LP domain for the RTC pins), Espressif ESP32-C6-DevKitC-1.
//! - **Console:** the USB-UART bridge on UART0; `esp-println` prints there.
//! - **Wake pin:** GPIO0-7 are LP pads, so the wake goes through EXT1 and the wake cause is
//!   `WakeCause::Ext1` with bit 2 set.
//! - GPIO4-7 are JTAG and GPIO8/9 are strapping pins (WS2812 and BOOT); the example leaves them alone.
//!
//! ## Wiring
//!
//! | Signal      | Connection                                       |
//! |:------------|:-------------------------------------------------|
//! | PIR OUT     | GPIO2                                            |
//! | PIR VCC     | 5V                                               |
//! | PIR GND     | GND                                              |
//! | Pull-down   | external 10 kΩ from GPIO2 to GND (required)      |
//!
//! The pull-down is the pull *against* the wake level: the wake level is HIGH, so the pin rests LOW.
//! The internal pulls are not available in deep sleep on the C6, so the external resistor is required.
//! The HC-SR501 output swings to about 3.3 V, so no level shifter is needed.
//! Meter the OUT pin first anyway: the C6 pads are not 5 V tolerant.
//!
//! ## Run
//!
//! ```shell
//! just run-hal-c6 hal_c6_deep_sleep_gpio
//! ```
//!
//! ## Expected output (serial monitor, not yet bench-verified on the C6)
//!
//! ```text
//! Reset reason: DeepSleep
//! Wake cause: Ext1 — fired pins: 0x4, GPIO2 fired: true
//! GPIO2 level before sleep: High
//! Sleeping: GPIO2 AnyHigh + 60000 ms timer
//! try_sleep rejected (first): <error>, kind=Rejected
//! ```
//!
//! When the PIR releases, `try_sleep` succeeds and the chip sleeps again.
//! After a power-on the first lines read `Reset reason: PowerOn` and `Wake cause: PowerOn`.

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Pull},
    system::software_reset,
};
use esp_println::println;
use rustyfarian_esp_hal_power::{
    Error, EspHalResetReasonSource, EspHalSleepManager, EspHalWakeCauseSource, GpioWakeLevel,
    ResetReasonSource, SleepErrorKind, SleepManager, WakeCause, WakeCauseSource, WakeSource,
};

/// PIR output pin; bit position in the wake mask.
const PIR_PIN: u8 = 2;

/// Timer fallback in ms.
const TIMER_MS: u64 = 60_000;

/// Delay between rejected `try_sleep` attempts.
const RETRY_DELAY_MS: u32 = 50;

/// Log a compact count line every this many rejections.
const LOG_EVERY: u32 = 20;

/// Prints the message and restarts: unattended hardware must recover, not hang.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    Delay::new().delay_millis(50);
    software_reset()
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // The pull against the wake level; the driver lives until the chip sleeps or restarts.
    let mut pir = Input::new(
        peripherals.GPIO2,
        InputConfig::default().with_pull(Pull::Down),
    );

    // Capture the wake cause first (ADR-001): a rejected sleep attempt overwrites the wake record.
    let wake_cause = EspHalWakeCauseSource::capture(&[&pir]).last_wake_cause();
    println!(
        "Reset reason: {:?}",
        EspHalResetReasonSource.last_reset_reason()
    );
    match wake_cause {
        WakeCause::Ext1(m) => println!(
            "Wake cause: Ext1 — fired pins: 0x{:x}, GPIO{} fired: {}",
            m.0,
            PIR_PIN,
            m.contains_pin(PIR_PIN)
        ),
        other => println!("Wake cause: {:?}", other),
    }
    println!("GPIO{} level before sleep: {:?}", PIR_PIN, pir.level());

    let mut manager = EspHalSleepManager::new(peripherals.LPWR);
    let sources = [
        WakeSource::GpioLevel {
            pin_mask: 1 << PIR_PIN,
            level: GpioWakeLevel::AnyHigh,
        },
        WakeSource::Timer {
            duration_ms: TIMER_MS,
        },
    ];

    println!("Sleeping: GPIO{} AnyHigh + {} ms timer", PIR_PIN, TIMER_MS);

    let mut rejected: u32 = 0;
    loop {
        let mut pins = [&mut pir];
        // On success the chip is asleep and this call does not return.
        let res = manager.bind_wake_pins(&mut pins).try_sleep(&sources);
        match res {
            // Only reachable if the platform returned without sleeping.
            Ok(()) => break,
            Err(e) if e.kind() == SleepErrorKind::Rejected => {
                rejected += 1;
                if rejected == 1 {
                    println!("try_sleep rejected (first): {e}, kind={:?}", e.kind());
                } else if rejected.is_multiple_of(LOG_EVERY) {
                    println!("try_sleep rejected: {} so far", rejected);
                }
                delay.delay_millis(RETRY_DELAY_MS);
            }
            Err(e) => {
                println!("try_sleep failed: {e} — falling back to a timer-only sleep");
                // A restart would retry the same failing configuration in a loop, so sleep
                // on the timer alone; sleep() does not return unless that fails too.
                let mut none: [&mut Input<'_>; 0] = [];
                if let Err(e) = manager
                    .bind_wake_pins(&mut none)
                    .sleep(&[WakeSource::Timer {
                        duration_ms: TIMER_MS,
                    }])
                {
                    println!("timer-only sleep failed: {e} — restarting");
                    delay.delay_millis(50);
                    software_reset();
                }
            }
        }
    }

    // try_sleep returned Ok without sleeping; do not stay awake on battery.
    delay.delay_millis(50);
    software_reset()
}
