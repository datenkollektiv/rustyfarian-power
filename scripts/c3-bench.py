#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["pyserial>=3.5"]
# ///
"""Host driver for the ESP32-C3 deep-sleep bench (`hal_c3_sleep_bench`).

Reads the DUT console passively, drives the ESP32-WROOM stimulus board
(`hal_esp32_bench_stimulus`), detects hangs, writes one CSV row per `BENCH` line plus a raw log,
and prints a per-case summary.

Hardware context:
- DUT: ESP32-C3-MINI-1 on a CH340 (RTS -> EN, DTR -> GPIO9).
  The port is opened with DTR and RTS released and they are never touched, except for the
  hang-recovery reset (RTS pulse, DTR released). DTR asserted with RTS released would hold GPIO9
  low through a reset, which is download mode.
- Stimulus: ESP32-WROOM, ch1 = GPIO25 -> 1 kOhm -> C3 GPIO4, ch2 = GPIO26 -> 1 kOhm -> C3 GPIO5.
  10 kOhm pull-downs on the C3 side own the lines while the stimulus is tri-stated.
- Serial timestamps cannot judge sub-50 ms or +-5 % timing; those checks are listed as MANUAL.

Usage (from the repository root):

    just bench-c3 --case W1 --cycles 3
    just bench-c3 --list-ports
    just bench-c3 --self-test

Exit codes: 0 pass, 1 fail, 2 infrastructure (ports, stimulus, wrong firmware).
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import io
import os
import queue
import random
import re
import statistics
import sys
import threading
import time
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Dict, Iterator, List, Optional, Protocol, TextIO, Tuple

# ── Constants ───────────────────────────────────────────────────────────────────────────────────

CASES = ("W1", "W2", "W3", "H1", "H2", "H3", "H4", "H5", "H6", "H7", "H8", "R1")

DEFAULT_CYCLES: Dict[str, int] = {
    "W1": 100,
    "W2": 100,
    "W3": 100,
    "H1": 100,
    "H2": 10,
    "H3": 100,
    "H4": 1,
    "H5": 1000,
    "H6": 10,
    "H7": 10,
    "H8": 100,
    "R1": 20,
}

DEFAULT_DUT_PORT = "/dev/cu.usbserial-210"
BAUD = 115200

PIR_BIT = 1 << 4
"""GPIO4, the wake pin, in the `mask` field."""
FOREIGN_BIT = 1 << 5
"""GPIO5, the H8 foreign pad."""

PULSE_US = 100_000
"""Stimulus pulse width: long enough for any wake path, short enough to release before boot."""

GRACE_S = 2.0
"""Added to every state deadline before a cycle is declared hung."""
BOOT_TO_ACTION_S = 1.5
"""After a boot line, the firmware prints its next structured line well within this."""
WAKE_TO_BOOT_S = 1.0
"""Stimulus edge or timer expiry to the boot line (ROM + 2nd-stage bootloader ~0.4 s)."""
NEXT_CYCLE_S = 2.0
"""The firmware's timer-only sleep between cycles (`NEXT_CYCLE_MS`)."""
KICK_DELAY_S = 2.0
"""W1/H2/H4/H8: wait after the sleep starts before the stimulus edge."""
SYNC_TIMEOUT_S = 8.0
"""No boot line this long after start or after a reset: reset the DUT (not counted as a hang)."""
MAX_SYNC_RESETS = 2
MAX_CONSECUTIVE_HANGS = 5

W3_TIMER_S = 5.0
W3_MARGIN_S = 0.3
H4_REJECTS = 1000
H4_REJECT_PERIOD_S = 0.05
H4_OVERSHOOT_WINDOW_S = 0.3
H7_BUSY_MIN_S = 0.1
H7_BUSY_MAX_S = 0.4
H8_SILENCE_S = 3.0

PASS = "pass"
FAIL = "fail"
AMBIGUOUS = "ambiguous"
HANG = "hang"

CSV_FIELDS = (
    "host_ts",
    "host_cycle",
    "case",
    "tier",
    "cycle",
    "reset",
    "cause",
    "mask",
    "result",
    "guard_us",
    "drain_us",
    "dt_sleep_to_boot_s",
    "stim_event",
    "note",
    "verdict",
)

MANUAL: Dict[str, List[str]] = {
    "W1": ["W1: wake within 50 ms of the GPIO4 edge needs the LA (ch0 GPIO4 vs ch3 UART0 TX)."],
    "W2": ["W2: wake at 5 s +- 5 % needs the LA; serial dt is indicative only."],
    "W3": ["W3: wake-to-edge timing needs the LA (ch0, ch3)."],
    "H5": [
        "H5: guard-to-sleep-entry needs the PPK2 (D0 = GPIO10 marker vs the VBUS current step).",
        "H5: derive MIN_TIMER_MS and POST_ARM_MARGIN_US from the PPK2 traces (ADR-001 s. 4c).",
        "H5: confirm on LA ch3 that the UART0 TX burst ends before the marker edge (drain_us).",
    ],
    "H6": ["H6: LA must show GPIO4 and the deadline untouched by both rejected calls."],
}


class InfraError(Exception):
    """Ports, stimulus board or firmware selection are wrong: exit 2."""


# ── Parser ──────────────────────────────────────────────────────────────────────────────────────

_LOG_TS = re.compile(r"^(\d{2}):(\d{2}):(\d{2})\.(\d{3}) ")
_ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
_BENCH = re.compile(r"BENCH (case=\S+.*)$")
_KV = re.compile(r"(\w+)=(\S+)")
_BENCH_KEYS = ("case", "tier", "cycle", "reset", "cause", "mask", "result", "guard_us", "drain_us")
STRUCTURED = ("bench", "note", "sleeping", "rejected", "panic")


@dataclass(frozen=True)
class Bench:
    """One `BENCH` line of the firmware."""

    case: str
    tier: str
    cycle: int
    reset: str
    cause: str
    mask: int
    result: str
    guard_us: int
    drain_us: int

    @property
    def is_boot(self) -> bool:
        return self.result == "boot"

    @property
    def error(self) -> Optional[str]:
        """Error variant name for `result=err:<Variant>`, else None."""
        return self.result[4:] if self.result.startswith("err:") else None

    def gpio_wake(self, bit: int = PIR_BIT) -> bool:
        return self.cause == "Gpio" and bool(self.mask & bit)


@dataclass(frozen=True)
class Event:
    """A classified console line."""

    kind: str
    """bench | note | sleeping | rejected | panic | other"""
    text: str
    bench: Optional[Bench] = None
    tag: str = ""
    """First NOTE token when it is not a key=value pair (e.g. `h2`, `h3`, `h7`)."""
    kv: Dict[str, str] = field(default_factory=dict)


def clean(raw: str) -> str:
    """Strips line endings, NULs and ANSI colour codes."""
    return _ANSI.sub("", raw).replace("\x00", "").strip("\r\n").rstrip()


def split_log_ts(line: str) -> Tuple[Optional[float], str]:
    """Splits the `HH:MM:SS.mmm ` prefix of the raw logs into seconds of the day and the text."""
    m = _LOG_TS.match(line)
    if not m:
        return None, line
    h, mi, s, ms = (int(g) for g in m.groups())
    return h * 3600 + mi * 60 + s + ms / 1000.0, line[m.end():]


def parse_bench(payload: str) -> Optional[Bench]:
    kv = dict(_KV.findall(payload))
    if any(k not in kv for k in _BENCH_KEYS):
        return None
    try:
        return Bench(
            case=kv["case"],
            tier=kv["tier"],
            cycle=int(kv["cycle"]),
            reset=kv["reset"],
            cause=kv["cause"],
            mask=int(kv["mask"], 0),
            result=kv["result"],
            guard_us=int(kv["guard_us"]),
            drain_us=int(kv["drain_us"]),
        )
    except ValueError:
        return None


def parse_line(text: str) -> Event:
    """Classifies one cleaned console line (no timestamp prefix)."""
    m = _BENCH.search(text)
    if m:
        b = parse_bench(m.group(1))
        if b is not None:
            return Event("bench", text, bench=b)
    if text.startswith("NOTE "):
        body = text[5:]
        first = body.split(" ", 1)[0]
        return Event("note", text, tag="" if "=" in first else first, kv=dict(_KV.findall(body)))
    if text.startswith("Sleeping:"):
        return Event("sleeping", text)
    if "PANIC:" in text:
        return Event("panic", text)
    if text.startswith("try_sleep rejected"):
        return Event("rejected", text)
    return Event("other", text)


# ── Case state machines ─────────────────────────────────────────────────────────────────────────


@dataclass
class Options:
    case: str
    seed: int = 1
    h5_hold_high: bool = False
    bench_gpio: int = 1
    timer_ms: int = 5
    method: str = "try"
    uart_q: int = 0
    reset_on_hang: bool = True


@dataclass
class Reaction:
    """What a line caused: stimulus commands, and a verdict when it completed a cycle."""

    cmds: List[str] = field(default_factory=list)
    verdict: str = ""
    note: str = ""


