# Roadmap

*Last updated: May 2026*

Charging detection shipped as a dedicated `ChargingMonitor` trait rather than a `PowerSource` enum variant, keeping power-source identity and charge-state lifecycle as orthogonal concerns.
Hardware wiring documentation for both Heltec V3 and Feather V2 is complete.
Near-term focus shifts to wake-cause disambiguation for multi-source GPIO configurations and a calibration example for ADC validation in the field.

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

    Near term : Harden EspWakeCauseSource multi-source disambiguation
              : Calibration example — raw ADC readings with statistics

    Mid term  : Extend BatteryMonitor trait for multi-cell battery packs
              : Mock BatteryMonitor implementation for host-side integration testing

    Long term : Embassy async integration for non-blocking ADC sampling
              : Power profiling toolchain — measure sleep vs active current draw
              : Publish to crates.io once API is stable
```
