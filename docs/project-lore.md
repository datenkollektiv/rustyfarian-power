# Project Lore

This file records what bit us: hardware quirks, build gotchas and SDK behaviours that caused surprising failures.
One bullet per fact; a trailing scenario name in backticks means a `bench/` check proves the fix.
Refer to `AGENTS.md` and the `/project-lore` skill for recording guidelines.

---

## Build & Validation

- With no `deny.toml`, `cargo-deny` defaults to an empty licence allow-list and rejects every crate, even MIT and Apache-2.0; keep a `[licenses].allow` list in `deny.toml`.
- The `multiple-versions = "warn"` duplicate-crate warnings from the `embuild`/`esp-idf-sys` tree are expected and harmless.
- ESP-IDF v5.3.3's prebuilt `esp-clang` 16 needs `libxml2.so.2`, which Ubuntu 26.04 (`ubuntu-latest`) no longer ships, and `idf_tools.py` treats that self-check failure as fatal; v6.1's `esp-clang` (`esp-21.1.3`) has no libxml2 dependency, so the IDF jobs run on `ubuntu-latest`, and moving the pin back below v6 needs `runs-on: ubuntu-24.04` again.
- `RUSTUP_TOOLCHAIN: stable` must stay set in every workflow, because `rust-toolchain.toml` selects the `esp` channel, which CI does not install.
- The host gates (`just check`, `test`, `clippy`) run with `--no-default-features` and never compile the `esp-idf`-gated modules (`esp_adc`, `esp_charging`, `esp_sleep`, `esp_gate`); validate a driver change with `just check-all`, `just check-esp32` and `just check-c3`.
- The tier conformance modules (`src/conformance.rs`, ADR-003 point 4) compile only under the chip recipes, so a `stoker` trait change breaks there and not under `just verify`; `check-hal` compiles the sleep, wake-cause and reset checks only for C3 and S3.
- `esp-idf-sys` emits a bare cfg named after the MCU (`esp32c6`) and `embuild` relays it, but check-cfg knows only the names the crate's own `build.rs` registers, and clippy runs with `-D warnings`; the C6 and H2 share `riscv32imac-esp-espidf`, so `TARGET` cannot identify the chip either, and `build.rs` must emit `cargo:rustc-check-cfg=cfg(esp32c6)`.
- `cargo:rustc-cfg` from a build script applies only to that crate, so every crate that reads `cfg(esp32)` carries its own `build.rs`; `stoker` keeps one only for the legacy `validate_gpio_level_source`.
- `linker = "ldproxy"` is required in `.cargo/config.toml` for every `-espidf` target: `embuild::espidf::sysenv::output()` emits `--ldproxy-linker`/`--ldproxy-cwd` link args that `xtensa-esp-elf-gcc` rejects as unrecognized options (exit 101).
- `.cargo/config.toml` forces the ESP target, so a host `cargo test` without `--target <host triple>` tries to link with `ldproxy`; the `just` host recipes pass the target.
- `scripts/build-example.sh` passes only the features an example lists in `[[example]] required-features`, so gate an example that needs `critical-section-impl` there, not in the script.
- `cargo clippy -p <crate>` lints only the selected packages and silently drops lint coverage of dependencies, which is why `clippy-all` names `stoker` and the IDF crate.
- The hal recipes pass `-Zbuild-std=core,alloc` to override the config's `build-std = ["std", "panic_abort"]`, because std does not build for `*-none-elf` (it fails in gimli).
- The hal C6 build needs `-Tlinkall.x` in `.cargo/config.toml` for `riscv32imac-unknown-none-elf`, or the riscv-rt symbols are undefined at link time.
- `.cargo/config.toml` is tracked although this is a library workspace, because a fresh clone cannot link any firmware without its `ESP_IDF_VERSION`, `ldproxy`, `build-std` and `-Tlinkall.x` settings; Cargo never reads a dependency's config, so downstream users are unaffected.
- Changing `ESP_IDF_VERSION` in a target dir that already holds an IDF build fails in `esp-idf-sys`'s build script with `CMake Error: The source ".../v6.1/components/bootloader/subproject/CMakeLists.txt" does not match the source ".../v5.3.3/..."`, because the bootloader sub-build keeps its old `CMakeCache.txt`; delete `<target-dir>/<triple>/*/build/esp-idf-sys-*` for the affected triples.
- When the RAM disk `/Volumes/RustBuilds` is attached, `just build-example` writes the release ELF under `/Volumes/RustBuilds/targets/idf/<repo>`; that 12 GB disk is shared by every sibling repo and fills up, `just clean-hal` is the cheapest reclaim, and a cold C6 build without it takes about 20 minutes.

