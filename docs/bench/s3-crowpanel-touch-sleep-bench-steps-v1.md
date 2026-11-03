# Bench Record: ESP32-S3 CrowPanel Touch Wake, Both Tiers v1

2026-10-14, CrowPanel 1.28" HMI ESP32-S3 (rev v0.2, 16 MB flash), no external wiring.
Firmware: the uncommitted working tree on top of `ea40d22`, ESP-IDF v5.3.3 and esp-hal `=1.2.2`.
Examples `idf_s3_deep_sleep_gpio` and `hal_s3_deep_sleep_gpio`, sleeping on GPIO5 `AnyLow` plus a 60 s timer.

## How it was run

The bench builds carried extra logging (a boot `BENCH` line, a 2 s console wait, per-attempt rejection lines, and a hal switch that skipped the touch bring-up) and a passive serial logger.
That tooling was removed after the run.
The committed examples print the wake cause and stay awake 3 s, so `just serial-capture` sees every wake.

- The board's USB-C reaches the S3's native USB-Serial-JTAG (`/dev/cu.usbmodem*`), not a CH340 as the knob docs say.
- The port drops off the bus in deep sleep and re-enumerates on wake; boot output sent before the host reopens it is lost.
- `espflash monitor` (4.3) cannot follow that without resetting the chip; see `docs/project-lore.md` → Bench tooling.
- A sleeping board, or one spinning in a rejection loop, is flashed from download mode: unplug, hold BOOT, plug in, release.
- After that flash the chip stays in download mode (`boot:0x10 (DOWNLOAD(USB/UART0))`), even through `espflash reset`: unplug and replug without BOOT, or press RESET.
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
| `foreign-wake-pin`                                  | not run on the S3           |
| `short-timer-sweep`, `held-level-custom-handler`    | not run on the S3           |
| Touch count                                         | by hand, 3 per tier         |

## Pad-hold follow-up (knob, 2026-10-28)

`rustyfarian-knob` re-ran the touch wake at `31c52fb` with `Output::set_pad_hold` holding GPIO1 and GPIO13 (`hal_esp32s3_knob_sleep`).

| Case           | Result                                                  |
|:---------------|:--------------------------------------------------------|
| Touch wake ×3  | 3/3 `Ext1(GpioWakeMask(32))`, retained count 1 → 2 → 3  |
| Timer wake     | `Timer` 60.2 s after sleep entry, count 4               |
| Finger resting | Re-wake ~0.7 s after each sleep entry, no `Rejected`    |
| INT held LOW   | Never seen                                              |

The CST816S answered every tap after deep sleep, so both holds kept the controller powered and out of reset.

## R2-S3: `try_sleep` post-rejection postcondition (tier parity, 2026-11-03)

Checks ADR-003 point 1 on the S3, where the hardware rejects and the IDF adapter's EXT1 release (`rtc_gpio_hold_dis` + `rtc_gpio_deinit`) runs; the C3 R2 bench could not reach that code.
Firmware: `idf_s3_deep_sleep_gpio` and `hal_s3_deep_sleep_gpio` built with `S3_R2=1 S3_SLEEP_TIMER_MS=5000` on top of `94dcde1`; see the examples' doc comments for the line formats.
The esp-hal run used the then-opt-in `S3_CLEAR_AUTOHOLD=1`; the committed example clears the bit unconditionally, the same behaviour.

```shell
S3_R2=1 S3_SLEEP_TIMER_MS=5000 just flash idf_s3_deep_sleep_gpio
S3_R2=1 S3_SLEEP_TIMER_MS=5000 just flash-hal-s3 hal_s3_deep_sleep_gpio
just serial-capture /dev/cu.usbmodem2101 240
```

Method:
- The firmware makes the stimulus: odd cycles and every `Ext1` wake are forced cycles, which cut `LCD_3V3` (GPIO1) and hold touch reset (GPIO13) LOW, so INT (GPIO5) reads `L` and the S3 rejects the sleep.
- Right after the rejection the firmware reads GPIO5 through its existing input driver, re-reads the wake cause, and (IDF) retries once with GPIO0 + GPIO5 in the mask.
- Two seconds into an 8 s window it restores the rail and reset, which gives an `L->H` edge on GPIO5, then retries until the sleep succeeds.
- The recount is measured in firmware: the RTC time of each sleep entry is kept in RTC memory and subtracted from the RTC clock at the next `Timer` boot (`EspRtcClock`, `EspHalRtcClock`), at 1 ms resolution.
  `SLEPT` includes the boot time up to the measurement, the same on forced and plain cycles, so forced cycles are compared with plain ones.
