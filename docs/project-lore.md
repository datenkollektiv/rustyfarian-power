# Project Lore

This file records non-obvious technical discoveries: facts that caused surprising
failures, took significant time to debug, or would save a future developer 30+
minutes if known upfront.

Refer to `AGENTS.md` and the `/project-lore` skill for recording guidelines.

---

## Build & Validation

**`just deny` fails on licenses because the repo has no `deny.toml` at all.**
With no config file present, `cargo-deny` 0.19 defaults to an empty license allow-list and rejects *every* crate — including MIT, Apache-2.0, and Unlicense — so the error "license is not explicitly allowed" shows up even for the most permissive licenses, which looks baffling.
This blocks `just deny`, `just ci`, and `just verify` regardless of the dependency tree, so it is unrelated to whatever change you are validating.
Fix: add a `deny.toml` with a `[licenses]` `allow` list (MIT, Apache-2.0, Unlicense, BSD-2-Clause, BSD-3-Clause, …) before relying on `just deny`.

**The GitHub `ubuntu-latest` runner moved to Ubuntu 26.04, where the ESP-IDF tool install in CI fails on `esp-clang`.**
Ubuntu 26.04 ships libxml2 2.14 with soname `libxml2.so.16`, but ESP-IDF v5.3.3's prebuilt `esp-clang` 16 is linked against `libxml2.so.2`; `idf_tools.py` self-checks every installed tool and treats the exit-127 loader error ("Failed to check the tool while installed") as fatal, so every `esp-idf-sys` build dies — both `check-c3` (riscv32 install) and `check-s3` (`build-example`, xtensa install) fail this way.
`apt-get install libxml2` does NOT fix it on 26.04: the distro package provides the new soname only.
The image changed between 2026-10-30 (last green run) and 2026-11-02 (first failure), and the failing PR changed only doc comments, so a locally green `just check-c3` plus doc-only diffs is the signature of this failure, not a code regression.
Fix: pin `runs-on: ubuntu-24.04` on every CI job that builds ESP-IDF code; this is the family-canonical fix (rustyfarian-network, commit 06c5bd2, breakage 2026-10-30; this repo 2026-11-02).
Repos whose CI builds only esp-hal/AVR targets are unaffected (rustyfarian-ws2812 stayed green on `ubuntu-latest`).

**Host-side gates (`just check` / `test` / `clippy`) never compile the ESP-IDF driver code.**
These recipes run with `--no-default-features`, which drops the default `esp-idf` feature, so `esp_adc.rs`, `esp_charging.rs`, and `esp_sleep.rs` are excluded from the build — a green host run says nothing about whether the hardware drivers still compile.
Fix: validate any change to a feature-gated `esp_*.rs` module with `just check-all` (ESP32-S3) and `just check-esp32` (original ESP32); the host gates only cover the platform-independent modules.

**`cfg!(esp32c6)` fails the IDF tier with "unexpected `cfg` condition name" although `esp-idf-sys` emits that cfg.**
`esp-idf-sys` 0.38.1 emits a bare cfg named after the target MCU, and `embuild::espidf::sysenv::output()` relays it, but check-cfg only knows the names our `build.rs` registers, and clippy runs with `-D warnings`.
The C6 triple `riscv32imac-esp-espidf` is shared with the H2, so parsing `TARGET` cannot identify the chip either.
Fix: add `println!("cargo:rustc-check-cfg=cfg(esp32c6)")` to the tier's `build.rs` and rely on the relayed cfg (2026-11-04).

---

## Hardware — Heltec V3 & LiPo batteries

**MakerFocus LiPo packs use the OPPOSITE polarity to Heltec on the same JST-1.25 connector.**
The plug fits physically, but `+`/`−` are swapped (MakerFocus is wired for RAK hardware), so plugging a MakerFocus pack straight into a Heltec V3 reverse-connects the battery.
The Heltec battery input has no reverse-polarity protection — this can damage the charger/regulator and, short of that, leaves the battery-sense ADC reading a flat 0 mV.
Fix: re-pin the JST (swap the two crimp pins) or hand-wire crossed, and ALWAYS confirm battery `+` terminal → board `+` pad with a multimeter before power-on — never trust cable colours. Find the board's `+`/`−` by continuity to GND when the pad reads 0 V (charger idle with no battery is normal).

**A LiPo protection board latches its output to 0 V after a reverse-polarity/short fault.**
Symptom: the bare cell still measures ~3.9 V but the pack's plug reads 0.0 V — the cell is usually fine, the PCM FETs are just open.
This presents to the board as "No battery" / 0 mV. Don't mistake a tripped pack for a dead board or a damaged sense path; test the board's sense path with a separate known-good source (current-limited bench PSU at ~3.7 V, polarity metered).