@dataclass
class CycleResult:
    index: int
    verdict: str
    cls: str = ""
    dt_s: Optional[float] = None
    note: str = ""


class CaseMachine:
    """Consumes (ts, event) and timer ticks, emits stimulus commands and per-cycle verdicts.

    `expect_by` is the host time by which the next structured line is due (None while the host
    itself owns the next step); the runner declares a hang at `expect_by + GRACE_S`.
    `at` is the next host-side timed action, handled in `on_timer`.
    """

    name = ""

    def __init__(self, target: int, opts: Options) -> None:
        self.target = target
        self.opts = opts
        self.results: List[CycleResult] = []
        self.classes: Counter = Counter()
        self.panics = 0
        self.state = "sync"
        self.synced = False
        self.expect_by: Optional[float] = None
        self.at: Optional[float] = None

    @property
    def done(self) -> bool:
        return len(self.results) >= self.target

    def setup_cmds(self) -> List[str]:
        return ["ZZ"]

    def start(self, now: float) -> List[str]:
        """(Re)starts from an unknown DUT state; returns the stimulus setup."""
        self.state = "sync"
        self.synced = False
        self.at = None
        self.expect_by = now + SYNC_TIMEOUT_S
        return self.setup_cmds()

    def on_line(self, ts: float, ev: Event) -> Reaction:
        if ev.kind == "panic":
            self.panics += 1
            self.state = "sync"
            self.synced = False
            self.at = None
            self.expect_by = ts + SYNC_TIMEOUT_S
            return Reaction(note="panic")
        if ev.kind == "bench" and ev.bench is not None and ev.bench.is_boot:
            self.synced = True
            self.expect_by = ts + BOOT_TO_ACTION_S
            return self.on_boot(ts, ev.bench)
        if not self.synced:
            return Reaction()
        return self.on_event(ts, ev)

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        raise NotImplementedError

    def on_event(self, ts: float, ev: Event) -> Reaction:
        return Reaction()

    def on_tick(self, now: float) -> List[str]:
        if self.at is not None and now >= self.at:
            self.at = None
            return self.on_timer(now)
        return []

    def on_timer(self, now: float) -> List[str]:
        return []

    def on_hang(self, now: float) -> List[str]:
        self.results.append(CycleResult(len(self.results), HANG, note=f"state={self.state}"))
        return self.start(now)

    def complete(self, verdict: str, cls: str = "", dt_s: Optional[float] = None, note: str = "") -> Reaction:
        self.results.append(CycleResult(len(self.results), verdict, cls, dt_s, note))
        if cls:
            self.classes[cls] += 1
        return Reaction(verdict=verdict, note=note)

    def row_annotation(self) -> str:
        return ""

    def passed(self) -> bool:
        return self.panics == 0 and all(r.verdict in (PASS, AMBIGUOUS) for r in self.results)


class W1(CaseMachine):
    """GPIO4 wake: 2 s after `Sleeping:` drive GPIO4 high for 100 ms; expect a Gpio boot."""

    name = "W1"
    pulse_ts = 0.0

    def setup_cmds(self) -> List[str]:
        # The kick pulse wakes a DUT that is already in its 60 s sleep.
        return ["ZZ", f"P 1 H {PULSE_US}"]

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state == "pulsed":
            d = ts - self.pulse_ts
            ok = b.gpio_wake() and d <= WAKE_TO_BOOT_S
            r = self.complete(PASS if ok else FAIL, dt_s=d, note=f"edge_to_boot={d:.3f}s cause={b.cause} mask=0x{b.mask:x}")
        elif self.state == "wait_pulse":
            r = self.complete(FAIL, note=f"woke before the pulse: cause={b.cause} mask=0x{b.mask:x}")
        self.state = "await_sleep"
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if ev.kind == "sleeping" and self.state == "await_sleep":
            self.state = "wait_pulse"
            self.at = ts + KICK_DELAY_S
            self.expect_by = None
        return Reaction()

    def on_timer(self, now: float) -> List[str]:
        if self.state != "wait_pulse":
            return []
        self.state = "pulsed"
        self.pulse_ts = now
        self.expect_by = now + WAKE_TO_BOOT_S
        return [f"P 1 H {PULSE_US}"]


class W2(CaseMachine):
    """Timer fallback: GPIO4 held low; expect a Timer boot after the 5 s timer."""

    name = "W2"
    timer_s = W3_TIMER_S
    sleep_ts = 0.0

    def setup_cmds(self) -> List[str]:
        return ["L 1"]

    def judge(self, b: Bench, d: float) -> Reaction:
        return self.complete(PASS if b.cause == "Timer" else FAIL, dt_s=d, note=f"sleep_to_boot={d:.3f}s cause={b.cause}")

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = self.judge(b, ts - self.sleep_ts) if self.state == "sleeping" else Reaction()
        self.state = "await_sleep"
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if ev.kind == "sleeping" and self.state == "await_sleep":
            self.state = "sleeping"
            self.sleep_ts = ts
            self.expect_by = ts + self.timer_s + WAKE_TO_BOOT_S
        return Reaction()


class R1(W2):
    """RTC-fast boot counter: `cycle` must be previous + 1 on every boot of a W2-style run."""

    name = "R1"

    def __init__(self, target: int, opts: Options) -> None:
        super().__init__(target, opts)
        self.prev: Optional[int] = None

    def start(self, now: float) -> List[str]:
        self.prev = None
        return super().start(now)

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state == "sleeping" and self.prev is not None:
            d = ts - self.sleep_ts
            ok = b.cycle == self.prev + 1
            r = self.complete(PASS if ok else FAIL, dt_s=d, note=f"cycle {self.prev}->{b.cycle} cause={b.cause}")
        self.prev = b.cycle
        self.state = "await_sleep"
        return r


W3_MIN_DELAY_S = 0.2  # `Sleeping:` precedes the attempt; an earlier pulse lands while still awake


class W3(CaseMachine):
    """GPIO and timer together: GPIO4 pulse at a seeded uniform 0-6 s; whichever comes first wins."""

    name = "W3"
    sleep_ts = 0.0
    delay_s = 0.0

    def __init__(self, target: int, opts: Options) -> None:
        super().__init__(target, opts)
        self.rng = random.Random(opts.seed)

    def setup_cmds(self) -> List[str]:
        return ["ZZ"]

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state == "sleeping":
            d = self.delay_s
            if d < W3_TIMER_S - W3_MARGIN_S:
                expected: Optional[str] = "Gpio"
            elif d > W3_TIMER_S + W3_MARGIN_S:
                expected = "Timer"
            else:
                expected = None
            got = "Gpio" if b.gpio_wake() else ("Gpio-nobit4" if b.cause == "Gpio" else b.cause)
            verdict = AMBIGUOUS if expected is None else (PASS if got == expected else FAIL)
            r = self.complete(verdict, cls=got, dt_s=ts - self.sleep_ts, note=f"d={d:.3f}s expected={expected or 'either'} got={got}")
            r.cmds.append("Z 1")  # not X: X leaves a mid-flight pulse latched HIGH
        self.state = "await_sleep"
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if ev.kind == "sleeping" and self.state == "await_sleep":
            self.delay_s = self.rng.uniform(W3_MIN_DELAY_S, 6.0)
            self.state = "sleeping"
            self.sleep_ts = ts
            self.expect_by = ts + min(self.delay_s, W3_TIMER_S) + WAKE_TO_BOOT_S
            return Reaction(cmds=[f"P 1 H {PULSE_US} {round(self.delay_s * 1e6)}"])
        return Reaction()


class H1(CaseMachine):
    """GPIO4 held high at entry: classify rejected vs slept-woke; pass if one class all cycles."""

    name = "H1"

    def setup_cmds(self) -> List[str]:
        return ["H 1"]

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state == "attempt":
            cls = "slept-woke" if b.gpio_wake() else f"other:{b.cause}"
            r = self.complete(PASS if cls == "slept-woke" else FAIL, cls=cls, note=f"class={cls}")
        self.state = "await_sleep"
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if ev.kind == "sleeping" and self.state == "await_sleep":
            self.state = "attempt"
            self.expect_by = ts + BOOT_TO_ACTION_S
        elif ev.kind == "bench" and ev.bench is not None and self.state == "attempt":
            cls = "rejected" if ev.bench.error == "Rejected" else f"other:{ev.bench.result}"
            self.state = "next"
            self.expect_by = ts + NEXT_CYCLE_S + WAKE_TO_BOOT_S
            return self.complete(PASS if cls == "rejected" else FAIL, cls=cls, note=f"class={cls}")
        return Reaction()

    def passed(self) -> bool:
        return super().passed() and len(self.classes) == 1