- Touches by hand gave the `Ext1` wakes; the stimulus needs no hands.
- Logs: `tmp/bench/2026-11-03/s3-idf-r2.log`, `tmp/bench/2026-11-03/s3-hal-r2.log`.

| Check                                | ESP-IDF                                                      | esp-hal                                  |
|:-------------------------------------|:-------------------------------------------------------------|:-----------------------------------------|
| Hardware rejection on forced cycles  | 8 of 8 logged `Deep sleep request is rejected`; 1 unobserved | 12 of 12 reached the post-rejection read |
| GPIO5 `L` at rejection, then `L->H`  | 9 of 9                                                       | 12 of 12                                 |
| Changed-mask retry                   | 8 of 8 `err:Rejected`; 1 unobserved                          | not run (IDF-only bug)                   |
| `SLEPT` after forced vs plain cycles | 5400-5402 ms (7) vs 5401-5402 ms (9)                         | 5146-5158 ms (2) vs 5149-5160 ms (5)     |
| Wake cause re-read after a rejection | `Ext1(0x20)` → `Ext1(0x0)`, 1 of 1                           | `Ext1(0x20)` → `Ext1(0x0)`, 6 of 6       |
| Faults                               | none                                                         | none                                     |

- Recount: forced and plain cycles agree within 2 ms (IDF) and 14 ms (esp-hal), inside the ± 150 ms band, so the timer counts from the retry.
- Unobserved: in one IDF forced cycle the USB port dropped during the rejected attempt and its lines up to `MASKRETRY` were lost; its `L->H` edge and `high=1 low=1` arrived.
- esp-hal prints no `PINREAD end` or second `ATTEMPT`: the chip sleeps before the USB-Serial-JTAG buffer drains; `start=L` plus the `L->H` edge carry the readability result.
- Some `Ext1` wakes on esp-hal followed a forced cycle's rail restore without a touch; they are real `Ext1` wakes (mask `0x20`) and count as wake-cause samples, but not every one was a finger.
- Wake cause: on both tiers a rejection keeps the cause and clears the pin mask, so the capture-before-the-first-`try_sleep` rule applies to the IDF tier too.

Finding: a rejected deep sleep hangs the S3 while the global digital-pad hold (`DG_PAD_AUTOHOLD_EN`) is enabled, on both tiers; see `docs/project-lore.md` → ESP-IDF tier — ESP32-S3 deep sleep.
The first IDF runs reset by interrupt watchdog on every forced cycle (`LASTSTAGE stage=3`, inside `try_sleep`), and the pre-change adapter `8669290` did the same; the first esp-hal runs stalled silently with `autohold_en=1` inherited from the IDF firmware.
Both examples now clear the hold before a forced attempt.

## R3-S3: `try_sleep` hold guard with the global pad hold kept on (2026-11-04)

Checks the library guard against the R2-S3 hang: with the hold effective, `try_sleep` must return `Rejected` from a pre-entry read instead of hanging.
Firmware: both S3 examples with `S3_R2=1 S3_SLEEP_TIMER_MS=5000 S3_KEEP_AUTOHOLD=1`, so forced cycles keep the hold on (`FORCED ... autohold=1`); working tree on top of `8199212`.
Logs: `tmp/bench/2026-11-03/s3-negctl.log`, `s3-guard-idf.log`, `s3-guard-hal.log`.

| Run                                 | Forced cycles | Result                                                                                                     |
|:------------------------------------|--------------:|:-----------------------------------------------------------------------------------------------------------|
| Negative control, IDF without guard |             2 | `rst:0x8 TG1WDT_SYS_RST` at `LASTSTAGE stage=3` on both: the bench detects the hang                        |
| IDF with guard                      |             6 | 6 `Rejected`, no IDF reject line (the guard answered), GPIO5 `L` then `L->H`, `MASKRETRY` 6 `err:Rejected` |
| esp-hal with guard                  |             5 | 5 `Rejected` (`autohold_en=1` at boot), GPIO5 `L` then `L->H`, cycles 1-9 continuous                       |

- Recount after a guarded rejection, IDF: `SLEPT` 5401-5402 ms after forced cycles against 5401-5402 ms after plain ones (RTC clock, 1 ms resolution).
- Recount after a guarded rejection, esp-hal: unobserved; the boots after forced cycles printed nothing the capture received, although the cycle counter shows they ran.
  The esp-hal recount after a hardware rejection is covered by R2-S3.
- No watchdog reset, panic or stall in the guarded runs.
- Not tested: the race (a pin crossing its wake level after the read) and non-pin rejects with the hold on; the ESP32 guard is compile-only.
