# Bench — ESP32-C3-MINI-1 / DevKitM-1

Console on UART0 through the USB bridge (CH340 or CP2102N), so the port stays up through deep sleep.
Wiring for `pir-wake` only: HC-SR501 OUT on GPIO4 (10 kΩ pull-down to GND), VCC on 5V, GND on GND.

| Check       | Examples                                          | Verdict                                                  |
|:------------|:--------------------------------------------------|:---------------------------------------------------------|
| `retention` | `hal_c3_rtc_retention`, `idf_c3_rtc_retention`    | two `RET reset=DeepSleep wake=Timer load=ok` lines       |
| `pir-wake`  | `hal_c3_deep_sleep_gpio`, `idf_c3_deep_sleep_gpio` | unattended: a timer wake, no GPIO wake; attended: a `Wake cause: Gpio`, the held pin `Rejected`, then 20 s without a wake loop |

Each step waits for Enter before it starts timing; read the instruction, press Enter, then act.

```shell
just bench c3
```

## If it hangs

- A CP210x port open can reset the board, so the first `RET` line may show `reset=PowerOn`; the checks judge the timer wakes only.
- Let the PIR warm up (about 60 s after power-on) before the `pir-wake` wave step; set its hold-time pot low.
- An unattended `pir-wake` that fails on `Wake cause: Gpio` with `GPIO4 level before sleep: High` at power-on saw the PIR fire; it is inconclusive, not a failure — rerun with nobody in view.
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- SuperMini: espflash's USB-Serial-JTAG reset parks it in download mode (`docs/project-lore.md` → Bench tooling — ESP32-C3 SuperMini), so it needs a UART adapter and a manual power cycle.
- Retention across restart, panic and watchdog resets needs `C3_RET_ACTION`, which the runner cannot set.
