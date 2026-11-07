# Bench — Heltec WiFi LoRa 32 V3 (ESP32-S3)

Console on UART0 through the CP2102 bridge, so the port stays up through deep sleep.
No wiring for `retention` and `rail-gate`; `battery` needs a LiPo on the JST-1.25 connector.

> **Check the battery's polarity with a meter before connecting it.**
> MakerFocus packs are wired opposite to the Heltec connector, and the board has no reverse-polarity protection (`docs/project-lore.md` → Hardware).

| Check       | Examples                                           | Verdict                                                  |
|:------------|:---------------------------------------------------|:---------------------------------------------------------|
| `retention` | `idf_s3_rtc_retention`                             | two `RET reset=DeepSleep wake=Timer load=ok` lines       |
| `rail-gate` | `hal_s3_power_rail_gate`, `idf_s3_power_rail_gate` | rail cycles, sleeps with the rail off, timer wake        |
| `battery`   | `hal_s3_battery`, `idf_esp32s3_battery`            | three `Battery:` lines, voltage matches a meter          |

Each step waits for Enter before it starts timing; read the instruction, press Enter, then act.

```shell
just bench heltec
```

## If it hangs

- With USB and no battery, `battery` reads the charger rail as a false ~80 %; that is expected, not a failure (`docs/key-insights.md`).
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- `hal_s3_rtc_retention` does not exist yet, so `retention` runs the IDF tier only.
