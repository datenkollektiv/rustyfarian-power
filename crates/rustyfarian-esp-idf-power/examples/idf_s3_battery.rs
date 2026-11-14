//! Heltec WiFi LoRa 32 V3 (measured on V3.1) — Battery Monitor Example (ESP-IDF)
//!
//! Reads the battery voltage on GPIO1 via [`BatteryConfig::heltec_v3`] and prints the
//! decoded status once every 2 seconds.
//!
//! ## Board / wiring
//!
//! - **Chip:** ESP32-S3 (Heltec WiFi LoRa 32 V3.1)
//! - **Battery sense:** GPIO1 (ADC1_CH0), through an on-board divider that is
//!   **always connected** — GPIO37/`ADC_CTRL` does *not* gate it on V3.1 (it may on
//!   other revisions), so no control-pin handling is needed.
//! - `heltec_v3()` carries the hardware-calibrated `divider_ratio` (5.05); compare the
//!   reported voltage to a multimeter and nudge it if your unit differs.
//!
//! ## ⚠️ Battery polarity
//!
//! Verify polarity with a multimeter before connecting. This board has no
//! reverse-polarity protection, and MakerFocus packs ship with the **opposite**
//! polarity on the same JST-1.25 connector — plugging one in directly reverses it and
//! can destroy the board. See `docs/project-lore.md` → Hardware.
//!
//! ## Output
//!
//! Output is emitted via `esp_rom_printf`. Rust `std` `println!` does not surface on
//! this board's UART console (ESP-IDF's own logs do); a tiny `log` logger forwards the
//! library's diagnostics through the same ROM printf path. This ROM-printf path is
//! specific to this board's quirk — on ESP32 variants where Rust stdout reaches the
//! console, a plain `println!` and `esp_idf_svc::log::EspLogger` work fine instead.
//!
//! ## Expected output (serial monitor)
//!
//! ```text
//! [INFO] Battery monitor initialized (divider: 5.05x, range: 3000-4200mV)
//! Heltec V3.1 battery monitor — reading GPIO1 every 2 s
//! Battery: 3842mV (70%)
//! ```
//!
//! ## No battery + USB caveat
//!
//! With USB connected and no battery, GPIO1 reads the charger/BMS rail through the
//! always-on divider, so the no-cell reading is ambiguous and varies by board/run.
//! Observed 2026-06-19: `Battery ~80 %` (rail ~3950–4080 mV compensated, unstable).
//! Observed 2026-11-09: `No battery` (rail below the 3000 mV floor).
//! The Heltec V3 exposes no charge-status or battery-present GPIO — the TP4054's CHRG
//! output drives only the orange LED (schematic-confirmed) — so either reading remains
//! a bench/USB artifact; in battery-only use a missing cell means the board is off.
//! See `docs/project-lore.md` → Hardware — Heltec V3 & LiPo batteries.
//!
//! ## Run
//!
//! ```shell
//! just run idf_s3_battery
//! ```

use std::ffi::CString;

use esp_idf_hal::peripherals::Peripherals;
use rustyfarian_esp_idf_power::{BatteryConfig, BatteryMonitor, EspAdcBatteryMonitor};

/// Print a line to the serial console via `esp_rom_printf`.
///
/// Rust `std` `println!`/stdout is not visible on this board's UART console; ESP-IDF's
/// own logs reach it because they use `esp_rom_printf`, so we route output the same way.
fn say(msg: &str) {
    let Ok(c) = CString::new(msg) else { return };
    // SAFETY: `c` is a valid NUL-terminated C string alive for the duration of the call,
    // and `c"%s\n"` is a static NUL-terminated format string. esp_rom_printf writes
    // straight to the ROM UART console.
    unsafe {
        esp_idf_hal::sys::esp_rom_printf(c"%s\n".as_ptr(), c.as_ptr());
    }
}

/// Minimal `log` logger forwarding records through [`say`], so the library's ADC
/// warnings/errors are visible (no logger is installed by default).
struct RomLogger;

impl log::Log for RomLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        say(&format!("[{}] {}", record.level(), record.args()));
    }
    fn flush(&self) {}
}

static LOGGER: RomLogger = RomLogger;

use stoker::bench::variant_index;

/// `BENCH_VARIANT` at build time (`just bench-build NAME--VARIANT`): `calib` adds a
/// `Calib:` line printing the compensated and averaged raw pin mV in every power state, for
/// divider-ratio calibration (see `bench/s3-heltec/battery-calib.toml`). Unknown values fail the build.
const CALIB: bool = variant_index(option_env!("BENCH_VARIANT"), &["calib"]).is_some();

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches must run before any ESP-IDF API call.
    esp_idf_hal::sys::link_patches();

    // `set_logger` fails only if a logger is already installed (e.g. when this code is
    // lifted into a larger app). Surface that rather than swallowing it, so missing
    // library diagnostics are easy to diagnose.
    if log::set_logger(&LOGGER).is_err() {
        say("[WARN] a logger is already installed; library diagnostics route elsewhere");
    }
    log::set_max_level(log::LevelFilter::Info);

    let peripherals = Peripherals::take()?;

    // GPIO1 is ADC1_CH0 on the Heltec V3.1; the on-board divider is always connected,
    // so no GPIO37/ADC_CTRL handling is required.
    let mut battery = EspAdcBatteryMonitor::new(
        peripherals.adc1,
        peripherals.pins.gpio1,
        BatteryConfig::heltec_v3(),
    )?;

    say("Heltec V3.1 battery monitor — reading GPIO1 every 2 s");

    // Kept deliberately simple: each iteration allocates via `format!`/`CString`. The 2 s
    // cadence makes the allocator churn negligible; a long-running sample would preformat
    // fixed strings or log less often instead.
    loop {
        let status = if CALIB {
            let raw_mv = battery.read_raw_mv();
            let status = battery.config().evaluate_reading(raw_mv);
            say(&format!("Calib: comp={} raw={raw_mv}", status.voltage_mv));
            status
        } else {
            battery.read()
        };
        say(&format!("Battery: {status}"));
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