class H2(CaseMachine):
    """Wake cause across a rejection: kick a Gpio boot, hold, record NOTE h2 read1/read2."""

    name = "H2"
    level = "L"
    read1 = ""
    rejected = False
    boot_ok = False

    def setup_cmds(self) -> List[str]:
        return ["L 1"]

    def start(self, now: float) -> List[str]:
        cmds = super().start(now)
        self.level = "L"
        self.at = now + KICK_DELAY_S
        return cmds

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        if self.state == "kicked":
            self.state = "gpio_boot"
            self.boot_ok = b.gpio_wake()
            self.read1 = ""
            self.rejected = False
        elif self.level == "H":
            self.state = "drain"
        else:
            # The attempt sleeps on the low line; kick it awake.
            self.state = "wait_kick"
            self.at = ts + KICK_DELAY_S
            self.expect_by = None
        return Reaction()

    def on_event(self, ts: float, ev: Event) -> Reaction:
        r = Reaction()
        if ev.kind == "note" and ev.tag == "h2" and "read1" in ev.text:
            self.read1 = ev.kv.get("caused_wakeup", "?")
        elif ev.kind == "bench" and ev.bench is not None and not ev.bench.is_boot:
            if ev.bench.error == "Rejected":
                self.rejected = True
            if self.level == "H":
                self.level = "L"
                r.cmds.append("L 1")
        elif ev.kind == "note" and ev.tag == "h2" and "read2" in ev.text:
            read2 = ev.kv.get("caused_wakeup", "?")
            if self.state == "gpio_boot":
                ok = self.boot_ok and self.rejected
                cls = f"read1={self.read1} read2={read2}"
                r2 = self.complete(PASS if ok else FAIL, cls=cls, note=f"{cls} rejected={self.rejected} gpio_boot={self.boot_ok}")
                r2.cmds.extend(r.cmds)
                r = r2
            self.state = "next"
            self.expect_by = ts + NEXT_CYCLE_S + WAKE_TO_BOOT_S
        return r

    def on_timer(self, now: float) -> List[str]:
        if self.state not in ("wait_kick", "sync"):
            return []
        self.level = "H"
        if self.state == "wait_kick":
            self.state = "kicked"
            self.expect_by = now + WAKE_TO_BOOT_S
        return ["H 1"]


class H3(CaseMachine):
    """Custom handler: `err:Rejected` and `NOTE h3 handler_calls=0` every cycle, GPIO4 held."""

    name = "H3"
    result = ""

    def setup_cmds(self) -> List[str]:
        return ["H 1"]

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state == "attempt":
            r = self.complete(FAIL, note="boot before NOTE h3 handler_calls")
        self.state = "attempt"
        self.result = ""
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if self.state != "attempt":
            return Reaction()
        if ev.kind == "bench" and ev.bench is not None:
            self.result = ev.bench.result
        elif ev.kind == "note" and ev.tag == "h3":
            calls = ev.kv.get("handler_calls", "?")
            ok = self.result == "err:Rejected" and calls == "0"
            self.state = "next"
            self.expect_by = ts + NEXT_CYCLE_S + WAKE_TO_BOOT_S
            return self.complete(PASS if ok else FAIL, note=f"result={self.result or '-'} handler_calls={calls}")
        return Reaction()


class H4(CaseMachine):
    """Retry loop: 1000 x `err:Rejected` with GPIO4 held, release, pulse, expect a Gpio boot."""

    name = "H4"
    count = 0
    extra = 0
    late = 0
    release_ts = 0.0
    pulse_ts = 0.0

    def setup_cmds(self) -> List[str]:
        return ["H 1"]

    def _begin(self) -> None:
        self.state = "rejecting"
        self.count = self.extra = self.late = 0

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state == "pulsed":
            d = ts - self.pulse_ts
            ok = b.gpio_wake() and self.late == 0
            r = self.complete(PASS if ok else FAIL, dt_s=d, note=f"rejects={self.count} overshoot={self.extra} late={self.late} cause={b.cause}")
            if not self.done:
                r.cmds.append("H 1")
        elif self.state == "rejecting" and self.count > 0:
            r = self.complete(FAIL, note=f"reboot after {self.count} rejects")
        elif self.state in ("released",):
            r = self.complete(FAIL, note=f"woke during the release window: cause={b.cause}")
            r.cmds.append("H 1")
        self._begin()
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if not (ev.kind == "bench" and ev.bench is not None and ev.bench.error == "Rejected"):
            return Reaction()
        if self.state == "rejecting":
            self.count += 1
            self.expect_by = ts + H4_REJECT_PERIOD_S
            if self.count >= H4_REJECTS:
                self.state = "released"
                self.release_ts = ts
                self.at = ts + KICK_DELAY_S
                self.expect_by = None
                return Reaction(cmds=["Z 1"], note=f"{self.count} rejects, releasing")
        elif self.state == "released":
            # One or two attempts may still see the line high while `Z 1` travels.
            self.extra += 1
            if ts - self.release_ts > H4_OVERSHOOT_WINDOW_S:
                self.late += 1
        return Reaction()

    def on_timer(self, now: float) -> List[str]:
        if self.state != "released":
            return []
        self.state = "pulsed"
        self.pulse_ts = now
        self.expect_by = now + WAKE_TO_BOOT_S
        return [f"P 1 H {PULSE_US}"]


class H5(CaseMachine):
    """Short-timer sweep: pass = no hangs, no panics; records guard_us / drain_us and results."""

    name = "H5"
    attempt = ""

    def __init__(self, target: int, opts: Options) -> None:
        super().__init__(target, opts)
        self.guard: List[int] = []
        self.drain: List[int] = []
        self.knob_warning = ""

    def setup_cmds(self) -> List[str]:
        return ["H 1"] if self.opts.h5_hold_high else ["L 1"]

    def _cycle_deadline(self, ts: float) -> float:
        return ts + self.opts.timer_ms / 1000.0 + NEXT_CYCLE_S + WAKE_TO_BOOT_S + BOOT_TO_ACTION_S

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state == "cycle":
            r = self.complete(PASS, cls=self.attempt or "slept", note=self.attempt or "slept")
        self.state = "cycle"
        self.attempt = ""
        self.expect_by = self._cycle_deadline(ts)
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if self.state != "cycle":
            return Reaction()
        if ev.kind == "bench" and ev.bench is not None:
            self.attempt = ev.bench.result
            if ev.bench.error is not None:
                self.guard.append(ev.bench.guard_us)
                self.drain.append(ev.bench.drain_us)
        elif ev.kind == "note" and ev.tag == "h5":
            want = {
                "timer_ms": str(self.opts.timer_ms),
                "method": self.opts.method,
                "uart_q": str(self.opts.uart_q),
                "gpio": str(self.opts.bench_gpio),
            }
            diff = [f"{k}={ev.kv.get(k)}!={v}" for k, v in want.items() if ev.kv.get(k) != v]
            if diff:
                self.knob_warning = "firmware knobs differ from the CLI: " + " ".join(diff)
                return Reaction(note=self.knob_warning)
        return Reaction()

    def row_annotation(self) -> str:
        o = self.opts
        return f"timer_ms={o.timer_ms} method={o.method} uart_q={o.uart_q} gpio={o.bench_gpio} hold_high={int(o.h5_hold_high)}"


class H6(CaseMachine):
    """Timer range limits: `err:TimerTooShort` then `err:TimerTooLong` every boot."""

    name = "H6"

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state in ("short", "long"):
            r = self.complete(FAIL, note=f"boot before both range errors (state={self.state})")
        self.state = "short"
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if not (ev.kind == "bench" and ev.bench is not None):
            return Reaction()
        err = ev.bench.error
        if self.state == "short":
            if err == "TimerTooShort":
                self.state = "long"
                self.expect_by = ts + BOOT_TO_ACTION_S
                return Reaction()
            self.state = "next"
            return self.complete(FAIL, note=f"expected err:TimerTooShort, got {ev.bench.result}")
        if self.state == "long":
            self.state = "next"
            self.expect_by = ts + NEXT_CYCLE_S + WAKE_TO_BOOT_S
            ok = err == "TimerTooLong"
            return self.complete(PASS if ok else FAIL, note=f"TimerTooShort, {ev.bench.result}")
        return Reaction()


class H7(CaseMachine):
    """UART busy: `err:UartBusy` 100-400 ms after `NOTE h7 queued=16`."""

    name = "H7"
    queued_ts = 0.0
    queued = ""

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state in ("note", "busy"):
            r = self.complete(FAIL, note=f"boot before the UartBusy report (state={self.state})")
        self.state = "note"
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if ev.kind == "note" and ev.tag == "h7" and self.state == "note":
            self.queued_ts = ts
            self.queued = ev.kv.get("queued", "?")
            self.state = "busy"
            self.expect_by = ts + H7_BUSY_MAX_S
        elif ev.kind == "bench" and ev.bench is not None and self.state == "busy":
            d = ts - self.queued_ts
            ok = ev.bench.error == "UartBusy" and H7_BUSY_MIN_S <= d <= H7_BUSY_MAX_S
            self.state = "next"
            # The firmware waits NEXT_CYCLE_MS and then restarts (software reset).
            self.expect_by = ts + NEXT_CYCLE_S + WAKE_TO_BOOT_S
            return self.complete(PASS if ok else FAIL, dt_s=d, note=f"queued={self.queued} busy_after={d:.3f}s result={ev.bench.result}")
        return Reaction()


