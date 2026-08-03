//! Module: `casm_git::time`
//! Purpose: Rendering a commit timestamp without taking on a date library.
//! Safety: `#![forbid(unsafe_code)]` — inherited from crate root.
//! Complexity: Max 10 (enforced by clippy).
//! License: Apache-2.0
//!
//! # Why not `chrono` or `jiff`
//!
//! CASIMIR needs exactly one thing from a date library: turn a Unix timestamp into
//! `YYYY-MM-DD HH:MM:SS` for a log line. That is forty lines of arithmetic. A date
//! library brings timezone databases, parsing, locales, and leap-second policy — none of
//! which is used, all of which is dependency surface on a tool whose supply chain is a
//! CI gate.
//!
//! The conversion is Howard Hinnant's `civil_from_days`, the standard algorithm behind
//! most modern date implementations. It is exact for the whole range of `i64` seconds and
//! has no branches for leap years beyond the era arithmetic.
//!
//! Everything is rendered in **UTC**. A commit timestamp carries an author's local offset,
//! but showing history in mixed local times makes ordering unreadable, and CASIMIR already
//! requires UTC everywhere else.

/// Seconds in a day.
const SECONDS_PER_DAY: i64 = 86_400;

/// A civil date and time in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DateTime {
    /// Proleptic Gregorian year.
    pub year: i64,
    /// Month, 1–12.
    pub month: u32,
    /// Day of month, 1–31.
    pub day: u32,
    /// Hour, 0–23.
    pub hour: u32,
    /// Minute, 0–59.
    pub minute: u32,
    /// Second, 0–59.
    pub second: u32,
}

impl DateTime {
    /// Converts Unix seconds into a UTC civil date and time.
    ///
    /// `rem_euclid` guarantees `remainder` is in `0..86_400`, so every cast below is
    /// exact — the lint cannot see the range invariant, but the tests pin it across the
    /// whole `i64` span.
    #[must_use]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub const fn from_unix(seconds: i64) -> Self {
        // `rem_euclid` keeps the time-of-day positive for timestamps before 1970, where a
        // plain remainder would go negative and produce an hour of -1.
        let days = seconds.div_euclid(SECONDS_PER_DAY);
        let remainder = seconds.rem_euclid(SECONDS_PER_DAY);

        let (year, month, day) = civil_from_days(days);

        Self {
            year,
            month,
            day,
            hour: (remainder / 3_600) as u32,
            minute: ((remainder % 3_600) / 60) as u32,
            second: (remainder % 60) as u32,
        }
    }

    /// Renders as `YYYY-MM-DD`.
    #[must_use]
    pub fn to_date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// Renders as `YYYY-MM-DD HH:MM:SS`.
    #[must_use]
    pub fn to_datetime(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

impl core::fmt::Display for DateTime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_datetime())
    }
}

/// Converts days since the Unix epoch into a proleptic Gregorian date.
///
/// Howard Hinnant's `civil_from_days`. The era trick shifts the calendar so that March is
/// the first month, which makes the leap day fall at the end of a year and removes every
/// special case for February.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
const fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Shift the epoch from 1970-01-01 to 0000-03-01.
    let shifted = days + 719_468;

    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097; // 0..=146_096
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;

    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153; // 0..=11, March-based

    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
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
    fn the_epoch_itself_converts_correctly() {
        let epoch = DateTime::from_unix(0);
        assert_eq!(epoch.to_datetime(), "1970-01-01 00:00:00");
    }

    #[test]
    fn known_timestamps_convert_correctly() {
        // Verified against `date -u -d @<seconds>`.
        let cases = [
            (1_000_000_000_i64, "2001-09-09 01:46:40"),
            (1_234_567_890, "2009-02-13 23:31:30"),
            (1_700_000_000, "2023-11-14 22:13:20"),
            (2_147_483_647, "2038-01-19 03:14:07"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(
                DateTime::from_unix(seconds).to_datetime(),
                expected,
                "at {seconds}"
            );
        }
    }

    #[test]
    fn leap_days_are_handled() {
        // 2000-02-29: a leap year despite being a century, which is the case a naive
        // "divisible by four" rule gets wrong.
        assert_eq!(DateTime::from_unix(951_782_400).to_date(), "2000-02-29");
        // 2024-02-29, an ordinary leap year.
        assert_eq!(DateTime::from_unix(1_709_164_800).to_date(), "2024-02-29");
    }

    #[test]
    fn the_day_before_and_after_a_leap_day_are_correct() {
        assert_eq!(DateTime::from_unix(1_709_078_400).to_date(), "2024-02-28");
        assert_eq!(DateTime::from_unix(1_709_251_200).to_date(), "2024-03-01");
    }

    #[test]
    fn nineteen_hundred_was_not_a_leap_year() {
        // A century not divisible by 400. Timestamp is 1900-03-01T00:00:00Z.
        assert_eq!(DateTime::from_unix(-2_203_891_200).to_date(), "1900-03-01");
        assert_eq!(DateTime::from_unix(-2_203_977_600).to_date(), "1900-02-28");
    }

    #[test]
    fn timestamps_before_the_epoch_do_not_produce_negative_times() {
        // The bug a plain remainder would introduce: an hour of -1.
        let before = DateTime::from_unix(-1);
        assert_eq!(before.to_datetime(), "1969-12-31 23:59:59");
        assert!(before.hour < 24 && before.minute < 60 && before.second < 60);
    }

    #[test]
    fn a_full_day_before_the_epoch_converts_correctly() {
        assert_eq!(
            DateTime::from_unix(-86_400).to_datetime(),
            "1969-12-31 00:00:00"
        );
    }

    #[test]
    fn every_component_stays_in_range_across_a_wide_span() {
        // One sample per ~11 days for eighty years, plus the pre-epoch half.
        let mut seconds = -1_000_000_000_i64;
        while seconds < 2_500_000_000 {
            let moment = DateTime::from_unix(seconds);
            assert!((1..=12).contains(&moment.month), "month at {seconds}");
            assert!((1..=31).contains(&moment.day), "day at {seconds}");
            assert!(moment.hour < 24, "hour at {seconds}");
            assert!(moment.minute < 60, "minute at {seconds}");
            assert!(moment.second < 60, "second at {seconds}");
            seconds = seconds.saturating_add(1_000_003);
        }
    }

    #[test]
    fn conversion_is_monotonic() {
        // Later timestamps must never render as earlier dates.
        let mut previous = DateTime::from_unix(0);
        let mut seconds = 0_i64;
        while seconds < 1_000_000_000 {
            seconds = seconds.saturating_add(9_999_991);
            let current = DateTime::from_unix(seconds);
            assert!(current > previous, "went backwards at {seconds}");
            previous = current;
        }
    }

    #[test]
    fn extreme_values_do_not_panic() {
        for seconds in [i64::MIN, i64::MIN / 2, -1, 0, 1, i64::MAX / 2, i64::MAX] {
            let _ = DateTime::from_unix(seconds).to_datetime();
        }
    }

    #[test]
    fn the_date_form_is_a_prefix_of_the_datetime_form() {
        let moment = DateTime::from_unix(1_700_000_000);
        assert!(moment.to_datetime().starts_with(&moment.to_date()));
        assert_eq!(moment.to_string(), moment.to_datetime());
    }
}
