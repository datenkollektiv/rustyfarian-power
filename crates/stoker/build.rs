// Emit chip-specific cfg flags based on the Cargo target triple.
//
// `sleep.rs` uses #[cfg(esp32)] / #[cfg(not(esp32))] to select the ESP32 vs
// ESP32-S3 RTC-GPIO valid mask in `validate_gpio_level_source`. A build
// script's `rustc-cfg` flags apply only to the crate that emits them and do
// NOT propagate to dependents, so `stoker` must emit these itself — relying on
// the `rustyfarian-esp-idf-power` build script would leave the cfg unset when
// `stoker` is compiled (whether standalone on the host or as a dependency).
//
// On the host (or any non-ESP target) neither cfg is set, so the ESP32-S3
// path is used — which is exactly what the host-side unit tests exercise.
//
// The cargo:rustc-check-cfg lines register each cfg key so Cargo's check-cfg
// lint does not warn about unexpected_cfgs. `stoker` is pure: it has no
// ESP-IDF dependency and no `embuild` linker step (that lives only in the
// `rustyfarian-esp-idf-power` build script, which links the examples).

fn main() {
    println!("cargo:rustc-check-cfg=cfg(esp32)");
    println!("cargo:rustc-check-cfg=cfg(esp32s3)");

    let target = std::env::var("TARGET").unwrap_or_default();
    match target.as_str() {
        "xtensa-esp32-espidf" => println!("cargo:rustc-cfg=esp32"),
        "xtensa-esp32s3-espidf" => println!("cargo:rustc-cfg=esp32s3"),
        _ => {}
    }
}
