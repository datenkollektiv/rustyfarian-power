#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["pyserial>=3.5"]
# ///
"""Stream a serial console with timestamps until Ctrl-C and mark the end of the PIR warm-up.

Usage (from the repository root):

    just console /dev/cu.usbserial-0001
    just console /dev/cu.usbserial-0001 60
    just console /dev/cu.usbserial-0001 60 115200

Each line is prefixed with the seconds since the console opened.
After <warmup> seconds (default 60, the HC-SR501 settling time) a banner line marks that the
PIR output can be trusted and waving may start.
DTR and RTS are driven low at open and never touched again, so an adapter whose DTR/RTS are
unwired (the SuperMini rig) never resets the chip.
Do not use it on a USB-Serial-JTAG port (S3 native USB): lowering DTR before RTS resets the
chip there; use `just serial-capture` instead (docs/project-lore.md -> Bench tooling).
If the port drops (board unplugged) the console waits and reopens it.

Exit codes: 0 on Ctrl-C, 2 bad arguments.
"""

import sys
import time

import serial


def usage(message):
    print(f"error: {message}", file=sys.stderr)
    print(__doc__, file=sys.stderr)
    return 2


def open_port(port, baud):
    ser = serial.Serial()
    ser.port = port
    ser.baudrate = baud
    ser.timeout = 0.5
    ser.dtr = False
    ser.rts = False
    ser.open()
    return ser


def main():
    if len(sys.argv) < 2 or len(sys.argv) > 4:
        return usage("expected <port> [warmup_seconds] [baud]")
    port = sys.argv[1]
    try:
        warmup = float(sys.argv[2]) if len(sys.argv) > 2 else 60.0
        baud = int(sys.argv[3]) if len(sys.argv) > 3 else 115200
    except ValueError:
        return usage("[warmup_seconds] must be a number and [baud] an integer")
    if warmup < 0 or baud <= 0:
        return usage("[warmup_seconds] must be >= 0 and [baud] positive")

    start = time.monotonic()
    warmup_done = warmup == 0
    ser = None
    buf = b""

    def stamp():
        return f"{time.monotonic() - start:7.2f}"

    print(f"{stamp()}  [console on {port} at {baud} baud; PIR warm-up marker after {warmup:.0f} s; Ctrl-C to stop]", flush=True)
    try:
        while True:
            if ser is None:
                try:
                    ser = open_port(port, baud)
                    print(f"{stamp()}  [port open]", flush=True)
                # termios.error (a wedged CP2102 after a hot unplug) is neither of the two.
                except Exception as exc:  # noqa: BLE001
                    print(f"{stamp()}  [waiting for port: {exc}]", flush=True)
                    time.sleep(1.0)
                    continue
            try:
                data = ser.read(4096)
            except (serial.SerialException, OSError) as exc:
                print(f"{stamp()}  [port lost: {exc}]", flush=True)
                ser.close()
                ser = None
                buf = b""
                continue
            if not warmup_done and time.monotonic() - start >= warmup:
                warmup_done = True
                print(f"{stamp()}  === PIR warm-up over ({warmup:.0f} s): output is valid, start waving ===", flush=True)
            if data:
                buf += data
                while b"\n" in buf:
                    raw, buf = buf.split(b"\n", 1)
                    print(f"{stamp()}  {raw.decode('utf-8', 'replace').rstrip(chr(13))}", flush=True)
    except KeyboardInterrupt:
        print(f"\n{stamp()}  [stopped]", flush=True)
        return 0


if __name__ == "__main__":
    sys.exit(main())
