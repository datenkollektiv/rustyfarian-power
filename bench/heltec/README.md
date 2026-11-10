# Bench — Heltec WiFi LoRa 32 V3 (ESP32-S3)

Console on UART0 through the CP2102 bridge, so the port stays up through deep sleep.

> **Check the battery's polarity with a meter before connecting it.**
> MakerFocus packs are wired opposite to the Heltec connector, and the board has no reverse-polarity protection (`docs/project-lore.md` → Hardware — Heltec V3 & LiPo batteries).

| Check           | Examples                               | Wiring                                | A pass proves                                                              |
|:----------------|:---------------------------------------|:--------------------------------------|:---------------------------------------------------------------------------|
| `retention`     | `idf_s3_rtc_retention`                 | none                                  | two `RET reset=DeepSleep wake=Timer load=ok` lines, no `load=err`          |
| `rail-gate`     | `{hal,idf}_s3_power_rail_gate`         | none                                  | rail cycles, `event=sleep powered=false`, then the 30 s timer wake         |
| `rail-gate-ve`  | `{hal,idf}_s3_power_rail_gate+meter`   | meter from `Ve` to GND                | attended: `Ve` reads about 3.3 V awake and about 0 V asleep (two confirms) |
| `battery`       | `hal_s3_battery`, `idf_esp32s3_battery`| LiPo on JST-1.25                      | three `Battery: <mV>` lines                                                |
| `battery-calib` | `{hal,idf_esp32s3}_battery+calib`      | LiPo on JST-1.25; meter at the pack   | three non-zero `Calib:` raws; attended: meter within 40 mV of `comp`      |

`battery-calib` examples are `hal_s3_battery+calib` and `idf_esp32s3_battery+calib`.
Most steps wait for Enter before they start timing; read the instruction, press Enter, then act.
`battery-calib` starts on its own; `rail-gate-ve` asks for one Enter once the meter is on `Ve` and GND, then restarts the board.
Both announce the board's state and give you a 60 s window per measurement.

```shell
just bench heltec
```

## If it hangs

- `battery` and `battery-calib` must be judged with a cell fitted; the no-cell reading on USB is not a failure (`docs/project-lore.md` → Hardware — Heltec V3 & LiPo batteries).
- A run printing `USB/Ext` instead of a voltage means `divider_ratio` is off for this unit; recalibrate with `battery-calib` before re-running `battery`.
- A missed `rail-gate-ve` meter window cannot be retried in place; answer no, decline the retry, and run the scenario again.
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- `hal_s3_rtc_retention` does not exist yet, so `retention` runs the IDF tier only.
