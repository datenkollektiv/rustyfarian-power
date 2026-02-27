# Project Vision

## North Star
Give every rustyfarian application on ESP32 a single, ergonomic power management layer so battery-powered field deployments run reliably for months without intervention.

## Long-Term Goals
- Provide deep sleep with configurable wake-up sources (timer, GPIO, touch, and more) as first-class primitives.
- Offer radio power gating interfaces that coordinate cleanly with `rustyfarian-network` crates without duplicating network logic.
- Surface battery status (voltage, percentage, power source) with hardware-independent abstractions that any rustyfarian app can consume.
- Support solar-assisted deployments by detecting and exposing charging/boost input state.
- Keep the API ergonomic enough that a new rustyfarian application can integrate power management in under a day.

## Target Beneficiaries
Developers building battery-powered IoT applications in the rustyfarian ecosystem — for example, a remote beehive monitoring sensor that must operate in a field for a full season with minimal maintenance.

## Non-Goals
- Wi-Fi connection management, MQTT, or LoRaWAN protocol logic — these belong in `rustyfarian-network`.
- LED or visual status indicators — handled by `rustyfarian-network`.
- Application-level business logic (e.g., deciding *when* to transmit sensor data).
- Supporting ESP32 targets beyond the Heltec WiFi LoRa 32 V3 until ecosystem needs arise.

## Success Signals
- A new rustyfarian application can integrate power management in under a day.
- The beehive monitoring sensor reports reliably for months on a single charge or with solar assistance.
- Downstream rustyfarian crates rarely need updates due to breaking changes in this library.
- Core logic remains testable on the host without hardware.

## Open Questions
- **Solar integration depth:** Does solar support mean detecting charging input state only, or should the library manage interaction with an MPPT controller directly?

## Vision History
- 2026-02-27 — Initial vision established.
  Project scope expanded from battery monitoring to full power management layer for the rustyfarian ESP32 ecosystem.
  Covers deep sleep, wake sources, radio power gating, battery monitoring, and solar input awareness.
