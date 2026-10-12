# rustyfarian-esp-hal-power

Bare-metal (**esp-hal**, `no_std`) battery monitoring, charging detection, and deep-sleep drivers for ESP32-C3, ESP32-C6, ESP32, and ESP32-S3.
This is the bare-metal hardware tier of the [rustyfarian-power](../../README.md) stack.
It implements the traits of the platform-agnostic [`stoker`](../stoker) crate on top of `esp-hal`, and re-exports `stoker`'s public surface so firmware needs a single import.

## Status

Scaffold: no drivers yet.
The sleep module lands first, see [esp-hal-sleep-tier-v1](../../docs/features/esp-hal-sleep-tier-v1.md).
ADC battery monitoring and charging detection follow, see [esp-hal-parity-v1](../../docs/features/esp-hal-parity-v1.md).

## Chip features

Exactly one chip feature must be enabled; they are mutually exclusive.

- `esp32c3` (default, RISC-V)
- `esp32c6` (RISC-V)
- `esp32` (Xtensa LX6)
- `esp32s3` (Xtensa LX7)

The default feature set is `esp32c3` plus `unstable`, because esp-hal gates ADC and the sleep API behind `unstable`.
For any other chip use `--no-default-features --features <chip>,unstable`.

## Building

Check the crate for all four chips:

```shell
just check-hal
```

Lint it for all four chips:

```shell
just clippy-hal
```

Builds run on the pinned `esp` toolchain with `-Zbuild-std=core,alloc` and write into `target/hal`.
The crate is not part of the workspace default members, so bare `cargo` commands and the `*-all` recipes skip it.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
