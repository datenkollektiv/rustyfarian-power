# Bench — CrowPanel 1.28" HMI (ESP32-S3)

Console on the S3's native USB-Serial-JTAG: the port drops in deep sleep and comes back after the wake; the runner reopens it.
No wiring for any check; the touch screen is the stimulus of `touch-wake`, and the `r*` checks make their own stimulus in firmware.
`bench.toml` pins this board by its MAC (`1c:db:d4:b3:6d:40`, as espflash reports it; the USB serial number differs), so the Heltec V3 (also an ESP32-S3) can stay attached; pass `--port` only while the PPK2 is plugged in.
The `r*` cases were first run by hand on 2026-11-03/04 (git history of `docs/bench/s3-crowpanel-touch-sleep-bench-steps-v1.md`, removed 2026-11-09); the scenario comments carry the reasoning.

| Check        | Examples                                                   | Verdict                                                                 |
|:-------------|:-----------------------------------------------------------|:------------------------------------------------------------------------|
| `sleep`      | `hal_s3_deep_sleep_gpio`, `idf_s3_deep_sleep_gpio`         | two 60 s timer wakes with new cycle numbers and `SLEPT` ≈ 60 s, no touch wake |
| `touch-wake` | same                                                       | attended: a tap wakes with `cause=Ext1 mask=0x20`; a moving finger re-wakes twice without a rejection; then the timer |
| `r2`         | `{hal,idf}_s3_deep_sleep_gpio+r2`                          | forced rejection: INT `L` at the read, `L->H` after the restore, recount 5 s on forced and plain cycles |
| `r3`         | `{hal,idf}_s3_deep_sleep_gpio+r3`                          | pad hold kept on: the guard rejects (`autohold=1`), no watchdog reset   |
| `r4`         | `{hal,idf}_s3_deep_sleep_gpio+r4`                          | probe pads and holds unchanged across a hardware-path rejection (`same=1 holdok=1`) |
| `r4-guard`   | `{hal,idf}_s3_deep_sleep_gpio+r4-guard`                    | the same across a guard-path rejection                                  |

The `r*` variants set the 5 s wake timer; `sleep` and `touch-wake` use the stock 60 s timer.
`touch-wake` asks for one Enter before the resting-finger step; the other steps start on their own.

```shell
just bench s3
```

## If it hangs

- A `rst:0x8 TG1WDT_SYS_RST` on an `r3` or `r4-guard` forced cycle is the hang the guard exists to prevent (`docs/project-lore.md` → ESP-IDF tier — ESP32-S3 deep sleep); it fails the step.
- A serial reader that lowers DTR before RTS resets this board on every reopen; the runner leaves both alone (`docs/project-lore.md` → Bench tooling).
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- Deep-sleep current and wake latency are not measured here.
- The R4 probe pads (GPIO38, 39, 47) are assumed unwired; the CrowPanel schematic is not verified.
