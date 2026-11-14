# Bench Plan: ESP32-C3 DevKitM-1 Deep Sleep and Wake v1

Operating guide for `just bench-c3` (`scripts/c3-bench.py`), the two-board driver for the C3 sleep cases with the ESP32-WROOM stimulus, pinned to esp-hal `=1.2.2` and ESP-IDF v6.1.
The guided single-board checks (`just bench c3-mini1`) are in [bench/c3-mini1/README.md](../../bench/c3-mini1/README.md); this driver covers the cases that need a scripted stimulus or a logic analyser.

## Prerequisites

- Hal tier: sleep module in `crates/rustyfarian-esp-hal-power/src/sleep.rs` with the `bench` feature; `set_marker_pin(u8)` and `last() -> BenchTimings { drain_us, guard_us }`.
- Firmware: `examples/hal_c3_sleep_bench.rs` (case selected at build time by the `BENCH_CASE` env var) and the stimulus `examples/hal_esp32_bench_stimulus.rs` (ESP32-WROOM bare-metal, esp-hal 1.2.2, UART0 and GPIO25/26 Flex pins).
- Host script: `scripts/c3-bench.py` (Python 3, pyserial 3.5, PEP 723 header) via `just bench-c3`, which wraps `uv run`.
- `just` recipes: `build-example-hal-c3`, `flash-hal-c3`, `build-example-hal-esp32`, `flash-hal-esp32`; the hal targets come from `.cargo/config.toml` (espflash runner, `-Tlinkall.x`).

## Instruments

