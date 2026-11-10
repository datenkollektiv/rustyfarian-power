# Bench — Adafruit ESP32 Feather V2

Console on UART0 through the on-board USB bridge, so the port stays up through deep sleep.
`battery` needs a LiPo on the JST-PH connector and, attended, a meter on the BAT and GND header pins with USB in, read while the board prints (a pack metered unplugged rests about 150 mV lower); `sleep` needs no wiring (GPIO25 uses its internal pull-down).
`ext1-wake` needs one jumper wire from A1 (GPIO25) to **3V3** (never 5V/USB), moved by the operator; `anylow` the same wire to **GND**; `anylow-multi` needs no wiring.

| Check     | Examples                                  | Verdict                                       |
|:----------|:------------------------------------------|:----------------------------------------------|
| `battery` | `hal_esp32_battery`, `idf_esp32_battery`  | a `Battery:` line; attended: the shown mV within 100 mV of the meter on BAT (3.85 V vs 3898/3900 mV, 2026-11-10) |
| `sleep`     | `hal_esp32_deep_sleep_gpio`, `idf_esp32_deep_sleep_gpio` | two timer wakes, no `UartBusy`, no GPIO wake; the RTC clock continues |
| `retention` | `hal_esp32_rtc_retention`                 | two `RET reset=DeepSleep wake=Timer load=ok` lines |
| `anylow` | `{hal,idf}_esp32_deep_sleep_gpio+anylow` | unattended: timer wake only, pull-up holds; attended: jumper A1→GND: `Ext1` bit 25, held pin `Rejected`, timer wake after removal |
| `anylow-multi` | `{hal,idf}_esp32_deep_sleep_gpio+anylow-multi` | `try_sleep failed: … kind=Unsupported` (`AnyLowMultiplePins`), then the fallback timer wake |
| `ext1-wake` | `hal_esp32_deep_sleep_gpio`, `idf_esp32_deep_sleep_gpio` | jumper A1→3V3: `Ext1` bit 25 wake, held pin `Rejected`, no wake loop after removal |

```shell
just bench esp32
```

## If it hangs

- With USB and no battery, the charger holds BAT at ~4.16 V and reads a false ~96 %; that is expected, not a failure (`docs/key-insights.md`).
- `idf_esp32_battery` prints one reading per boot, then deep-sleeps 60 s.
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open


- Charging status: the stock Feather V2 has no charge-status GPIO (`docs/project-lore.md` → Hardware — Adafruit ESP32 Feather V2).
