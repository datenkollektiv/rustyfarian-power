//! Shared bench line grammar and pin-window bookkeeping for the tier benches.
//!
//! The ESP-IDF and esp-hal bench examples print the same machine-read lines so one capture
//! script judges both tiers.
//! This module owns the grammar (the formatters) and the window arithmetic ([`PinWindow`]);
//! the loops, the retained records, the stimulus and the environment switches stay in each example.
//!
//! Lines are written without a trailing newline.
//! Levels are `bool`, `true` meaning high.
//!
//! | Line | Formatter |
//! |---|---|
//! | `ATTEMPT tier=<t> n=<k> t_ms=<n>` | [`write_attempt`] |
//! | `PINREAD start tier=<t> pin=<p> level=<H\|L> t_ms=<n>` | [`write_pinread_start`] |
//! | `PINREAD edge tier=<t> pin=<p> <A>-><B> t_ms=<n>` | [`write_pinread_edge`] |
//! | `PINREAD end tier=<t> pin=<p> edges=<n> high=<0\|1> low=<0\|1> t_ms=<n>` | [`write_pinread_end`] |
//! | `NONWAKE tier=<t> path=<hw\|guard> same=<0\|1> holdok=<0\|1> diff=<text>` | [`write_nonwake`] |
//!
//! Only compiled with the off-by-default `bench-conformance` feature (and in host tests).
//! `no_std`, no allocation.
//!
//! # Example
//!
//! ```
//! use stoker::bench::{write_pinread_edge, write_pinread_end, write_pinread_start, LineBuf, PinWindow};
//!
//! let mut window = PinWindow::start(false, 100);
//! let mut line = LineBuf::<96>::new();
//! write_pinread_start(&mut line, "idf", 4, false, 100).unwrap();
//! assert_eq!(line.as_str(), "PINREAD start tier=idf pin=4 level=L t_ms=100");
//!
//! let edge = window.sample(true, 150).unwrap();
//! line.clear();
//! write_pinread_edge(&mut line, "idf", 4, &edge).unwrap();
//! assert_eq!(line.as_str(), "PINREAD edge tier=idf pin=4 L->H t_ms=150");
//!
//! line.clear();
//! write_pinread_end(&mut line, "idf", 4, &window.finish(), 200).unwrap();
//! assert_eq!(line.as_str(), "PINREAD end tier=idf pin=4 edges=1 high=1 low=1 t_ms=200");
//! ```

use core::fmt::{self, Write};

/// One level change seen by [`PinWindow::sample`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// Level before the change (`true` = high).
    pub from: bool,
    /// Level after the change (`true` = high).
    pub to: bool,
    /// Timestamp of the sample that saw the change, in ms.
    pub t_ms: u64,
}

/// What a [`PinWindow`] saw, for the `PINREAD end` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSummary {
    /// Number of level changes.
    pub edges: u32,
    /// The pin was high at the start or at any sample.
    pub high: bool,
    /// The pin was low at the start or at any sample.
    pub low: bool,
}

/// Edge counter for one pin-polling window.
///
/// Create it with the level read right after `try_sleep` returned, feed it every poll with
/// [`sample`](Self::sample), and read the totals with [`finish`](Self::finish).
#[derive(Debug, Clone, Copy)]
pub struct PinWindow {
    start_ms: u64,
    last: bool,
    edges: u32,
    high: bool,
    low: bool,
}

impl PinWindow {
    /// Opens a window at `level`.
    ///
    /// `t_ms` is the start timestamp, kept for [`start_ms`](Self::start_ms); the window has no
    /// clock of its own, the caller bounds the polling loop.
    #[must_use]
    pub const fn start(level: bool, t_ms: u64) -> Self {
        Self {
            start_ms: t_ms,
            last: level,
            edges: 0,
            high: level,
            low: !level,
        }
    }

    /// The start timestamp given to [`start`](Self::start).
    #[must_use]
    pub const fn start_ms(&self) -> u64 {
        self.start_ms
    }

    /// Records one poll; returns the [`Edge`] when the level differs from the previous sample.
    #[must_use]
    pub fn sample(&mut self, level: bool, t_ms: u64) -> Option<Edge> {
        self.high |= level;
        self.low |= !level;
        if level == self.last {
            return None;
        }
        let edge = Edge {
            from: self.last,
            to: level,
            t_ms,
        };
        self.edges += 1;
        self.last = level;
        Some(edge)
    }