**On the Heltec V3.1, GPIO37 (ADC_CTRL) does NOT gate the battery divider — it is always-on.**
Toggling GPIO37 HIGH vs LOW left the GPIO1 reading unchanged (706 vs 707 mV), so the community/ropg "drive GPIO37 LOW to enable" procedure did not apply to this **V3.1** unit (it may still apply to V3.0/V3.2 — revisions differ; always confirm against the board in hand).
Read GPIO1 directly — no ADC_CTRL handling needed, which matches the repo's existing approach; only the ratio was wrong.
Calibrate the ratio against a meter, never compute it from the resistor values alone.
Corrected 2026-11-09: the old "the ~80 kΩ source impedance loads the ESP-IDF oneshot ADC, so it reads ~11 % low and the ratio is ~5.5" explanation is superseded — the esp-hal tier reads the same always-on divider accurately (810 mV raw ≈ 3970/4.9), so impedance is not the cause. The 5.55 ratio had folded in the uncalibrated transfer curve of the ESP-IDF oneshot path (`Calibration::None` on S3); with `Calibration::Curve` both tiers read the same net within 7 mV (hal 810 / IDF 817) and the physical ratio applies: `divider_ratio` = 5.05 (4140 mV metered at the pack ÷ 820 raw, this V3.1 unit, pack connected and charging).
Take the calibration pair **at the same moment and with the pack connected**: read the meter at the pack terminals while the `Calib:` lines stream.
A meter read before plugging the pack in (the polarity check) is its open-circuit voltage; paired with a later ADC read of the charging net it gave a 2 % low ratio (4.98 instead of 5.05) on 2026-11-09.
Charging does not skew the ratio itself; only a non-simultaneous pair does.

