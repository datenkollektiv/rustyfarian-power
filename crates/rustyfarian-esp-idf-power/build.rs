// Emit chip-specific cfg flags based on the Cargo target triple.
//
// This allows code in this crate to use #[cfg(esp32)] / #[cfg(esp32s3)] to
// handle chip-specific ESP-IDF API differences (e.g., EXT1 wakeup mode
// constants) without depending on the MCU environment variable or on
// cfg flags emitted by other crates' build scripts (which do not propagate
// to dependents).
//
// The cargo:rustc-check-cfg lines register each cfg key so Cargo's
// check-cfg lint does not warn about unexpected_cfgs.
//
// For ESP-IDF targets, `embuild::espidf::sysenv::output()` propagates the
// linker arguments (ldproxy-linker, ldproxy-cwd, ESP-IDF library paths) that
// are required to link examples and tests from this library crate.
// Without this call, the linker receives no ESP-IDF library paths and all
// ESP-IDF symbols (`esp_sleep_get_wakeup_cause`, `adc_oneshot_*`, etc.)
// are undefined.
//
// cargo:rustc-link-arg directives from a dependency's build script
// (e.g., esp-idf-hal) do NOT automatically propagate to dependents that
// link binary outputs; each library crate that builds examples must call
// sysenv::output() itself. This mirrors the pattern in esp-idf-hal's own
// build.rs comment: "Only necessary for building the examples."

fn main() {
    println!("cargo:rustc-check-cfg=cfg(esp32)");
    println!("cargo:rustc-check-cfg=cfg(esp32s3)");
    // Emitted bare by esp-idf-sys for the ESP32-C6 and relayed by `sysenv::output()`
    // (the triple `riscv32imac-esp-espidf` is shared with the ESP32-H2, so no triple match).
    println!("cargo:rustc-check-cfg=cfg(esp32c6)");
    // Emitted bare by esp-idf-sys and relayed the same way; `esp_adc.rs` keeps ADC line
    // fitting on these two chips (esp-idf-hal defines `Calibration::Curve` only for S3/C3/C6).
    println!("cargo:rustc-check-cfg=cfg(esp32c2)");
    println!("cargo:rustc-check-cfg=cfg(esp32s2)");
    // ESP-IDF v6.1 Kconfig name (v5.3.3: `soc_gpio_support_deepsleep_wakeup`), emitted by
    // esp-idf-sys and relayed by `sysenv::output()`; gates `esp_sleep_get_gpio_wakeup_status`.
    // v6.1 also sets it on the ESP32 and ESP32-S3, so it no longer means "deep-sleep GPIO
    // wake chip" on its own.
    println!("cargo:rustc-check-cfg=cfg(esp_idf_soc_gpio_support_hp_periph_pd_sleep_wakeup)");
    // Emitted on chips with EXT1 wakeup (ESP32, ESP32-S3, ESP32-C6); together with
    // the cfg above it (with `not(ext1)`) selects the C2/C3 deep-sleep GPIO wake path.
    println!("cargo:rustc-check-cfg=cfg(esp_idf_soc_pm_support_ext1_wakeup)");
    // Relayed by `sysenv::output()`; `lib.rs` refuses to build against ESP-IDF older than v6.0.
    println!("cargo:rustc-check-cfg=cfg(esp_idf_version_at_least_6_0_0)");
    // esp-idf-sys relays CONFIG_FREERTOS_PORTMUX_DEBUG as this cfg; `esp_critical_section.rs`
    // adds the debug fields of `portMUX_TYPE` under it.
    println!("cargo:rustc-check-cfg=cfg(esp_idf_freertos_portmux_debug)");

    let target = std::env::var("TARGET").unwrap_or_default();
    match target.as_str() {
        "xtensa-esp32-espidf" => println!("cargo:rustc-cfg=esp32"),
        "xtensa-esp32s3-espidf" => println!("cargo:rustc-cfg=esp32s3"),
        _ => {}
    }

    if target.ends_with("-espidf") {
        embuild::espidf::sysenv::output();
    }
}
