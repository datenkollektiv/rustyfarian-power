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

**On the Heltec V3.1, a missing battery reads as a ~80 % battery whenever USB is connected — and it cannot be detected in software.**
With USB power and no cell, the always-on divider sees the charger/BMS rail, so GPIO1 reads ~711–735 mV (×5.55 → ~3946–4079 mV) → reported as `Battery ~80 %`, not `No battery`.
The charge controller is a **TP4054** whose open-drain CHRG status drives only the orange LED — schematic-confirmed (V3.0 and V3.1) not wired to any GPIO — and there is no USB/VBUS-detect pin, so no software presence signal exists (every pin map — ropg, arduino-esp32, ESPHome, Meshtastic — reads only the GPIO1 voltage; Meshtastic's >4.2 V inference is a known source of wrong USB/charge indicators).
The only (weak) tell is instability: a real LiPo holds within a few mV per sample, the no-cell rail jumps ~130–180 mV.
Fix: don't trust a battery reading taken on USB power; for real detection, hand-wire CHRG (or a VBUS divider) to a free GPIO (GPIO2/4/5/6/7/19/20/47/48).

---

## Hardware — Adafruit ESP32 Feather V2

**The stock Feather V2 exposes no charge-status (STAT) or USB-VBUS-detect signal on any readable GPIO — don't trust a pin map you can't trace on the schematic.**
Our `EspChargingMonitor` was wired to GPIO13 (as MCP73831 STAT) and GPIO34 (as VBUS detect), producing a permanent `Charging: Unknown`.
Those assignments were wrong: they match the **Unexpected Maker FeatherS2 Neo** ("VBUS detection on IO34"), a different board the pin map was apparently copied from.
On the real Feather V2, the MCP73831 STAT drives only the on-board CHG LED, GPIO13 is the user LED, and GPIO35 (A13, 2×200 kΩ ÷2) is the only power-sense pin.
Proof came from an analog probe (`idf_esp32_chargeprobe`, USB connected, ~3.95 V cell): GPIO35 ≈ 1973 mV (correct), GPIO34 ≈ 147 mV (a real VBUS divider would force ~2500 mV), GPIO13 ≈ 134 mV drifting (a 4.7 kΩ pull-up would force ~3300 mV).
The decisive checks are state-independent — a pull-up or divider, if present, forces a specific voltage *now*, so you don't need to unplug USB (which would kill the serial console anyway) to disprove the wiring.
Fix: read suspected pins as **analog millivolts**, not digital levels, when verifying wiring — a floating input-only pin reads a small leakage (~150 mV), nothing like a driven level.
Also: MCP73831 STAT is **tri-state**, not open-drain (that's the MCP73832).
Schematic-confirmed (EagleCAD rev F): the CHG-LED net is `VBUS → CHG LED → 5.1 kΩ → STAT`, STAT net has two nodes only, no ESP32 trace — the orange CHG LED is not on any GPIO. To read charge state you must hand-wire STAT to the free GPIO34 (A2).

**On the Feather V2, a missing battery reads as a ~96 % FULL battery whenever USB is connected — there is no software way to tell them apart.**
With no cell the MCP73831 holds the BAT net at its ~4.16 V CV regulation point, so the GPIO35 divider reads ~2081 mV (×2 = 4162 mV) — above the empty threshold and below USB-detect, so it classifies as `Battery ~96 %`.
A real 4.16 V cell reads identically, and the only disambiguator (STAT) is not on a GPIO, so no threshold tweak can fix it.
This is a bench artifact of USB + no battery: with no USB the BAT net sinks to ~0 mV and a missing cell correctly reads `No battery`.
Fix: don't chase it with thresholds — document the behaviour, or tap STAT to GPIO34 for true presence detection.

## ESP-IDF / Rust on ESP32-S3

**Rust `std` `println!`/stdout does NOT appear on the Heltec V3 (ESP32-S3) UART console, even though ESP-IDF's own `I (..)` logs do.**
The board's primary console is UART0 with `CONFIG_ESP_CONSOLE_SECONDARY_USB_SERIAL_JTAG=y`; ESP-IDF logging writes via `esp_rom_printf` straight to UART, but Rust stdout writes are swallowed and `flush()` does not help.
The original ESP32 (Feather V2) has no second console, so `println!` happens to work there — which masks the problem until the first S3 example.
Fix: in examples, route output through `esp_idf_hal::sys::esp_rom_printf` (e.g. a tiny `log::Log` impl forwarding to it) — no `esp-idf-svc` dependency needed. This is also why the library's `log::debug!`/`warn!` diagnostics are invisible by default: nothing installs a `log` logger.
