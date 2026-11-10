# Roadmap

*Last updated: November 2026*

`stoker` plus the ESP-IDF and esp-hal tiers cover battery monitoring, charging detection, deep-sleep wake sources, RTC retention and clock, and the Vext rail gate on the C3, C6, ESP32 and S3.
Hardware checks are rustyfarian-bench scenarios in `bench/`; the shipped state per release is in `CHANGELOG.md`.
Neither the Feather V2 nor the Heltec V3 exposes a charge-status GPIO on the stock board, so charging detection stays a documented limitation there (`docs/project-lore.md`).
The ESP-IDF tier tracks the family stack of `rustyfarian-network`, `-ws2812` and `-peripherals`, so downstream apps combine the crates in one `esp-idf-sys` graph.

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

    Near term : C6-DevKitC-1 hardware pass, both tiers — timer and EXT1 wake, held-pin guard policy (Never or Always), UART drain, RTC clock, retention across resets
              : Regression benches — C3 R1-R4 and Heltec V3 R2-R4 on both tiers
              : Cross-chip retry bench — three try_sleep retries after a software and after a hardware Rejected behave as fresh attempts; C3 IDF clears the wake pin's deep-sleep enable after a software Rejected
              : Cross-chip retention bench — the session id survives deep sleep, GPIO wake, brownout and every reset kind each tier supports
              : Cross-chip wake-mask bench — the reported mask matches the physical pin and polarity for AnyHigh and AnyLow
              : Shortest-timer bench on the hal tier — continuous UART output and GPIO interrupts neither miss the deadline nor report UartBusy
              : esp-hal RTC clock drift — ten 60 s sleeps after an hour within ±1 % of the wall clock, and against EspRtcClock
              : Deep-sleep current with the PPK2 on every board, plus the WIFI_FORCE_ISO awake cost on the hal S3
              : Harden EspWakeCauseSource multi-source disambiguation
              : Tighten scenarios — assert the wake mask in c3/pir-wake, make the s3/r2 changed-mask retry step mandatory and judge its WAKECAUSE value, add a C3 R2 twin, add hal_s3_rtc_retention and an IDF ESP32 retention twin, cover retention across software/panic/watchdog resets

    Mid term  : Extend BatteryMonitor trait for multi-cell battery packs
              : Contract test scaffold — shared test fn run against both NoopBatteryMonitor and EspAdcBatteryMonitor

    Long term : PM Locks and light sleep — FreeRTOS PM lock wrapper behind optional pm-locks feature
              : ULP coprocessor sampling — ADC reads during deep sleep without waking the main CPU
```
