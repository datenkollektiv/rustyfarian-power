# Bench Plan: ESP32-C3 DevKitM-1 Deep Sleep and Wake v1

Draft, 2026-10-12.
Validates [ADR-001](../adr/001-hal-neutral-sleep-contract.md) and the hardware items of `esp-hal-sleep-tier-v1`, `c3-gpio-deep-sleep-wake-v1` and `rtc-retention-v1` on both tiers.
Pinned to esp-hal `=1.2.2` and ESP-IDF v5.3.3.
The consumer's go/no-go gate is 2026-10-25, so the P0 cases run first and report on their own.

## Gating

| Priority | Decides                                                      | Cases               |
|:---------|:-------------------------------------------------------------|:--------------------|
| P0       | Consumer stuck-HIGH retry and timer fallback; ADR acceptance | W1-W3, H1-H4, I1-I3 |
| P1       | Timer constants and the § 4b-4d guarantees                   | H5-H8               |
| P2       | Open questions in the feature docs; retention                | I4-I7, R1-R5        |

ADR-001 moves to Accepted when every P0 and P1 case passes and `MIN_TIMER_MS` and `POST_ARM_MARGIN_US` are set from H5.

## Prerequisites

**Built 2026-10-15:**
- Hal tier: sleep module in `crates/rustyfarian-esp-hal-power/src/sleep.rs` with `bench` feature; `set_marker_pin(u8)` and `last() -> BenchTimings { drain_us, guard_us }`.
- Bench firmware: `examples/hal_c3_deep_sleep_gpio.rs` (plain twin of IDF example) and `examples/hal_c3_sleep_bench.rs`, case selected at build time by `BENCH_CASE` env var; both compile when `BENCH_CASE` is set.
- Stimulus firmware: `examples/hal_esp32_bench_stimulus.rs` (ESP32-WROOM bare-metal, esp-hal 1.2.2) with UART0 and GPIO25/26 Flex pins; flashed via `just flash-hal-esp32 hal_esp32_bench_stimulus`.
- Host script: `scripts/c3-bench.py` (Python 3, pyserial 3.5, PEP 723 header) runs via `uv run` or `just bench-c3 --case <X> ...`; reads the DUT serial log, drives the WROOM via its UART, detects hangs and writes one CSV row per cycle.
- `just` recipes: `build-example-hal-c3`, `run-hal-c3`, `flash-hal-c3`, `build-example-hal-esp32`, `run-hal-esp32`, `flash-hal-esp32`, and `bench-c3 *args` wrapping the Python script.
- Toolchain: `.cargo/config.toml.dist` with `[target.riscv32imc-unknown-none-elf]` (C3, RISC-V) and `[target.xtensa-esp32-none-elf]` (WROOM stimulus, Xtensa), both with espflash runner and `-Tlinkall.x` rustflag.

**Not yet built:**
- IDF tier: the C3 GPIO wake path and `try_sleep` from `c3-gpio-deep-sleep-wake-v1`.
- IDF bench firmware: `idf_c3_sleep_bench`, case selected at build time by `BENCH_CASE`.

## Setup

### Instruments

- **Bench board:** ESP32-C3-MINI-1 with CH340 on UART0 (`/dev/cu.usbserial-210`). The plan assumes DevKitM-1 with CP2102N; both use micro-USB and have GPIO20/21 on UART0.
- **Stimulus board:** ESP32-WROOM running `hal_esp32_bench_stimulus` (built 2026-10-15) on a separate USB port. It drives two 3.3 V GPIO channels (GPIO25 and GPIO26) via the serial protocol in its doc comment; see [hal_esp32_bench_stimulus.rs](../../crates/rustyfarian-esp-hal-power/examples/hal_esp32_bench_stimulus.rs) for the complete command reference.
- Logic analyser, 4 channels, at least 4 MS/s.
- Current: Nordic PPK2 in ampere-meter mode in series with VBUS through a USB data pass-through breakout, so serial keeps working; its digital inputs record the marker on the same time base.
- Without a PPK2, H5 falls back to the rejected-path upper bound (see H5) and the plan records that.

### Wiring