class H8(CaseMachine):
    """Foreign wake pin: `err:ForeignWakePin`; then a GPIO5 pulse must not wake, a GPIO4 one must."""

    name = "H8"
    pulse_ts = 0.0
    failed = False

    def __init__(self, target: int, opts: Options) -> None:
        super().__init__(target, opts)
        self.silence: List[str] = []

    def setup_cmds(self) -> List[str]:
        return ["ZZ", f"P 1 H {PULSE_US}"]

    def on_boot(self, ts: float, b: Bench) -> Reaction:
        r = Reaction()
        if self.state == "pulsed" and not self.failed:
            ok = b.gpio_wake() and not self.silence
            note = f"cause={b.cause} mask=0x{b.mask:x}"
            if self.silence:
                note += f" lines_during_gpio5_window={len(self.silence)}"
            r = self.complete(PASS if ok else FAIL, dt_s=ts - self.pulse_ts, note=note)
        elif self.state == "silence" and not self.failed:
            r = self.complete(FAIL, note=f"woke on the GPIO5 pulse: cause={b.cause} mask=0x{b.mask:x}")
        elif self.state in ("foreign", "expect_sleep", "wait_p2") and not self.failed:
            r = self.complete(FAIL, note=f"unexpected boot in state {self.state}: cause={b.cause}")
        self.state = "foreign"
        self.failed = False
        self.silence = []
        return r

    def on_event(self, ts: float, ev: Event) -> Reaction:
        if self.state == "silence" and ev.text:
            self.silence.append(ev.text)
            return Reaction()
        if self.state == "foreign" and ev.kind == "bench" and ev.bench is not None:
            self.state = "expect_sleep"
            if ev.bench.error != "ForeignWakePin":
                self.failed = True
                return self.complete(FAIL, note=f"expected err:ForeignWakePin, got {ev.bench.result}")
        elif self.state == "expect_sleep" and ev.kind == "sleeping":
            self.state = "wait_p2"
            self.at = ts + KICK_DELAY_S
            self.expect_by = None
        return Reaction()

    def on_timer(self, now: float) -> List[str]:
        if self.state == "wait_p2":
            self.state = "silence"
            self.silence = []
            self.at = now + H8_SILENCE_S
            return [f"P 2 H {PULSE_US}"]
        if self.state == "silence":
            self.state = "pulsed"
            self.pulse_ts = now
            self.expect_by = now + WAKE_TO_BOOT_S
            return [f"P 1 H {PULSE_US}"]
        return []


MACHINES = {m.name: m for m in (W1, W2, W3, H1, H2, H3, H4, H5, H6, H7, H8, R1)}


def make_machine(opts: Options, cycles: int) -> CaseMachine:
    return MACHINES[opts.case](cycles, opts)


# ── Ports ───────────────────────────────────────────────────────────────────────────────────────


class Clock(Protocol):
    def now(self) -> float: ...


class DutPort(Protocol):
    def read(self, timeout: float) -> Optional[Tuple[float, str]]: ...

    def reset(self) -> None: ...

    def close(self) -> None: ...


class StimPort(Protocol):
    def command(self, cmd: str) -> str: ...

    def poll(self) -> List[str]: ...

    def close(self) -> None: ...


class MonotonicClock:
    def now(self) -> float:
        return time.monotonic()


def _open_quiet(port: str):  # type: ignore[no-untyped-def]
    """Opens a port with DTR and RTS released, set before `open()` so no edge reaches EN/IO0."""
    try:
        import serial  # pyserial, imported lazily so --self-test and py_compile need no install
    except ImportError as e:
        raise InfraError("pyserial is missing; run via `just bench-c3` (uv) or install pyserial") from e

    s = serial.Serial()
    s.port = port
    s.baudrate = BAUD
    s.timeout = 0.05
    s.dsrdtr = False
    s.rtscts = False
    s.dtr = False
    s.rts = False
    try:
        s.open()
    except serial.SerialException as e:
        raise InfraError(f"cannot open {port}: {e}") from e
    return s


class _LineReader:
    """Background thread splitting a serial stream into (monotonic, text) lines."""

    def __init__(self, ser) -> None:  # type: ignore[no-untyped-def]
        self.ser = ser
        self.q: "queue.Queue[Tuple[float, Optional[str]]]" = queue.Queue()
        self._stop = threading.Event()
        self._t = threading.Thread(target=self._run, daemon=True)
        self._t.start()

    def _run(self) -> None:
        buf = b""
        while not self._stop.is_set():
            try:
                data = self.ser.read(self.ser.in_waiting or 1)
            except Exception:  # noqa: BLE001 - any serial failure ends the stream
                self.q.put((time.monotonic(), None))
                return
            if not data:
                continue
            buf += data
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                self.q.put((time.monotonic(), line.decode("utf-8", "replace")))

    def stop(self) -> None:
        self._stop.set()
        self._t.join(timeout=0.5)


class SerialDut:
    """Passive DUT console reader; DTR/RTS only move in `reset()`."""

    def __init__(self, port: str) -> None:
        self.ser = _open_quiet(port)
        self.reader = _LineReader(self.ser)

    def read(self, timeout: float) -> Optional[Tuple[float, str]]:
        try:
            ts, text = self.reader.q.get(timeout=timeout)
        except queue.Empty:
            return None
        if text is None:
            raise InfraError("DUT serial port failed (unplugged?)")
        return ts, text

    def reset(self) -> None:
        # CH340: RTS -> EN. Keep DTR (-> GPIO9) released so the chip boots from flash.
        self.ser.dtr = False
        self.ser.rts = True
        time.sleep(0.1)
        self.ser.rts = False

    def close(self) -> None:
        self.reader.stop()
        self.ser.close()


class SerialStim:
    """Client for `hal_esp32_bench_stimulus`: one reply per command, `DONE` lines async."""

    REPLIES = ("OK", "ERR", "PONG", "VER", "T ")

    def __init__(self, port: str) -> None:
        self.ser = _open_quiet(port)
        self.reader = _LineReader(self.ser)
        self.done: List[str] = []
        # The WROOM may reset on port open; give it time for the banner, then confirm liveness.
        deadline = time.monotonic() + 2.5
        while time.monotonic() < deadline:
            line = self._get(deadline - time.monotonic())
            if line is not None and line.startswith("STIM ready"):
                break
        for _ in range(3):
            try:
                if self.command("PING").startswith("PONG"):
                    return
            except InfraError:
                pass
        self.close()
        raise InfraError(f"stimulus board on {port} does not answer PING; flash it with `just flash-hal-esp32 hal_esp32_bench_stimulus`")

    def _get(self, timeout: float) -> Optional[str]:
        try:
            _, text = self.reader.q.get(timeout=max(timeout, 0.0))
        except queue.Empty:
            return None
        if text is None:
            raise InfraError("stimulus serial port failed (unplugged?)")
        return clean(text)

    def command(self, cmd: str) -> str:
        self.ser.write((cmd + "\n").encode("ascii"))
        self.ser.flush()
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            line = self._get(deadline - time.monotonic())
            if not line:
                continue
            if line.startswith("DONE"):
                self.done.append(line)
            elif line.startswith("STIM ready"):
                raise InfraError("stimulus board rebooted during the run")
            elif line.startswith(self.REPLIES):
                return line
        raise InfraError(f"stimulus board did not reply to {cmd!r}")

    def poll(self) -> List[str]:
        while True:
            line = self._get(0.0)
            if line is None:
                break
            if line.startswith("DONE"):
                self.done.append(line)
        out, self.done = self.done, []
        return out

    def close(self) -> None:
        self.reader.stop()
        self.ser.close()


class NullStim:
    """--no-stim: prints each command as a MANUAL instruction (PIR or jumper on the bench)."""

    def __init__(self, echo: Callable[[str], None]) -> None:
        self.echo = echo

    def command(self, cmd: str) -> str:
        self.echo(f"MANUAL STIM: {cmd}")
        return "OK"

    def poll(self) -> List[str]:
        return []

    def close(self) -> None:
        pass


# ── Runner ──────────────────────────────────────────────────────────────────────────────────────