    /// Totals so far; the window stays usable.
    #[must_use]
    pub const fn finish(&self) -> WindowSummary {
        WindowSummary {
            edges: self.edges,
            high: self.high,
            low: self.low,
        }
    }
}

const fn level_char(level: bool) -> char {
    if level {
        'H'
    } else {
        'L'
    }
}

/// Which path produced a rejection in a `NONWAKE` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonWakePath {
    /// The hardware rejected the sleep.
    Hw,
    /// The adapter's software pre-read rejected it.
    Guard,
}

impl NonWakePath {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Hw => "hw",
            Self::Guard => "guard",
        }
    }
}

/// Writes `ATTEMPT tier=<tier> n=<n> t_ms=<t_ms>`.
///
/// # Errors
///
/// Returns the writer's error.
pub fn write_attempt<W: Write>(w: &mut W, tier: &str, n: u32, t_ms: u64) -> fmt::Result {
    write!(w, "ATTEMPT tier={tier} n={n} t_ms={t_ms}")
}

/// Writes `PINREAD start tier=<tier> pin=<pin> level=<H|L> t_ms=<t_ms>`.
///
/// # Errors
///
/// Returns the writer's error.
pub fn write_pinread_start<W: Write>(
    w: &mut W,
    tier: &str,
    pin: u8,
    level: bool,
    t_ms: u64,
) -> fmt::Result {
    write!(
        w,
        "PINREAD start tier={tier} pin={pin} level={} t_ms={t_ms}",
        level_char(level)
    )
}

/// Writes `PINREAD edge tier=<tier> pin=<pin> <A>-><B> t_ms=<n>`.
///
/// # Errors
///
/// Returns the writer's error.
pub fn write_pinread_edge<W: Write>(w: &mut W, tier: &str, pin: u8, edge: &Edge) -> fmt::Result {
    write!(
        w,
        "PINREAD edge tier={tier} pin={pin} {}->{} t_ms={}",
        level_char(edge.from),
        level_char(edge.to),
        edge.t_ms
    )
}

/// Writes `PINREAD end tier=<tier> pin=<pin> edges=<n> high=<0|1> low=<0|1> t_ms=<t_ms>`.
///
/// # Errors
///
/// Returns the writer's error.
pub fn write_pinread_end<W: Write>(
    w: &mut W,
    tier: &str,
    pin: u8,
    summary: &WindowSummary,
    t_ms: u64,
) -> fmt::Result {
    write!(
        w,
        "PINREAD end tier={tier} pin={pin} edges={} high={} low={} t_ms={t_ms}",
        summary.edges,
        u8::from(summary.high),
        u8::from(summary.low)
    )
}

/// Writes `NONWAKE tier=<tier> path=<hw|guard> same=<0|1> holdok=<0|1> diff=<diff>`.
///
/// # Errors
///
/// Returns the writer's error.
pub fn write_nonwake<W: Write>(
    w: &mut W,
    tier: &str,
    path: NonWakePath,
    same: bool,
    holdok: bool,
    diff: &str,
) -> fmt::Result {
    write!(
        w,
        "NONWAKE tier={tier} path={} same={} holdok={} diff={diff}",
        path.as_str(),
        u8::from(same),
        u8::from(holdok)
    )
}

/// Fixed-capacity line buffer that always keeps a NUL after the text.
///
/// Implements [`core::fmt::Write`] and truncates silently (at a character boundary) when the
/// text does not fit, so a formatter never fails on it.
/// The usable text length is `N - 1`; `N` must be at least 1.
/// [`as_c_bytes`](Self::as_c_bytes) is the NUL-terminated slice for `esp_rom_printf("%s\n")`.
#[derive(Debug, Clone)]
pub struct LineBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> LineBuf<N> {
    /// An empty buffer.
    ///
    /// # Panics
    ///
    /// Fails to compile for `N == 0`.
    #[must_use]
    pub const fn new() -> Self {
        const { assert!(N >= 1, "LineBuf needs room for the NUL") };
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    /// Empties the buffer.
    pub fn clear(&mut self) {
        self.buf[0] = 0;
        self.len = 0;
    }

    /// The text written so far.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Only whole characters are appended, so this never falls back.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    /// The text followed by its terminating NUL.
    #[must_use]
    pub fn as_c_bytes(&self) -> &[u8] {
        &self.buf[..=self.len]
    }
}

impl<const N: usize> Default for LineBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Write for LineBuf<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = N - 1 - self.len;
        let mut take = s.len().min(room);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        self.buf[self.len] = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line<const N: usize>(f: impl FnOnce(&mut LineBuf<N>) -> fmt::Result) -> std::string::String {
        let mut b = LineBuf::<N>::new();
        f(&mut b).unwrap();
        std::string::String::from(b.as_str())
    }

