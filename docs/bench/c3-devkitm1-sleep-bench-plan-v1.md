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
| P2       | Open questions in the feature docs; retention                | I4-I5, R1-R5        |

ADR-001 moves to Accepted when every P0 and P1 case passes and `MIN_TIMER_MS` and `POST_ARM_MARGIN_US` are set from H5.

## Prerequisites (not yet built)

- Hal tier: the sleep module from ADR-001, replacing `spike_pin_borrow.rs`, with a `bench` cargo feature that drives the marker pin at the post-arm guard.
- IDF tier: the C3 GPIO wake path and `try_sleep` from `c3-gpio-deep-sleep-wake-v1`.
- Bench firmware per tier: `hal_c3_sleep_bench` and `idf_c3_sleep_bench`, case selected at build time by `BENCH_CASE`, one flash per case.
- `just` recipes: `build-example-hal-c3 <name>`, `flash-hal-c3 <name>`, `flash-c3 <name>` (IDF), each accepting `BENCH_CASE`.
- Host script `scripts/c3-bench.py`: reads the serial log, drives the stimulus, detects hangs, writes one CSV row per cycle.

## Setup

### Instruments

- ESP32-C3 DevKitM-1, powered and logged over its micro-USB port (CP2102N bridge on UART0, GPIO20/21).
- Stimulus: any host-controlled 3.3 V GPIO source, for example a spare RP2040 board running a serial-to-GPIO sketch, two channels.
- Logic analyser, 4 channels, at least 4 MS/s.
- Current: Nordic PPK2 in ampere-meter mode in series with VBUS through a USB data pass-through breakout, so serial keeps working; its digital inputs record the marker on the same time base.
- Without a PPK2, H5 falls back to the rejected-path upper bound (see H5) and the plan records that.

### Wiring

| Signal       | C3 pin | Connection                                            | Note                               |
|:-------------|:-------|:------------------------------------------------------|:-----------------------------------|
| Wake (PIR)   | GPIO4  | stimulus ch1 via 1 kΩ; 10 kΩ pull-down to GND; LA ch0 | MTMS (JTAG TMS)                    |
| Foreign pad  | GPIO5  | stimulus ch2 via 1 kΩ; 10 kΩ pull-down to GND; LA ch1 | H8 only                            |
| Guard marker | GPIO10 | LA ch2 and PPK2 D0                                    | not a low-power pad, not strapping |
| UART0 TX     | GPIO21 | LA ch3 (already wired to the bridge)                  | H5 drain timing                    |
| GND          | GND    | stimulus, LA and PPK2 ground                          | one common ground                  |

Avoid GPIO2, GPIO8 and GPIO9 (strapping; GPIO8 also drives the RGB LED, GPIO9 the BOOT button).
Meter the stimulus at 3.3 V before connecting; the C3 pads are not 5 V tolerant.

### Firmware log line

Every boot and every `try_sleep` return prints one line on UART0:

```text
BENCH case=<id> tier=<hal|idf> cycle=<n> reset=<reason> cause=<cause> mask=0x<hex> result=<ok|err:<variant>> guard_us=<n> drain_us=<n>
```

`cycle` comes from the RTC retention record when R1 has passed; until then the host script counts boots and treats the firmware value as advisory.

### Host procedure per case

```shell
just flash-hal-c3 hal_c3_sleep_bench BENCH_CASE=H1
python3 scripts/c3-bench.py --case H1 --port /dev/cu.usbserial-0001 --cycles 100
```

A hang is no log line within the expected sleep time plus 2 s; the script then resets the board through the bridge's RTS line and logs `result=hang`.
A panic is any line from the panic handler; the script logs it and continues.

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

| ID | Tier | Case                           | Steps                                                                                | Pass                                                                  |
|:---|:-----|:-------------------------------|:-------------------------------------------------------------------------------------|:----------------------------------------------------------------------|
| I1 | idf  | Reject maps to `Rejected`      | as H1                                                                                | `kind() == Rejected` from the `esp_deep_sleep_try_to_start` call site |
| I2 | idf  | Empty sources                  | `sleep(&[])`, `try_sleep(&[])`                                                       | `err:NoWakeSource` both; chip stays awake                             |
| I3 | idf  | Stale GPIO mask                | arm GPIO4, `try_sleep` rejected; then `try_sleep` timer-only 5 s; pulse GPIO4 at 2 s | no wake at 2 s; `cause=Timer` at 5 s                                  |
| I4 | idf  | Isolation with a held wake pin | `isolate_gpio` on, GPIO4 wake; repeat W1                                             | record whether W1 still passes; answers the C3 doc's open question    |
| I5 | idf  | Mask outside `0x3F`            | `GpioLevel` with bit 6                                                               | `err:GpioMaskOutOfRange`; no SDK call made (no IDF error log)         |

### Retention (P2)

| ID | Tier | Case                               | Steps                                                       | Pass                                                                    |
|:---|:-----|:-----------------------------------|:------------------------------------------------------------|:------------------------------------------------------------------------|
| R1 | hal  | RTC fast memory through deep sleep | store a counter in the `Persistable` cell; W2 for 20 cycles | counter increments every boot; answers the `rtc_fastmem_pd_en` question |
| R2 | both | GPIO and timer wake                | as W1/W2 with the envelope                                  | `load` returns the stored value every boot                              |
| R3 | both | Watchdog and software reset        | trigger RWDT, then `software_reset`                         | `load` succeeds after both                                              |
| R4 | both | Power cycle                        | unplug USB for 5 s                                          | `load` returns `Invalid::PowerOn`                                       |
| R5 | both | Brownout                           | setup to be decided, see risks                              | `load` returns `Invalid::Brownout`                                      |

## Results

One CSV per case and tier in `tmp/bench/<date>/<case>-<tier>.csv`, columns from the log line plus `host_ts`.
A summary table per run goes into the feature docs' session logs, and constants from H5 into ADR-001 § 4c.

## Setup risks to resolve before running

- R5 needs the supply below the brownout threshold while serial still works; check the DevKitM-1 schematic for how the CP2102N is powered before cutting VBUS or feeding the 5 V pin.
- The VBUS current step in H5 sits on top of the bridge and LDO draw; confirm on the first cycle that the CPU power-down step is resolvable before running 1000.
- GPIO4 and GPIO5 are JTAG pads; on the default eFuse settings JTAG uses the built-in USB-JTAG, so these pads are free, but check that no probe is attached.