class Runner:
    """Feeds DUT lines and ticks into a case machine and records rows, hangs and the summary."""

    def __init__(
        self,
        *,
        dut: DutPort,
        stim: StimPort,
        machine: CaseMachine,
        clock: Clock,
        opts: Options,
        csv_out: TextIO,
        raw_out: TextIO,
        wall: Callable[[float], dt.datetime],
        echo: Callable[[str], None],
    ) -> None:
        self.dut = dut
        self.stim = stim
        self.m = machine
        self.clock = clock
        self.opts = opts
        self.raw_out = raw_out
        self.wall = wall
        self.echo = echo
        self.writer = csv.writer(csv_out)
        self.writer.writerow(CSV_FIELDS)
        self.pending_stim: List[str] = []
        self.last_struct_ts: Optional[float] = None
        self.causes: Counter = Counter()
        self.hangs = 0
        self.consecutive_hangs = 0
        self.sync_resets = 0
        self.rows = 0
        self.aborted = ""

    def raw(self, text: str, ts: Optional[float] = None) -> None:
        stamp = self.wall(self.clock.now() if ts is None else ts).strftime("%H:%M:%S.%f")[:-3]
        self.raw_out.write(f"{stamp} {text}\n")

    def send(self, cmds: List[str]) -> None:
        for c in cmds:
            self.raw(f"STIM> {c}")
            reply = self.stim.command(c)
            self.raw(f"STIM< {reply}")
            if reply.startswith("ERR"):
                raise InfraError(f"stimulus rejected {c!r}: {reply}")
            self.pending_stim.append(c)

    def run(self) -> int:
        """Runs until the machine is done or aborted; returns 0 pass / 1 fail (2 is raised)."""
        m = self.m
        self.send(m.start(self.clock.now()))
        start_ts = self.clock.now()
        try:
            while not m.done and not self.aborted:
                item = self.dut.read(0.05)
                for d in self.stim.poll():
                    self.raw(f"STIM< {d}")
                    self.pending_stim.append(d)
                if item is not None:
                    ts, text = item[0], clean(item[1])
                    if text:
                        self.raw(text, ts)
                        # Lines queued before the stimulus setup belong to an unknown state.
                        if ts >= start_ts:
                            self.handle(ts, parse_line(text))
                now = self.clock.now()
                self.send(m.on_tick(now))
                self.check_hang(now)
        except KeyboardInterrupt:
            self.aborted = "interrupted"
        return 0 if (m.passed() and m.done and not self.aborted) else 1

    def handle(self, ts: float, ev: Event) -> None:
        m = self.m
        b = ev.bench
        if b is not None and b.case != self.opts.case:
            raise InfraError(f"DUT runs BENCH_CASE={b.case}, expected {self.opts.case}; reflash with BENCH_CASE={self.opts.case}")
        d: Optional[float] = None
        if b is not None and b.is_boot and self.last_struct_ts is not None:
            d = ts - self.last_struct_ts
        if ev.kind in STRUCTURED:
            self.last_struct_ts = ts
        host_cycle = len(m.results)
        r = m.on_line(ts, ev)
        if b is not None:
            if b.is_boot:
                self.causes[b.cause] += 1
                self.consecutive_hangs = 0
                self.sync_resets = 0
            note = "; ".join(x for x in (r.note, m.row_annotation()) if x)
            self.write_row(ts, host_cycle, b, d, note, r.verdict)
        if r.verdict:
            self.echo(f"{m.name} {len(m.results)}/{m.target} {r.verdict} {r.note}")
        self.send(r.cmds)

    def write_row(self, ts: float, host_cycle: int, b: Optional[Bench], d: Optional[float], note: str, verdict: str) -> None:
        fields: List[object] = [self.wall(ts).isoformat(timespec="milliseconds"), host_cycle]
        if b is None:
            fields += [self.opts.case, "", "", "", "", "", HANG, "", ""]
        else:
            fields += [b.case, b.tier, b.cycle, b.reset, b.cause, f"0x{b.mask:x}", b.result, b.guard_us, b.drain_us]
        fields += ["" if d is None else f"{d:.3f}", ";".join(self.pending_stim), note, verdict]
        self.pending_stim = []
        self.writer.writerow(fields)
        self.rows += 1

    def check_hang(self, now: float) -> None:
        m = self.m
        if m.expect_by is None or now <= m.expect_by + GRACE_S:
            return
        if not m.synced:
            self.sync_resets += 1
            if self.sync_resets > MAX_SYNC_RESETS:
                raise InfraError(
                    f"no BENCH boot line from the DUT after {MAX_SYNC_RESETS} resets; "
                    f"check --port, the firmware (BENCH_CASE={self.opts.case}) and the stimulus wiring"
                )
            self.raw("HOST sync timeout: no BENCH boot line; resetting the DUT")
            if self.opts.reset_on_hang:
                self.dut.reset()
            self.send(m.start(now))
            return
        self.hangs += 1
        self.consecutive_hangs += 1
        note = f"no line by the deadline (state={m.state})"
        self.raw(f"HOST hang: {note}")
        self.echo(f"{m.name} {len(m.results) + 1}/{m.target} hang {note}")
        self.write_row(now, len(m.results), None, None, note, FAIL)
        cmds = m.on_hang(now)
        if self.opts.reset_on_hang:
            self.dut.reset()
        self.send(cmds)
        if self.consecutive_hangs >= MAX_CONSECUTIVE_HANGS:
            self.aborted = f"{MAX_CONSECUTIVE_HANGS} consecutive hangs"

    def summary(self) -> List[str]:
        m = self.m
        res = m.results
        verdicts = Counter(r.verdict for r in res)
        lines = [
            f"== {m.name} summary ==",
            f"cycles: {len(res)}/{m.target}  pass: {verdicts[PASS]}  fail: {verdicts[FAIL]}  "
            f"ambiguous: {verdicts[AMBIGUOUS]}  hangs: {self.hangs}  panics: {m.panics}  rows: {self.rows}",
            "boot causes: " + (" ".join(f"{k}={v}" for k, v in sorted(self.causes.items())) or "-"),
        ]
        dts = [r.dt_s for r in res if r.dt_s is not None]
        if dts:
            lines.append(f"dt (s): min {min(dts):.3f}  mean {statistics.mean(dts):.3f}  max {max(dts):.3f}")
        if m.classes:
            lines.append("classes: " + " ".join(f"{k}={v}" for k, v in sorted(m.classes.items())))
        if isinstance(m, H5):
            for label, vals in (("guard_us", m.guard), ("drain_us", m.drain)):
                if vals:
                    lines.append(f"{label}: min {min(vals)}  mean {statistics.mean(vals):.1f}  max {max(vals)}  (n={len(vals)})")
            if m.knob_warning:
                lines.append(f"WARNING: {m.knob_warning}")
        for r in res:
            if r.verdict in (FAIL, HANG):
                lines.append(f"  cycle {r.index}: {r.verdict} {r.note}")
        if self.aborted:
            lines.append(f"aborted: {self.aborted}")
        manual = MANUAL.get(m.name, [])
        if manual:
            lines.append("MANUAL:")
            lines += [f"  - {x}" for x in manual]
        ok = m.passed() and m.done and not self.aborted
        lines.append(f"verdict: {'PASS' if ok else 'FAIL'}")
        return lines


# ── Port discovery and CLI ──────────────────────────────────────────────────────────────────────

_USB_SERIAL = re.compile(r"^/dev/(cu\.usbserial-|cu\.wchusbserial|ttyUSB)")


def same_port(a: str, b: str) -> bool:
    """True for the same device, treating macOS `/dev/tty.X` and `/dev/cu.X` as one port."""
    def norm(p: str) -> str:
        return os.path.realpath(p).replace("/dev/tty.", "/dev/cu.")
    return norm(a) == norm(b)


def list_ports() -> List[Tuple[str, str, str]]:
    from serial.tools import list_ports as lp

    return [(p.device, p.description or "", p.hwid or "") for p in sorted(lp.comports(), key=lambda p: p.device)]


def resolve_stim_port(arg: str, dut: str) -> str:
    if arg != "auto":
        if same_port(arg, dut):
            raise InfraError(f"--stim-port {arg} is the DUT port")
        return arg
    candidates = [d for d, _, _ in list_ports() if _USB_SERIAL.match(d) and not same_port(d, dut)]
    if len(candidates) == 1:
        return candidates[0]
    if not candidates:
        raise InfraError("no stimulus port found besides the DUT; pass --stim-port <path> or --no-stim")
    raise InfraError(f"several candidate stimulus ports: {', '.join(candidates)}; pass --stim-port <path>")


def default_out(case: str) -> Path:
    base = Path("tmp") / "bench" / dt.date.today().isoformat()
    path = base / f"{case}-hal.csv"
    n = 2
    while path.exists():
        path = base / f"{case}-hal-{n}.csv"
        n += 1
    return path


def cmd_list_ports(dut: str) -> int:
    try:
        ports = list_ports()
    except ImportError:
        print("pyserial is not installed; run via `just bench-c3` (uv)", file=sys.stderr)
        return 2
    if not ports:
        print("no serial ports found")
    for dev, desc, hwid in ports:
        tag = "  <- DUT (--port)" if same_port(dev, dut) else ""
        if not tag and _USB_SERIAL.match(dev):
            tag = "  <- stimulus candidate (--stim-port auto)"
        print(f"{dev}  {desc}  [{hwid}]{tag}")
    return 0


def cmd_reset_test(port: str) -> int:
    """Pulses RTS once and checks that the DUT reboots from flash (not into download mode)."""
    dut = SerialDut(port)
    try:
        t_end = time.monotonic() + 0.5
        while time.monotonic() < t_end:
            dut.read(0.05)
        print(f"resetting {port} (DTR released, RTS pulsed 100 ms)")
        dut.reset()
        boot: Optional[Bench] = None
        rom = ""
        t_end = time.monotonic() + 5.0
        while time.monotonic() < t_end and boot is None:
            item = dut.read(0.05)
            if item is None:
                continue
            text = clean(item[1])
            if not rom and text.startswith("rst:"):
                rom = text
            ev = parse_line(text)
            if ev.bench is not None and ev.bench.is_boot:
                boot = ev.bench
        if rom:
            print(f"ROM: {rom}")
            if "DOWNLOAD" in rom.upper():
                print("FAIL: the reset entered download mode; check the DTR wiring")
                return 2
        if boot is None:
            print("FAIL: no BENCH boot line within 5 s after the reset")
            return 2
        print(f"OK: BENCH boot case={boot.case} reset={boot.reset} cause={boot.cause}")
        return 0
    finally:
        dut.close()