**No battery fitted still cannot be detected on the Heltec V3.1 — and its no-battery reading is NOT stable across boards or runs: false ~80 % or `No battery`, both on USB power.**
Observed 2026-06-19: USB power and no cell → the always-on divider sees the charger/BMS rail, GPIO1 reads ~711–735 mV (×5.55 → ~3946–4079 mV) → `Battery ~80 %`, unstable (~130–180 mV jumps).
Observed 2026-11-09 (bench run, both tiers): same setup → `Battery: No battery` every 2 s, stable — the rail sat below the 3000 mV classification floor (raw < ~540 mV, before the 2026-11-09 `Calibration::Curve` switch; note the June false-~80 % observation itself was later identified as the uncalibrated IDF curve, see the ratio entry above).
The 2026-06-19 and 2026-11-09 rails agree that USB alone is ambiguous, but the rail value differs, so re-verify with the raw mV when a no-battery reading surprises you rather than assuming the false-full masquerade.
The charge controller is a **TP4054** whose open-drain CHRG status drives only the orange LED — schematic-confirmed (V3.0 and V3.1) not wired to any GPIO — and there is no USB/VBUS-detect pin, so no software presence signal exists (every pin map — ropg, arduino-esp32, ESPHome, Meshtastic — reads only the GPIO1 voltage; Meshtastic's >4.2 V inference is a known source of wrong USB/charge indicators).
Fix: don't trust a battery reading taken on USB power; for real detection, hand-wire CHRG (or a VBUS divider) to a free GPIO (GPIO2/4/5/6/7/19/20/47/48).

---

## Hardware — Adafruit ESP32 Feather V2

**The stock Feather V2 exposes no charge-status (STAT) or USB-VBUS-detect signal on any readable GPIO — don't trust a pin map you can't trace on the schematic.**
Our `EspChargingMonitor` was wired to GPIO13 (as MCP73831 STAT) and GPIO34 (as VBUS detect), producing a permanent `Charging: Unknown`.
Those assignments were wrong: they match the **Unexpected Maker FeatherS2 Neo** ("VBUS detection on IO34"), a different board the pin map was apparently copied from.
On the real Feather V2, the MCP73831 STAT drives only the on-board CHG LED, GPIO13 is the user LED, and GPIO35 (A13, 2×200 kΩ ÷2) is the only power-sense pin.
Proof came from an analog probe (`idf_esp32_chargeprobe`, USB connected, ~3.95 V cell): GPIO35 ≈ 1973 mV (correct), GPIO34 ≈ 147 mV (a real VBUS divider would force ~2500 mV), GPIO13 ≈ 134 mV drifting (a 4.7 kΩ pull-up would force ~3300 mV).
The decisive checks are state-independent — a pull-up or divider, if present, forces a specific voltage *now*, so you don't need to unplug USB (which would kill the serial console anyway) to disprove the wiring.
Fix: read suspected pins as **analog millivolts**, not digital levels, when verifying wiring — a floating input-only pin reads a small leakage (~150 mV), nothing like a driven level.
Also: MCP73831 STAT is **tri-state**, not open-drain (that's the MCP73832).
Schematic-confirmed (EagleCAD rev F): the CHG-LED net is `VBUS → CHG LED → 5.1 kΩ → STAT`, STAT net has two nodes only, no ESP32 trace — the orange CHG LED is not on any GPIO. To read charge state you must hand-wire STAT to the free GPIO34 (A2).

**On the Feather V2, a missing battery reads as a ~96 % FULL battery whenever USB is connected — there is no software way to tell them apart.**
With no cell the MCP73831 holds the BAT net at its ~4.16 V CV regulation point, so the GPIO35 divider reads ~2081 mV (×2 = 4162 mV) — above the empty threshold and below USB-detect, so it classifies as `Battery ~96 %`.
A real 4.16 V cell reads identically, and the only disambiguator (STAT) is not on a GPIO, so no threshold tweak can fix it.
This is a bench artifact of USB + no battery: with no USB the BAT net sinks to ~0 mV and a missing cell correctly reads `No battery`.
Fix: don't chase it with thresholds — document the behaviour, or tap STAT to GPIO34 for true presence detection.

## ESP-IDF / Rust on ESP32-S3

**Rust `std` `println!`/stdout does NOT appear on the Heltec V3 (ESP32-S3) UART console, even though ESP-IDF's own `I (..)` logs do.**
The board's primary console is UART0 with `CONFIG_ESP_CONSOLE_SECONDARY_USB_SERIAL_JTAG=y`; ESP-IDF logging writes via `esp_rom_printf` straight to UART, but Rust stdout writes are swallowed and `flush()` does not help.
The original ESP32 (Feather V2) has no second console, so `println!` happens to work there — which masks the problem until the first S3 example.
Fix: in examples, route output through `esp_idf_hal::sys::esp_rom_printf` (e.g. a tiny `log::Log` impl forwarding to it) — no `esp-idf-svc` dependency needed. This is also why the library's `log::debug!`/`warn!` diagnostics are invisible by default: nothing installs a `log` logger.

## ESP-IDF tier — ESP32-C3 deep sleep (IDF v5.3.3)

**On the ESP32-C3, `esp_deep_sleep_try_to_start` with a GPIO wake pin already at its wake level does NOT return `ESP_ERR_SLEEP_REJECT`; the chip sleeps and wakes about 20 ms later.**
`RTC_GPIO_TRIG_EN` is in the C3 `RTC_SLEEP_REJECT_MASK` and the IDF writes it to `RTC_CNTL_SLP_REJECT_CONF_REG`, so the source reads as if a held pin is rejected.
On hardware it is not: with an HC-SR501 on GPIO4 held HIGH, every entry woke at once with `cause=Gpio mask=0x10` (bench H1, 2026-10-12, 248 of 250 entries; the other 2 slept normally, the PIR likely released between the read and entry), with no reject log and no `Err` returned.
A stuck-HIGH sensor therefore boot-loops at about 2.5 Hz instead of spinning in a `try_sleep` retry loop.
The hardware reason is not confirmed; the deep-sleep GPIO wake on the C3 may latch after the reject window closes.
Fix: `EspSleepManager::try_sleep` on C2/C3 reads the wake pins right before sleep entry and returns `Rejected` itself.
The read needs the pin configured as an input (`FUN_IE`; arming does not set it) and an external bias, and a pin that changes after the read still wakes the chip at once.


## ESP-IDF tier — ESP32 EXT1 wake status (IDF v5.3.3)

**On the ESP32, an `AnyLow` (EXT1 `ALL_LOW`) wake reports `Ext1(0x0)` from `app_main`, although the wake works and `AnyHigh` reports the right bit.**
ESP-IDF's boot code runs `esp_deep_sleep_wakeup_io_reset()` on a deep-sleep reset (`esp_system/port/cpu_start.c:752`), which clears `RTC_CNTL_EXT_WAKEUP1_SEL` before releasing the pad holds (`esp_hw_support/sleep_gpio.c:156-163`); `esp_sleep_get_ext1_wakeup_status()` then reads an empty status in low-level mode.
esp-hal reads the register in the app through `caused_wakeup()`, and a solid held-wire wake got the bit 3 of 3 boots at 01:01; the same firmware then read `0x0` on 5 of 5 later boots the same day, including a 3 s hold with the pin Low at boot, so the register read is not reliable on this tier (bench hal `anylow`, Feather V2, 2026-11-09).
Fix: mirror the IDF tier — `armed_ext1` (`crates/rustyfarian-esp-hal-power/src/armed_ext1.rs`) keeps the armed mask and level in `.rtc_fast.persistent`, `EspHalSleepManager` stores it on any non-timer-only arm and invalidates on arm start and on disarm, and `EspHalWakeCauseSource` resolves the empty status through the shared `stoker::ext1_fired_mask`; the rerun passed 4/4 on both tiers with the bit reported on every touch wake, register empty or not (bench `anylow`, Feather V2, 2026-11-09).
On this tier only `.rtc_fast.persistent` survives deep sleep — `RtcSleepConfig::deep()` powers down the RTC slow memories and the repo's `deep_sleep_config()` rescues fast memory; a `.rtc_slow` static is lost every sleep.
That the low-level status is SEL-gated is inferred, not confirmed at register level; no surviving register names the pin before `app_main`.
Fix: `EspSleepManager` records the armed EXT1 mask and level in `.rtc_noinit` (RTC slow memory, kept powered by IDF's AUTO rule while it holds data), and `EspWakeCauseSource` resolves an empty status to the single armed low pin (`stoker::ext1_fired_mask`); exact because the ESP32 refuses multi-pin `AnyLow`.
Do not force `ESP_PD_DOMAIN_RTC_SLOW_MEM` off, or the record is lost.

## ESP-IDF tier — ESP32-S3 deep sleep (IDF v5.3.3)

**On the ESP32-S3, `esp_deep_sleep_try_to_start` does reject a deep sleep whose EXT1 wake pin is already at its wake level, unlike the C3's GPIO wake.**
With GPIO5 held LOW and `AnyLow` armed, every attempt logged `E sleep: Deep sleep request is rejected` and `try_sleep` returned `Rejected` (more than 800 in a row, no boot loop; CrowPanel 1.28", 2026-10-14).
The C2/C3 pre-sleep level read in `EspSleepManager::try_sleep` is therefore not needed on the S3, and it stays gated to C2/C3.

**On the CrowPanel 1.28" the CST816S touch INT (GPIO5) idles LOW until firmware enables the `LCD_3V3` rail (GPIO1) and releases touch reset (GPIO13).**
A sleep example that only configures GPIO5 sees a permanently asserted wake pin and never sleeps.
Fix: drive GPIO1 and GPIO13 HIGH (knob sequence: rail on, 20 ms, RST low 10 ms, high, 50 ms) and hold both through deep sleep: IDF `gpio_hold_en` plus `gpio_deep_sleep_hold_en`; the hal example uses esp-hal's `Output::set_pad_hold` (unstable) on both outputs.

**A finger resting on the CrowPanel does not hold the CST816S INT LOW; INT pulses, so deep sleep with a touch wake loops instead of being rejected.**
GPIO5 reads HIGH at each sleep entry, the S3 sleeps, and the next pulse wakes it within 0.3-0.6 s, on both tiers (bench H1, 2026-10-14).
Firmware that must not loop while touched should wait for the touch to release before sleeping, or change the CST816S IRQ mode (register not verified).

**A rejected `esp_deep_sleep_try_to_start` leaves the wake pins RTC-held AND RTC-routed on EXT1 chips (ESP32/S3/C6) — the routing, not the hold, is what breaks digital reads.**
`ext1_wakeup_prepare` runs before the reject decision inside `esp_sleep_start` (IDF v5.3.3, `sleep_modes.c:840`; `rtcio_hal_function_select` at 1890 sets MUX_SEL), and the deep-sleep reject tail restores only CPU frequency and UARTs, so both survive the rejection — with no reset to undo them, and esp-idf-hal's `PinDriver::input` does not call `rtc_gpio_deinit` either.
The boot-time SDK-private `esp_deep_sleep_wakeup_io_reset()` clears only holds and wake pins, not the routing.
Fix (implemented 2026-11-03, tier-parity chunk 2 + review fix): on the reject return, `EspSleepManager::try_sleep` calls `rtc_gpio_hold_dis` then `rtc_gpio_deinit` per masked pin (best-effort, the reject path keeps `kind: Rejected`).
Bench-confirmed 2026-11-03 on the CrowPanel S3 (bench R2-S3): after a hardware rejection, GPIO5 read `L` through its existing `PinDriver` and tracked the `L->H` edge in 9 of 9 forced cycles.

**`esp_sleep_disable_wakeup_source(ESP_SLEEP_WAKEUP_ALL)` does not clear `s_config.ext1_rtc_gpio_mask`, so a retry with a changed EXT1 pin/level combination can fail with `ESP_ERR_NOT_ALLOWED` even though the new request alone is valid.**
The ALL branch skips the per-source clear and the enable call OR-merges the mask (`sleep_modes.c:1544-1559`, `1791-1810`), so stale EXT1 bits survive into the next `configure()` within one boot.
Same-mask retries (all benched retry loops) are unaffected, which is why it never surfaced.
Fix (implemented 2026-11-03, tier-parity chunk 2): `configure()` calls `esp_sleep_disable_ext1_wakeup_io(0)` after the disable-all.
Bench R2-S3 (2026-11-03) retried with GPIO0 + GPIO5 after a GPIO5-only rejection: 8 of 8 `Rejected`, no `ESP_ERR_NOT_ALLOWED`; the pre-fix failure itself was not reproduced.

**A rejected deep sleep hangs the ESP32-S3 while the global digital-pad hold (`RTC_CNTL_DG_PAD_AUTOHOLD_EN`) is enabled — on both tiers.**
With the hold enabled, sleep entry isolates every unheld digital pad, the flash (MSPI) pads included (IDF `esp_sleep_isolate_digital_gpio`, `sleep_gpio.c:117-153`; esp-hal `isolate_pads_for_deep_sleep`), and a rejection never restores them.
IDF: `esp_deep_sleep_try_to_start` never returns, garbage `0xFF` bytes reach the console, and the interrupt watchdog resets the chip (`rst:0x8 TG1WDT_SYS_RST`); the pre-change adapter `8669290` hangs the same way.
esp-hal: the chip stalls silently inside `sleep_deep_with_rejection` with the USB port still enumerated (no watchdog).
`gpio_deep_sleep_hold_en()` sets the bit; it lives in RTC_CNTL, survives flashing and USB resets (`reset=Other`), and esp-hal never clears it, so an esp-hal firmware flashed after an IDF one inherits it (`autohold_en=1` read at boot).
Fix (library, 2026-11-04): while the hold is effective, `try_sleep` on both tiers (ESP32 and S3 on IDF, S3 on esp-hal) reads the wake pins right before sleep entry and returns `Rejected` from software, matching the hardware's ANY/ALL condition (`stoker::digital_hold_effective`, `wake_pins_asserted`, `wake_pins_all_low`); bench R3-S3 kept the hold on and saw no hang on either tier.
Residual: a pin that crosses its wake level after the read, or a non-pin hardware reject, still hangs with the hold on; firmware that does not need the global hold should not enable it (per-pin holds on RTC pads, `gpio_hold_en` / `set_pad_hold`, are not the trigger).
The mechanism (flash pads isolated) is read from the source and fits the `0xFF` output; the trigger and the fix are bench-confirmed (bench R2-S3, CrowPanel 1.28", 2026-11-03).
Downstream: any firmware that enables `gpio_deep_sleep_hold_en` and retries rejected sleeps (rustyfarian-knob's IDF sleep path) resets instead of retrying when the wake pin is asserted at entry.

**A rejected esp-hal deep sleep on the ESP32-S3 leaves `RTC_CNTL_WIFI_FORCE_ISO` (bit 28 of `RTC_CNTL_DIG_ISO_REG`) cleared; pads and holds are untouched.**
esp-hal 1.2.2 `base_settings` sets the bit to keep the WiFi/BLE power domain isolated, `RtcSleepConfig::apply()` clears it at sleep entry (`modem_pd_en`), and nothing sets it again on the reject return (bench R4-S3, `iso:10000080>00000080` on 7 of 7 hardware-path cycles; the ESP-IDF hardware path and the esp-hal guard path left the register unchanged).
It is power-domain configuration, not pad state, so the ADR-003 pad postcondition holds; the awake effect is unmeasured (possibly a small current increase until WiFi is brought up).
No fix in the adapter; a restore of that bit after the rejection would be the change if it ever matters.

**The bench CrowPanel's USB-C reaches the S3's native USB-Serial-JTAG (`/dev/cu.usbmodem*`), not a CH340 as the knob docs say.**
`esp-println` built with `uart` prints nowhere visible; use `auto` (or `jtag-serial`).
Once the app spins in a tight `try_sleep` rejection loop, `espflash flash` cannot connect through the USB-JTAG reset; enter download mode by hand (hold BOOT, tap RST).

---

## Bench tooling — USB-Serial-JTAG boards

**`espflash monitor` (4.3) cannot follow a USB-Serial-JTAG board across deep sleep without disturbing it.**
The port vanishes in sleep and re-enumerates on wake, which ends the monitor, and every reattach syncs with the bootloader: `--before no-reset` and `--before no-reset-no-sync` (which also needs `--chip`) both fail with "Error while connecting to device" against a running app.
Fix: read the port passively, for example with pyserial, and reopen it after each re-enumeration.

**A pyserial reader that sets `dtr = False` and then `rts = False` resets an S3 on USB-Serial-JTAG, and the reattach turns that into a reset loop.**
pyserial lowers DTR before RTS, so the line state passes through RTS=1 with DTR=0, which is the USB-Serial-JTAG chip-reset condition (`rst:0x15 (USB_UART_CHIP_RESET)`).
The reset re-enumerates the port, the reader reopens it and resets the chip again, about every 0.8 s, and every boot reports `PowerOn` instead of the deep-sleep wake.
Fix: leave DTR and RTS at their asserted open-time defaults.

**A bench firmware on a USB-Serial-JTAG board must print its machine-read lines in the middle of its awake window, in every build.**
Two things hide output from the host.
The port re-enumerates on every wake and the host needs about 2.5 s to see and open it, so a line printed at boot goes to nobody.
A line printed right before sleep entry is not drained either: on esp-hal the `ATTEMPT` line written just before `try_sleep` never arrived in any of three reader variants (the bench driver's open sequence, a plain pyserial open, DTR kept high), and the ESP-IDF path delivers such a line with a margin of about 50 ms.
Seen 2026-11-10 on the CrowPanel 1.28" with `{hal,idf}_s3_deep_sleep_gpio` while writing `bench/s3/`: the driver's serial log stopped at `Wake cause: PowerOn` on both tiers although a port-presence monitor showed the board cycling 60 s asleep / 6 s awake exactly.
The main cause was in the examples, not in the transport: the `S3BENCH` boot line was printed only in R2 builds, so the default build used by `sleep` and `touch-wake` had nothing to say after `Wake cause`, and the hand-run W1/W2 cases had matched a `BENCH case=demo` line that no longer existed.
Fix: print the boot line and record the sleep entry in every build; keep 6 s awake after each boot and print the boot lines at 3 s and again at 5 s, then wait 1 s before the first sleep attempt (`AWAKE_MS`, `DRAIN_MS`); scenarios tell a new wake apart by its cycle number, since every boot now prints the line three times.
Before suspecting the serial driver, read the firmware's actual output with `just serial-capture` and check that the expected line exists in that build at all.

---

## esp-hal tier (esp-hal 1.2.2)

**A sleep manager that borrows `&mut Input` wake pins must give the pins their own lifetime, not the manager's `LowPower<'d>` lifetime.**
`&mut Input<'d>` is invariant in `'d`, so `&mut [&mut Input<'d>]` with the manager's `'d` fails with "lifetime may not live long enough" once the manager and the pin come from different scopes.
Fix: add a separate `'i` parameter, as in `pins: &'a mut [&'p mut Input<'i>]`.
`Input` has no public pin-number method; use `peripheral_input().gpio_number()`.
See `docs/adr/001-hal-neutral-sleep-contract.md` § 2.

**A rejected `sleep_deep_with_rejection` overwrites the boot's wake record, so `caused_wakeup()` read after a rejection no longer names the pin that woke the chip.**
Sleep entry clears the per-pin wake status, and the rejected sleep then runs `record_wakeup()` again.
Fix: capture the wake cause and its pin mask at boot, before the first `try_sleep`.
Bench-confirmed on the S3 (bench R2-S3, 2026-11-03): a re-capture after a rejection read `Ext1(0x0)` in 6 of 6 `Ext1(0x20)` wakes; the ESP-IDF tier showed the same in 1 of 1, so the rule applies to both tiers on the S3. Not yet bench-confirmed on the C3.

**A custom esp-hal GPIO handler that calls `unlisten()` on a wake pin silently removes it from the deep-sleep wake set.**
`Input::listen` enables the CPU interrupt immediately, so a pin already at its wake level runs the handler before sleep entry.
`unlisten()` clears the pin's `listening` record and `int_type`, sleep entry then drops the GPIO source, and with no timer armed `sleep_deep` panics on "no wakeup source".
The default `rt` handler is safe: it clears only `int_ena` and keeps the wake condition.
Fix: arm, sleep and clean up inside `critical_section::with`; found by code reading, bench case `held-level-custom-handler` in ADR-001 § 4b.

**esp-hal 1.2.2 drains the UART TX FIFOs inside sleep entry, after both deadline checks, so a short sleep timer can expire before the chip sleeps.**
`sleep_uart_prepare` runs after `sleep_deep`'s `deadline_missed` assert; on the C3 it spins until the 128-byte FIFO is empty and the transmitter idle, about 11 ms per full FIFO at 115200 baud.
Masking interrupts does not shorten the wait, and with `sleep_deep_with_rejection` a deadline missed there is a sleep that never ends.
Fix: inside the critical section, wait for every UART to drain before arming the deadline; see ADR-001 § 4c.

**esp-hal 1.2.2 writes the sleep deadline into a 48-bit RTC comparator and silently drops the higher bits.**
An oversized timer passes `u64` µs arithmetic and programs a different deadline (`sleep/timer.rs` `arm`).
Fix: cap requests in software (`MAX_TIMER_MS`); see ADR-001 § 4a.

**esp-hal `Output::new`, `Input::new` and `Flex::new` release a pad hold before they drive the level, so a hold from the previous run does not survive re-creating the driver.**
`init_gpio` disables the output driver and calls `set_pad_hold(false)` first, then `set_level` runs (`gpio/mod.rs:658-668`, `1728-1736`).
On a wake boot the pad is therefore briefly undriven, and the rail is not glitch-free as a "drive, then release" reading suggests.
Fix: call `Output::set_pad_hold(true)` again after re-creating the drivers (`gpio/mod.rs:857-866`, unstable), and expect the brief undriven window; the earlier raw-register version passed the bench 3/3, the `set_pad_hold` version is not yet re-benched.

**An ADC2 pad passed to an esp-hal 1.2.2 `AdcConfig<ADC1>` compiles and silently reads the ADC1 channel with the same number.**
`AdcChannel` is implemented for every analog pad and carries only the channel index (`analog/adc/mod.rs:285`), so ESP32 GPIO4 (ADC2_CH0) reads GPIO36, and S3 GPIO11 reads GPIO1.
The result is a plausible but wrong battery voltage, not an error.
Fix: `EspHalAdcBatteryMonitor` bounds `PIN` on the sealed `Adc1Pin` trait; re-check its hand-written pad lists on every esp-hal bump.

**A UART TX drain that waits for FSM state 0 never finishes on the ESP32-C6.**
UART v2 (`uart/low_level/v2.rs`) rests at TX FSM state `0x0F` (TX_WAIT_SEND) as well as `0x00`, so a C3/S3-style drain times out and every `sleep`/`try_sleep` returns `UartBusy`.
Fix: treat `0x00` and `0x0F` as idle on the C6 (`sleep.rs` drain, 2026-11-04); the bench in the final hardware pass confirms it.

---

**On the ESP32, `.rtc_fast.persistent` does not survive esp-hal 1.2.2 deep sleep: `RtcRetainedCell` loads `Invalid::Magic` and `EspHalRtcClock` restarts at 0 after every timer wake.**
`RtcSleepConfig::deep()` sets `rtc_fastmem_pd_en`, and the ESP32 `apply()` turns that into cleared `fastmem_force_pu`/`force_lpu`/`force_noiso`; it never writes `fastmem_pd_en` (`rtc_cntl/sleep/esp32.rs:349`), so reading the registers suggests the memory stays powered, but it does not keep its contents.
The identical code passes on the C3, where the flag is unused.
Fix: build the config with `set_rtc_fastmem_pd_en(false)` on the ESP32 (`deep_sleep_config()` in `sleep.rs`); bench-confirmed on a WROOM-32 2026-11-08 (`load=err:Magic` before, `load=ok` after, same board).
Found only because a bench example printed the RTC clock at every boot.

## stoker — RTC retention cell

**A `Sync` cell with safe `&self` methods that do unsynchronised volatile copies is a data race from safe code, and no documented single-context rule makes it sound.**
`RetainedCell` first shipped that way (volatile access, fences and a CRC catch a torn copy, but the race itself is undefined behaviour); an external review flagged it after the hal wrapper widened the surface.
Fix: every `load`, `store` and `invalidate` copies inside `critical_section::with`, which is also the only portable lock on riscv32imc (no compare-and-swap); `stoker` takes `critical-section` as its single dependency.

**A missing `critical-section` implementation fails only at link time, never under `cargo check`.**
The symptom is an undefined `_critical_section_1_0_acquire` (no provider) or a duplicate one (two providers); esp-hal supplies the implementation through its `rt` feature, the host through the crate's `std` feature as a dev-dependency, and the IDF tier through its opt-in `critical-section-impl` feature.
Fix: the firmware links exactly one provider; the retention examples require `critical-section-impl` via `required-features`, and `just build-example` is the gate for the retention code, not `check`.

**A library that enables a `critical-section` provider by default breaks every firmware whose graph already has one.**
`rustyfarian-esp-idf-network`'s `lora` feature enables esp-idf-hal's `critical-section`, so with `critical-section-impl` in our `default` a LoRa firmware failed to link with a duplicate `_critical_section_1_0_acquire` (beekeeper, 2026-10-29); `cargo check` passed.
Fix: `critical-section-impl` is opt-in; only the final firmware picks the provider, as the `critical-section` crate prescribes.

**esp-idf-hal's `critical-section` provider is a task-level recursive mutex, not a critical section, so it cannot serialise a task against an ISR.**
FreeRTOS forbids taking that mutex from an interrupt, and an interrupt that preempts the holder runs as the same task and re-enters it, so the exclusion `RetainedCell` relies on is silently lost.
Fix: the IDF tier provides the `critical-section` implementation over the FreeRTOS port critical section (`vPortEnterCritical`/`xPortEnterCriticalTimeout` on a `portMUX`), which masks interrupts and spins out the other core.

---

## Bench tooling — ESP32-C3 SuperMini

**espflash's USB-Serial-JTAG reset leaves the ESP32-C3 SuperMini in download mode; the app never starts by itself after `just flash` or `espflash reset`.**
UART0 showed `rst:0x15 (USB_UART_CHIP_RESET),boot:0x5 (DOWNLOAD(USB/UART0/1))` after every espflash reset (2026-10-23, espflash 4.3, chip v0.4), and the USB device never dropped, so the chip sat in the ROM downloader until a USB power cycle; the board's RST tap did not produce a boot on UART0 either.
Open the USB-Serial-JTAG port only to flash, read the console on UART0 (GPIO21) with a USB-UART adapter whose DTR/RTS are unwired, and power-cycle the board to start the app.

**A deep-sleep GPIO wake that "never fires" can be a sensor that never drives the pin; probe the pin while awake, with a pull-up, before suspecting the sleep path.**
On the SuperMini the W1 PIR wake never fired while the DevKitM-1 woke 8 times in 30 s on the same firmware (spookhouse, 2026-10-23).
The example's build-time `C3_AWAKE_PROBE_MS` probe polled GPIO0-5 for 30 s per boot: with the internal pull-down GPIO4 read Low in 5 windows; with `C3_PROBE_PULL=up` it still read Low while GPIO0-3 and 5 read High, and it read High once the PIR wire was pulled, so the pad was fine and the PIR output was holding the pin Low without ever going High.
A second HC-SR501 on the same pins woke the board on the first wave (`cause=Gpio mask=0x10`), after a 1520-rejection H4 loop while its output was held.
The wake registers, chip revision (v0.4 vs v0.3: no erratum, IDF v5.3.3's only C3 sleep gate is below v0.3) and `try_sleep` were never the fault.
Rule: with a pull-up probe an open pin reads High, a pin wired to a push-pull sensor reads Low at rest and High on motion, and a pin stuck Low has a dead or unpowered source; `C3_PIR_PIN` moves the wake pin if the signal lands on a neighbouring header pin.

## Bench tooling — C3 bench driver

**Bench R1 reported PASS on a run with no deep-sleep wake at all, because its only check was "counter incremented".**
A software reset, panic or watchdog reset also keeps a retained counter once the record lives in `.rtc_noinit` or `.rtc_fast.persistent`, so `Sleeping:` followed by `reset=Software cause=PowerOn` passed.
Fix: R1 requires `reset=DeepSleep` and `cause=Timer` on every judged boot besides the increment; the `R1 warm reset` self-test injects the software-reset sequence and expects one FAIL.

**Opening the CP210x port of the ESP32-C3-DevKitM-1 resets the chip through EN even with DTR and RTS driven low, and ESP-IDF and esp-hal both report it as a power-on reset.**
Every `just serial-capture` therefore starts with `reset=PowerOn` and a fresh retention session, which is indistinguishable from a USB replug in the log.
Fix: start the capture before the sequence under test and read the power-on boot at the top as the port open, not as evidence.

## Bench tooling — ESP32 bench hal builds

**The `hal_esp32_*` bench apps boot through a stale `ESP-IDF v6.1-beta1` second-stage bootloader left in flash by an earlier IDF app.**
`espflash` writes the app image without erasing the bootloader sector, so the hal logs carry IDF boot banners and the `ESP-IDF v` line does not say which tier the firmware is.
The bootloader is the same across every bench log and was ruled out as a factor; read the app's own `Reset reason:` line (and the `kind=` text) to identify the tier, not the boot banners.
