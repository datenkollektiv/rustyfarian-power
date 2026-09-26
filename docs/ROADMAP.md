# Roadmap

*Last updated: September 2026*

Charging detection and deep-sleep wake-source handling are working on both Heltec V3 and Adafruit Feather V2.
The Feather V2 has a complete `EspChargingMonitor`; the Heltec V3 charging implementation is blocked on schematic verification of the charge controller IC and its GPIO — an inversion relative to the README's "primary target" framing that near-term work must resolve.
Simultaneous battery + USB operation is confirmed safe and intended on both boards (vendor docs cited in `docs/key-insights.md`); what remains for Heltec is the exact charger-IC marking and a STAT/CHRG status GPIO.
The ESP-IDF tier moved to the September 2026 family stack (`esp-idf-hal 0.47` / `esp-idf-sys 0.38.1`) on 2026-09-26, in step with `rustyfarian-network`, `-ws2812` and `-peripherals`.
Both crates shipped as `0.2.0` on 2026-09-26, so downstream apps can combine power with the siblings in one `esp-idf-sys` graph.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "cScale0": "#e8f5e9",
    "cScaleLabel0": "#2e7d32",
    "cScale1": "#c8f7c5",
    "cScaleLabel1": "#1b5e20",
    "cScale2": "#fff3cd",
    "cScaleLabel2": "#7a5a00",
    "cScale3": "#e3f2fd",
    "cScaleLabel3": "#0d47a1"
  }
}}%%

timeline
    title rustyfarian-power Roadmap

    Ready     : esp-hal parity v1 — no_std stoker, ADC battery + charging monitors on C3/C6/ESP32/S3, blocking + async (feature-doc)

    Near term : Verify Heltec V3 schematic — battery/USB power-path confirmed; still need VEXT GPIO, charge-controller IC marking, and CHRG/STAT GPIO
              : Add dual-target CI matrix — separate build jobs for xtensa-esp32s3-espidf and xtensa-esp32-espidf
              : Harden EspWakeCauseSource multi-source disambiguation
              : Calibration example — raw ADC readings with statistics

    Mid term  : Radio power gating — GPIO-controlled MOSFET for SX1262 and OLED, SX1262 sleep sequencing via SPI
              : Heltec V3 EspChargingMonitor — blocked on schematic verification above
              : Extend BatteryMonitor trait for multi-cell battery packs
              : Contract test scaffold — shared test fn run against both NoopBatteryMonitor and EspAdcBatteryMonitor
              : esp-hal sleep/wake v2 — ADR for a HAL-neutral SleepManager/WakeCauseSource contract first

    Long term : PM Locks and light sleep — FreeRTOS PM lock wrapper behind optional pm-locks feature
              : ULP coprocessor sampling — ADC reads during deep sleep without waking the main CPU
              : Power profiling toolchain — measure sleep vs active current draw
              : Publish to crates.io once API is stable
```
