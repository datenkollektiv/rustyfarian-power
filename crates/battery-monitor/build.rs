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