def cmd_run(args: argparse.Namespace) -> int:
    case = args.case
    opts = Options(
        case=case,
        seed=args.seed,
        h5_hold_high=args.h5_hold_high,
        bench_gpio=args.bench_gpio,
        timer_ms=args.timer_ms,
        method=args.method,
        uart_q=args.uart_q,
        reset_on_hang=args.reset_on_hang,
    )
    cycles = args.cycles or DEFAULT_CYCLES[case]
    out = Path(args.out) if args.out else default_out(case)
    out.parent.mkdir(parents=True, exist_ok=True)
    raw_path = out.with_suffix(".log")

    def echo(s: str) -> None:
        print(s, flush=True)

    stim: Optional[StimPort] = None
    dut: Optional[SerialDut] = None
    try:
        if args.no_stim:
            stim = NullStim(echo)
        else:
            stim_port = resolve_stim_port(args.stim_port, args.port)
            echo(f"stimulus: {stim_port}")
            stim = SerialStim(stim_port)
        dut = SerialDut(args.port)
        echo(f"DUT: {args.port}  case: {case}  cycles: {cycles}  csv: {out}  log: {raw_path}")
        offset = time.time() - time.monotonic()
        with out.open("w", newline="") as csv_f, raw_path.open("w") as raw_f:
            runner = Runner(
                dut=dut,
                stim=stim,
                machine=make_machine(opts, cycles),
                clock=MonotonicClock(),
                opts=opts,
                csv_out=csv_f,
                raw_out=raw_f,
                wall=lambda ts: dt.datetime.fromtimestamp(ts + offset),
                echo=echo,
            )
            code = runner.run()
            for line in runner.summary():
                echo(line)
            return code
    except InfraError as e:
        print(f"INFRA: {e}", file=sys.stderr)
        return 2
    finally:
        if stim is not None:
            try:
                # Never leave a line driven: it back-powers the DUT through 1 kOhm when unplugged.
                stim.command("ZZ")
            except Exception:  # noqa: BLE001 - best effort on the way out
                pass
            stim.close()
        if dut is not None:
            dut.close()


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="c3-bench.py",
        description="ESP32-C3 deep-sleep bench host driver (DUT reader + WROOM stimulus).",
        epilog="Exit codes: 0 pass, 1 fail, 2 infrastructure.",
    )
    p.add_argument("--case", type=str.upper, choices=CASES, help="bench case; must match the flashed BENCH_CASE")
    p.add_argument("--port", default=DEFAULT_DUT_PORT, help=f"DUT console port (default {DEFAULT_DUT_PORT})")
    p.add_argument("--stim-port", default="auto", help="stimulus board port, or auto (the other USB serial port)")
    p.add_argument("--cycles", type=int, help="cycles to run (default per case: " + " ".join(f"{k}={v}" for k, v in DEFAULT_CYCLES.items()) + ")")
    p.add_argument("--out", help="CSV path (default tmp/bench/<date>/<case>-hal.csv); the raw .log goes beside it")
    p.add_argument("--seed", type=int, default=1, help="W3 pulse-delay seed (default 1)")
    p.add_argument("--bench-gpio", type=int, choices=(0, 1), default=1, help="H5 annotation: BENCH_GPIO of the build")
    p.add_argument("--timer-ms", type=int, default=5, help="H5 annotation: BENCH_TIMER_MS of the build")
    p.add_argument("--method", choices=("try", "sleep"), default="try", help="H5 annotation: BENCH_METHOD of the build")
    p.add_argument("--uart-q", type=int, choices=(0, 1), default=0, help="H5 annotation: BENCH_UART_Q of the build")
    p.add_argument("--h5-hold-high", action="store_true", help="H5: hold GPIO4 high (rejected-path upper bound)")
    p.add_argument("--no-stim", action="store_true", help="no stimulus board; print stimulus steps as MANUAL")
    p.add_argument("--list-ports", action="store_true", help="list serial ports and exit")
    p.add_argument("--reset-test", action="store_true", help="pulse the DUT reset once and check it boots from flash")
    p.add_argument("--reset-on-hang", action=argparse.BooleanOptionalAction, default=True, help="reset the DUT via RTS on a hang (default on)")
    p.add_argument("--self-test", action="store_true", help="run the parser and case machines against simulated and logged lines")
    return p


def main(argv: Optional[List[str]] = None) -> int:
    p = build_parser()
    args = p.parse_args(argv)
    if args.self_test:
        return self_test()
    if args.list_ports:
        return cmd_list_ports(args.port)
    try:
        if args.reset_test:
            return cmd_reset_test(args.port)
    except InfraError as e:
        print(f"INFRA: {e}", file=sys.stderr)
        return 2
    if not args.case:
        p.error("--case is required (or --list-ports / --reset-test / --self-test)")
    return cmd_run(args)


# ── Simulation (self-test) ──────────────────────────────────────────────────────────────────────
#
# A small discrete-event model of the bench: the stimulus channels as level timelines, and the
# firmware as a generator mirroring `hal_c3_sleep_bench.rs` (same line texts). Time is virtual,
# so a 1000-attempt H4 run takes milliseconds.


class VirtualClock:
    def __init__(self) -> None:
        self.t = 0.0

    def now(self) -> float:
        return self.t


@dataclass
class Print:
    line: str


@dataclass
class Delay:
    s: float


@dataclass
class Attempt:
    """`try_sleep`; resumes with "rejected" only, a sleep drops the generator."""

    gpio: bool
    timer_ms: int


@dataclass
class Restart:
    pass


@dataclass
class Stall:
    pass


@dataclass
class SimConfig:
    firmware_case: Optional[str] = None
    held: str = "reject"
    """reject | sleep | alternate: what an attempt does with GPIO4 already high."""
    stall_at_boot: Optional[int] = None
    panic_at_boot: Optional[int] = None
    foreign_wakes: bool = False
    uart_busy_s: float = 0.2
    h5_timer_ms: int = 5


@dataclass
class SimBoot:
    index: int
    reset: str
    cause: str
    mask: int
    rtc: int


class Channel:
    """Stimulus channel as seen by the C3 pad: H drives high, L/Z read low (10 kOhm pull-down)."""

    def __init__(self) -> None:
        self.base: List[Tuple[float, str]] = [(-1.0, "Z")]
        self.pending: List[Tuple[float, str]] = []
        self.pulses: List[Tuple[float, float]] = []

    def _commit(self, now: float) -> None:
        self.base += [e for e in self.pending if e[0] <= now]
        self.pending = [e for e in self.pending if e[0] > now]

    def set(self, now: float, level: str) -> None:
        self._commit(now)
        self.pending = []
        self.base.append((now, level))

    def pulse(self, now: float, level: str, width_s: float, delay_s: float, end: str) -> None:
        self._commit(now)
        self.pending = [(now + delay_s, level), (now + delay_s + width_s, end)]
        self.pulses = [(now + delay_s, now + delay_s + width_s)]

    def cancel(self, now: float) -> None:
        self._commit(now)
        self.pending = []
        self.pulses = [p for p in self.pulses if p[1] <= now]

    def _entries(self) -> List[Tuple[float, str]]:
        return self.base + self.pending

    def high_at(self, t: float) -> bool:
        level = "Z"
        for et, lv in self._entries():
            if et <= t:
                level = lv
        return level == "H"

    def first_high(self, t0: float, t1: float) -> Optional[float]:
        if self.high_at(t0):
            return t0
        for et, lv in self._entries():
            if t0 < et <= t1 and lv == "H":
                return et
        return None


