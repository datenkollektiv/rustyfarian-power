# Key Insights

This file is the persistent knowledge base for the rustyfarian-power project.
All agents and Claude Code sessions must read this file at the start of relevant work and update it when new insights are discovered.

Organise entries by topic, not chronologically.
Keep each insight concise: one paragraph or a short bullet list per topic.
Remove or update entries that are superseded.

---

## CI and Build Validation

- **GitHub Actions workflows live in `.github/workflows/`** with four files: `rust.yml` (CI: deny + check + test), `fmt.yml` (format check), `clippy.yml` (clippy), `audit.yml` (cargo-audit, runs on schedule + push).
  Each workflow calls a `just` recipe (`just deny`/`check`/`test`/`fmt-check`/`clippy`/`audit`) via `extractions/setup-just@v4`, so the justfile is the single source of truth and CI cannot drift from local `just verify`/`just ci` (pattern adopted from rustyfarian-network PR #79).
  Keep the four workflow files structurally consistent (checkout → toolchain → setup-just → cache → recipe); if the boilerplate grows, factor it into a reusable workflow rather than letting them diverge.
  Host-side recipes already pass `--no-default-features --target <host>` (host detected inside the recipe via `scripts/host-target.sh`), avoiding the ESP-IDF cross-compile toolchain (not installed on GitHub-hosted runners).
  **`RUSTUP_TOOLCHAIN: stable` must stay set in every workflow** — it overrides this repo's `rust-toolchain.toml` (`channel = "esp"`), which is not installed on CI.
- **`deny.toml` is required for `just deny` / `just ci` to pass.**
  Without it, `cargo-deny` defaults to an empty licence allowlist and rejects every dependency.
  The allowlist for this repo is: `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-3-Clause`, `ISC`, `Unlicense`, `Unicode-3.0`, `Zlib`.
  No LGPL entry is needed: `r-efi` is `MIT OR Apache-2.0 OR LGPL-2.1-or-later` and cargo-deny accepts it via the OR semantics once MIT/Apache-2.0 are listed.
  The `multiple-versions = "warn"` setting produces duplicate-crate warnings from the `embuild`/`esp-idf-sys` tree — these are expected and harmless.
- **No cross-compilation in CI (no `espup`).**
  The sibling repos (rustyfarian-ws2812, etc.) gate only host-testable code in CI.
  This repo follows the same pattern: host-only check + test under `--no-default-features`.
  Cross-compilation for `xtensa-esp32s3-espidf` must be done locally with `just check-all` / `just build-example <name>`.
- **`just audit` generates a `Cargo.lock` if absent** (the lockfile is gitignored for this library) before running `cargo audit`; `just deny` needs no explicit lockfile step because `cargo deny` resolves the graph via `cargo metadata`.
- **MSRV is set for family consistency, not a hard requirement: `stoker` 1.88 (workspace floor), `rustyfarian-esp-idf-power` 1.95.**
  No code uses 1.88-specific features (the host logic needs ~1.82 for `Option::is_none_or`), and the ESP-IDF stack (`esp-idf-hal 0.47`) needs only 1.82.
  The ESP tier's 1.95 follows the family policy set in the September 2026 wave, where rustyfarian-ws2812 and rustyfarian-peripherals raised their ESP tier crates in step with `esp-hal 1.2`.
  There is no MSRV-pinned CI job, so treat both as declared floors — change them in step with the siblings.
- **Action runtimes: read `runs.using` in each action's `action.yml` at the tag in use.**
  `actions/checkout@v4` and `extractions/setup-just@v2` were Node 20 (removed from runners 2026-09-23); `@v5` / `@v4` are node24 / composite. `Swatinem/rust-cache@v2` already resolves to node24.

## Hardware

- **Target board:** Heltec Wi-Fi LoRa 32 V3, which uses the ESP32-S3 chip.
  The Cargo target is `xtensa-esp32s3-espidf`, not the generic `xtensa-esp32-none-elf`.
- **Battery ADC pin:** GPIO1 on an ADC1 channel.
  The Heltec V3 routes the battery through a voltage divider before the ADC pin; the divider ratio is captured in `BatteryConfig`.
- **USB detection threshold:** When powered via USB/external, the measured voltage is above the `usb_detection_threshold` configured in `BatteryConfig::heltec_v3()`.
  Do not confuse this with the battery full voltage.

## Architecture

- **Hardware-independence boundary:** Core logic (`config.rs`, `lib.rs`) has no ESP-IDF dependency and compiles on the host for testing.
  Only `esp_adc.rs` is feature-gated behind `esp-idf` (the default feature).
  Always keep this separation clean.
- **Test strategy:** Unit tests live in `lib.rs` and target the host.
  Never add `esp-idf-hal` calls to test code — use the `BatteryConfig` methods directly.

## Build System

- **Toolchain:** Requires the `esp` Rust toolchain channel (Espressif's Xtensa fork), installed via `espup`.
  The channel is declared in `rust-toolchain.toml`.
  Standard `rustup` toolchains cannot compile for `xtensa-esp32s3-espidf`.
- **ESP-IDF version:** Pinned to v5.3.3 in `.cargo/config.toml` via `ESP_IDF_VERSION`.
  Do not float this; bindgen output changes with every ESP-IDF version.
- **Linker and `ldproxy`:** `linker = "ldproxy"` is required in `.cargo/config.toml` for every Xtensa ESP-IDF target.
  `rustyfarian-esp-idf-power/build.rs` calls `embuild::espidf::sysenv::output()`, which emits `--ldproxy-linker` and `--ldproxy-cwd` link args that are consumed by `ldproxy`.
  Without it, those args are passed directly to `xtensa-esp-elf-gcc`, which rejects them with "unrecognized command-line option" and exits with code 101.
  Both `xtensa-esp32s3-espidf` and `xtensa-esp32-espidf` must have `linker = "ldproxy"` in `.cargo/config.toml`.
- **Host tests:** Run with `cargo test -p stoker` (no cross-compilation needed).
  `stoker` is the pure crate with no ESP-IDF dependency, so host tests run without the ESP-IDF build tooling. The ESP-IDF `rustyfarian-esp-idf-power` crate cannot build on the host at all.

## ESP32-S3 Sleep and Power Management

- **Deep sleep current:** The ESP32-S3 datasheet specifies ~7 µA in deep sleep (RTC domain only, all others off).
  Real-world Heltec V3 boards measure significantly higher (50–150 µA) due to the LDO regulator, charge controller, and LoRa radio quiescent draw.
  The chip-only number is not achievable in practice on this board without hardware modifications.
- **Wake-up sources in deep sleep (ESP32-S3):** Timer (`esp_sleep_enable_timer_wakeup`), EXT0 (single RTC GPIO, GPIOs 0–21), EXT1 (multiple RTC GPIOs 0–21, ANY_LOW or ANY_HIGH), Touch sensor, ULP FSM co-processor, and no GPIO deep sleep wakeup (`esp_deep_sleep_enable_gpio_wakeup` needs `SOC_GPIO_SUPPORT_DEEPSLEEP_WAKEUP`, which is C2/C3/C6 only in IDF v5.3.3; `esp_sleep_get_gpio_wakeup_status` has no S3/ESP32 binding).
  Timer wakeup is the primary source for a LoRaWAN duty-cycle sensor.
  EXT1 or GPIO wakeup are useful for alarm / interrupt-driven sensors (e.g., rain gauge tip, door sensor).
- **ULP coprocessor:** ESP32-S3 has a ULP FSM coprocessor (`SOC_ULP_FSM_SUPPORTED = 1`, `SOC_ULP_HAS_ADC = 1`).
  It can sample the ADC while the main CPU is in deep sleep, enabling battery monitoring without full wake-up.
  This is a useful capability for intelligent wake/no-wake decisions.
- **Sleep API (ESP-IDF v5.3.3):** Public API is in `esp_sleep.h` (component `esp_hw_support`).
  Key functions: `esp_sleep_enable_timer_wakeup(us)`, `esp_sleep_enable_ext1_wakeup_io(mask, mode)`, `esp_deep_sleep_enable_gpio_wakeup(mask, mode)`, `esp_sleep_pd_config(domain, option)`, `esp_deep_sleep_start()`, `esp_light_sleep_start()`, `esp_sleep_get_wakeup_cause()`.
  `esp_deep_sleep_start()` never returns; use `esp_deep_sleep_try_to_start()` if you need rejection detection.
  Use `esp_sleep_enable_ext1_wakeup_io()` (v5.x preferred API) — not the deprecated `esp_sleep_enable_ext1_wakeup()`.
- **EXT1 wakeup status functions:** Call `esp_sleep_get_ext1_wakeup_status()` when the wakeup cause is `ESP_SLEEP_WAKEUP_EXT1` to get the bitmask of fired pins.
  For `ESP_SLEEP_WAKEUP_GPIO` (C3/C6 deep-sleep GPIO wake), use `esp_sleep_get_gpio_wakeup_status()` instead — the EXT1 status register is not populated.
  Both functions return a `u64` bitmask (bit N = GPIO N).
  The EXT1 register is preserved by hardware until the next sleep entry, so it can be read at any point after boot.
  Recommended practice (per Espressif) is to read it early in `main()` before peripheral initialisation, and store the result — not because the register is likely to be cleared, but for clarity and defensive coding.
- **GPIO wake sources (EXT1):** Only RTC GPIOs 0–21 are supported on ESP32-S3.
  External pull resistors (10–100 kΩ) are mandatory on every wake GPIO; RTC internal pulls are unavailable during deep sleep when `RTC_PERIPH` is powered down.
  Missing external resistors are a common field failure mode — floating pins may false-trigger or fail to trigger.
  When using `WakeSource::GpioLevel`, set `EspSleepManager { isolate_gpio: false }` — GPIO isolation (the default) may prevent wake-capable pins from triggering.
- **GPIO wake sources (C2/C3, deep-sleep GPIO wake, IDF v5.3.3):** selected in `esp_sleep.rs` by `cfg(all(esp_idf_soc_gpio_support_deepsleep_wakeup, not(esp_idf_soc_pm_support_ext1_wakeup)))`; esp-idf-sys emits these SOC cfgs and `embuild::espidf::sysenv::output()` relays them. C6 has both and stays on EXT1.
  Valid pins are GPIO0-5 (`0x3F`) on C3 and C2.
  `esp_sleep_disable_wakeup_source(ALL)` only clears `RTC_GPIO_TRIG_EN`; the per-pin hardware enable and the SDK's `s_config.gpio_wakeup_mask` survive, so the manager calls `gpio_deep_sleep_wakeup_disable` on every unused pin whose `RTC_CNTL_GPIO_WAKEUP_REG` enable bit is set (never on unarmed pins: the call also re-enables sleep-select). The SDK mask itself has no public clear, so an earlier-armed pin still gets hold + internal pull at later GPIO-wake entries in the same boot (it can no longer wake).
  SDK bug: switching a pin from HIGH to LOW leaves its trigger-mode bit set (`&= ~(0 << idx)`), so it keeps the internal pull-down during sleep; a reset clears it.
  The IDF enables the internal pull against the wake level at entry by default (`CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS=y`), and after a rejected `try_sleep` the wake pin stays held (`gpio_hold_en`).
  The C3 reject mask includes GPIO, but on hardware a held wake pin is NOT rejected: `try_sleep` sleeps and the chip wakes about 20 ms later with `cause=Gpio` (bench H1, 2026-10-12). `EspSleepManager::try_sleep` therefore reads the input-enabled wake pins right before sleep entry and returns `Rejected` itself. See `docs/project-lore.md` → ESP-IDF tier — ESP32-C3 deep sleep.
- **GPIO wake sources (S3, EXT1, IDF and esp-hal):** a held EXT1 level IS rejected by the S3 hardware on both tiers (`esp_deep_sleep_try_to_start`, esp-hal `sleep_deep_with_rejection`; CrowPanel bench 2026-10-14), so the S3 needs no pre-sleep level check, unlike the C2/C3.
  The wake cause is `WakeCause::Ext1(mask)` on both tiers (normalised per chip, ADR-001 § 1).
  On the CrowPanel 1.28" the CST816S INT only works with `LCD_3V3` (GPIO1) and touch RST (GPIO13) held HIGH through sleep, and it pulses during a resting touch, so a finger on the screen gives a wake loop, not a rejection. See `docs/project-lore.md` → ESP-IDF tier — ESP32-S3 deep sleep.
- **Power domain control:** `esp_sleep_pd_config(ESP_PD_DOMAIN_*, ESP_PD_OPTION_OFF/ON/AUTO)` controls RTC peripherals, RTC slow/fast memory, XTAL, CPU, VDDSDIO, and Modem domains during sleep.
  Powering down the MODEM domain (`ESP_PD_DOMAIN_MODEM`) in deep sleep is automatic when WiFi/BT are not in use.
- **EXT1 mode chip difference — `ANY_LOW` vs `ALL_LOW`:** `ESP_EXT1_WAKEUP_ANY_LOW` does not exist on the original ESP32; only `ESP_EXT1_WAKEUP_ALL_LOW` is available (all configured pins must be low simultaneously).
  On ESP32-S3 and newer chips, `ESP_EXT1_WAKEUP_ANY_LOW` is the preferred API and `ESP_EXT1_WAKEUP_ALL_LOW` is deprecated.
  `esp_sleep.rs` uses `#[cfg(esp32)]` to select the correct constant at compile time.
  The semantics differ (any vs all), but `ALL_LOW` is the closest available mode on original ESP32.
  This was confirmed by a compile failure against `xtensa-esp32-espidf` when adding Adafruit Feather V2 support.
- **`cfg(esp32)` requires a `build.rs` in *each* crate that reads it:** `cargo:rustc-cfg` outputs from a build script only apply to that crate, not its dependents.
  Even though `esp-idf-sys` emits `cargo:rustc-cfg=esp32`, that flag is not visible to either workspace crate.
  Both `stoker` and `rustyfarian-esp-idf-power` therefore carry a `build.rs` that reads `TARGET` and emits `cargo:rustc-cfg=esp32` / `cargo:rustc-cfg=esp32s3` directly, plus `cargo:rustc-check-cfg` to silence the `unexpected_cfgs` lint. `stoker` needs its own copy because `sleep.rs::validate_gpio_level_source` uses `#[cfg(esp32)]` — the crate-split does **not** let it inherit the cfg from the ESP-IDF crate's build script. `stoker`'s `build.rs` does the cfg emission only (no `embuild` linker step).
- **GPIO isolation:** Call `esp_sleep_config_gpio_isolate()` and `esp_sleep_isolate_digital_gpio()` before deep sleep to prevent floating digital GPIO pins from leaking current.
  This is one of the most commonly missed optimizations on real boards.
- **Deep sleep wake stub:** A minimal function marked `RTC_IRAM_ATTR` can execute immediately on wake before the full boot, useful for deciding whether to fully boot or return to sleep.
  Set via `esp_set_deep_sleep_wake_stub()`.
- **Light sleep vs deep sleep for LoRaWAN:** Deep sleep is the correct choice for a sensor transmitting every few minutes.
  Light sleep preserves RAM and peripheral state but consumes ~800 µA–2 mA (too high for months-long battery life).
  Deep sleep with timer wakeup achieves the lowest average current.
- **Heltec V3 SX1262 radio power:** The SX1262 is powered by a dedicated GPIO-controlled MOSFET/LDO on the Heltec V3 board.
  GPIO 3 (VEXT) controls external power to the radio and OLED display on some Heltec board revisions; the specific power control GPIO must be verified against the V3 schematic.
  The SX1262 has its own sleep mode (400 nA) accessible via SPI command — `SetSleep(0x00)`.
  For deep sleep of the ESP32, the SPI bus must be fully idle and the radio must be put to sleep first.

## Trait Design Decisions

- **BatteryMonitor infallible convention:** `BatteryMonitor::read` returns `BatteryStatus` (not `Result`) by absorbing ADC errors into `PowerSource::Unknown`.
  New traits that read best-effort status (e.g., `ChargingMonitor::read_charging_state`) should follow the same convention for consistency.
  Traits whose failure is operationally critical (e.g., `SleepManager::sleep`, or the planned radio power gate's `power_on`) return `Result<(), Self::Error>` with a typed `stoker` error for the pure implementation; `SleepManager::Error` is bounded by `stoker::sleep::Error` (`Debug` + `kind()`), and `EspSleepManager` uses its own `EspSleepError` (since 2026-10-12, 0.3.0).
- **Sleep/wake asymmetry:** On ESP32, `esp_deep_sleep_start()` never returns.
  This means `WakeCause` cannot be returned from a `sleep()` call — it must be read at the next boot via a separate `WakeCauseSource` trait.
  Always keep `SleepManager` and `WakeCauseSource` as two distinct traits; merging them implies a round-trip that does not exist in hardware.
- **Resets and sleep are separate traits (since 2026-10-12):** `ResetReasonSource` answers "how did the last reset happen?" while `WakeCauseSource` folds every non-sleep reset into `WakeCause::PowerOn`.
  `Retained::load` needs the reset reason because RTC memory is arbitrary after power-on and may be partially written after a brownout, so both are refused regardless of checksum.
- **RTC retention envelope (`stoker::retain`, since 2026-10-12):** `Retained<N>` is integers plus a byte array only, so any bit pattern is a valid value; the checksum is table-less CRC-32 over the little-endian header and the first `len` payload bytes, and payloads serialise through `Retainable::write`/`read` so padding is never checksummed.
  `N >= T::SIZE` is runtime-only (`Invalid::Length`) — stable Rust cannot use `T::SIZE` in an array type, so slice with `get`/`get_mut`, never index.
  The tier cells placing it in `.rtc_noinit` / `.rtc_fast.persistent` must read through `MaybeUninit` or volatile access: a `NOLOAD` static's initialiser is never applied and the compiler may otherwise constant-fold it.
  `#[derive(Default)]` is not available on `[u8; N]` for arbitrary `N`; use the `const fn empty()`.
- **IDF-tier RTC placement (`#[link_section = ".rtc_noinit"]`, since 2026-10-16):** ESP-IDF v5.3.3 collects `.rtc_noinit` and `.rtc_noinit.*` into one `NOLOAD` section (`esp_system/ld/<chip>/sections.ld.in`): RTC fast memory at `0x50000000` on the C3 (`rtc_data_location` aliases `rtc_iram_seg`), RTC slow memory on ESP32/S3.
  `sleep_modes.c::get_power_down_flags` keeps that domain on in deep sleep (the C3 build has `CONFIG_ESP_SYSTEM_ALLOW_RTC_FAST_MEM_AS_HEAP=y`, which forces RTC fast memory on; the heap only takes the RAM after the linker-placed sections), and `EspSleepManager` never calls `esp_sleep_pd_config`.
  A plain Rust `static` with the attribute and a zeroed `const fn` initialiser needs no `#[used]` (it is referenced) and no `unsafe` on edition 2021; verify placement with `riscv32-esp-elf-nm -n -S <example ELF> | grep RETAINED` from `~/.espressif/tools/riscv32-esp-elf/<ver>/riscv32-esp-elf/bin` (address in `0x5000_0000..0x5000_2000` on the C3; proven 2026-10-16 at `0x50000018`); `rust-nm` fails under the pinned `esp` toolchain because it has no llvm-tools.
  `just build-example` writes the release ELF under `<idf_dir>/riscv32imc-esp-espidf/release/examples/`, where `idf_dir` is the RAM disk (`/Volumes/RustBuilds/targets/idf/<repo>`) when attached; that 12 GB disk is shared by all sibling repos and fills up, and `just clean-hal` is the cheapest local reclaim.
  All 16 `esp_reset_reason_t_ESP_RST_*` constants are bound on C3, S3 and ESP32, so `EspResetReasonSource` needs no `cfg`.
- **esp-hal RTC placement (`#[esp_hal::ram(unstable(rtc_fast, persistent))]`, since 2026-10-16):** the macro expands to `#[unsafe(link_section = ".rtc_fast.persistent")]` plus a `Persistable` assertion on the static's type; the orphan rule means the hal crate wraps `stoker::RetainedCell` in the `#[repr(transparent)]` newtype `RtcRetainedCell` with `unsafe impl Persistable` (sound: integers and a byte array only), and consumers must depend on `esp-hal` directly for the macro to resolve.
  esp-riscv-rt (`rtc-ram`, enabled by esp-hal for the C3) zeroes the section before `main` on `ChipPowerOn` only; esp-hal 1.2.2 never power-gates RTC fast memory (`apply()` ignores `rtc_fastmem_pd_en`) and the bench on 2026-10-16 showed the record surviving 23 consecutive deep-sleep wakes, so no force-on workaround is needed (bench case R1, `docs/bench/c3-hal-rtc-retention-bench-v1.md`).
  `SocResetReason` CPU-local names differ per chip: C3 `Cpu0Sw`/`Cpu0Mwdt0`/`Cpu0Mwdt1`/`Cpu0RtcWdt`, S3 `CpuSw`/`CpuMwdt0`/`CpuMwdt1`/`CpuRtcWdt` (no `Cpu1*` variants); `EspHalResetReasonSource` cfg-splits those arms, and it never reports `Panic` because the examples' panic handler restarts through `software_reset()` (`CoreSw`).
  `esp_hal::init` disables every watchdog, so a self-inflicted watchdog reset must arm one: `TimerGroup::new(TIMG0).wdt`, `set_timeout(MwdtStage::Stage0, 500 ms)`, `enable()`, spin → `TG0WDT_SYS_RST` / `CoreMwdt0` → `Watchdog`.
  The generated sdkconfig has `CONFIG_ESP_SYSTEM_PANIC_PRINT_REBOOT=y` (a Rust panic reboots as `ESP_RST_PANIC`) but no `CONFIG_ESP_TASK_WDT_PANIC`: a starved task only warns, so a self-inflicted watchdog reset must trip the 300 ms interrupt watchdog (`esp_idf_hal::interrupt::free` around a spin loop → `ESP_RST_INT_WDT`).
- **Radio power gating — who implements the trait:** `rustyfarian-power` defines and implements `RadioPowerGate`; `rustyfarian-network` holds a reference to it.
  Dependencies must flow downward: application -> network -> power.
  The power crate must never import `rustyfarian-network`.
- **Reference-counted radio gate:** When multiple subsystems share one power rail, use a coordinator that wraps `RadioPowerGate` with a reference count.
  The radio powers on when the count goes 0 -> 1 and off when the count returns to 0.
  The `RadioPowerGate` trait itself exposes only the primitive three methods (`power_on`, `power_off`, `is_powered`); coordination is the implementor's concern.
- **ChargingState vs PowerSource are orthogonal:** `PowerSource` answers "what supplies energy now"; `ChargingState` answers "what is the battery's charge trajectory".
  They can vary independently (e.g., on solar with a full battery: `PowerSource::Solar`, `ChargingState::Full`).
  Do not merge them.
- **`PowerSource::Solar` is a planned breaking change:** Adding a `Solar` variant to the existing `PowerSource` enum is a deliberate semver break.
  Downstream match arms must add the arm.
  If the library is published to crates.io before the solar hardware is confirmed, add `#[non_exhaustive]` first.
- **`ChargingSource` as a field in `ChargingState::Charging`:** Rather than separate `ChargingFromUsb` and `ChargingFromSolar` variants, use `ChargingState::Charging { source: ChargingSource }`.
  This avoids proliferating variants for every source permutation.
- **⚠️ The stock Adafruit ESP32 Feather V2 has NO readable charge-status or VBUS-detect GPIO — the earlier "GPIO13 (STAT) + GPIO34 (VBUS)" pin map was wrong (likely inherited from the Unexpected Maker FeatherS2 Neo).** Confirmed on hardware 2026-06-19 with the `idf_esp32_chargeprobe` example (analog reads, USB connected, ~3.95 V cell): GPIO35 ≈ 1973 mV (battery ÷2, correct), but **GPIO34 ≈ 147 mV** (a real VBUS divider would force ~2500 mV → none exists) and **GPIO13 ≈ 134 mV** drifting (the claimed 4.7 kΩ pull-up to 3.3 V would read ~3300 mV → no pull-up; it is the user LED, floating). **Schematic-confirmed (EagleCAD rev F):** the CHG LED net is `VBUS → CHG LED → 5.1 kΩ (R2) → MCP73831 STAT`, and the STAT net (`N$3`) has exactly two nodes — the resistor and STAT — with no trace to the ESP32. The only power-related GPIOs on the board are GPIO35 (battery voltage) and GPIO2 (NeoPixel/I2C power *enable*, an output); there is no other pin worth probing. To get real charge/presence detection you must hand-wire the STAT side of R2 to the free **GPIO34 (A2)** and use `EspChargingMonitor`. (See `docs/project-lore.md` → Hardware.)
- **⚠️ On the Feather V2, "no battery" is undetectable in software while USB is connected — it masquerades as a ~96 % battery.** With no cell, the MCP73831 holds the BAT net at its ~4.16 V CV regulation point, so GPIO35 reads **~2081 mV** → ×2 → 4162 mV, which is `> min (3000)` and `< usb_detection (4300)` → classified as `Battery` at ~96 % (verified 2026-06-19). A real cell at 4.16 V reads identically; the only disambiguator (STAT) is not on a GPIO, so no threshold change can fix it. **This is a bench artifact of USB + no battery only**: in battery-only operation (no USB) a missing cell lets the BAT net sink to ~0 mV → correctly `Unknown`/`No battery`. Do not try to "fix" the false-full reading with thresholds; document it instead (or do the GPIO34 hardware mod above).
- **MCP73831 STAT is a tri-state logic output, not open-drain** (open-drain is the MCP73832). It is driven LOW during charging, actively HIGH on charge-complete, and Hi-Z in shutdown/no-battery; with an external pull-up a digital read collapses complete/no-battery into HIGH, so STAT + VBUS are both needed to resolve all four states. An ADC read of STAT can separate the three native levels.
- **`EspChargingMonitor` is generic and still valid for a board that genuinely routes STAT + a VBUS divider to GPIOs** — it is just not wired up on a stock Feather V2. API note: `set_pull` in esp-idf-hal requires `T: InputPin + OutputPin`, so the STAT generic must be `InputPin + OutputPin` even though the pin is only read; a VBUS pin that never has `set_pull` called (e.g. an input-only GPIO34–39 on the original ESP32) can stay `InputPin` only.
- **⚠️ BEFORE connecting any LiPo: verify polarity with a multimeter — these boards have NO reverse-polarity protection and a backwards cell destroys the charger/regulator.** Never trust cable colours: **MakerFocus packs use the OPPOSITE polarity to Heltec on the same JST-1.25 connector** (the plug fits but `+`/`−` are swapped), so plugging one in directly reverse-connects the battery. Confirm the battery's `+` lead by meter, find the board's `+` pad by voltage or by continuity to GND (the pad reads 0 V with no battery — charger idle — so use continuity), and connect measured `+` → measured `+`. Re-pin the JST or hand-wire crossed if needed. See `docs/project-lore.md` → Hardware.
- **Battery + USB can be connected simultaneously on both boards — it is the intended charging mode** (this is about the *simultaneity*, which is safe; it does NOT relax the polarity warning above). Verified against vendor documentation (links below); safe to keep a correctly-wired LiPo plugged in while powering or flashing over USB.
  - **Feather V2:** automatic power-path management — a changeover diode selects USB over battery when USB is present, and the MCP73831 charges the LiPo at the same time ("hot-swap"; the LiPo stays as backup for when USB is removed).
    The MCP73831 has an internal MOSFET that blocks reverse current into the charger, so there is no conflict.
    LiPo only — alkaline/NiMH/7.4 V RC packs destroy the charger.
  - **Heltec V3:** integrated lithium BMS with automatic USB/battery switching, overcharge protection, and battery-voltage detection.
    Charges at ~500 mA on a standard CC/CV profile (holds 4.2 V, then recharges from ~90 %).
    Separate rule: do not feed USB and the board's dedicated 5 V pin at the same time — pick one 5 V input (battery + USB is fine).
  - With USB connected, expect `PowerSource::External` (USB sits above the ~4.3 V detect threshold) with charging active.
  - Sources: [Adafruit Feather V2 — Power Management](https://learn.adafruit.com/adafruit-esp32-feather-v2/power-management-2); [MCP73831 reverse-current MOSFET (Adafruit forums)](https://forums.adafruit.com/viewtopic.php?t=222339); [Heltec WiFi LoRa 32 V3 — product page](https://heltec.org/project/wifi-lora-32-v3/); [Heltec V3 — docs](https://wiki.heltec.org/docs/devices/open-source-hardware/esp32-series/lora-32/wifi-lora-32-v3/); [ropg/heltec_esp32_lora_v3 — charging behaviour & VBAT divider](https://github.com/ropg/heltec_esp32_lora_v3/blob/main/README.md).
- **Heltec V3/V3.1 charge controller is a TP4054, and charge status (CHRG) is LED-only — NOT on any GPIO (schematic-confirmed).** Verified against the official V3.0 and V3.1 schematics: the TP4054's open-drain CHRG pin connects only to the charger and the orange charge LED (`VDD_5V → 330 Ω → LED → CHRG`); it is not wired to the ESP32-S3 and not on the header. There is also no USB/VBUS-detect net (VBUS feeds only the 5 V rail). This corrects the earlier "BQ/TI part, unconfirmed" guess. No `EspChargingMonitor` is possible on a stock Heltec V3 — every established pin map (ropg, the official arduino-esp32 variant, ESPHome, Meshtastic) reads only the GPIO1 voltage, and Meshtastic's voltage-only USB/charge inference (>4.2 V) is a documented source of wrong indicators. The charger IC sits on the back behind the reset switch and runs hot while charging a depleted cell (normal; thermally limited to ~100 °C). The only path to real detection is a hardware mod: tap CHRG (or a VBUS divider) to a free GPIO (GPIO2/4/5/6/7/19/20/47/48).
- **⚠️ On the Heltec V3.1, "no battery" is undetectable in software while USB is connected — it masquerades as a ~80 % battery.** With USB power and no cell, the always-on divider sees the charger/BMS rail, so GPIO1 reads ~711–735 mV → ×5.55 → ~3946–4079 mV → classified as `Battery` ~78–89 % (verified on hardware 2026-06-19). A real ~4 V cell reads identically; the only (heuristic, unreliable) tell is *instability* — the no-cell reading jumps ~130–180 mV between 2 s samples, while a real LiPo holds within a few mV. Bench artifact only: in field use (no USB) a missing cell just means the board is off. Mirrors the Feather V2 no-battery masquerade.
  VBAT sense path (measured on a spare **Heltec V3.1**): GPIO1 (ADC1_CH0) reads the divided battery voltage and the divider is **always-on** — toggling GPIO37/ADC_CTRL HIGH vs LOW had no effect (706 vs 707 mV), disproving the community "pull GPIO37 LOW to enable" model for V3.1 (it may still apply to V3.0/V3.2 — revisions differ). Read GPIO1 directly; no GPIO37 handling needed.
  The physical divider is ~390 kΩ/100 kΩ (textbook ≈4.9), but the ~80 kΩ source impedance makes the esp-idf oneshot ADC read ~11% low, so the *effective* `divider_ratio` is higher (~5.5). Calibrate it as metered_VBAT ÷ raw_pin_mV, not from the resistor values.
- **Module layout for new traits:** New trait modules (`sleep.rs`, `radio_gate.rs`, `charging.rs`) are unconditionally compiled (no feature gate).
  ESP-IDF implementations (`esp_sleep.rs`, `esp_radio.rs`, `esp_charging.rs`) are feature-gated behind `esp-idf`.
  This mirrors the existing `lib.rs` / `esp_adc.rs` split.
- **WakeSource::GpioLevel uses `pin_mask: u64` (decision record):** Mirrors the ESP-IDF `esp_sleep_enable_ext1_wakeup_io()` API directly; preserves `WakeSource: Copy` (consistent with `Timer { duration_ms: u64 }`).
  Range validation (pins 0–21 only, non-zero mask) is enforced in `EspSleepManager::sleep()` before the FFI call with a clear error message.
  `GpioWakeMask(u64)` on the read side provides a `contains_pin(u8) -> bool` helper so callers do not need raw bit manipulation.

## ADC Calibration: Original ESP32 vs ESP32-S3

- **`Calibration::None` (the `AdcChannelConfig` default) is unreliable on original ESP32 at 11 dB attenuation.**
  On ESP32-S3 the default is acceptable; on original ESP32 the ADC non-linearity is severe enough that
  uncalibrated readings can be off by hundreds of mV, producing `PowerSource::Unknown` for a healthy battery.
  Always use `Calibration::Line` for original ESP32 ADC at 11 dB.
- **DB_11 usable ceiling differs by chip: 2450 mV on original ESP32, 3100 mV on ESP32-S3.**
  The `DirectConverter` in esp-idf-hal applies the correct chip-specific ceiling at compile time, but the
  doc comment in `EspAdcBatteryMonitor::new()` that cites "0–3100 mV range" is only accurate for ESP32-S3.
  At 2450 mV ceiling with a 2:1 divider, the usable battery range is 0–4900 mV (still covers 4200 mV max).
- **Calibration must be selected per chip via `#[cfg(esp32)]` / `#[cfg(esp32s3)]` in `EspAdcBatteryMonitor::new()`.**
  Pattern:
  ```rust
  #[cfg(esp32)]
  let channel_config = AdcChannelConfig { attenuation: DB_11, calibration: Calibration::Line, ..Default::default() };
  #[cfg(not(esp32))]
  let channel_config = AdcChannelConfig { attenuation: DB_11, ..Default::default() };
  ```
  `build.rs` emits `cargo:rustc-cfg=esp32` and `cargo:rustc-cfg=esp32s3` so this `cfg` is already available.
- **Diagnostic: log raw ADC value before divider compensation.**
  Adding `log::info!("raw_mv={}", raw_mv)` in `read_averaged_mv()` immediately distinguishes between
  ADC not reading at all (0), ADC reading but uncalibrated (unexpectedly low), and correct readings.

## GPIO Configuration and Diagnosis

- **`InputEn: 0` in ESP-IDF GPIO log is a cosmetic artifact, not a misconfiguration.**
  (Historical — analysed on esp-idf-hal 0.45.2. Since 0.46 there is no `into_mode` chain and no intermediate `PinDriver` to drop: `PinDriver::input` goes straight to `new_gpio`, which in 0.47 (#585) calls `esp_rom_gpio_pad_select_gpio` and then `gpio_set_direction`. `Drop` still calls `gpio_reset_without_pull`. The `InputEn: 0` line may therefore no longer appear at construction; re-confirm on hardware before relying on the explanation below. Checked against the 0.47.0 sources on 2026-09-26.)
  When esp-idf-hal 0.45.2 constructs a `PinDriver` via `PinDriver::input()`, the call chain is:
  `input()` → `into_input()` → `into_mode(GPIO_MODE_INPUT)`.
  Inside `into_mode`, `drop(self)` is called first on the intermediate temporary `PinDriver`.
  That `Drop` impl calls `gpio_reset_without_pull()`, which calls `gpio_config()` with `mode = GPIO_MODE_DISABLE`.
  **This is the call that emits the `InputEn: 0` log line.**
  After drop returns, `gpio_set_direction(pin, GPIO_MODE_INPUT)` is called — this uses the `gpio_set_direction()` C API directly, which does NOT call `gpio_config()` and produces no log.
  The pin IS correctly configured as input; the `InputEn: 0` log reflects the transient reset, not the final state.
  This applies to every pin configured through `PinDriver::input()`, including input-only pins like GPIO34.
- **`set_pull()` uses `gpio_set_pull_mode()`, not `gpio_config()`, and does not reset the pin mode.**
  Calling `stat.set_pull(Pull::Floating)` after `PinDriver::input()` calls `gpio_set_pull_mode(pin, GPIO_FLOATING)`.
  This function sets pull registers independently of the direction register.
  It does not call `gpio_config()` and does not clear the `GPIO_MODE_INPUT` set by `gpio_set_direction()`.
  The direction is preserved.
- **GPIO34–39 are input-only on the original ESP32, have NO internal pull resistors, and reject `set_pull()` with `ESP_ERR_INVALID_ARG`.**
  With nothing external driving them they float and a digital read commonly latches LOW; an analog read shows a small leakage voltage (~150 mV on the Feather V2's unconnected GPIO34).
  (Historical note: a previous entry claimed GPIO34 carried a Feather V2 VBUS divider reading ~2.5 V with USB — that was the wrong pin map; the pin is unconnected. See the Hardware section.)
- **A digital input fed by a half-rail (~2.5 V) divider is a marginal HIGH on the ESP32, not a reliable one.**
  V_IH(min) = 0.75·VDD = 2.475 V at 3.3 V, rising to 2.55 V at the allowed 3.6 V rail; a 100k/100k divider off USB (4.75–5.1 V) sits at 2.375–2.55 V, straddling the threshold (worst corner: low USB + high 3.3 V rail reads a solid LOW).
  If you ever need a real "USB present" signal, read it via ADC and threshold in software rather than as a digital level.
- **GPIO34 strapping pin: consider `rtc_gpio_deinit` before first use.**
  GPIO34 is an RTC GPIO (RTC_GPIO4) that is sampled at boot for strapping.
  ESP-IDF's boot ROM may leave it in the RTC domain.
  `gpio_reset_without_pull` (called by esp-idf-hal's Drop) does NOT call `rtc_gpio_deinit()`.
  If the first read of GPIO34 after boot seems wrong, calling `rtc_gpio_deinit(34)` explicitly before
  `PinDriver::input(vbus_pin)` would release it from the RTC domain and ensure digital input is active.
- **"Charging: Unknown" from `EspChargingMonitor` on the Feather V2 was NOT a transient — it was unconnected pins.**
  The `(STAT_LOW, VBUS_LOW)` branch fired because GPIO13 and GPIO34 are not wired to STAT/VBUS on that board, so both float and read LOW.
  No settle delay or majority-vote read fixes this — the signals do not exist on those pins (confirmed by `idf_esp32_chargeprobe`; see Hardware).
  The fix was to remove charging detection from the Feather V2 example, not to debounce it.
- **Missing `log::warn!` in the charging monitor output is a log level issue, not a code path issue.**
  If "Charging: Unknown" appears in `println!` output but the `log::warn!` call is not visible in the
  serial log, the crate's log component is filtered below WARN level.
  Configure via `sdkconfig.defaults` (`CONFIG_LOG_DEFAULT_LEVEL=4` for INFO, `3` for WARN) or
  call `esp_log_level_set` for the component at runtime.

## esp-hal tier — capability findings (2026-09-26) and scaffold (2026-10-12)

- **The `rustyfarian-esp-hal-power` crate exists as a scaffold (2026-10-12):** `default-members` in the workspace `Cargo.toml` excludes it, so bare `cargo` and the `*-all` recipes never compile it, and the host gates only build `stoker`.
  Build it with the `*-hal` recipes (`check-hal`, `check-hal-c3`, `clippy-hal`), which write into `target/hal`.
- **Hal recipes run on the pinned `esp` toolchain with `-Zbuild-std=core,alloc`:** the flag overrides the config's `build-std = ["std", "panic_abort"]`, because std does not build for `*-none-elf` (it fails in gimli).
  The riscv32 none-elf targets come from `rust-src`, so no `rustup target add` is needed.
- **ADC and deep sleep are behind esp-hal's `unstable` feature, and the sleep API changed shape between releases:** esp-hal 1.1 had `Ext1WakeupSource`/`TimerWakeupSource`; 1.2.2 replaced them with `LowPower::sleep_deep() -> !` plus per-pin `Input::listen(Event::LowLevel)` arming and `set_wakeup_deadline(Instant)`. Expect churn every wave.
- **`stoker`'s sleep/wake contract is IDF-shaped and does not port directly:** esp-hal has no caller-supplied `pin_mask`, no single EXT1 status bitmask (only per-pin `caused_wakeup()`, or the raw register), panics instead of returning `Result` when no wake source is armed, and isolates pads automatically. A bare-metal tier needs a trait redesign, not just a new driver: `docs/adr/001-hal-neutral-sleep-contract.md` (2026-10-12).
- **esp-hal 1.2.2 pin borrowing at the sleep call compiles, but only with the pin lifetime decoupled from the manager (2026-10-12 spike):** `&mut Input<'d>` is invariant in `'d`, so a signature that reuses the `LowPower<'d>` lifetime for the pins fails with "lifetime may not live long enough" as soon as manager and pin come from different scopes. Use a separate `'i` (`&'a mut [&'p mut Input<'i>]`). `Input` has no public pin-number method; use `peripheral_input().gpio_number()`.
- **A rejected esp-hal deep sleep overwrites the wake record:** `sleep_deep_with_rejection` ends with `record_wakeup()` after sleep entry cleared the per-pin status (`gpio/wakeup/per_pin.rs` `prepare_gpio_wakeup`), so `caused_wakeup()` and the mask built from it must be captured at boot before the first `try_sleep`. Also release the pins after a rejection (`unlisten`, `clear_interrupt`, default `WakeupConfig`): `apply_wakeup_config` persists beyond the driver and a latched level with `rt` storms.
- **esp-hal 1.2.2 `Input::listen` enables the CPU interrupt at once, so a held wake level runs the GPIO handler before sleep entry:** the default `rt` handler only clears `int_ena` and the pin stays a wake source, but a custom handler that calls `unlisten()` removes it from the wake set (`set_int_enable` clears `listening`), and `sleep_deep` then panics with no source. Arm, sleep and clean up inside `critical_section::with` (ADR-001 § 4b).
- **esp-hal 1.2.2 drains every UART TX FIFO inside sleep entry, after the deadline checks:** `sleep_uart_prepare` runs after `sleep_deep`'s `deadline_missed` assert, and on the C3 (UART v1, 128-byte FIFO) it spins until the FIFO is empty and the transmitter idle, about 11 ms per full FIFO at 115200 baud. Drain with interrupts masked before arming a short deadline (ADR-001 § 4c).
- **esp-hal 1.2.2 writes the sleep deadline into a 48-bit RTC comparator and silently drops the higher bits** (`sleep/timer.rs` `arm`); cap timers in software (`MAX_TIMER_MS`, ADR-001 § 4a).
- **esp-hal arms every listening low-power pad for deep sleep, not just the ones a driver passes in** (`gpio/wakeup/mod.rs` `collect`); the low-power flag is a private static, so foreign pads can only be detected conservatively via `GPIO::regs().pin(n).int_type()` (ADR-001 § 4d).
- **`ESP_ERR_SLEEP_REJECT` is `ESP_ERR_INVALID_STATE` (`0x103`) in IDF v5.3.3:** map rejection by call site (the return of `esp_deep_sleep_try_to_start`), never by code.
- **esp-hal's `#[ram(unstable(rtc_fast, persistent))]` asserts `esp_hal::Persistable` on the static's type at compile time:** the orphan rule forbids implementing it for `stoker::RetainedCell`, so the hal tier wraps the cell in its own newtype and `unsafe impl Persistable` for that.
- **`stoker` is `#![no_std]` with no dependencies (since 2026-10-12):** validators return `SleepError`; `SleepManager::Error: stoker::sleep::Error`, and since the orphan rule forbids implementing that trait for `anyhow::Error`, implementors (including `EspSleepManager`, via `EspSleepError`) need a local error type; `?` into `anyhow` in firmware still works through `core::error::Error`. Still open: `build.rs` only matches `*-espidf` triples, so a `*-none-elf` ESP32 build would silently get the S3 wake-pin mask.
- **ADC and GPIO port easily:** calibrated mV on ESP32 (`AdcCalLine` only), S3/C3/C6 (`Line`/`Curve`; C6 also `Basic`); `Input` + `Pull` are stable.
- **Joining means exact-pinning `esp-hal` in lockstep with network/ws2812/peripherals** (all `=1.2.2` since 2026-09-26), or apps combining the hal tiers will not resolve.

## Known Gotchas

- Running `cargo build` without the `esp` toolchain active will fail with a linker error.
  Ensure `espup` has been run and the environment is sourced (`source ~/export-esp.sh`).
- The `esp-idf` feature is enabled by default, which triggers the ESP-IDF CMake build on the first compiler.
  This can take 10–20 minutes on a cold cache.
  Subsequent builds are fast due to the `.embuild` cache.
- **Host tests require an explicit target flag.**
  `.cargo/config.toml` forces `target = "xtensa-esp32s3-espidf"` for all builds, so `cargo test` without `--target` tries to link with `ldproxy` and fails.
  Always run host tests as: `cargo test -p stoker --target aarch64-apple-darwin` (adjust the triple for non-Apple hosts). The `just` host recipes do this for you.
- **`esp_sleep_config_gpio_isolate()` returns `void` in ESP-IDF v5.x**, not `esp_err_t`.
  Do not wrap it in an error check; call it directly and document the `SAFETY` comment.
  (The key-insights entry about calling this function before sleep is still correct — just no error check needed.)