| Signal       | C3 pin | Connection                                                 | Note                               |
|:-------------|:-------|:-----------------------------------------------------------|:-----------------------------------|
| Wake (PIR)   | GPIO4  | WROOM GPIO25 → 1 kΩ → GPIO4; 10 kΩ pull-down to GND; LA ch0 | MTMS (JTAG TMS)                    |
| Foreign pad  | GPIO5  | WROOM GPIO26 → 1 kΩ → GPIO5; 10 kΩ pull-down to GND; LA ch1 | H8 only                            |
| Guard marker | GPIO10 | LA ch2 and PPK2 D0                                         | not a low-power pad, not strapping |
| UART0 TX     | GPIO21 | LA ch3 (already wired to the bridge)                       | H5 drain timing                    |
| GND          | GND    | WROOM, C3, LA and PPK2 ground                              | one common ground                  |

Avoid GPIO2, GPIO8 and GPIO9 (strapping; GPIO8 also drives the RGB LED, GPIO9 the BOOT button).
Meter the stimulus at 3.3 V before connecting; the C3 pads are not 5 V tolerant.

### Firmware log line

Every boot and every `try_sleep` return prints one line on UART0:

```text
BENCH case=<id> tier=<hal|idf> cycle=<n> reset=<reason> cause=<cause> mask=0x<hex> result=<ok|err:<variant>> guard_us=<n> drain_us=<n>
```

`cycle` comes from the RTC retention record when R1 has passed; until then the host script counts boots and treats the firmware value as advisory.

The IDF example `idf_c3_deep_sleep_gpio` also prints `result=boot` on the boot line; the host script must accept it.
The owner-desk runbook for W1, W2, H1 and H4 with a real HC-SR501 PIR is [c3-idf-pir-bench-steps-v1](c3-idf-pir-bench-steps-v1.md).

### Case equipment requirements

| Case | P0/P1 | Tier | PIR or jumper | GPIO stimulus | LA (ch0/1/2/3) | PPK2 |
|:-----|:-----:|:-----|:-------------:|:-------------:|:--------------:|:----:|
| W1   |  P0   | both |      Yes      |     GPIO4     |     ch0,3      |  —   |
| W2   |  P0   | both |      Yes      |       —       |       —        |  —   |
| W3   |  P0   | both |      Yes      |     GPIO4     |     ch0,3      |  —   |
| H1   |  P0   | both |       —       |     GPIO4     |       —        |  —   |
| H2   |  P0   | hal  |       —       |     GPIO4     |       —        |  —   |
| H3   |  P0   | hal  |       —       |     GPIO4     |       —        |  —   |
| H4   |  P0   | both |       —       |     GPIO4     |       —        |  —   |
| H5   |  P1   | both |       —       |     GPIO4     |      ch3       |  D0  |
| H6   |  P1   | hal  |       —       |       —       |       —        |  —   |
| H7   |  P1   | hal  |       —       |       —       |       —        |  —   |
| H8   |  P1   | hal  |       —       |     GPIO5     |       —        |  —   |
| R1   |  P2   | hal  |      Yes      |       —       |       —        |  —   |

PIR or jumper: use the HC-SR501 PIR for realistic wake-up timing (W1-W3, R1), or a manual jumper to GND for gate-level testing.
GPIO stimulus: any host-controlled 3.3 V GPIO source, or spare GPIO driven from the bench script.
LA (Logic Analyser): record the channels and timing around critical sections; ch3 shows UART0 TX activity for H5 drain measurement.
PPK2 (Nordic Power Profiler Kit II): in ampere-meter mode in series with VBUS, digital input D0 records the marker on the same time base; required to measure guard-to-sleep-entry in H5.

### Host procedure per case

**Setup (run once per test session):**
- Flash the stimulus board: `ESPFLASH_PORT=<wroom-port> just flash-hal-esp32 hal_esp32_bench_stimulus`; on boot it prints `STIM ready v1`.
- With both boards attached, always set `ESPFLASH_PORT` (espflash reads it), or espflash prompts or picks the wrong board.
- Verify WROOM connectivity: `just bench-c3 --reset-test` (sends `PING`, confirms the WROOM and DUT are on independent ports).

