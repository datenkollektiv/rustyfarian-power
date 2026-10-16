#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["pyserial>=3.5"]
# ///
"""Capture a serial port for a fixed number of seconds and timestamp every line.

Usage (from the repository root):

    just serial-capture /dev/cu.usbserial-210 30
    just serial-capture /dev/cu.usbserial-210 30 115200

Each line is prefixed with the seconds since the capture started.
DTR and RTS are driven low at open and never touched again during the capture.
Opening the port may still pulse EN once on boards with a CP210x auto-reset
circuit (the ESP32-C3-DevKitM-1 showed a POWERON boot at open); start the
capture before the sequence you want to observe.

Exit codes: 0 capture finished, 2 bad arguments or the port could not be opened.
"""

import sys
import time

import serial


def usage(message):
    print(f"error: {message}", file=sys.stderr)
    print(__doc__, file=sys.stderr)
    return 2


def main():
    if len(sys.argv) < 3 or len(sys.argv) > 4:
        return usage("expected <port> <seconds> [baud]")
    port = sys.argv[1]
    try:
        seconds = float(sys.argv[2])
        baud = int(sys.argv[3]) if len(sys.argv) > 3 else 115200
    except ValueError:
        return usage("<seconds> must be a number and [baud] an integer")
    if seconds <= 0 or baud <= 0:
        return usage("<seconds> and [baud] must be positive")

    ser = serial.Serial()
    ser.port = port
    ser.baudrate = baud
    ser.timeout = 0.5
    ser.dtr = False
    ser.rts = False
    try:
        ser.open()
    except serial.SerialException as exc:
        return usage(f"cannot open {port}: {exc}")

    start = time.monotonic()
    buf = b""

    def emit(raw):
        stamp = time.monotonic() - start
        print(f"{stamp:7.2f}  {raw.decode('utf-8', 'replace').rstrip(chr(13))}", flush=True)

    try:
        while time.monotonic() - start < seconds:
            chunk = ser.read(4096)
            if not chunk:
                continue
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                emit(line)
        if buf:
            emit(buf)
    finally:
        ser.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