    #[test]
    fn window_without_edges() {
        let mut w = PinWindow::start(true, 0);
        assert_eq!(w.start_ms(), 0);
        assert_eq!(w.sample(true, 10), None);
        assert_eq!(w.sample(true, 20), None);
        assert_eq!(
            w.finish(),
            WindowSummary {
                edges: 0,
                high: true,
                low: false
            }
        );
        let w = PinWindow::start(false, 0);
        assert_eq!(
            w.finish(),
            WindowSummary {
                edges: 0,
                high: false,
                low: true
            }
        );
    }

    #[test]
    fn window_counts_multiple_edges() {
        let mut w = PinWindow::start(false, 5);
        assert_eq!(
            w.sample(true, 15),
            Some(Edge {
                from: false,
                to: true,
                t_ms: 15
            })
        );
        assert_eq!(w.sample(true, 25), None);
        assert_eq!(
            w.sample(false, 35),
            Some(Edge {
                from: true,
                to: false,
                t_ms: 35
            })
        );
        assert!(w.sample(true, 45).is_some());
        assert_eq!(
            w.finish(),
            WindowSummary {
                edges: 3,
                high: true,
                low: true
            }
        );
    }

    #[test]
    fn lines_match_the_example_output() {
        assert_eq!(
            line::<64>(|w| write_attempt(w, "idf", 2, 1234)),
            "ATTEMPT tier=idf n=2 t_ms=1234"
        );
        assert_eq!(
            line::<96>(|w| write_pinread_start(w, "hal", 4, true, 7)),
            "PINREAD start tier=hal pin=4 level=H t_ms=7"
        );
        let edge = Edge {
            from: true,
            to: false,
            t_ms: 99,
        };
        assert_eq!(
            line::<96>(|w| write_pinread_edge(w, "hal", 5, &edge)),
            "PINREAD edge tier=hal pin=5 H->L t_ms=99"
        );
        let summary = WindowSummary {
            edges: 3,
            high: true,
            low: false,
        };
        assert_eq!(
            line::<128>(|w| write_pinread_end(w, "idf", 4, &summary, 8000)),
            "PINREAD end tier=idf pin=4 edges=3 high=1 low=0 t_ms=8000"
        );
        assert_eq!(
            line::<128>(|w| write_nonwake(w, "idf", NonWakePath::Hw, true, false, "none")),
            "NONWAKE tier=idf path=hw same=1 holdok=0 diff=none"
        );
        assert_eq!(
            line::<128>(|w| write_nonwake(w, "hal", NonWakePath::Guard, false, true, "gpio39:ie")),
            "NONWAKE tier=hal path=guard same=0 holdok=1 diff=gpio39:ie"
        );
    }

    #[test]
    fn line_buf_truncates_and_terminates() {
        let mut b = LineBuf::<6>::new();
        assert_eq!(b.as_c_bytes(), b"\0");
        b.write_str("abc").unwrap();
        assert_eq!(b.as_c_bytes(), b"abc\0");
        b.write_str("defgh").unwrap();
        assert_eq!(b.as_str(), "abcde");
        assert_eq!(b.as_c_bytes(), b"abcde\0");
        b.write_str("x").unwrap();
        assert_eq!(b.as_c_bytes(), b"abcde\0");
        b.clear();
        assert_eq!(b.as_str(), "");
        assert_eq!(b.as_c_bytes(), b"\0");
    }

    #[test]
    fn line_buf_truncates_at_char_boundary() {
        let mut b = LineBuf::<4>::new();
        b.write_str("a\u{e9}\u{e9}").unwrap();
        assert_eq!(b.as_str(), "a\u{e9}");
        assert_eq!(b.as_c_bytes(), "a\u{e9}\0".as_bytes());
    }
}