**Per case:**
1. Flash the DUT: `BENCH_CASE=H1 ESPFLASH_PORT=/dev/cu.usbserial-210 just flash-hal-c3 hal_c3_sleep_bench esp32c3,unstable,examples,bench` (set `BENCH_CASE` for the desired case name: W1–W3, H1–H8, R1).
2. Run the case: `just bench-c3 --case H1 --port /dev/cu.usbserial-210 --cycles 100` (defaults: 100 cycles for W/H1/H3/H8, 20 for R1, 1 for H4).
3. When the run completes, send `ZZ` to tri-state the WROOM pins before unplugging: `just bench-c3` accepts `--no-stim` to skip stimulus control; manually send `ZZ` via a serial client if needed.

**Observations:**
- A hang is no log line within the expected state-change window plus 2 s; the script then toggles the DUT's RTS (reset) and logs `result=hang` for that cycle.
- A panic is any `PANIC:` line from the firmware's panic handler; the script logs it and continues.
- The script writes one CSV per case in `tmp/bench/<YYYY-MM-DD>/<case>-hal.csv` and a raw `.log` beside it; a summary including hangs, panics, cause histogram and MANUAL items prints to stdout.

**Protocol reference:**
The WROOM accepts ASCII commands (case-insensitive, `\n` terminated, 64-byte line limit):
- `PING` → `PONG hal_esp32_bench_stimulus v1` (liveness check)
- `VER` → `VER 1 ch1=GPIO25 ch2=GPIO26` (version and pin map)
- `H <ch>` / `L <ch>` / `Z <ch>` → `OK H <ch>` etc. (drive high / low / tri-state now)
- `ZZ` → `OK ZZ` (tri-state both channels, cancel schedules)
- `P <ch> <H|L> <width_us> [<delay_us> [<end H|L|Z>]]` → `OK P <ch> armed`, later `DONE P <ch> t_start_us=<n> t_end_us=<n>` (schedule a pulse with optional delay and end state)
- `X <ch>` → `OK X <ch>` (cancel pending pulse, leave pin as is)
- `T` → `T <now_us>` (read device time in microseconds since boot)

See [hal_esp32_bench_stimulus.rs](../../crates/rustyfarian-esp-hal-power/examples/hal_esp32_bench_stimulus.rs) for the full documentation.

## Cases

Each case runs on the tier given; "both" means once per tier with the same wiring.

### Wake path (P0)

| ID | Tier | Case                    | Steps                                                              | Pass                                                                      |
|:---|:-----|:------------------------|:-------------------------------------------------------------------|:--------------------------------------------------------------------------|
| W1 | both | GPIO4 wake              | `try_sleep` GPIO4 `AnyHigh`; after 2 s drive GPIO4 high for 100 ms | wake within 50 ms of the edge; `cause=Gpio`, `mask` has bit 4; 100 cycles |
| W2 | both | Timer fallback          | GPIO4 plus 5 s timer, GPIO4 held low                               | wake at 5 s ± 5 %; `cause=Timer`; 100 cycles                              |
| W3 | both | GPIO and timer together | GPIO4 plus 5 s timer, GPIO4 pulse at random 0-6 s                  | whichever comes first wakes, cause matches; 100 cycles                    |

### Held wake pin and rejection (P0)

| ID | Tier | Case                            | Steps                                                                                                    | Pass                                                                                                                                               |
|:---|:-----|:--------------------------------|:---------------------------------------------------------------------------------------------------------|:---------------------------------------------------------------------------------------------------------------------------------------------------|
| H1 | both | `wake-pin-held-at-entry`        | GPIO4 held high, `try_sleep` GPIO4 `AnyHigh` plus 60 s timer                                             | record which: `err:Rejected` returned, or slept and woke at once; the consumer needs the answer, so either outcome passes if it repeats 100 of 100 |
| H2 | hal  | `wake-cause-survives-rejection` | boot from a GPIO4 wake; read cause; GPIO4 held; one `try_sleep` (rejected); read `caused_wakeup()` again | record both reads; expected per code reading: the second read no longer names GPIO4                                                                |
| H3 | hal  | `held-level-custom-handler`     | custom GPIO handler that counts calls and calls `unlisten`; GPIO4 held; `try_sleep`                      | `err:Rejected`; handler count 0 during the attempt                                                                                                 |
| H4 | both | `rejection-retry-loop`          | GPIO4 held for 1000 attempts, then released; next attempt sleeps; pulse GPIO4                            | 1000 × `err:Rejected`, then sleep, then `cause=Gpio`; no hang, no panic                                                                            |