def sim_firmware(case: str, fw_case: str, b: SimBoot, cfg: SimConfig) -> Iterator[object]:
    """Mirrors `main()` and the case functions of `hal_c3_sleep_bench.rs`."""
    rom = {"PowerOn": "rst:0x1 (POWERON)", "DeepSleep": "rst:0x5 (DSLEEP)", "Software": "rst:0x3 (RTC_SW_SYS_RST)"}
    yield Print(rom.get(b.reset, "rst:0x1 (POWERON)") + ",boot:0xc (SPI_FAST_FLASH_BOOT)")
    yield Delay(0.05)
    cycle = b.rtc if case in ("R1", "H5") else 0

    def bench(result: str, guard: int = 0, drain: int = 0) -> Print:
        return Print(
            f"BENCH case={fw_case} tier=hal cycle={cycle} reset={b.reset} cause={b.cause} "
            f"mask=0x{b.mask:x} result={result} guard_us={guard} drain_us={drain}"
        )

    def report_err(name: str, display: str) -> Iterator[object]:
        yield bench(f"err:{name}", 48, 3)
        yield Print(f"NOTE {display} / {name}")

    yield Print(f"Reset reason: Some(..) ({b.reset})")
    yield Print(f"Wake cause: {b.cause}")
    yield bench("boot")
    if case in ("R1", "H5"):
        yield Print(f"NOTE rtc_counter={b.rtc}")
    yield Print("GPIO4 level before sleep: Low")
    if cfg.stall_at_boot == b.index:
        yield Stall()
        return
    if cfg.panic_at_boot == b.index:
        yield Print("PANIC: panicked at examples/hal_c3_sleep_bench.rs:1:1: injected")
        yield Delay(0.05)
        yield Restart()
        return

    def retry(timer_ms: int, log_each: bool) -> Iterator[object]:
        yield Print(f"Sleeping: GPIO4 AnyHigh + {timer_ms} ms timer")
        rejected = 0
        while True:
            yield Attempt(True, timer_ms)
            rejected += 1
            if rejected == 1:
                yield Print("try_sleep rejected (first): deep sleep rejected, kind=Rejected")
            elif rejected % 20 == 0:
                yield Print(f"try_sleep rejected: {rejected} so far")
            if log_each:
                yield bench("err:Rejected", 48, 3)
            yield Delay(0.05)

    rejected_text = "deep sleep rejected: a wake source was already asserted"
    if case == "W1":
        yield from retry(60_000, False)
    elif case in ("W2", "W3", "R1"):
        yield from retry(5_000, False)
    elif case == "H4":
        yield from retry(60_000, True)
    elif case == "H1":
        yield Print("Sleeping: GPIO4 AnyHigh + 60000 ms timer")
        yield Attempt(True, 60_000)
        yield from report_err("Rejected", rejected_text)
    elif case == "H2":
        gpio = "true" if b.cause == "Gpio" else "false"
        yield Print(f"NOTE h2 read1 caused_wakeup={gpio} cause={b.cause}")
        yield Attempt(True, 60_000)
        yield from report_err("Rejected", rejected_text)
        yield Print("NOTE h2 read2 caused_wakeup=false cause=Timer")
    elif case == "H3":
        yield Attempt(True, 60_000)
        yield from report_err("Rejected", rejected_text)
        yield Print("NOTE h3 handler_calls=0")
    elif case == "H5":
        yield Print(f"NOTE h5 timer_ms={cfg.h5_timer_ms} method=try uart_q=0 gpio=1")
        yield Delay(0.05)
        yield Attempt(True, cfg.h5_timer_ms)
        yield from report_err("Rejected", rejected_text)
    elif case == "H6":
        for ms, err in ((0, "TimerTooShort"), (86_400_001, "TimerTooLong")):
            yield Print(f"NOTE h6 timer_ms={ms}")
            yield from report_err(err, "timer out of range")
    elif case == "H7":
        yield Print("NOTE h7 queued=16")
        yield Delay(cfg.uart_busy_s)
        yield from report_err("UartBusy", "UART1 TX did not drain")
        yield Delay(NEXT_CYCLE_S)
        yield Restart()
        return
    elif case == "H8":
        yield from report_err("ForeignWakePin", "GPIO5 is armed for wake but not bound")
        yield from retry(60_000, False)
    # next_cycle_sleep(): timer-only 2 s sleep.
    yield Attempt(False, int(NEXT_CYCLE_S * 1000))


@dataclass
class _Sleep:
    t0: float
    gpio: bool
    timer_s: float


class SimBench:
    """Simulated DUT and stimulus on one virtual clock; implements DutPort and StimPort."""

    def __init__(self, case: str, cfg: SimConfig, clock: VirtualClock) -> None:
        self.case = case
        self.cfg = cfg
        self.clock = clock
        self.ch = {1: Channel(), 2: Channel()}
        self.boots = 0
        self.rtc = 0
        self.resets = 0
        self.commands: List[str] = []
        self.sleep: Optional[_Sleep] = None
        self.fw: Optional[Iterator[object]] = None
        self.fw_t = 0.0
        self.stalled = False
        self.done_sent: List[Tuple[int, float, float]] = []
        self._boot(0.1, "PowerOn", "PowerOn", 0)

    # firmware execution

    def _boot(self, t: float, reset: str, cause: str, mask: int) -> None:
        self.rtc = self.rtc + 1 if reset == "DeepSleep" else 0
        boot = SimBoot(self.boots, reset, cause, mask, self.rtc)
        self.boots += 1
        self.fw = sim_firmware(self.case, self.cfg.firmware_case or self.case, boot, self.cfg)
        self.fw_t = t
        self.sleep = None
        self.stalled = False

    def _held_rejects(self) -> bool:
        if self.cfg.held == "alternate":
            return self.boots % 2 == 0
        return self.cfg.held == "reject"

    def _wake(self, target: float) -> Optional[Tuple[float, str, int]]:
        s = self.sleep
        assert s is not None
        best: Optional[Tuple[float, str, int]] = None
        if s.timer_s and s.t0 + s.timer_s <= target:
            best = (s.t0 + s.timer_s, "Timer", 0)
        if s.gpio:
            for chan, bit, enabled in ((1, PIR_BIT, True), (2, FOREIGN_BIT, self.cfg.foreign_wakes and self.case == "H8")):
                if not enabled:
                    continue
                t = self.ch[chan].first_high(s.t0, target)
                if t is not None and (best is None or t < best[0]):
                    best = (t, "Gpio", bit)
        return best

    def _run_until(self, target: float) -> Optional[Tuple[float, str]]:
        while True:
            if self.sleep is not None:
                w = self._wake(target)
                if w is None:
                    return None
                self._boot(w[0] + 0.3, "DeepSleep", w[1], w[2])
                continue
            if self.stalled or self.fw is None or self.fw_t > target:
                return None
            action = next(self.fw)
            if isinstance(action, Print):
                t = self.fw_t
                self.fw_t += 0.001
                return t, action.line
            if isinstance(action, Delay):
                self.fw_t += action.s
            elif isinstance(action, Attempt):
                if action.timer_ms < 1:
                    raise AssertionError("sim: attempt with an invalid timer")
                if action.gpio and self.ch[1].high_at(self.fw_t) and self._held_rejects():
                    self.fw_t += 0.0001
                    continue
                self.sleep = _Sleep(self.fw_t, action.gpio, action.timer_ms / 1000.0)
                self.fw = None
            elif isinstance(action, Restart):
                self._boot(self.fw_t + 0.05, "Software", "PowerOn", 0)
            elif isinstance(action, Stall):
                self.stalled = True
                return None

    # DutPort

    def read(self, timeout: float) -> Optional[Tuple[float, str]]:
        target = self.clock.t + timeout
        item = self._run_until(target)
        if item is not None:
            self.clock.t = max(self.clock.t, item[0])
            return self.clock.t, item[1] + "\r"
        self.clock.t = target
        return None

    def reset(self) -> None:
        self.resets += 1
        self._boot(self.clock.t + 0.3, "PowerOn", "PowerOn", 0)

    def close(self) -> None:
        pass

    # StimPort

    def command(self, cmd: str) -> str:
        now = self.clock.t
        self.commands.append(cmd)
        tok = cmd.split()
        op = tok[0].upper()
        if op == "PING":
            return "PONG hal_esp32_bench_stimulus v1"
        if op == "ZZ":
            for c in self.ch.values():
                c.set(now, "Z")
            return "OK ZZ"
        ch = self.ch.get(int(tok[1])) if len(tok) > 1 and tok[1].isdigit() else None
        if ch is None:
            return "ERR badch"
        if op in ("H", "L", "Z") and len(tok) == 2:
            ch.set(now, op)
            return f"OK {op} {tok[1]}"
        if op == "X":
            ch.cancel(now)  # like the firmware: a pulse already driving keeps its level
            return f"OK X {tok[1]}"
        if op == "P" and 4 <= len(tok) <= 6:
            delay = int(tok[4]) if len(tok) > 4 else 0
            end = tok[5].upper() if len(tok) > 5 else "Z"
            ch.pulse(now, tok[2].upper(), int(tok[3]) / 1e6, delay / 1e6, end)
            return f"OK P {tok[1]} armed"
        return "ERR unknown"

    def poll(self) -> List[str]:
        out = []
        for n, c in self.ch.items():
            for p in c.pulses:
                key = (n, p[0], p[1])
                if p[1] <= self.clock.t and key not in self.done_sent:
                    self.done_sent.append(key)
                    out.append(f"DONE P {n} t_start_us={int(p[0] * 1e6)} t_end_us={int(p[1] * 1e6)}")
        return out