- **Bench board:** ESP32-C3-MINI-1 with CH340 on UART0 (`/dev/cu.usbserial-210`, the script's default `--port`); the DevKitM-1 with CP2102N works the same, both have GPIO20/21 on UART0.
- **Stimulus board:** ESP32-WROOM running `hal_esp32_bench_stimulus` on a separate USB port; it drives two 3.3 V channels (GPIO25, GPIO26) over the protocol in its doc comment, see [hal_esp32_bench_stimulus.rs](../../crates/rustyfarian-esp-hal-power/examples/hal_esp32_bench_stimulus.rs).
- MANUAL items only: a 4-channel logic analyser at 4 MS/s or more, and for H5 a Nordic PPK2 in ampere-meter mode in series with VBUS (USB data pass-through breakout, D0 on the marker); without the PPK2, H5 falls back to the rejected-path upper bound (`--h5-hold-high`).

## Wiring

| Signal       | C3 pin | Connection                                                  | Note                               |
|:-------------|:-------|:------------------------------------------------------------|:-----------------------------------|
| Wake (PIR)   | GPIO4  | WROOM GPIO25 → 1 kΩ → GPIO4; 10 kΩ pull-down to GND; LA ch0 | MTMS (JTAG TMS)                    |
| Foreign pad  | GPIO5  | WROOM GPIO26 → 1 kΩ → GPIO5; 10 kΩ pull-down to GND; LA ch1 | H8 only                            |
| Guard marker | GPIO10 | LA ch2 and PPK2 D0                                          | not a low-power pad, not strapping |
| UART0 TX     | GPIO21 | LA ch3 (already wired to the bridge)                        | H5 drain timing                    |
| GND          | GND    | WROOM, C3, LA and PPK2 ground                               | one common ground                  |

Avoid GPIO2, GPIO8 and GPIO9 (strapping; GPIO8 also drives the RGB LED, GPIO9 the BOOT button); GPIO4 and GPIO5 are JTAG pads, free with the default eFuses (USB-JTAG), so check that no probe is attached.
Meter the stimulus at 3.3 V before connecting; the C3 pads are not 5 V tolerant.

## Firmware log line

Every boot and every `try_sleep` return prints one line on UART0; this is the script's parser contract:

```text
BENCH case=<id> tier=<hal|idf> cycle=<n> reset=<reason> cause=<cause> mask=0x<hex> result=<ok|err:<variant>> guard_us=<n> drain_us=<n>
```

The host script counts boots itself and treats the firmware `cycle` as advisory; a boot line may carry `result=boot` (the IDF examples `idf_c3_deep_sleep_gpio` and `idf_c6_deep_sleep_gpio` print it).
A `PANIC:` line is the firmware's panic handler; the script counts it and continues.

## Case equipment

| Case | Tier | PIR or jumper | GPIO stimulus | LA (ch0/1/2/3) | PPK2 |
|:-----|:-----|:-------------:|:-------------:|:--------------:|:----:|
| W1   | both |      Yes      |     GPIO4     |     ch0,3      |  —   |
| W2   | both |      Yes      |       —       |       —        |  —   |
| W3   | both |      Yes      |     GPIO4     |     ch0,3      |  —   |
| H1   | both |       —       |     GPIO4     |       —        |  —   |
| H2   | hal  |       —       |     GPIO4     |       —        |  —   |
| H3   | hal  |       —       |     GPIO4     |       —        |  —   |
| H4   | both |       —       |     GPIO4     |       —        |  —   |
| H5   | both |       —       |     GPIO4     |      ch3       |  D0  |
| H6   | hal  |       —       |       —       |       —        |  —   |
| H7   | hal  |       —       |       —       |       —        |  —   |
| H8   | hal  |       —       |     GPIO5     |       —        |  —   |
| R1   | hal  |      Yes      |       —       |       —        |  —   |

PIR or jumper: the HC-SR501 PIR for realistic wake timing (W1-W3, R1), or a jumper to GND for gate-level testing; `--no-stim` prints the stimulus steps as MANUAL instructions instead of driving the WROOM.

## Host procedure

Once per session:

1. Flash the stimulus board: `ESPFLASH_PORT=<wroom-port> just flash-hal-esp32 hal_esp32_bench_stimulus`; on boot it prints `STIM ready v1`. With both boards attached, always set `ESPFLASH_PORT`, or espflash prompts or picks the wrong board.
2. Check the ports: `just bench-c3 --reset-test` pulses the DUT reset once and confirms it boots from flash on a port independent of the WROOM.

Per case:

1. Flash the DUT with the case baked in: `BENCH_CASE=H1 ESPFLASH_PORT=/dev/cu.usbserial-210 just flash-hal-c3 hal_c3_sleep_bench esp32c3,unstable,examples,bench`.
2. Run it: `just bench-c3 --case H1 --port /dev/cu.usbserial-210 --cycles 100`; `--case` must match the flashed `BENCH_CASE` (default cycles: 100 for W1-W3, H1, H3, H8; 10 for H2, H6, H7; 1 for H4; 1000 for H5; 20 for R1). At the end the script sends `ZZ` to tri-state the WROOM pins; send it by hand from a serial client if the run was interrupted.

A hang is no log line within the expected state-change window plus 2 s; the script then pulses the DUT's RTS (reset), logs `result=hang` for that cycle, and aborts after 5 consecutive hangs.
The summary prints verdict counts, hangs, panics, a cause histogram and the MANUAL items; exit codes are 0 pass, 1 fail, 2 infrastructure (ports, stimulus, wrong firmware).

## Cases

"both" means once per tier with the same wiring; for R1 a warm reset that keeps the counter (software reset, panic, watchdog) fails the cycle, only `reset=DeepSleep cause=Timer` proves the sleep.

| ID | Tier | Case                            | Steps                                                                                                    | Pass                                                                                                                      |
|:---|:-----|:--------------------------------|:---------------------------------------------------------------------------------------------------------|:--------------------------------------------------------------------------------------------------------------------------|
| W1 | both | GPIO4 wake                      | `try_sleep` GPIO4 `AnyHigh`; after 2 s drive GPIO4 high for 100 ms                                       | wake within 50 ms of the edge; `cause=Gpio`, `mask` has bit 4; 100 cycles                                                 |
| W2 | both | Timer fallback                  | GPIO4 plus 5 s timer, GPIO4 held low                                                                     | wake at 5 s ± 5 %; `cause=Timer`; 100 cycles                                                                              |
| W3 | both | GPIO and timer together         | GPIO4 plus 5 s timer, GPIO4 pulse at random 0-6 s (`--seed`)                                             | whichever comes first wakes, cause matches; 100 cycles                                                                    |
| H1 | both | `wake-pin-held-at-entry`        | GPIO4 held high, `try_sleep` GPIO4 `AnyHigh` plus 60 s timer                                             | record which: `err:Rejected` returned, or slept and woke at once; either outcome passes if it repeats 100 of 100           |
| H2 | hal  | `wake-cause-survives-rejection` | boot from a GPIO4 wake; read cause; GPIO4 held; one `try_sleep` (rejected); read `caused_wakeup()` again | record both reads; expected per code reading: the second read no longer names GPIO4                                       |
| H3 | hal  | `held-level-custom-handler`     | custom GPIO handler that counts calls and calls `unlisten`; GPIO4 held; `try_sleep`                      | `err:Rejected`; handler count 0 during the attempt                                                                        |
| H4 | both | `rejection-retry-loop`          | GPIO4 held for 1000 attempts, then released; next attempt sleeps; pulse GPIO4                            | 1000 × `err:Rejected`, then sleep, then `cause=Gpio`; no hang, no panic                                                   |
| H5 | both | `short-timer-sweep`             | matrix below, 1000 cycles per cell, H3 handler active                                                    | zero hangs, zero panics; record guard-to-entry per cycle                                                                  |
| H6 | hal  | Timer range limits              | `Timer` 0 ms, `MIN_TIMER_MS - 1`, `MAX_TIMER_MS + 1`                                                     | `TimerTooShort`, `TimerTooShort`, `TimerTooLong`; GPIO4 and deadline untouched (LA shows no change)                       |
| H7 | hal  | UART busy                       | enable CTS flow control on UART1, hold CTS inactive, queue 16 bytes, `try_sleep`                         | `err:UartBusy` after about 200 ms; no hang                                                                                |
| H8 | hal  | `foreign-wake-pin`              | firmware arms GPIO5 for low-power wake and listens; `try_sleep` binds only GPIO4                         | `err:ForeignWakePin{pin:5}`; then `unlisten` GPIO5, sleep, pulse GPIO5: no wake; pulse GPIO4: wake                        |
| R1 | hal  | RTC fast memory through sleep   | store a counter in the `Persistable` cell; W2 for 20 cycles                                              | every judged boot is a `DeepSleep`/`Timer` wake with the counter incremented (a warm reset with a kept counter fails)     |

H5 matrix (one build per cell, annotated with `--bench-gpio`, `--timer-ms`, `--method`, `--uart-q`):

| Sources          | Timer (ms)   | UART0 at the call                  | Methods              |
|:-----------------|:-------------|:-----------------------------------|:---------------------|
| timer only       | 1, 2, 5, 10  | idle; 128 bytes queued at 115200   | `try_sleep`, `sleep` |
| GPIO4 plus timer | 1, 2, 5, 10  | idle; 128 bytes queued at 115200   | `try_sleep`, `sleep` |

H5 measurement: guard-to-sleep-entry is the time from the GPIO10 marker edge to the VBUS current step of the CPU powering down on the PPK2 trace (the firmware's guard-to-return on the rejected path is an upper bound); record `drain_us` and the UART0 TX burst end on LA ch3 to confirm the drain finished before the marker.

## Manual verification

Serial timestamps cannot judge sub-50 ms or ±5 % timing; the script passes on return codes, hang counts and panic absence and prints these as a `MANUAL:` list:

| Item                  | Case | Measurement                                                                            | Tool                       |
|:----------------------|:-----|:---------------------------------------------------------------------------------------|:---------------------------|
| W1 edge-to-boot 50 ms | W1   | WROOM GPIO25 rising edge (LA ch0) to the C3 boot line on UART0 TX (LA ch3)             | LA ch0/2/3                 |
| W2 timer ±5 %         | W2   | expected 5 s sleep ±5 % between boot lines; serial dt is indicative only               | LA ch3 or serial timestamps|
| W3 wake-to-edge       | W3   | pulse edge to wake                                                                     | LA ch0/3                   |
| H5 guard-to-entry     | H5   | GPIO10 marker edge (PPK2 D0) to the VBUS current step; sets `POST_ARM_MARGIN_US` and `MIN_TIMER_MS` (ADR-001 § 4c) | PPK2 digital + current, LA ch2 |
| H6 pins untouched     | H6   | GPIO4 and the deadline unchanged by both rejected calls                                | LA                         |

## Results

One CSV per case in `tmp/bench/<YYYY-MM-DD>/<case>-hal.csv` (numbered suffix if it exists, `--out` overrides) with the raw `.log` beside it; columns are the log-line fields plus `host_ts`, `host_cycle`, `dt_sleep_to_boot_s`, `stim_event`, `note` and `verdict`.
