//! The US stock and options market calendar: weekends, the exchange holidays and the days with
//! an early close (13:00). Pure logic on dates and times in New York (`coding_guideline.md`
//! section 2): the caller passes the New York date and time, nothing here reads the clock.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

/// Where the regular session of a day stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    /// Regular trading hours (09:30 until the close).
    Open,
    /// A trading day, before 09:30.
    BeforeOpen,
    /// A trading day, after the close (16:00, or 13:00 on an early-close day).
    AfterClose,
    /// Saturday or Sunday.
    Weekend,
    /// An exchange holiday: its English and German name.
    Holiday(&'static str, &'static str),
}

/// The date of the `n`-th (1-based) `weekday` of a month.
fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: u32) -> Option<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let offset = (7 + weekday.num_days_from_monday() - first.weekday().num_days_from_monday()) % 7;
    first.checked_add_signed(Duration::days(i64::from(offset) + i64::from(n.checked_sub(1)?) * 7))
}

/// The date of the last `weekday` of a month.
fn last_weekday(year: i32, month: u32, weekday: Weekday) -> Option<NaiveDate> {
    let next_month_first =
        if month == 12 { NaiveDate::from_ymd_opt(year.checked_add(1)?, 1, 1)? } else { NaiveDate::from_ymd_opt(year, month.checked_add(1)?, 1)? };
    let last = next_month_first.checked_sub_signed(Duration::days(1))?;
    let back = (7 + last.weekday().num_days_from_monday() - weekday.num_days_from_monday()) % 7;
    last.checked_sub_signed(Duration::days(i64::from(back)))
}

/// Easter Sunday (the anonymous Gregorian algorithm).
fn easter(year: i32) -> Option<NaiveDate> {
    let golden = year % 19;
    let century = year / 100;
    let in_century = year % 100;
    let leap_centuries = century / 4;
    let century_rest = century % 4;
    let moon_step = (century + 8) / 25;
    let moon_fix = (century - moon_step + 1) / 3;
    let moon = (19 * golden + century - leap_centuries - moon_fix + 15) % 30;
    let leap_years = in_century / 4;
    let leap_rest = in_century % 4;
    let weekday = (32 + 2 * century_rest + 2 * leap_years - moon - leap_rest) % 7;
    let correction = (golden + 11 * moon + 22 * weekday) / 451;
    let month = (moon + weekday - 7 * correction + 114) / 31;
    let day = (moon + weekday - 7 * correction + 114) % 31 + 1;
    NaiveDate::from_ymd_opt(year, u32::try_from(month).ok()?, u32::try_from(day).ok()?)
}

/// A fixed-date holiday: on a Saturday the exchange closes the Friday before, on a Sunday the
/// Monday after.
fn observed(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    match date.weekday() {
        Weekday::Sat => date.checked_sub_signed(Duration::days(1)),
        Weekday::Sun => date.checked_add_signed(Duration::days(1)),
        _ => Some(date),
    }
}

/// The exchange holiday on `date`, as its English and German name.
#[must_use]
pub fn holiday(date: NaiveDate) -> Option<(&'static str, &'static str)> {
    let year = date.year();
    // New Year's Day on a Saturday is not made up on the Friday (it would be in the old year).
    let new_year = NaiveDate::from_ymd_opt(year, 1, 1).filter(|d| d.weekday() != Weekday::Sat).and_then(|d| {
        if d.weekday() == Weekday::Sun {
            d.checked_add_signed(Duration::days(1))
        } else {
            Some(d)
        }
    });
    let candidates: [(Option<NaiveDate>, &'static str, &'static str); 10] = [
        (new_year, "New Year's Day", "Neujahr"),
        (nth_weekday(year, 1, Weekday::Mon, 3), "Martin Luther King Jr. Day", "Martin-Luther-King-Tag"),
        (nth_weekday(year, 2, Weekday::Mon, 3), "Presidents' Day", "Presidents' Day"),
        (easter(year).and_then(|e| e.checked_sub_signed(Duration::days(2))), "Good Friday", "Karfreitag"),
        (last_weekday(year, 5, Weekday::Mon), "Memorial Day", "Memorial Day"),
        (observed(year, 6, 19), "Juneteenth", "Juneteenth"),
        (observed(year, 7, 4), "Independence Day", "Unabhängigkeitstag"),
        (nth_weekday(year, 9, Weekday::Mon, 1), "Labor Day", "Labor Day"),
        (nth_weekday(year, 11, Weekday::Thu, 4), "Thanksgiving", "Thanksgiving"),
        (observed(year, 12, 25), "Christmas Day", "Weihnachten"),
    ];
    candidates.iter().find(|(d, _, _)| *d == Some(date)).map(|(_, en, de)| (*en, *de))
}

