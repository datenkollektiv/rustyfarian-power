# Bench Record: ESP32-C3 IDF Tier RTC Retention v1

Run 2026-10-16 on an ESP32-C3-DevKitM-1 (CP2102N on `/dev/cu.usbserial-210`) with `idf_c3_rtc_retention` built as `C3_RET_ACTION=cycle` and the default 5 s timer.
It checks the IDF half of [RTC retention v1](../features/rtc-retention-v1.md): a `RetainedCell<16>` in `.rtc_noinit` carrying a session id, a boot counter and the previous boot's reset code.
No wiring is needed.

## Setup

Flash with the cycle action, then capture the UART without opening the espflash monitor:

```shell
C3_RET_ACTION=cycle just flash idf_c3_rtc_retention
just serial-capture /dev/cu.usbserial-210 50
```

Every boot prints one line in this format:

```text
RET reset=<ResetReason> wake=<WakeCause> load=<ok|err:<Invalid>> boots=<n> session=0x<id>
```

## Placement

`riscv32-esp-elf-nm -n -S` on the release ELF shows `RETAINED` at `0x50000018`, 28 bytes, between `_rtc_noinit_start` and `_rtc_noinit_end`.
`objdump -h` lists `.rtc_noinit` with `ALLOC` only, so the section is `NOLOAD` and the zeroed initialiser never reaches flash.

## Results

| Boot ends with                       | Next boot reports | Record  | Session | Counter |
|:-------------------------------------|:------------------|:--------|:--------|:--------|
| Flash, then power-on                 | `reset=PowerOn`   | refused | new     | 1       |
| Deep sleep, 5 s timer                | `reset=DeepSleep` | kept    | same    | +1      |
| `esp_restart()`                      | `reset=Software`  | kept    | same    | +1      |
| `panic!` (reboots in this sdkconfig) | `reset=Panic`     | kept    | same    | +1      |
| Interrupt watchdog (interrupts off)  | `reset=Watchdog`  | kept    | same    | +1      |
| USB unplug and replug                | `reset=PowerOn`   | refused | new     | 1       |

Three full cycles ran without a single `load=err` on a warm boot; the counter reached 11 with one session id.
After the USB power cycle the first captured boot was a power-on with a new session id and the counter at 1, and the following cycle kept that session.
Opening the CP210x port resets the DevKitM-1 through EN even with DTR and RTS held low; every capture started with such a power-on boot, so the replug and the port open are indistinguishable here.
ESP-IDF reports both as `ESP_RST_POWERON`, the record is refused the same way, and the firmware path is the one under test.

## Not covered

- GPIO wake: the example has no wake pin; the PIR example covers the wake path and the record is independent of the wake source.
- Brownout: needs a bench supply that can dip the rail; `load` refuses it by reset reason, which the host tests cover.
- The task watchdog cannot be used for a reset here because `CONFIG_ESP_TASK_WDT_PANIC` is unset in the generated sdkconfig; it only warns.

Re-run 2026-10-16 after `RetainedCell` moved its volatile copies into `critical_section::with` (review finding P1): three cycles, every warm boot `load=ok`, one session id; behaviour unchanged.

## RTC clock (`EspRtcClock::now_us`, follow-up item 2)

Cycle build on 2026-10-16, `RTC rtc_us=<n>` printed after each `RET` line; the delta is between consecutive boots.

| Boot ends with        | Next boot reads   | Delta to previous boot                |
|:----------------------|:------------------|:--------------------------------------|
| Flash, then power-on  | `rtc_us=129805`   | starts near zero                      |
| Deep sleep, 5 s timer | `rtc_us=6508678`  | 6.38 s (5 s sleep + 1 s delay + boot) |
| `esp_restart()`       | `rtc_us=7890197`  | 1.38 s                                |
| `panic!` (reboot)     | `rtc_us=9688057`  | 1.80 s                                |
| Interrupt watchdog    | `rtc_us=11780349` | 2.09 s                                |

Three cycles gave the same deltas within 1 ms, and the clock never went backwards on a warm reset.
It restarts only with the power-on at the top of a capture.