### Timing, exclusivity, UART (P1, hal tier unless noted)

| ID | Tier | Case                | Steps                                                                            | Pass                                                                                                |
|:---|:-----|:--------------------|:---------------------------------------------------------------------------------|:----------------------------------------------------------------------------------------------------|
| H5 | both | `short-timer-sweep` | matrix below, 1000 cycles per cell, H3 handler active                            | zero hangs, zero panics; record guard-to-entry per cycle                                            |
| H6 | hal  | Timer range limits  | `Timer` 0 ms, `MIN_TIMER_MS - 1`, `MAX_TIMER_MS + 1`                             | `TimerTooShort`, `TimerTooShort`, `TimerTooLong`; GPIO4 and deadline untouched (LA shows no change) |
| H7 | hal  | UART busy           | enable CTS flow control on UART1, hold CTS inactive, queue 16 bytes, `try_sleep` | `err:UartBusy` after about 200 ms; no hang                                                          |
| H8 | hal  | `foreign-wake-pin`  | firmware arms GPIO5 for low-power wake and listens; `try_sleep` binds only GPIO4 | `err:ForeignWakePin{pin:5}`; then `unlisten` GPIO5, sleep, pulse GPIO5: no wake; pulse GPIO4: wake  |

H5 matrix:

| Sources          | Timer (ms)   | UART0 at the call                  | Methods              |
|:-----------------|:-------------|:-----------------------------------|:---------------------|
| timer only       | 1, 2, 5, 10  | idle; 128 bytes queued at 115200   | `try_sleep`, `sleep` |
| GPIO4 plus timer | 1, 2, 5, 10  | idle; 128 bytes queued at 115200   | `try_sleep`, `sleep` |

H5 measurement: guard-to-sleep-entry is the time from the GPIO10 marker edge to the VBUS current step of the CPU powering down, read from the PPK2 trace.
Cross-check on the rejected path: the firmware logs guard-to-return in cycles, an upper bound for the same entry code.
Record `drain_us` and the UART0 TX burst end on LA ch3, to confirm the drain finished before the marker.
Set `POST_ARM_MARGIN_US` to at least 4x the worst guard-to-entry, and `MIN_TIMER_MS` to at least that margin plus 16 slow-clock ticks.

### IDF-tier open questions (P0/P2)

| ID | Tier | Case                           | Steps                                                                                                                                                                                                                                                                                                                                                      | Pass                                                                                                                                                                                                                                            |
|:---|:-----|:-------------------------------|:-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|:------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| I1 | idf  | Reject maps to `Rejected`      | as H1                                                                                                                                                                                                                                                                                                                                                      | `kind() == Rejected` from the `esp_deep_sleep_try_to_start` call site                                                                                                                                                                           |
| I2 | idf  | Empty sources                  | `sleep(&[])`, `try_sleep(&[])`                                                                                                                                                                                                                                                                                                                             | `err:NoWakeSource` both; chip stays awake                                                                                                                                                                                                       |
| I3 | idf  | Stale GPIO mask                | arm GPIO4, `try_sleep` rejected; then `try_sleep` timer-only 5 s; pulse GPIO4 at 2 s                                                                                                                                                                                                                                                                       | no wake at 2 s; `cause=Timer` at 5 s                                                                                                                                                                                                            |
| I4 | idf  | Isolation with a held wake pin | `isolate_gpio` on, GPIO4 wake; repeat W1                                                                                                                                                                                                                                                                                                                   | record whether W1 still passes; answers the C3 doc's open question                                                                                                                                                                              |
| I5 | idf  | Mask outside `0x3F`            | `GpioLevel` with bit 6                                                                                                                                                                                                                                                                                                                                     | `err:GpioMaskOutOfRange`; no SDK call made (no IDF error log)                                                                                                                                                                                   |
| I6 | idf  | `gpio-level-high-to-low`       | stimulus on GPIO4 (not the PIR), one boot: GPIO4 held LOW, `try_sleep` GPIO4 `AnyHigh` with GPIO4 driven HIGH (rejected); then drive GPIO4 HIGH (inactive for `AnyLow`), `try_sleep` GPIO4 `AnyLow` plus 60 s timer; after 2 s drive GPIO4 LOW for 100 ms. Separately, tri-state the stimulus during the `AnyLow` sleep and meter the GPIO4 pull direction | `AnyLow` sleeps (not rejected) and wakes on the LOW pulse with `cause=Gpio`, bit 4; record the pull direction measured (pull-down = the SDK trigger-mode bug, `sleep_modes.c:2004-2008`); if it misbehaves, the IDF tier rejects the transition |
| I7 | idf  | `unrelated-pin-sleep-select`   | firmware calls `gpio_sleep_sel_dis(GPIO3)` and drives GPIO3 HIGH; `try_sleep` timer-only 5 s; meter GPIO3 during sleep                                                                                                                                                                                                                                     | GPIO3 keeps its awake configuration during sleep (sleep-select still disabled): the manager only disarms pins whose wake enable is set                                                                                                          |