/// A trading day that closes at 13:00: the day after Thanksgiving, July 3 and December 24 when
/// they fall on a weekday and are not holidays themselves.
#[must_use]
pub fn early_close(date: NaiveDate) -> bool {
    if holiday(date).is_some() || matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    let day_after_thanksgiving = nth_weekday(date.year(), 11, Weekday::Thu, 4).and_then(|d| d.checked_add_signed(Duration::days(1)));
    day_after_thanksgiving == Some(date) || (date.month() == 7 && date.day() == 3) || (date.month() == 12 && date.day() == 24)
}

/// The time the regular session of `date` ends (16:00, or 13:00 on an early-close day), New York time.
#[must_use]
pub fn close_time(date: NaiveDate) -> NaiveTime {
    let hour = if early_close(date) { 13 } else { 16 };
    NaiveTime::from_hms_opt(hour, 0, 0).unwrap_or(NaiveTime::MIN)
}

/// Where the regular session stands at `now` (New York date and time).
#[must_use]
pub fn session_at(now: NaiveDateTime) -> Session {
    let date = now.date();
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return Session::Weekend;
    }
    if let Some((en, de)) = holiday(date) {
        return Session::Holiday(en, de);
    }
    let open = NaiveTime::from_hms_opt(9, 30, 0).unwrap_or(NaiveTime::MIN);
    let close = close_time(date);
    let time = now.time();
    if time < open {
        Session::BeforeOpen
    } else if time < close {
        Session::Open
    } else {
        Session::AfterClose
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_holidays_of_2026_are_found() {
        for (d, name) in [
            (date(2026, 1, 1), "New Year's Day"),
            (date(2026, 1, 19), "Martin Luther King Jr. Day"),
            (date(2026, 2, 16), "Presidents' Day"),
            (date(2026, 4, 3), "Good Friday"),
            (date(2026, 5, 25), "Memorial Day"),
            (date(2026, 6, 19), "Juneteenth"),
            (date(2026, 7, 3), "Independence Day"), // July 4 is a Saturday: closed on the Friday
            (date(2026, 9, 7), "Labor Day"),
            (date(2026, 11, 26), "Thanksgiving"),
            (date(2026, 12, 25), "Christmas Day"),
        ] {
            assert_eq!(holiday(d).map(|(en, _)| en), Some(name), "{d}");
        }
        assert_eq!(holiday(date(2026, 7, 4)), None, "the Saturday itself is a weekend, not the holiday");
        assert_eq!(holiday(date(2026, 11, 13)), None);
    }

    #[test]
    fn a_sunday_holiday_moves_to_the_monday_and_a_saturday_new_year_does_not_move() {
        assert_eq!(holiday(date(2027, 12, 24)).map(|(en, _)| en), Some("Christmas Day"), "Dec 25, 2027 is a Saturday");
        assert_eq!(holiday(date(2028, 1, 3)).map(|(en, _)| en), None, "Jan 1, 2028 is a Saturday: no make-up day");
        assert_eq!(holiday(date(2028, 1, 1)), None);
        assert_eq!(holiday(date(2023, 1, 2)).map(|(en, _)| en), Some("New Year's Day"), "Jan 1, 2023 was a Sunday");
        assert_eq!(holiday(date(2027, 3, 26)).map(|(en, _)| en), Some("Good Friday"), "Easter 2027 is March 28");
    }

    #[test]
    fn the_early_close_days_are_found() {
        assert!(early_close(date(2026, 11, 27)), "the day after Thanksgiving");
        assert!(early_close(date(2026, 12, 24)));
        assert!(!early_close(date(2026, 7, 3)), "July 3, 2026 is the observed holiday");
        assert!(early_close(date(2028, 7, 3)), "a Monday before July 4");
        assert!(!early_close(date(2026, 11, 12)));
    }

    #[test]
    fn the_session_follows_weekend_holiday_and_hours() {
        assert_eq!(session_at(at(2026, 11, 14, 12, 0)), Session::Weekend);
        assert_eq!(session_at(at(2026, 11, 26, 11, 0)), Session::Holiday("Thanksgiving", "Thanksgiving"));
        assert_eq!(session_at(at(2026, 11, 13, 9, 29)), Session::BeforeOpen);
        assert_eq!(session_at(at(2026, 11, 13, 9, 30)), Session::Open);
        assert_eq!(session_at(at(2026, 11, 13, 15, 59)), Session::Open);
        assert_eq!(session_at(at(2026, 11, 13, 16, 0)), Session::AfterClose);
        // An early-close day is over at 13:00.
        assert_eq!(session_at(at(2026, 11, 27, 12, 59)), Session::Open);
        assert_eq!(session_at(at(2026, 11, 27, 13, 0)), Session::AfterClose);
    }
}
