# Bench — Adafruit ESP32 Feather V2

Console on UART0 through the on-board USB bridge, so the port stays up through deep sleep.
The jumper wire for `ext1-wake` goes from A1 (GPIO25) to **3V3**, never to 5V/USB; for `anylow` the same wire goes to **GND**.
Meter and board must read the same net at the same moment: BAT and GND header pins with USB in, while the board prints (a pack metered unplugged rests about 150 mV lower).

| Check          | Examples                                   | Wiring                              | A pass proves                                                                  |
|:---------------|:-------------------------------------------|:------------------------------------|:-------------------------------------------------------------------------------|
| `battery`      | `{hal,idf}_esp32_battery`                  | LiPo on JST-PH; meter on BAT/GND    | a `Battery:` line; attended: meter within 100 mV of the printed mV             |
| `sleep`        | `{hal,idf}_esp32_deep_sleep_gpio`          | none (GPIO25 internal pull-down)    | two timer wakes, no `UartBusy`, no `Ext1`; `rtc_us` ≥ 10 s after a wake        |
| `retention`    | `hal_esp32_rtc_retention`                  | none                                | two `RET reset=DeepSleep wake=Timer load=ok` lines, no `load=err`              |
| `anylow`       | `{hal,idf}_esp32_deep_sleep_gpio+anylow`   | jumper A1 to GND, moved by hand     | quiet: timer only, pull-up holds; wire: `Ext1` bit 25, held `Rejected`, timer  |
| `anylow-multi` | `{hal,idf}_esp32_deep_sleep_gpio+anylow-multi` | none                            | `try_sleep failed: … kind=Unsupported`, then the fallback timer wake           |
| `ext1-wake`    | `{hal,idf}_esp32_deep_sleep_gpio`          | jumper A1 to 3V3, moved by hand     | `Ext1` bit 25 wake; held pin `Rejected` 20+ times; timer wake after release    |

The wire steps of `anylow` and `ext1-wake` start at once (no Enter); each expects a fresh wake, so earlier lines cannot satisfy it.

```shell
just bench esp32
```

## If it hangs

- `battery` must be judged with a cell fitted; the no-battery reading on USB is not a failure (`docs/project-lore.md` → Hardware — Adafruit ESP32 Feather V2).
- `idf_esp32_battery` prints one reading per boot, then deep-sleeps 60 s; the attended step sees the boot reading, so the single line counts.
- Ctrl-C ends a run; it is recorded as `aborted`.

## Open

- Charging status: the stock Feather V2 has no charge-status GPIO (`docs/project-lore.md` → Hardware — Adafruit ESP32 Feather V2).
