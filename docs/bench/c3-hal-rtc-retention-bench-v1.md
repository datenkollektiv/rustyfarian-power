# Bench Record: ESP32-C3 esp-hal Tier RTC Retention v1

Run 2026-10-16 on an ESP32-C3-DevKitM-1 (CP2102N on `/dev/cu.usbserial-210`) with `hal_c3_rtc_retention`, the bare-metal twin of the IDF example.
It checks the esp-hal half of [RTC retention v1](../features/rtc-retention-v1.md) and answers bench case R1 of the [C3 sleep bench plan](c3-devkitm1-sleep-bench-plan-v1.md): an `RtcRetainedCell<16>` in `.rtc_fast.persistent` through esp-hal 1.2.2 deep sleep.
No wiring is needed.

## Setup

Two builds, flashed in turn and read with the timed capture:

```shell
C3_RET_ACTION=cycle just flash-hal-c3 hal_c3_rtc_retention
just serial-capture /dev/cu.usbserial-210 60
C3_SLEEP_TIMER_MS=2000 just flash-hal-c3 hal_c3_rtc_retention
just serial-capture /dev/cu.usbserial-210 75
```

Every boot prints the same line as the IDF twin:

```text
RET reset=<ResetReason> wake=<WakeCause> load=<ok|err:<Invalid>> boots=<n> session=0x<id>
```

## Placement

`riscv32-esp-elf-nm -n -S` on the release ELF shows `RETAINED` at `0x50000000`, 28 bytes, spanning `_rtc_fast_persistent_start` to `_rtc_fast_persistent_end`.
`objdump -h` lists `.rtc_fast.persistent` with `ALLOC` only, so the section is `NOLOAD`.

## Results

| Boot ends with                            | Next boot reports | Record  | Session | Counter |
|:------------------------------------------|:------------------|:--------|:--------|:--------|
| Flash, then power-on                      | `reset=PowerOn`   | refused | new     | 1       |
| Deep sleep, 5 s timer                     | `reset=DeepSleep` | kept    | same    | +1      |
| `software_reset()`                        | `reset=Software`  | kept    | same    | +1      |
| `panic!` (handler calls `software_reset`) | `reset=Software`  | kept    | same    | +1      |
| TIMG0 main watchdog, 500 ms               | `reset=Watchdog`  | kept    | same    | +1      |
| Port open (EN pulse, reads as power-on)   | `reset=PowerOn`   | refused | new     | 1       |

Cycle build: four full cycles in 60 s without a `load=err` on a warm boot; the counter reached 16 with one session id.
The ROM reports the watchdog as `TG0WDT_SYS_RST` and esp-hal as `CoreMwdt0`.

R1, pure sleep build with a 2 s timer: 23 consecutive deep-sleep wakes in 75 s, every one `load=ok` with the same session id, counter 2 to 24.
RTC fast memory is retained through esp-hal deep sleep on the C3 even though `base_settings` clears `FASTMEM_FORCE_LPU`; no force-on workaround is needed.

The CP210x port open resets the board through EN, so every capture starts with a power-on boot and a fresh session, as on the IDF tier.

## Differences from the IDF tier

- `panic` reads `reset=Software`: bare-metal esp-hal records no panic reset, and the handler restarts through `software_reset()`.
- The `wdt` action arms the TIMG0 watchdog itself because `esp_hal::init` disables every watchdog.
- The runtime zeroes `.rtc_fast.persistent` on power-on; `load` refuses that boot by reset reason anyway.

## Not covered

- GPIO wake: the example has no wake pin; the record does not depend on the wake source.
- Brownout: needs a bench supply; the reset gate covers it and the host tests prove the refusal.
- ESP32-S3: the types build under the `esp32s3` feature, but no S3 board was benched.

Re-run 2026-10-16 after `RetainedCell` moved its volatile copies into `critical_section::with` (review finding P1): three cycles, every warm boot `load=ok`, one session id; behaviour unchanged.
