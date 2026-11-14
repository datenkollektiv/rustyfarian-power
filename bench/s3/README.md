# Bench — CrowPanel 1.28" HMI (ESP32-S3)

Console on the S3's native USB-Serial-JTAG: the port drops in deep sleep and comes back after the wake; the runner reopens it.
No wiring for any check; the touch screen is the stimulus of `touch-wake`, and the `r*` checks make their own stimulus in firmware.
`bench.toml` pins this board by its MAC (`1c:db:d4:b3:6d:40`, as espflash reports it; the USB serial number differs), so the Heltec V3 (also an ESP32-S3) can stay attached.
Pass `--port` only while the PPK2 is plugged in.

| Check        | Examples                              | Wiring | A pass proves                                                                  |
|:-------------|:--------------------------------------|:-------|:-------------------------------------------------------------------------------|
| `sleep`      | `{hal,idf}_s3_deep_sleep_gpio`        | none   | two timer wakes with new cycle numbers, `SLEPT` 59-65 s, no `Ext1`             |
| `touch-wake` | `{hal,idf}_s3_deep_sleep_gpio`        | none   | tap: `Ext1 mask=0x20`; moving finger re-wakes twice, no `ATTEMPT n≥2`; timer   |
| `r2`         | `{hal,idf}_s3_deep_sleep_gpio--r2`     | none   | `FORCED autohold=0`: INT `L` at the read, `L->H` after; `SLEPT` 5.0-5.7 s both |
| `r3`         | `{hal,idf}_s3_deep_sleep_gpio--r3`     | none   | `FORCED autohold=1`: guard rejects, no `rst:0x8`; plain `SLEPT` 5.0-5.7 s      |
| `r4`         | `{hal,idf}_s3_deep_sleep_gpio--r4`     | none   | `selftest detect=1`; hardware-path rejection leaves `same=1 holdok=1`          |
| `r4-guard`   | `{hal,idf}_s3_deep_sleep_gpio--r4-guard` | none | `selftest detect=1`; guard-path rejection leaves `same=1 holdok=1`            |

`r2` additionally accepts an IDF-only `MASKRETRY tier=idf result=err:Rejected` line as an optional step.
The `r*` variants set the 5 s wake timer; `sleep` and `touch-wake` use the stock 60 s timer.
Every step starts on its own; the finger step of `touch-wake` needs a new cycle number, so the tap's lines cannot satisfy it.

```shell
just bench s3
```

## If it hangs

- A `rst:0x8 TG1WDT_SYS_RST` on an `r3` or `r4-guard` forced cycle is the hang the guard exists to prevent (`docs/project-lore.md` → ESP-IDF tier — ESP32-S3 deep sleep); it fails the step.
- A serial reader that lowers DTR before RTS resets this board on every reopen; the runner leaves both alone (`docs/project-lore.md` → Bench tooling — USB-Serial-JTAG boards).
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- Deep-sleep current and wake latency are not measured here.
- The R4 probe pads (GPIO38, 39, 47) are assumed unwired; the CrowPanel schematic is not verified.