---

## Hardware — Heltec V3 & LiPo batteries

- MakerFocus LiPo packs are wired with the opposite polarity to Heltec on the same JST-1.25 plug, and the Heltec battery input has no reverse-polarity protection; meter the pack's `+` and the board's `+` pad (by continuity to GND, since the pad reads 0 V with no battery) before connecting, and re-pin the JST if needed.
- A LiPo protection board latches its output to 0 V after a reverse-polarity or short fault while the bare cell still reads about 3.9 V; the board then shows `No battery`, so test the sense path with a known-good current-limited source before blaming the board.
- Battery plus USB together is the intended charging mode on both boards (Feather V2 power-path diode plus MCP73831, Heltec BMS); do not feed USB and the Heltec's 5 V pin at the same time.
  Sources: [Adafruit power management](https://learn.adafruit.com/adafruit-esp32-feather-v2/power-management-2), [MCP73831 reverse-current MOSFET](https://forums.adafruit.com/viewtopic.php?t=222339), [Heltec V3 docs](https://wiki.heltec.org/docs/devices/open-source-hardware/esp32-series/lora-32/wifi-lora-32-v3/), [ropg/heltec_esp32_lora_v3](https://github.com/ropg/heltec_esp32_lora_v3/blob/main/README.md).
- On the Heltec V3.1, GPIO37 (ADC_CTRL) does not gate the battery divider: toggling it leaves GPIO1 unchanged, so read GPIO1 directly; other revisions differ (V3/V3.2 reportedly enable the divider with LOW/HIGH), so confirm on the board in hand.
- The V3.1 divider ratio is 5.05 metered, not the textbook 4.9; the earlier 5.55 folded in the uncalibrated transfer curve of `Calibration::None` on the IDF oneshot path, not impedance loading, and both tiers read the same net within a few mV once `Calibration::Curve` is used (`heltec/battery-calib`).
- Community constants of 5.2 to 5.42 (ropg, MeshCore) scale raw counts against an assumed 3.3 V full scale and absorb the S3 ADC gain error, so they are not comparable to a calibrated ratio; Meshtastic uses 4.9 × 1.045.
- Take the calibration pair at the same moment with the pack connected: a meter read before plugging the pack in is its open-circuit voltage and gives a ratio about 2 % low.
- No battery on USB cannot be detected on the Heltec: the always-on divider sees the charger rail and reads either a false, unstable ~80 % or `No battery`, varying by board and run, so re-verify with the raw mV instead of assuming either.
- The TP4054 charger's CHRG pin drives only the orange LED and there is no VBUS-detect net, so no presence or charge signal exists in software on a stock board; hand-wire CHRG or a VBUS divider to a free GPIO (GPIO2/4/5/6/7/19/20/47/48) for real detection.
- IDF GPIO isolation (`esp_sleep_config_gpio_isolate`) releases output pads, so `Vext_Ctrl` (GPIO36, active-low P-MOSFET) floats in deep sleep and the Vext rail comes back on; a released pad turns the rail on, so no pull-up can be relied on, and `heltec_v3_vext` excludes the pad with `gpio_sleep_sel_dis` after configuring it, no pad hold needed (`heltec/rail-gate-ve`).
- `esp_sleep_config_gpio_isolate()` does not re-enable sleep-select on a pad, but `esp_sleep_enable_gpio_switch(true)` would.

---

## Hardware — Adafruit ESP32 Feather V2

- The stock Feather V2 exposes no charge-status (STAT) or VBUS-detect signal on any GPIO: the GPIO13/GPIO34 map came from the Unexpected Maker FeatherS2 Neo, STAT drives only the CHG LED (schematic rev F: `VBUS → LED → 5.1 kΩ → STAT`, no ESP32 trace), GPIO13 is the user LED, and GPIO35 (2 × 200 kΩ) is the only power-sense pin, so `EspChargingMonitor` on those pins reports a permanent `Charging: Unknown` that no debounce fixes.
- Verify a suspected pin map as analog millivolts (`idf_esp32_chargeprobe`): a pull-up or divider forces a specific voltage now, while a floating input-only pin reads about 150 mV of leakage.
- MCP73831 STAT is tri-state (LOW charging, HIGH complete, Hi-Z shutdown or no battery), not open-drain like the MCP73832; with a pull-up a digital read collapses complete and no-battery into HIGH, and an ADC read separates the three.
- With USB connected and no battery, the MCP73831 holds the BAT net at its ~4.16 V CV point, so GPIO35 reads a false ~96 % that is identical to a real full cell; battery-only power reads `No battery` correctly, so document it rather than tuning thresholds.
- GPIO34-39 are input-only on the ESP32, have no internal pulls, and `set_pull` returns `ESP_ERR_INVALID_ARG`; unconnected they float, latch LOW on a digital read and show about 150 mV on an analog read.
- A half-rail (~2.5 V) divider is a marginal digital HIGH on the ESP32 (V_IH min is 0.75 × VDD), so read such a signal through the ADC and threshold in software.

---

## ESP-IDF / Rust on ESP32-S3

- Rust `println!` output never reaches the Heltec V3 console while ESP-IDF's `I (..)` logs do: the primary console is UART0 with `CONFIG_ESP_CONSOLE_SECONDARY_USB_SERIAL_JTAG=y`, Rust stdout is swallowed and `flush()` does not help, and the ESP32 masks the problem because it has no second console; route output through `esp_idf_hal::sys::esp_rom_printf`.
- Nothing installs a `log` logger, so the library's `log::warn!` diagnostics are invisible unless the example forwards `log` to `esp_rom_printf`.

---

## ESP-IDF tier — ADC calibration

- `Calibration::None` (the `AdcChannelConfig` default) is off by hundreds of mV on the original ESP32 at 11 dB and turns a healthy battery into `PowerSource::Unknown`; `EspAdcBatteryMonitor::new` selects `Line` on ESP32/S2/C2 and `Curve` on S3/C3/C6, and naming the wrong variant is a compile error because esp-idf-hal defines each only for its chips (`esp32/battery`).

---

## ESP-IDF tier — ESP32-C3 deep sleep (IDF v6.1)

- `esp_deep_sleep_try_to_start` does not reject a GPIO wake pin already at its wake level on the C3, although `RTC_GPIO_TRIG_EN` is in the reject mask: the chip sleeps and wakes about 20 ms later with `cause=Gpio`, so a stuck sensor boot-loops at about 2.5 Hz; `try_sleep` on C2/C3 reads the wake pins right before entry and returns `Rejected` itself (`c3/pir-wake`).
- That pre-read needs the pin input-enabled (`FUN_IE`, which arming does not set) and an external bias, and a pin that crosses its level after the read still wakes the chip at once.
- The C2/C3 GPIO-wake path is selected by `cfg(all(esp_idf_soc_gpio_support_hp_periph_pd_sleep_wakeup, not(esp_idf_soc_pm_support_ext1_wakeup)))`; the C6 has both SOC caps and stays on EXT1.
- `esp_sleep_disable_wakeup_source(ALL)` clears only `RTC_GPIO_TRIG_EN`; the per-pin hardware enable and the SDK's `gpio_wakeup_mask` and `gpio_trigger_mode` survive, so a pin armed earlier in the boot would still get a hold and internal pull at later GPIO-wake entries.
- `esp_sleep_disable_wakeup_source(ESP_SLEEP_WAKEUP_GPIO)` clears that SDK state but is a no-op returning `ESP_ERR_INVALID_STATE` once `RTC_GPIO_TRIG_EN` is clear (`CHECK_SOURCE`), so after the disable-all the manager sets the bit again with `esp_sleep_enable_gpio_wakeup()` and then disables the GPIO source.
- The manager disarms with `gpio_wakeup_disable_on_hp_periph_powerdown_sleep` only pins whose `RTC_CNTL_GPIO_WAKEUP_REG` enable bit is set, because the call also re-enables sleep-select and would override an application's `gpio_sleep_sel_dis()` on unrelated pins.
- `esp_sleep_gpio_wakeup_prepare_on_hp_periph_powerdown` applies `gpio_hold_en` and the internal pull against the wake level (`CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS=y`) but leaves the pads digitally routed, so a reject release on C2/C3 is `gpio_wakeup_disable_on_hp_periph_powerdown_sleep` plus `gpio_hold_dis` per register-armed pin.
- The C3 rejection postcondition (wake pin readable after a rejection, timer recount after a retry, UART drained inside the hal critical section) is benched only by hand with `C3_REJECT_HOLD_MS`; `s3/r2` proves the S3 twin.

---

## ESP-IDF tier — ESP32 EXT1 wake status (IDF v6.1)

- ESP-IDF's boot code runs `esp_deep_sleep_wakeup_io_reset()` before `app_main`, which clears `RTC_CNTL_EXT_WAKEUP1_SEL`, so an `AnyLow` (`ALL_LOW`) wake reads `Ext1(0x0)` while `AnyHigh` keeps its bit; the SEL gating is inferred, not confirmed at register level.
- Fix: `EspSleepManager` records the armed EXT1 mask and level in `.rtc_noinit` and `EspWakeCauseSource` resolves an empty status through `stoker::ext1_fired_mask`, exact because the ESP32 refuses multi-pin `AnyLow` (`esp32/anylow`).
- Do not force `ESP_PD_DOMAIN_RTC_SLOW_MEM` off, or that `.rtc_noinit` record is lost.

---

## ESP-IDF tier — ESP32-S3 deep sleep (IDF v6.1)

- The S3 hardware rejects a held EXT1 wake pin on both tiers (`E sleep: Deep sleep request is rejected`, hundreds of `Rejected` in a row and no boot loop), unlike the C3, so the pre-sleep pin read is only needed while the global pad hold is effective (`s3/r2`).
- On the CrowPanel 1.28" the CST816S touch INT (GPIO5) idles LOW until `LCD_3V3` (GPIO1) is on and touch RST (GPIO13) is released, so a sleep example that configures only GPIO5 never sleeps; drive both HIGH (rail on, 20 ms, RST low 10 ms, high, 50 ms) and hold them through sleep with IDF `gpio_hold_en` plus `gpio_deep_sleep_hold_en` or esp-hal `Output::set_pad_hold` (`s3/touch-wake`).
- A finger resting on the CrowPanel does not hold INT LOW; INT pulses, so a touch wake loops every 0.3 to 0.7 s instead of being rejected, and on the IDF tier a finger already resting at sleep entry may produce no new event, which is why `bench/s3/touch-wake` asks for a moving finger.
- The tested fix for the resting-finger wake loop is to sleep only after touch release (CST816 finger count `0x02` reads 0 for 300 ms, rustyfarian-knob); writing `IrqCtl` (`0xFA`) from its default `0x60` to `0x20` stops the `EnTouch` pulses but adds a wake on lift and drops the touch and gesture pulses needed while awake.
- A rejected `esp_deep_sleep_try_to_start` leaves the wake pins RTC-held and RTC-routed on EXT1 chips (ESP32/S3/C6): `ext1_wakeup_prepare` runs before the reject decision and sets MUX_SEL, the reject tail restores only CPU frequency and UARTs, and esp-idf-hal's `PinDriver::input` never calls `rtc_gpio_deinit`, so the routing breaks every digital read of the pin; `try_sleep` calls `rtc_gpio_hold_dis` then `rtc_gpio_deinit` per masked pin on the reject return, best-effort, since `rtc_gpio_deinit` alone does not release the hold (`s3/r2`).
- After a real EXT1 wake with `RTC_PERIPH` kept on, a `PinDriver` likewise cannot read the pin until `rtc_gpio_deinit` runs.
- `esp_sleep_disable_wakeup_source(ESP_SLEEP_WAKEUP_ALL)` does not clear `s_config.ext1_rtc_gpio_mask` and the enable call OR-merges, so a retry with a changed EXT1 pin or level fails with `ESP_ERR_NOT_ALLOWED` although the new request alone is valid; `configure()` calls `esp_sleep_disable_ext1_wakeup_io(0)` after the disable-all (`s3/r2`).
- A rejected deep sleep hangs the S3 on both tiers while the global digital-pad hold (`RTC_CNTL_DG_PAD_AUTOHOLD_EN`, set by `gpio_deep_sleep_hold_en`) is on: sleep entry isolates every unheld digital pad including the flash pads and a rejection never restores them, so IDF never returns, prints `0xFF` garbage and resets with `rst:0x8 TG1WDT_SYS_RST`, while esp-hal stalls silently with the USB port still enumerated.
- That bit lives in RTC_CNTL, survives flashing and USB resets, and esp-hal never clears it, so an esp-hal firmware flashed after an IDF one inherits it.
- Fix: while the hold is effective, `try_sleep` on both tiers reads the wake pins right before entry and returns `Rejected` from software, matching the hardware ANY/ALL condition (`stoker::digital_hold_effective`, `wake_pins_asserted`, `wake_pins_all_low`) (`s3/r3`, `s3/r4-guard`).
- Residual: a pin that crosses its wake level after the read, or a non-pin hardware reject, still hangs with the global hold on; per-pin holds (`gpio_hold_en`, `set_pad_hold`) are not the trigger, so prefer them, and a downstream firmware that enables the global hold and retries rejected sleeps resets instead of retrying.
- The CrowPanel's USB-C reaches the S3's native USB-Serial-JTAG (`/dev/cu.usbmodem*`), not a CH340: `esp-println` built with `uart` prints nowhere visible (use `auto`), and once the app spins in a tight rejection loop `espflash flash` cannot connect, so enter download mode by hand (hold BOOT, tap RST).

---

## ESP-IDF tier — v5.3.3 to v6.1 API drift

- ESP-IDF v6.1 replaces the SOC cap `SOC_GPIO_SUPPORT_DEEPSLEEP_WAKEUP` with `SOC_GPIO_SUPPORT_HP_PERIPH_PD_SLEEP_WAKEUP` (the old name survives only as a C macro alias), so esp-idf-sys stops emitting `esp_idf_soc_gpio_support_deepsleep_wakeup` and emits `esp_idf_soc_gpio_support_hp_periph_pd_sleep_wakeup` instead.
- The new cap is also set on the ESP32 and ESP32-S3, which never had the old one, so it no longer means "deep-sleep GPIO wake chip"; `all(cap, not(esp_idf_soc_pm_support_ext1_wakeup))` still selects only C2/C3.
- `sys::FUN_IE_S` is absent from the v6.1 bindings on every chip, because `io_mux_reg.h` moved from `soc/<chip>/include/soc/` to `soc/<chip>/register/soc/`; the value is 9 on ESP32, S3, C2, C3 and C6 in both versions.
- In v6.1, `esp_sleep_get_gpio_wakeup_status()` returns a mask indexed by RTC IO number, not GPIO number, and `esp_sleep_wakeup_io_bit2num()` converts a bit to the GPIO.
- v6.1 removed the C2/C3 calls `esp_deep_sleep_enable_gpio_wakeup` and `gpio_deep_sleep_wakeup_disable` and the type `esp_deepsleep_gpio_wake_up_mode_t`; the replacements are `esp_sleep_enable_gpio_wakeup_on_hp_periph_powerdown`, `gpio_wakeup_disable_on_hp_periph_powerdown_sleep` and `esp_sleep_gpio_wake_up_mode_t`.
- The C3, ESP32 EXT1 and S3 deep-sleep bullets above were bench-proven on v5.3.3 and source-checked against v6.1 (esp-idf-hal 0.47, esp-idf-sys 0.38.1); re-run the bench before trusting one that a v6.1 symptom contradicts.

---

## ESP-IDF tier — RTC retention and clock

- `.rtc_noinit` is one `NOLOAD` section in RTC fast memory on the C3 (`0x5000_0000..0x5000_2000`, the record at `0x50000018`) and in RTC slow memory on ESP32/S3; the hal twin's `.rtc_fast.persistent` lands at `0x50000000`.
- Verify placement with `riscv32-esp-elf-nm -n -S <example ELF> | grep RETAINED` from `~/.espressif/tools/riscv32-esp-elf/<ver>/riscv32-esp-elf/bin`, because `rust-nm` is missing under the pinned `esp` toolchain (no llvm-tools).
- The C3 record survives timer deep sleep, `esp_restart`, a panic and the interrupt watchdog on both tiers and is refused by reset reason after a power-on or USB replug; the hal panic reads `reset=Software` because the handler restarts through `software_reset()`.
  That matrix needs `C3_RET_ACTION=cycle`, which the bench runner cannot drive, so only the deep-sleep leg has a scenario (`c3/retention`).
- `EspRtcClock` keeps counting through deep sleep, `esp_restart`, a panic and the interrupt watchdog and restarts only on power-on, whereas `esp_timer_get_time()` restarts at every boot; `esp_rtc_get_time_us` is absent from the esp-idf-sys 0.38 bindings, so the module declares the extern itself.
- The generated sdkconfig has `CONFIG_ESP_SYSTEM_PANIC_PRINT_REBOOT=y` (a Rust panic reboots as `ESP_RST_PANIC`) but no `CONFIG_ESP_TASK_WDT_PANIC`, so a starved task only warns and a self-inflicted watchdog reset must trip the 300 ms interrupt watchdog (`esp_idf_hal::interrupt::free` around a spin loop).
- `esp_hal::init` disables every watchdog, so the hal `wdt` bench action arms TIMG0 itself (`TG0WDT_SYS_RST`, `CoreMwdt0`).

---

## Bench tooling — USB-Serial-JTAG boards

- `espflash monitor` cannot follow a USB-Serial-JTAG board across deep sleep: the port vanishes in sleep and re-enumerates on wake, every reattach syncs with the bootloader, and `--before no-reset` and `--before no-reset-no-sync` both fail against a running app; read the port passively (pyserial) and reopen it after each re-enumeration.
- A reader that lowers DTR before RTS passes through RTS=1 with DTR=0, the USB-Serial-JTAG chip-reset condition (`rst:0x15 (USB_UART_CHIP_RESET)`), and the reattach turns that into a reset loop every ~0.8 s that always reports `PowerOn`; leave DTR and RTS at their asserted open-time defaults.
- Bench firmware on such a board must print its machine-read lines in the middle of the awake window in every build: the host needs about 2.5 s to open the re-enumerated port, so a boot-time line goes to nobody, and a line written right before sleep entry is not drained (esp-hal loses it, ESP-IDF delivers it with about 50 ms margin); print at 3 s and again at 5 s of a 6 s window and keep 1 s quiet before the first attempt (`AWAKE_MS`, `DRAIN_MS`), and tell a wake apart by its cycle number.
- Never gate a line a scenario matches behind a bench-case switch; a boot line printed only in R2 builds left the default build silent after `Wake cause` (`s3/sleep`).
- Before suspecting the serial driver, read the firmware's actual output with `just serial-capture` and check that the expected line exists in that build at all.
- A bench `confirm` step must show the value it asks about, captured in the same mode: captures do not cross the `--attended`/`--unattended` split, so give the attended step its own `expect` with a named group (`attended = true`, `arm = false`) and put the capture (`{mv}`) in the `confirm` text (`esp32/battery`).

---

## esp-hal tier (esp-hal 1.2.2)

- A sleep manager that borrows `&mut Input` wake pins must give the pins their own lifetime: `&mut Input<'d>` is invariant in `'d`, so reusing the manager's `LowPower<'d>` fails with "lifetime may not live long enough" once manager and pin come from different scopes; use `pins: &'a mut [&'p mut Input<'i>]` (ADR-001 § 2).
- `Input` has no public pin-number method; use `peripheral_input().gpio_number()`.
- A rejected `sleep_deep_with_rejection` overwrites the boot's wake record (sleep entry clears the per-pin status, then `record_wakeup()` runs again), and the IDF tier shows the same on the S3, so capture the wake cause and pin mask at boot before the first `try_sleep` (`s3/r2`).
- Release the pins after a rejection (`unlisten`, `clear_interrupt`, default `WakeupConfig`): `apply_wakeup_config` persists beyond the driver and a latched level storms the `rt` handler.
- `Input::listen` enables the CPU interrupt at once, so a pin already at its wake level runs the GPIO handler before sleep entry; a custom handler that calls `unlisten()` clears the pin's `listening` record and `int_type`, sleep entry drops the source, and `sleep_deep` panics with "no wakeup source", while the default `rt` handler only clears `int_ena` and is safe; arm, sleep and clean up inside `critical_section::with` (ADR-001 § 4b).
- esp-hal drains every UART TX FIFO inside sleep entry after both deadline checks (`sleep_uart_prepare`, about 11 ms per full 128-byte FIFO at 115200 baud on the C3), so a short timer can expire before the chip sleeps and a missed deadline inside `sleep_deep_with_rejection` is a sleep that never ends; drain every UART inside the critical section before arming the deadline (ADR-001 § 4c).
- The sleep deadline goes into a 48-bit RTC comparator that silently drops the higher bits, so an oversized timer programs a different deadline; cap requests in software (`MAX_TIMER_MS`, ADR-001 § 4a).
- `ext1` takes one level for its whole mask; mixed levels keep the LP peripherals powered.
- `Output::new`, `Input::new` and `Flex::new` release a pad hold before driving the level (`init_gpio` calls `set_pad_hold(false)` first), so a hold from the previous run does not survive re-creating the driver and the pad is briefly undriven on a wake boot; call `Output::set_pad_hold(true)` again after re-creating the drivers (`s3/touch-wake`).
- A rejected S3 sleep leaves `RTC_CNTL_WIFI_FORCE_ISO` (bit 28 of `RTC_CNTL_DIG_ISO_REG`) cleared: `base_settings` sets it, `RtcSleepConfig::apply()` clears it at entry and nothing restores it; it is power-domain configuration, not pad state, the awake cost is unmeasured, and the adapter does not restore it.
- `AdcChannel` is implemented for every analog pad and carries only the channel index, so an ADC2 pad passed to `AdcConfig<ADC1>` silently reads the ADC1 channel with the same number (ESP32 GPIO4 reads GPIO36, S3 GPIO11 reads GPIO1) and yields a plausible wrong voltage; `EspHalAdcBatteryMonitor` bounds `PIN` on the sealed `Adc1Pin` trait, whose hand-written pad lists must be re-checked on every esp-hal bump.
- UART v2 on the C6 rests at TX FSM state `0x0F` (TX_WAIT_SEND) as well as `0x00`, so a C3/S3-style drain that waits for state 0 times out and every `sleep`/`try_sleep` returns `UartBusy`; the C6 drain treats both states as idle.
- `.rtc_fast.persistent` does not survive esp-hal deep sleep on the ESP32: `RtcSleepConfig::deep()` sets `rtc_fastmem_pd_en`, which the ESP32 `apply()` turns into cleared `fastmem_force_pu`/`force_lpu`/`force_noiso`, so `RtcRetainedCell` loads `Invalid::Magic` and `EspHalRtcClock` restarts at 0 after every wake; `deep_sleep_config()` builds the config with `set_rtc_fastmem_pd_en(false)` on the ESP32, and the C3 needs no such workaround (`esp32/retention`, `c3/retention`).
- On the ESP32 only `.rtc_fast.persistent` survives esp-hal deep sleep; `RtcSleepConfig::deep()` powers down the RTC slow memories, so a `.rtc_slow` static is lost every sleep.
- `caused_wakeup()` is not a reliable read of a low-level EXT1 wake on the ESP32 (the bit is present on some boots and `0x0` on others), so `armed_ext1` keeps the armed mask and level in `.rtc_fast.persistent` and `EspHalWakeCauseSource` resolves an empty status through `stoker::ext1_fired_mask`, as the IDF tier does (`esp32/anylow`).
- `Rtc::time_since_power_up()` drifts after an hour of uptime (−8.5 % to +6.1 % for 60 s sleeps) because each boot re-converts the whole tick count; use `EspHalRtcClock::now_us()` for slept time.
- The first sleep cycle after a power-on reads about 190 ms short on the hal tier only, systematically and unexplained; otherwise the hal and IDF clocks agree within about 0.1 % once the estimated boot offsets (150 ms hal, 419 ms IDF) are accounted for.
- No S3 twin of the hal retention example exists; the hal S3 `RtcRetainedCell` rests on a downstream (rustyfarian-knob) run only.

---

## stoker — RTC retention cell

- A `Sync` cell with safe `&self` methods doing unsynchronised volatile copies is a data race from safe code, and no documented single-context rule makes it sound; every `load`, `store` and `invalidate` copies inside `critical_section::with`, which is also the only portable lock on riscv32imc (no compare-and-swap).
- A missing `critical-section` implementation fails only at link time, never under `cargo check`: undefined `_critical_section_1_0_acquire` means no provider, a duplicate means two; esp-hal supplies it through `rt`, the host through the crate's `std` dev-feature, and the IDF tier through its opt-in `critical-section-impl`, so `just build-example` is the gate for retention code.
- A library that enables a `critical-section` provider by default breaks every firmware whose graph already has one (rustyfarian-esp-idf-network's `lora` feature enables esp-idf-hal's provider, and `cargo check` still passes); `critical-section-impl` stays opt-in and only the final firmware picks the provider.
- esp-idf-hal's `critical-section` provider is a task-level recursive mutex, which FreeRTOS forbids in an ISR and which an interrupt preempting the holder re-enters as the same task, so it cannot serialise a task against an ISR; the IDF tier's provider uses the FreeRTOS port critical section (`vPortEnterCritical`, `xPortEnterCriticalTimeout` on a `portMUX`), which masks interrupts and spins out the other core.

---

## Bench tooling — ESP32-C3 SuperMini

- espflash's USB-Serial-JTAG reset parks the SuperMini in download mode (`rst:0x15 ... boot:0x5 (DOWNLOAD)`) and the USB device never drops, so the app never starts after `just flash` or `espflash reset`; open the USB-Serial-JTAG port only to flash, read the console on UART0 (GPIO21) with an adapter whose DTR/RTS are unwired, and power-cycle the board to start the app.
- A deep-sleep GPIO wake that "never fires" can be a sensor that never drives the pin: with a pull-up probe an open pin reads High, a pin wired to a push-pull sensor reads Low at rest and High on motion, and a pin stuck Low has a dead or unpowered source; probe the pin while awake (`C3_AWAKE_PROBE_MS`, `C3_PROBE_PULL=up`) before suspecting the sleep path, and `C3_PIR_PIN` moves the wake pin if the signal lands on a neighbouring header pin.
- C3 chip revision v0.3 versus v0.4 carries no sleep erratum; IDF v6.1's only C3 sleep gate is below v0.3.

---

## Bench tooling — C3 bench driver

- A retained counter also survives a software, panic or watchdog reset, so R1 requires `reset=DeepSleep` and `cause=Timer` on every judged boot besides the increment, and the `R1 warm reset` self-test injects the software-reset sequence and expects one FAIL.
- Opening the CP210x port of the DevKitM-1 resets the chip through EN even with DTR and RTS driven low, and both tiers report it as a power-on reset, so every `just serial-capture` starts with `reset=PowerOn` and a fresh retention session; read that boot as the port open, not as evidence.
- A hot-unplugged CP2102 wedges the driver; replug the adapter.

---

## Bench tooling — ESP32 bench hal builds

- The `hal_esp32_*` bench apps boot through a stale `ESP-IDF v6.1-beta1` second-stage bootloader left by an earlier IDF app, because `espflash` never erases the bootloader sector; the IDF boot banners say nothing about the tier, so identify the firmware by its own `Reset reason:` and `kind=` lines.
