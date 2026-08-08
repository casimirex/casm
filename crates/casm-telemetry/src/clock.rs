//! Module: `casm_telemetry::clock`
//! Purpose: Nanosecond UTC timestamps, and a way to make them deterministic in a test.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why the clock is a value
//!
//! Everything else in CASM is a pure function, which is what lets the test suite assert
//! on exact output. Telemetry cannot be: a span that does not know when it started is not
//! telemetry. Reading the system clock directly would make every record in this crate
//! untestable except by shape.
//!
//! So the clock is a value the recorder holds. [`Clock::System`] reads the real one;
//! [`Clock::stepping`] returns a fixed sequence, which makes an entire exported document
//! byte-for-byte reproducible. The tests use the second and the binaries use the first,
//! and neither path is special-cased in the code between them.
//!
//! # Two clocks, because they answer different questions
//!
//! Wall-clock time says *when* something happened and is what a collector correlates on.
//! It can also jump backwards, when NTP corrects it. Elapsed time must not go backwards,
//! or a span acquires a negative duration.
//!
//! This crate reads the wall clock once per timestamp and computes durations by
//! subtraction, saturating at zero. That is a deliberate simplification over carrying a
//! monotonic `Instant` alongside every timestamp: a backwards jump mid-span yields a
//! zero-length span rather than a wrong one, and spans here last milliseconds, which is
//! far below the granularity at which NTP corrections matter.

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

/// A point in time, as nanoseconds since the Unix epoch, UTC.
///
/// `u64` nanoseconds runs out in the year 2554. The alternative, `u128`, doubles the width
/// of every record for a range nothing needs.
///
/// Serialises as a **string**, not a number: JSON numbers are doubles, which lose precision
/// above 2^53, and a nanosecond timestamp passed that in 1970. This is the same reason OTLP
/// encodes them as strings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(u64);

impl Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl Timestamp {
    /// A timestamp from a raw nanosecond count.
    #[must_use]
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// The raw nanosecond count, which is what OTLP carries on the wire.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Whole milliseconds since the epoch, for a human-facing summary.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000_000
    }

    /// Nanoseconds from `self` to `later`, saturating at zero.
    ///
    /// Saturating rather than signed: a clock that stepped backwards mid-span is a
    /// property of the machine, not of the operation being measured, and a zero-length
    /// span is a less misleading record than a negative one.
    #[must_use]
    pub const fn elapsed_to(self, later: Self) -> u64 {
        later.0.saturating_sub(self.0)
    }
}

/// Where timestamps come from.
///
/// Cloneable and cheap: a recorder holds one, and handing out copies costs nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Clock {
    /// The machine's wall clock.
    #[default]
    System,
    /// A fixed sequence, for tests: starts at `start` and advances by `step` per reading.
    Stepping {
        /// The next value to be returned.
        next: u64,
        /// How far to advance after each reading.
        step: u64,
    },
}

impl Clock {
    /// A clock that starts at `start` nanoseconds and advances `step` per reading.
    ///
    /// One reading per timestamp means a span opened and closed with nothing between it
    /// lasts exactly `step` nanoseconds, which is what makes an exported document
    /// predictable enough to assert on in full.
    #[must_use]
    pub const fn stepping(start: u64, step: u64) -> Self {
        Self::Stepping { next: start, step }
    }

    /// Reads the current time, advancing a stepping clock.
    ///
    /// Never fails. A system clock set before the Unix epoch — which requires deliberate
    /// misconfiguration — reads as zero rather than propagating an error into every call
    /// site that only wanted to timestamp a span.
    pub fn now(&mut self) -> Timestamp {
        match self {
            Self::System => Timestamp(system_nanos()),
            Self::Stepping { next, step } => {
                let reading = *next;
                *next = next.saturating_add(*step);
                Timestamp(reading)
            }
        }
    }
}

/// Nanoseconds since the Unix epoch, or zero if the clock is set before it.
fn system_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn a_stepping_clock_is_a_predictable_sequence() {
        let mut clock = Clock::stepping(1_000, 250);

        assert_eq!(clock.now().as_nanos(), 1_000);
        assert_eq!(clock.now().as_nanos(), 1_250);
        assert_eq!(clock.now().as_nanos(), 1_500);
    }

    #[test]
    fn the_system_clock_is_after_the_epoch_and_moves_forward() {
        let mut clock = Clock::System;
        let first = clock.now();
        let second = clock.now();

        // 2020-01-01, comfortably before any machine that could run this.
        assert!(first.as_nanos() > 1_577_836_800_000_000_000, "{first:?}");
        assert!(second >= first);
    }

    #[test]
    fn elapsed_time_never_runs_backwards() {
        let later = Timestamp::from_nanos(500);
        let earlier = Timestamp::from_nanos(100);

        assert_eq!(earlier.elapsed_to(later), 400);
        assert_eq!(
            later.elapsed_to(earlier),
            0,
            "a backwards clock is not negative"
        );
    }

    #[test]
    fn a_stepping_clock_saturates_rather_than_wrapping() {
        // Overflow checks are on in dev builds, so a wrapping add here would panic — in
        // telemetry, which must never be the thing that takes a process down.
        let mut clock = Clock::stepping(u64::MAX - 1, 1_000);

        assert_eq!(clock.now().as_nanos(), u64::MAX - 1);
        assert_eq!(clock.now().as_nanos(), u64::MAX);
        assert_eq!(clock.now().as_nanos(), u64::MAX);
    }

    #[test]
    fn milliseconds_are_whole_and_truncating() {
        assert_eq!(Timestamp::from_nanos(1_999_999).as_millis(), 1);
        assert_eq!(Timestamp::from_nanos(2_000_000).as_millis(), 2);
        assert_eq!(Timestamp::from_nanos(0).as_millis(), 0);
    }
}
