# ADR-002: Power rail gate contract for the ESP-IDF and esp-hal tiers

## Status
Accepted (2026-10-24).

Pinned to ESP-IDF v5.3.3 and esp-hal `=1.2.2`, as in ADR-001.
The Heltec V3 pin facts were verified against the Zephyr board devicetree `heltec_wifi_lora32_v3_procpu.dts` on 2026-10-24.

## Context
`rustyfarian-network` needs to switch a power domain off between uplinks to save current.
The first name for this was `RadioPowerGate`, which assumed the radio itself could be switched.
On the Heltec V3 it cannot: the SX1262 sits on the always-on 3V3 rail and has no power domain of its own.
The only switchable rail is Vext, driven by GPIO36 (`Vext_Ctrl`), active-low, pulled up, through a P-MOSFET.
Vext feeds the OLED and the Ve header pins.

The gate therefore switches a rail, not a radio.
Putting the SX1262 sleep command (`SetSleep` over SPI) into the gate would make `stoker` depend on radio behaviour.
The power crates must not import network types, so that command stays in the network radio driver.

The contract has to be HAL-neutral, like ADR-001, and `stoker` must stay `#![no_std]` with `critical-section` as its only dependency.

## Decision

### 1. Trait shape

```rust
pub trait PowerRailGate {
    type Error: Error;
    #[must_use]
    fn power_on(&mut self) -> Result<(), Self::Error>;
    #[must_use]
    fn power_off(&mut self) -> Result<(), Self::Error>;
    fn is_powered(&self) -> bool;
}
```

- `power_on` and `power_off` are idempotent.
  Calling `power_on` on a rail that is already on returns `Ok` and drives the pin again.
- `is_powered` returns the cached state of the last successful call.
  It is infallible because `OutputPin` cannot read itself back on every HAL.
- Receivers are `&mut self` with no `Send` bound, mirroring `SleepManager`.
- A failed call leaves the cached state unchanged.

### 2. Error contract

```rust
pub trait Error: core::fmt::Debug {
    fn kind(&self) -> GateErrorKind;
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateErrorKind {
    Platform,
    Other,
}
```

- The trait mirrors `stoker::sleep::Error`: `Debug` plus `kind()`, and no `core::error::Error` bound.
  Generic consumers branch on `kind()`, and `defmt`-only error types stay possible.
- `Platform` is a pin or SDK failure, and it is the only kind the tiers produce.
- `Other` is the catch-all for future variants.
  No `InvalidConfig` kind exists because the gate takes no configuration request at call time.
- `GateError { Refused }` is `#[non_exhaustive]`, implements `gate::Error` with kind `Platform`, and is used by the mock.
- Tier errors wrap the pin error and map to `Platform`.

### 3. Coordinator semantics: `GatedSleep`

`GatedSleep<G: PowerRailGate, S: SleepManager>` owns both values by value.
It implements `SleepManager` with `type Error = GatedSleepError<G::Error, S::Error>`.

- `GatedSleepError` is `Gate(G::Error)` or `Sleep(S::Error)`.
- Its `kind()` maps `Gate(_)` to `SleepErrorKind::Platform` and `Sleep(e)` to `e.kind()`.
- `sleep` cuts the rail, then calls the inner `sleep`.
- If the inner sleep returns `Err`, the rail is restored to its pre-call state on a best-effort basis (powered on only if it was on) and the sleep error is returned.
- `try_sleep` does the same, and a `Rejected` result also restores the rail.
- A gate failure before sleep returns `Gate(..)` and the sleep is never attempted.
- On hardware, deep sleep never returns, so the rail stays off after a wake until the consumer calls `power_on`.
- The consumer calls `power_on` at boot, because the tier constructor leaves the rail off.
- The coordinator does not wait for the rail to settle.
  The consumer delays after `power_on`.

### 4. Where the generic GPIO gate lives

`GpioRailGate<P: embedded_hal::digital::OutputPin>` is defined once per tier, not in `stoker`.

- `GpioRailGate` holds `pin`, `polarity` and a cached `powered` flag.
- `Polarity` is `ActiveHigh` or `ActiveLow`.
- `GpioRailGate::new(pin, polarity)` drives the rail off immediately, so `is_powered` starts `false` and matches the pin.
- The Heltec V3 preset `heltec_v3_vext(pin)` fixes `Polarity::ActiveLow`.
- `HELTEC_V3_VEXT_GPIO: u8 = 36` is exported next to the preset.
- Each tier depends on `embedded-hal = "1"`, declared in `[workspace.dependencies]` and referenced with `workspace = true`.
- `stoker` gets no `embedded-hal` dependency.
  An optional feature was rejected because it would pull a HAL trait into a crate that only needs `critical-section`.
- `PinDriver<_, Output>` on ESP-IDF and `gpio::Output` on esp-hal both implement `OutputPin`.

### 5. What `rustyfarian-network` may rely on

- The network takes `&mut impl PowerRailGate` and calls `power_on` and `power_off` around an uplink.
- It must not assume `Send` on the gate; that bound is the consumer's choice.
- `SetSleep` on the SX1262 over SPI stays in the network radio driver.
- The settle delay after `power_on` is the consumer's responsibility.
- The gate carries no network types, and dependencies flow application to network to power.

## Consequences

Positive:
- One gate contract compiles against both tiers, and consumers host-test with `NoopPowerRailGate`.
- Sequencing (off before sleep, restore on error or reject) is host-tested in `stoker`, without hardware.
- `stoker` keeps `critical-section` as its only dependency.
- The name `RadioPowerGate` is retired before any consumer depends on it.

Negative:
- The GPIO gate code exists twice, once per tier, as a short generic wrapper.
- Consumers must call `power_on` at every boot, because the rail is off after a wake.
- The Heltec V3 gate does not save radio current; the bench measures the Vext load (OLED, Ve header) only.
- Whether the GPIO36 pull-up keeps Vext off through deep sleep on IDF and esp-hal is open and needs the bench.
- Both tiers gain a direct `embedded-hal` 1.0 dependency.

## Alternatives Considered
|                              Alternative | Pros                                | Cons                                              | Why Rejected                                                   |
|-----------------------------------------:|:------------------------------------|:--------------------------------------------------|:---------------------------------------------------------------|
|           Keep the `RadioPowerGate` name | Matches README and VISION wording   | The V3 radio cannot be switched, so the name lies | The trait switches a rail; docs follow the contract            |
|             Infallible `()` return types | Simpler call sites                  | A rail that refuses to switch goes unnoticed      | Refusal is an operational failure; `kind()` carries it         |
| `EspSleepManager` takes an optional gate | One object for consumers            | Puts gate logic into the IDF sleep path only      | Coordinator in `stoker` is host-testable and tier-neutral      |
|      Optional `embedded-hal` in `stoker` | One generic gate for both tiers     | Adds a HAL dependency to a `no_std` core crate    | `stoker` stays `critical-section`-only; wrapper is short       |
|            `StatefulOutputPin` read-back | `is_powered` reflects the pin       | Not implemented by every HAL's output pin         | Cached state from the last successful call is enough           |
|       Off, Powering, Ready state machine | Models the settle delay in the type | Settle time is load-dependent and hard to fix     | The consumer delays after `power_on`; no settle state in trait |
