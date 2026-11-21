//! Where the app gets the time from (`coding_guideline.md` section 7, MUSS): never from
//! `Utc::now()`/`Instant::now()` directly, but from a `Clock`. The running app uses `SystemClock`;
//! tests use `TestClock` and set the moment they test (a time of day in New York, a weekday, an
//! expiration day), so they do not depend on when they run.
//!
//! Times are kept in UTC (section 7); New York time is worked out from it where the market's
//! hours or an expiration date need it.

use chrono::{DateTime, Local, NaiveDateTime, Utc};
use std::time::Instant;

/// The source of the current time.
pub trait Clock {
    /// The current moment in UTC.
    fn utc(&self) -> DateTime<Utc>;
    /// A monotonic point in time, for timers and timeouts.
    fn instant(&self) -> Instant;

    /// The current time in New York (the market's and the expirations' time zone).
    fn new_york(&self) -> NaiveDateTime {
        self.utc().with_timezone(&chrono_tz::America::New_York).naive_local()
    }

    /// Today's date in New York as yyyyMMdd: option expirations are dated there.
    fn new_york_today(&self) -> String {
        self.new_york().format("%Y%m%d").to_string()
    }

    /// The current time in the computer's own time zone (for what the user reads: log lines, the
    /// day a trade was opened, file names).
    fn local(&self) -> DateTime<Local> {
        self.utc().with_timezone(&Local)
    }
}

/// The real time of the computer.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl SystemClock {
    /// The current moment as Unix seconds (UTC), for a thread that cannot share the app's clock (the webhook receiver).
    #[must_use]
    pub fn unix_seconds() -> i64 {
        Utc::now().timestamp()
    }
}

impl Clock for SystemClock {
    fn utc(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn instant(&self) -> Instant {
        Instant::now()
    }
}

/// Whether `period` has passed since `last` (or nothing was done yet); when it has, `last` becomes `now`. For work that runs at most
/// once every `period`.
pub fn due(last: &mut Option<Instant>, now: Instant, period: std::time::Duration) -> bool {
    if last.is_some_and(|at| now.saturating_duration_since(at) < period) {
        return false;
    }
    *last = Some(now);
    true
}

/// A clock for tests: it stands at the moment it is set to and moves only when told.
#[cfg(test)]
#[derive(Debug)]
pub struct TestClock {
    utc: std::cell::Cell<DateTime<Utc>>,
    instant: std::cell::Cell<Instant>,
}

#[cfg(test)]
impl TestClock {
    /// A clock at `new_york` (a New York local time, e.g. 15:01 on an expiration day).
    pub fn at_new_york(new_york: NaiveDateTime) -> Self {
        use chrono::TimeZone;
        let utc = chrono_tz::America::New_York.from_local_datetime(&new_york).earliest().map_or_else(Utc::now, |t| t.with_timezone(&Utc));
        Self { utc: std::cell::Cell::new(utc), instant: std::cell::Cell::new(Instant::now()) }
    }

    /// Moves the clock on by `by`.
    pub fn advance(&self, by: std::time::Duration) {
        if let Ok(step) = chrono::Duration::from_std(by) {
            self.utc.set(self.utc.get() + step);
        }
        self.instant.set(self.instant.get() + by);
    }
}

#[cfg(test)]
impl Clock for TestClock {
    fn utc(&self) -> DateTime<Utc> {
        self.utc.get()
    }

    fn instant(&self) -> Instant {
        self.instant.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn a_test_clock_stands_where_it_is_set_and_moves_only_when_told() {
        // 15:01 in New York on 13-11-2026 (EST, UTC-5) is 20:01 UTC.
        let clock = TestClock::at_new_york(ny("2026-11-13 15:01"));
        assert_eq!(clock.new_york(), ny("2026-11-13 15:01"));
        assert_eq!(clock.utc().format("%Y-%m-%d %H:%M").to_string(), "2026-11-13 20:01");
        assert_eq!(clock.new_york_today(), "20261113");
        let start = clock.instant();
        assert_eq!(clock.instant(), start, "it does not move by itself");
        // Past midnight in New York the expiration day changes.
        clock.advance(std::time::Duration::from_hours(9));
        assert_eq!((clock.new_york_today(), clock.instant() - start), ("20261114".to_string(), std::time::Duration::from_hours(9)));
    }

    #[test]
    fn summer_time_in_new_york_is_four_hours_behind_utc() {
        let clock = TestClock::at_new_york(ny("2026-07-01 09:30"));
        assert_eq!(clock.utc().format("%H:%M").to_string(), "13:30");
    }

    #[test]
    fn the_system_clock_is_the_real_time() {
        let before = Utc::now();
        let now = SystemClock.utc();
        assert!(now >= before && now <= Utc::now());
    }

    #[test]
    fn work_is_due_at_first_and_then_once_a_period() {
        let start = Instant::now();
        let period = std::time::Duration::from_secs(2);
        let mut last = None;
        assert!(due(&mut last, start, period), "nothing done yet");
        assert!(!due(&mut last, start + std::time::Duration::from_secs(1), period));
        assert_eq!(last, Some(start), "a check that is not due changes nothing");
        assert!(due(&mut last, start + period, period));
        assert_eq!(last, Some(start + period));
    }
}