def simulate(case: str, cycles: int, cfg: Optional[SimConfig] = None, opts: Optional[Options] = None) -> Tuple[int, Runner, SimBench, str]:
    clock = VirtualClock()
    sim = SimBench(case, cfg or SimConfig(), clock)
    o = opts or Options(case=case)
    csv_buf = io.StringIO()
    runner = Runner(
        dut=sim,
        stim=sim,
        machine=make_machine(o, cycles),
        clock=clock,
        opts=o,
        csv_out=csv_buf,
        raw_out=io.StringIO(),
        wall=lambda ts: dt.datetime(2026, 1, 1) + dt.timedelta(seconds=ts),
        echo=lambda s: None,
    )
    try:
        code = runner.run()
    except InfraError:
        code = 2
    return code, runner, sim, csv_buf.getvalue()


def _check(cond: bool, msg: str, failures: List[str]) -> None:
    if not cond:
        failures.append(msg)


def self_test() -> int:
    failures: List[str] = []

    # Parser on synthetic lines, exact firmware formats.
    e = parse_line(clean("\x1b[0mBENCH case=H1 tier=hal cycle=0 reset=DeepSleep cause=Gpio mask=0x10 result=err:Rejected guard_us=48 drain_us=3\r"))
    _check(e.kind == "bench" and e.bench is not None and e.bench.error == "Rejected" and e.bench.mask == 0x10 and e.bench.guard_us == 48, "parse BENCH err line", failures)
    _check(e.bench is not None and e.bench.gpio_wake(), "BENCH gpio_wake bit 4", failures)
    e = parse_line("BENCH case=R1 tier=hal cycle=7 reset=DeepSleep cause=Timer mask=0x0 result=boot guard_us=0 drain_us=0")
    _check(e.bench is not None and e.bench.is_boot and e.bench.cycle == 7 and not e.bench.gpio_wake(), "parse BENCH boot line", failures)
    _check(parse_line("BENCH case=H1 tier=hal cycle=x").kind == "other", "malformed BENCH is other", failures)
    e = parse_line("NOTE h3 handler_calls=0")
    _check(e.kind == "note" and e.tag == "h3" and e.kv.get("handler_calls") == "0", "parse NOTE h3", failures)
    e = parse_line("NOTE h2 read1 caused_wakeup=true cause=Gpio(WakeMask(16))")
    _check(e.tag == "h2" and e.kv.get("caused_wakeup") == "true" and e.kv.get("cause") == "Gpio(WakeMask(16))", "parse NOTE h2", failures)
    e = parse_line("NOTE rtc_counter=3")
    _check(e.kind == "note" and e.tag == "" and e.kv.get("rtc_counter") == "3", "parse NOTE rtc_counter", failures)
    _check(parse_line("Sleeping: GPIO4 AnyHigh + 60000 ms timer").kind == "sleeping", "parse Sleeping", failures)
    _check(parse_line("PANIC: panicked at src/main.rs:1:1").kind == "panic", "parse PANIC", failures)
    _check(parse_line("try_sleep rejected: 20 so far").kind == "rejected", "parse rejected count", failures)
    ts, text = split_log_ts("00:09:29.699 BENCH case=demo tier=idf")
    _check(ts is not None and abs(ts - 569.699) < 1e-6 and text.startswith("BENCH"), "split log timestamp", failures)

    # Real monitor logs (idf-tier `demo` firmware), when present.
    tmp = Path(__file__).resolve().parent.parent / "tmp"
    logs_checked = 0
    for name in ("bench-c3-monitor-w1-covered.log", "bench-c3-monitor-h1-fix.log"):
        path = tmp / name
        if not path.exists():
            continue
        logs_checked += 1
        raw_lines = path.read_text(errors="replace").splitlines()
        events = []
        for raw_line in raw_lines:
            stamp, body = split_log_ts(raw_line)
            events.append((stamp, parse_line(clean(body))))
        benches = [ev.bench for _, ev in events if ev.bench is not None]
        bench_raw = [ln for ln in raw_lines if "BENCH case=" in ln]
        _check(len(benches) == len(bench_raw) > 0, f"{name}: every BENCH line parses ({len(benches)}/{len(bench_raw)})", failures)
        _check(all(stamp is not None for stamp, ev in events if ev.kind == "bench"), f"{name}: BENCH lines carry timestamps", failures)
        want = Counter(re.search(r"cause=(\w+)", ln).group(1) for ln in bench_raw)  # type: ignore[union-attr]
        _check(Counter(b.cause for b in benches) == want, f"{name}: cause histogram", failures)
        _check(sum(b.error == "Rejected" for b in benches) == sum("result=err:Rejected" in ln for ln in bench_raw), f"{name}: rejected count", failures)
        _check(sum(ev.kind == "sleeping" for _, ev in events) == sum(" Sleeping: " in ln for ln in raw_lines), f"{name}: Sleeping count", failures)
        _check(all(b.gpio_wake() for b in benches if b.cause == "Gpio"), f"{name}: Gpio boots carry bit 4", failures)

    # Simulated bench runs: expected exit code plus a case-specific check.
    sim_runs: List[str] = []

    def run(label: str, case: str, cycles: int, want: int, cfg: Optional[SimConfig] = None, opts: Optional[Options] = None, extra: Optional[Callable[[Runner, SimBench, str], bool]] = None) -> None:
        sim_runs.append(label)
        code, runner, sim, csv_text = simulate(case, cycles, cfg, opts)
        ok = code == want and (extra is None or extra(runner, sim, csv_text))
        if not ok:
            detail = "\n    ".join(runner.summary())
            failures.append(f"sim {label}: exit {code}, want {want}\n    {detail}")

    def rows(csv_text: str) -> List[Dict[str, str]]:
        return list(csv.DictReader(io.StringIO(csv_text)))

    run("W1", "W1", 5, 0, extra=lambda r, s, c: r.causes["Gpio"] >= 5 and all(x.dt_s is not None and x.dt_s < 1.0 for x in r.m.results))
    run("W2", "W2", 5, 0, extra=lambda r, s, c: all(x.dt_s is not None and 4.9 < x.dt_s < 5.6 for x in r.m.results))
    run("W3", "W3", 30, 0, extra=lambda r, s, c: r.m.classes["Gpio"] > 0 and r.m.classes["Timer"] > 0 and "Z 1" in s.commands)
    run("H1 rejected", "H1", 5, 0, extra=lambda r, s, c: dict(r.m.classes) == {"rejected": 5})
    run("H1 slept-woke", "H1", 5, 0, SimConfig(held="sleep"), extra=lambda r, s, c: dict(r.m.classes) == {"slept-woke": 5})
    run("H1 mixed", "H1", 6, 1, SimConfig(held="alternate"))
    run("H2", "H2", 3, 0, extra=lambda r, s, c: dict(r.m.classes) == {"read1=true read2=false": 3})
    run("H3", "H3", 5, 0)
    run("H4", "H4", 1, 0, extra=lambda r, s, c: sum(x["result"] == "err:Rejected" for x in rows(c)) == H4_REJECTS)
    run("H4 x2", "H4", 2, 0)
    run("H5", "H5", 10, 0, opts=Options(case="H5"), extra=lambda r, s, c: rows(c)[0]["note"].startswith("timer_ms=5"))
    run("H5 hold-high", "H5", 5, 0, opts=Options(case="H5", h5_hold_high=True), extra=lambda r, s, c: isinstance(r.m, H5) and len(r.m.guard) >= 5)
    run("H5 knob mismatch", "H5", 3, 0, opts=Options(case="H5", timer_ms=50), extra=lambda r, s, c: isinstance(r.m, H5) and "timer_ms" in r.m.knob_warning)
    run("H6", "H6", 3, 0)
    run("H7", "H7", 3, 0, extra=lambda r, s, c: all(x.dt_s is not None and 0.15 < x.dt_s < 0.25 for x in r.m.results))
    run("H7 slow drain", "H7", 2, 1, SimConfig(uart_busy_s=0.6))
    run("H8", "H8", 3, 0, extra=lambda r, s, c: any(cmd.startswith("P 2") for cmd in s.commands))
    run("H8 foreign wakes", "H8", 2, 1, SimConfig(foreign_wakes=True))
    run("R1", "R1", 5, 0, extra=lambda r, s, c: [int(x["cycle"]) for x in rows(c)][-1] >= 5)
    run("W2 hang", "W2", 5, 1, SimConfig(stall_at_boot=3), extra=lambda r, s, c: r.hangs == 1 and s.resets == 1 and sum(x.verdict == PASS for x in r.m.results) == 4)
    run("R1 after hang", "R1", 4, 1, SimConfig(stall_at_boot=2), extra=lambda r, s, c: r.hangs == 1 and sum(x.verdict == PASS for x in r.m.results) == 3)
    run("H5 panic", "H5", 5, 1, SimConfig(panic_at_boot=2), extra=lambda r, s, c: r.m.panics == 1 and len(r.m.results) == 5)
    run("wrong firmware", "W1", 3, 2, SimConfig(firmware_case="W2"))

    # Port guard.
    try:
        resolve_stim_port("/dev/cu.usbserial-210", "/dev/cu.usbserial-210")
        failures.append("equal --port/--stim-port accepted")
    except InfraError:
        pass

    print(f"self-test: parser ok, {logs_checked} real log(s) checked, {len(sim_runs)} simulated runs")
    if failures:
        print(f"self-test FAILED ({len(failures)}):")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("self-test: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
