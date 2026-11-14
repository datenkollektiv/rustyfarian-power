# Bench — ESP32-C3-DevKitM-1 (ESP32-C3-MINI-1)

Console on UART0 through the USB bridge (CH340 or CP2102N), so the port stays up through deep sleep.

| Check       | Examples                        | Wiring                                   | A pass proves                                                                   |
|:------------|:--------------------------------|:-----------------------------------------|:-------------------------------------------------------------------------------|
| `retention` | `{hal,idf}_c3_rtc_retention`    | none                                     | two `RET reset=DeepSleep wake=Timer load=ok` lines, no `load=err`               |
| `pir-wake`  | `{hal,idf}_c3_deep_sleep_gpio`  | HC-SR501 OUT on GPIO4, 10 kΩ to GND; 5V  | quiet: timer wake, no `Gpio`; wave: `Gpio` wake, held pin `Rejected`, no loop   |

`pir-wake` splits by mode.
`--unattended` proves the 60 s timer wake with nobody in view.
`--attended` proves the PIR wake, the rejection while the PIR output is still HIGH, and 20 s without a GPIO wake loop afterwards.
No step waits for Enter: each starts timing at once, so read the instruction and act.

```shell
just bench c3-mini1
```

## If it hangs

- A first `RET` line with `reset=PowerOn` is the port open resetting the board (`docs/project-lore.md` → Bench tooling — C3 bench driver); the checks judge the timer wakes only.
- Let the PIR warm up (about 60 s after power-on) before the `pir-wake` wave step; set its hold-time pot low.
- An unattended `pir-wake` that fails on `Wake cause: Gpio` with `GPIO4 level before sleep: High` at power-on saw the PIR fire; it is inconclusive, not a failure — rerun with nobody in view.
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- SuperMini: espflash's reset parks it in download mode (`docs/project-lore.md` → Bench tooling — ESP32-C3 SuperMini), so it needs a UART adapter and a manual power cycle.
- Retention across restart, panic and watchdog resets needs `C3_RET_ACTION`, which the runner cannot set.
