# Roadmap

*Last updated: June 2026*

Charging detection and deep-sleep wake-source handling are working on both Heltec V3 and Adafruit Feather V2.
The Feather V2 has a complete `EspChargingMonitor`; the Heltec V3 charging implementation is blocked on schematic verification of the charge controller IC and its GPIO — an inversion relative to the README's "primary target" framing that near-term work must resolve.
Simultaneous battery + USB operation is confirmed safe and intended on both boards (vendor docs cited in `docs/key-insights.md`); what remains for Heltec is the exact charger-IC marking and a STAT/CHRG status GPIO.
The workspace `esp-idf-hal` dependency is pinned to `0.45` while the broader ecosystem moved to `0.46`+ in April 2026; the upgrade is the highest-priority near-term maintenance task because a downstream app combining `rustyfarian-power` and `rustyfarian-network` will hit a Cargo resolution conflict at their current versions.

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

    Ready     : Write a feature doc in docs/features/ to promote an item from Near term

    Near term : Bump esp-idf-hal to current ecosystem version — unblocks downstream apps that mix power and network crates
              : Verify Heltec V3 schematic — battery/USB power-path confirmed; still need VEXT GPIO, charge-controller IC marking, and CHRG/STAT GPIO
              : Add dual-target CI matrix — separate build jobs for xtensa-esp32s3-espidf and xtensa-esp32-espidf
              : Harden EspWakeCauseSource multi-source disambiguation
              : Calibration example — raw ADC readings with statistics

    Mid term  : Rename crate battery-monitor to rustyfarian-power — do before radio gating work adds RadioPowerGate
              : Radio power gating — GPIO-controlled MOSFET for SX1262 and OLED, SX1262 sleep sequencing via SPI
              : Heltec V3 EspChargingMonitor — blocked on schematic verification above
              : Extend BatteryMonitor trait for multi-cell battery packs
              : Contract test scaffold — shared test fn run against both NoopBatteryMonitor and EspAdcBatteryMonitor

    Long term : PM Locks and light sleep — FreeRTOS PM lock wrapper behind optional pm-locks feature
              : ULP coprocessor sampling — ADC reads during deep sleep without waking the main CPU
              : Embassy async integration — if a bare-metal consumer arrives; trait modules are already HAL-agnostic
              : Power profiling toolchain — measure sleep vs active current draw
              : Publish to crates.io once API is stable
```
