# Bench — Heltec WiFi LoRa 32 V3 (ESP32-S3)

Console on UART0 through the CP2102 bridge, so the port stays up through deep sleep.
No wiring for `retention` and `rail-gate`; `battery` needs a LiPo on the JST-1.25 connector.

> **Check the battery's polarity with a meter before connecting it.**
> MakerFocus packs are wired opposite to the Heltec connector, and the board has no reverse-polarity protection (`docs/project-lore.md` → Hardware).

| Check           | Examples                                                              | Verdict                                                     |
|:----------------|:----------------------------------------------------------------------|:------------------------------------------------------------|
| `retention`     | `idf_s3_rtc_retention`                                                | two `RET reset=DeepSleep wake=Timer load=ok` lines          |
| `rail-gate`     | `hal_s3_power_rail_gate`, `idf_s3_power_rail_gate`                    | rail cycles, sleeps with the rail off, timer wake           |
| `battery`       | `hal_s3_battery`, `idf_esp32s3_battery`                              | three `Battery: <mV>` lines, voltage matches a meter        |
| `battery-calib` | `hal_s3_battery+calib`, `idf_esp32s3_battery+calib`                   | three non-zero `Calib:` lines; attended: meter within 1 % of `comp` on both tiers |

Most steps wait for Enter before they start timing; read the instruction, press Enter, then act.
`battery-calib` starts on its own and gives you a 60 s window to read the meter.

```shell
just bench heltec
```

## If it hangs

- With USB and no battery, `battery` reads the charger rail; the no-cell reading is ambiguous (false ~80 % or `No battery`, by board/run) — that is not a failure.
- A run printing `USB/Ext` instead of a voltage means the compensated reading exceeds the USB-detection threshold and `divider_ratio` is off for this unit; recalibrate with `battery-calib` before re-running `battery`.
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- `hal_s3_rtc_retention` does not exist yet, so `retention` runs the IDF tier only.