### Retention (P2)

| ID | Tier | Case                               | Steps                                                       | Pass                                                                    |
|:---|:-----|:-----------------------------------|:------------------------------------------------------------|:------------------------------------------------------------------------|
| R1 | hal  | RTC fast memory through deep sleep | store a counter in the `Persistable` cell; W2 for 20 cycles | counter increments every boot; answers the `rtc_fastmem_pd_en` question |
| R2 | both | GPIO and timer wake                | as W1/W2 with the envelope                                  | `load` returns the stored value every boot                              |
| R3 | both | Watchdog and software reset        | trigger RWDT, then `software_reset`                         | `load` succeeds after both                                              |
| R4 | both | Power cycle                        | unplug USB for 5 s                                          | `load` returns `Invalid::PowerOn`                                       |
| R5 | both | Brownout                           | setup to be decided, see risks                              | `load` returns `Invalid::Brownout`                                      |

## Manual Verification

The script reports a `MANUAL:` list of items that require human measurement.
The script's CSV verdicts pass based on return codes, hang counts and panic absence; these items need analysis outside the script:

| Item | Case | Measurement | Tool | Purpose |
|:-----|:-----|:---|:---|:---|
| W1 edge-to-boot 50 ms | W1 | Time from WROOM GPIO25 rising edge (LA ch0) to C3 boot line on UART0 TX (LA ch3); mark GPIO10 marker (LA ch2) for timing reference. | Logic Analyser ch0/2/3 | Verify the C3 wake path latency is within 50 ms. |
| W2 timer ±5 % | W2 | Expected sleep duration (5 s) ±5 %; confirm via LA or elapsed time between boot lines. | Logic Analyser ch3 or serial timestamps | Verify timer accuracy (worst-case timer frequency). |
| H5 guard-to-entry | H5 | Time from GPIO10 marker edge (PPK2 D0 rising, LA ch2) to VBUS current step (PPK2 trace) at the CPU power-down of the first sleep attempt. | PPK2 digital + current trace, LA ch2 | Measure the guard delay and set `POST_ARM_MARGIN_US` constant. |

## Results

One CSV per case and tier in `tmp/bench/<date>/<case>-<tier>.csv`, columns from the log line plus `host_ts`.
A summary table per run goes into the feature docs' session logs, and constants from H5 into ADR-001 § 4c.

## Setup risks to resolve before running

- R5 needs the supply below the brownout threshold while serial still works; check the DevKitM-1 schematic for how the CP2102N is powered before cutting VBUS or feeding the 5 V pin.
- The VBUS current step in H5 sits on top of the bridge and LDO draw; confirm on the first cycle that the CPU power-down step is resolvable before running 1000.
- GPIO4 and GPIO5 are JTAG pads; on the default eFuse settings JTAG uses the built-in USB-JTAG, so these pads are free, but check that no probe is attached.
