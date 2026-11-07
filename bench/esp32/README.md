# Bench — Adafruit ESP32 Feather V2

Console on UART0 through the on-board USB bridge, so the port stays up through deep sleep.
`battery` needs a LiPo on the JST-PH connector.

| Check     | Examples                                  | Verdict                                       |
|:----------|:------------------------------------------|:----------------------------------------------|
| `battery` | `hal_esp32_battery`, `idf_esp32_battery`  | a `Battery:` line, voltage matches a meter    |

```shell
just bench esp32
```

## If it hangs

- With USB and no battery, the charger holds BAT at ~4.16 V and reads a false ~96 %; that is expected, not a failure (`docs/key-insights.md`).
- `idf_esp32_battery` prints one reading per boot, then deep-sleeps 60 s.
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- Charging status: the stock Feather V2 has no charge-status GPIO (`docs/project-lore.md` → Hardware — Adafruit ESP32 Feather V2).
