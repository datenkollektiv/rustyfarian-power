# stoker

Platform-agnostic battery monitoring, power-source detection, and deep-sleep
validation for embedded projects.

`stoker` is the pure, host-buildable core of the
[rustyfarian-power](https://github.com/datenkollektiv/rustyfarian-power) stack.
It has no ESP-IDF dependency and is fully testable on the host — the funfair
theme of the rustyfarian core crates (`bunting`, `pennant`, `ferriswheel`,
`juggler`): the stoker keeps the engine running for the long haul.

## What it provides

- **Battery math** — `BatteryConfig` with voltage thresholds, linear
  `voltage_to_percent()`, and `evaluate_reading()` (divider compensation +
  power-source detection).
- **Types** — `BatteryStatus`, `PowerSource`, `ChargingState`, `ChargingSource`.
- **Traits** — `BatteryMonitor`, `ChargingMonitor`, `SleepManager`,
  `WakeCauseSource`; program against these, not the concrete hardware types.
- **Sleep validation** — `WakeSource`, `WakeCause`, `GpioWakeMask`, and pure
  `validate_wake_sources()` / `validate_gpio_level_source()` helpers.
- **Host mocks** — `NoopBatteryMonitor`, `NoopChargingMonitor`,
  `NoopSleepManager` for testing consumer code without hardware.

## Hardware drivers

The ESP-IDF implementations of these traits live in the companion
[`rustyfarian-esp-idf-power`](https://crates.io/crates/rustyfarian-esp-idf-power)
crate, which re-exports everything here so device firmware needs only one import.

## Example

```rust
use stoker::{BatteryMonitor, NoopBatteryMonitor};

fn needs_transmit(monitor: &mut impl BatteryMonitor) -> bool {
    monitor.read().is_sufficient(3600, 20)
}

assert!(needs_transmit(&mut NoopBatteryMonitor::on_external()));
assert!(!needs_transmit(&mut NoopBatteryMonitor::on_battery(3400, 10)));
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
