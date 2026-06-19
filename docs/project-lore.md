# Project Lore

This file records non-obvious technical discoveries: facts that caused surprising
failures, took significant time to debug, or would save a future developer 30+
minutes if known upfront.

Refer to `AGENTS.md` and the `/project-lore` skill for recording guidelines.

---

## Build & Validation

**`just deny` fails on licenses because the repo has no `deny.toml` at all.**
With no config file present, `cargo-deny` 0.19 defaults to an empty license allow-list and rejects *every* crate — including MIT, Apache-2.0, and Unlicense — so the error "license is not explicitly allowed" shows up even for the most permissive licenses, which looks baffling.
This blocks `just deny`, `just ci`, and `just verify` regardless of the dependency tree, so it is unrelated to whatever change you are validating.
Fix: add a `deny.toml` with a `[licenses]` `allow` list (MIT, Apache-2.0, Unlicense, BSD-2-Clause, BSD-3-Clause, …) before relying on `just deny`.

**Host-side gates (`just check` / `test` / `clippy`) never compile the ESP-IDF driver code.**
These recipes run with `--no-default-features`, which drops the default `esp-idf` feature, so `esp_adc.rs`, `esp_charging.rs`, and `esp_sleep.rs` are excluded from the build — a green host run says nothing about whether the hardware drivers still compile.
Fix: validate any change to a feature-gated `esp_*.rs` module with `just check-all` (ESP32-S3) and `just check-esp32` (original ESP32); the host gates only cover the platform-independent modules.

---

## Hardware — Heltec V3 & LiPo batteries

**MakerFocus LiPo packs use the OPPOSITE polarity to Heltec on the same JST-1.25 connector.**
The plug fits physically, but `+`/`−` are swapped (MakerFocus is wired for RAK hardware), so plugging a MakerFocus pack straight into a Heltec V3 reverse-connects the battery.
The Heltec battery input has no reverse-polarity protection — this can damage the charger/regulator and, short of that, leaves the battery-sense ADC reading a flat 0 mV.
Fix: re-pin the JST (swap the two crimp pins) or hand-wire crossed, and ALWAYS confirm battery `+` terminal → board `+` pad with a multimeter before power-on — never trust cable colours. Find the board's `+`/`−` by continuity to GND when the pad reads 0 V (charger idle with no battery is normal).

**A LiPo protection board latches its output to 0 V after a reverse-polarity/short fault.**
Symptom: the bare cell still measures ~3.9 V but the pack's plug reads 0.0 V — the cell is usually fine, the PCM FETs are just open.
This presents to the board as "No battery" / 0 mV. Don't mistake a tripped pack for a dead board or a damaged sense path; test the board's sense path with a separate known-good source (current-limited bench PSU at ~3.7 V, polarity metered).

**On the Heltec V3.1, GPIO37 (ADC_CTRL) does NOT gate the battery divider — it is always-on.**
Toggling GPIO37 HIGH vs LOW left the GPIO1 reading unchanged (706 vs 707 mV), so the community/ropg "drive GPIO37 LOW to enable" procedure did not apply to this **V3.1** unit (it may still apply to V3.0/V3.2 — revisions differ; always confirm against the board in hand).
Read GPIO1 directly — no ADC_CTRL handling needed, which matches the repo's existing approach; only the ratio was wrong.
Separately, the ~80 kΩ divider source impedance loads the ESP-IDF oneshot ADC so it reads ~11 % low: the effective `divider_ratio` (metered VBAT ÷ ADC mV, ~5.5) runs higher than the textbook (390+100)/100 = 4.9. Calibrate the ratio against a meter, never compute it from the resistor values alone.
Take the calibration pair at a **settled** voltage (USB unplugged, ~10 s): a charging cell reads inflated (≈4.0–4.2 V toward the charge ceiling) and isn't representative of its resting state-of-charge, which skews the ratio.

---

## ESP-IDF / Rust on ESP32-S3

**Rust `std` `println!`/stdout does NOT appear on the Heltec V3 (ESP32-S3) UART console, even though ESP-IDF's own `I (..)` logs do.**
The board's primary console is UART0 with `CONFIG_ESP_CONSOLE_SECONDARY_USB_SERIAL_JTAG=y`; ESP-IDF logging writes via `esp_rom_printf` straight to UART, but Rust stdout writes are swallowed and `flush()` does not help.
The original ESP32 (Feather V2) has no second console, so `println!` happens to work there — which masks the problem until the first S3 example.
Fix: in examples, route output through `esp_idf_hal::sys::esp_rom_printf` (e.g. a tiny `log::Log` impl forwarding to it) — no `esp-idf-svc` dependency needed. This is also why the library's `log::debug!`/`warn!` diagnostics are invisible by default: nothing installs a `log` logger.
