# Bench Record: ESP32-S3 CrowPanel Touch Wake, Both Tiers v1

2026-10-14, CrowPanel 1.28" HMI ESP32-S3 (rev v0.2, 16 MB flash), no external wiring.
Firmware: the uncommitted working tree on top of `ea40d22`, ESP-IDF v5.3.3 and esp-hal `=1.2.2`.
Examples `idf_s3_deep_sleep_gpio` and `hal_s3_deep_sleep_gpio`, sleeping on GPIO5 `AnyLow` plus a 60 s timer.

## How it was run

The bench builds carried extra logging (a boot `BENCH` line, a 2 s console wait, per-attempt rejection lines, and a hal switch that skipped the touch bring-up) and a passive serial logger.
That tooling was removed after the run; the committed examples are lightweight demos.

- The board's USB-C reaches the S3's native USB-Serial-JTAG (`/dev/cu.usbmodem*`), not a CH340 as the knob docs say.
- The port drops off the bus in deep sleep and re-enumerates on wake; boot output sent before the host reopens it is lost.
- `espflash monitor` (4.3) cannot follow that without resetting the chip; see `docs/project-lore.md` → Bench tooling.
- A sleeping board, or one spinning in a rejection loop, is flashed from download mode: unplug, hold BOOT, plug in, release.
- Sleep duration was read from host timestamps of the port vanishing and reappearing.

## Results

| Case                         | IDF tier                              | esp-hal tier                          |
|:-----------------------------|:--------------------------------------|:--------------------------------------|
| B0 cold boot                 | GPIO5 HIGH, sleeps                    | GPIO5 HIGH, sleeps                    |
| W1 touch wake ×3             | 3/3 `Ext1 mask=0x20`                  | 3/3 `Ext1 mask=0x20`                  |
| W2 timer wake                | 60.1 s, `Timer`                       | 60.1 s, `Timer`                       |
| H1 finger held at entry      | wake loop, 6 wakes in ~14 s, no reject | wake loop, 6 wakes in ~12 s, no reject |
| H1-level INT held LOW        | >800 `Rejected`, no boot loop         | 675 `Rejected` in 34 s, no boot loop  |
| GPIO41/42/45 in the mask     | not run on the board                  | not run on the board                  |

## B0: touch bring-up and GPIO5 idle level

Without the bring-up, the CST816S is unpowered and holds INT LOW: GPIO5 read LOW at boot and every `try_sleep` was rejected.
With `LCD_3V3` (GPIO1) on and touch reset (GPIO13) released, both held HIGH through deep sleep, GPIO5 idles HIGH and the board sleeps.
GPIO5 also read HIGH at every entry after a deep-sleep wake, so the hold keeps the controller alive.

## W1: touch wake

Each brief tap woke the board before the timer with the EXT1 status bit for GPIO5.

```text
22:37:26.027 Wake cause: Ext1 — fired pins: 0x20, GPIO5 fired: true
22:37:26.028 BENCH case=demo tier=idf cycle=0 reset=DeepSleep cause=Ext1 mask=0x20 result=boot guard_us=0 drain_us=0
22:50:03.017 BENCH case=demo tier=hal cycle=0 reset=DeepSleep cause=Ext1 mask=0x20 result=boot guard_us=0 drain_us=0
```

## W2: timer wake

With the screen untouched, the port vanished at sleep entry and reappeared 60.1 s later on both tiers (IDF 22:36:17.4 → 22:37:17.5, hal 22:50:40.3 → 22:51:40.4).

```text
22:51:42.220 BENCH case=demo tier=hal cycle=0 reset=DeepSleep cause=Timer mask=0x0 result=boot guard_us=0 drain_us=0
```

## H1: finger resting on the screen at sleep entry

A resting finger does not hold INT LOW; the CST816S pulses it.
GPIO5 read HIGH at every entry, the board slept, and the next pulse woke it 0.3-0.6 s later, so the board looped through wake cycles until the finger lifted.
No `Rejected` appeared on either tier, and the board stayed asleep once the finger lifted.

## H1-level: INT held LOW

With the controller unpowered, INT stays LOW and the S3 hardware rejects every deep sleep on both tiers; no pre-sleep level check is needed, unlike the C2/C3.

```text
E (401) sleep: Deep sleep request is rejected
try_sleep rejected (first): deep sleep rejected: a wake source was already asserted (ESP-IDF error: 0x00000103), kind=Rejected
22:56:58.765 try_sleep rejected (first): sleep rejected: a wake source was already asserted
```

On esp-hal this ran the `sleep_deep_with_rejection` return, the pin release, `Err(Rejected)` and the retry 675 times, with no panic and USB staying up.

## GPIO mask out of range

GPIO41, 42 and 45 lie outside the S3 low-power pads (GPIO0-21), so `validate_gpio_level_source_against` rejects them with `GpioMaskOutOfRange` before any hardware call on both tiers.
Covered by the host-tested validators; not run on the board.

## What this bench cannot show

| Requirement                                         | Here                        |
|:----------------------------------------------------|:----------------------------|
| Deep-sleep current                                  | not measured                |
| Wake latency                                        | not measured                |
| `wake-cause-survives-rejection`, `foreign-wake-pin` | not run on the S3           |
| `short-timer-sweep`, `held-level-custom-handler`    | not run on the S3           |
| Touch count                                         | by hand, 3 per tier         |
