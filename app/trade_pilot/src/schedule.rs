//! schedules: a template trade that is entered by itself at a fixed time of day on chosen trading
//! days. This module holds what a schedule *is*: the
//! record, its check on saving, and its file (`schedules.json` per mode, written like the
//! templates: as a whole, an unreadable file is set aside and never overwritten). When a schedule
//! is due and what is done then are later stages.

use crate::list_file::{load_from, write_to, Loaded};
use crate::market_calendar::holiday;
use crate::storage::mode_dir;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

/// The trading days a schedule runs on: any of Monday to Friday (the exchange is closed at the
/// weekend). Stored in the file as the list of day names, e.g. `["Mon","Wed"]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<String>", into = "Vec<String>")]
pub struct Days(u8);

/// The names of the days as stored and shown, Monday first.
const DAY_NAMES: [(&str, Weekday); 5] = [("Mon", Weekday::Mon), ("Tue", Weekday::Tue), ("Wed", Weekday::Wed), ("Thu", Weekday::Thu), ("Fri", Weekday::Fri)];

impl Days {
    /// Monday to Friday.
    pub const WEEKDAYS: Self = Self(0b1_1111);

    /// No day at all (a schedule must have at least one).
    pub const NONE: Self = Self(0);

    /// The bit of `day`, `None` for Saturday and Sunday.
    fn bit(day: Weekday) -> Option<u8> {
        DAY_NAMES.iter().position(|(_, d)| *d == day).and_then(|i| u8::try_from(i).ok()).map(|i| 1 << i)
    }

    /// Whether the schedule runs on `day`; never at the weekend.
    #[must_use]
    pub fn contains(self, day: Weekday) -> bool {
        Self::bit(day).is_some_and(|bit| self.0 & bit != 0)
    }

    /// These days with `day` switched on or off (Saturday and Sunday are ignored).
    #[must_use]
    pub fn with(self, day: Weekday, on: bool) -> Self {
        match Self::bit(day) {
            Some(bit) if on => Self(self.0 | bit),
            Some(bit) => Self(self.0 & !bit),
            None => self,
        }
    }

    /// Whether no day is chosen (the schedule then runs Monday to Friday, as the days of a schedule that is not narrowed).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The days as text for the grid: "Mon-Fri" for all five, else the names, e.g. "Mon, Wed".
    #[must_use]
    pub fn label(self) -> String {
        if self == Self::WEEKDAYS {
            return "Mon-Fri".to_string();
        }
        Vec::<String>::from(self).join(", ")
    }
}

impl From<Days> for Vec<String> {
    fn from(days: Days) -> Self {
        DAY_NAMES.iter().filter(|(_, day)| days.contains(*day)).map(|(name, _)| (*name).to_string()).collect()
    }
}

impl From<Vec<String>> for Days {
    /// Names that are no weekday (`"Sat"`, a typo) are left out; the check on saving then asks for a day
    /// if none is left.
    fn from(names: Vec<String>) -> Self {
        DAY_NAMES.iter().filter(|(name, _)| names.iter().any(|n| n.eq_ignore_ascii_case(name))).fold(Self::NONE, |days, (_, day)| days.with(*day, true))
    }
}

/// How many contracts a schedule enters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuantityRule {
    /// Always `count` contracts.
    Fixed {
        /// The number of contracts.
        count: u32,
    },
    /// As many contracts as fit into `budget` dollars of premium: the budget divided by the
    /// entry price of one contract, rounded down, but at least `min` and (if `max` is not 0) at
    /// most `max`. Debit and credit trades alike.
    PremiumAllocation {
        /// The dollars of premium to use.
        budget: Decimal,
        /// The most contracts; 0 = no limit.
        max: u32,
        /// The fewest contracts, entered even if the budget is too small.
        min: u32,
    },
    /// As many contracts as `percent` of the net liquidation value of the account buy, divided by
    /// the buying power one contract needs, rounded down, within `min` and `max` like above.
    PctOfNetLiq {
        /// The percent of the net liquidation value (e.g. 2.5).
        percent: Decimal,
        /// The most contracts; 0 = no limit.
        max: u32,
        /// The fewest contracts.
        min: u32,
    },
}

impl Default for QuantityRule {
    /// No contracts: a schedule read from an incomplete file does not enter anything until it is set.
    fn default() -> Self {
        Self::Fixed { count: 0 }
    }
}

impl QuantityRule {
    /// The number of contracts for a trade priced at `entry_price` (credit or debit, per contract
    /// in points), or `None` when there are none to enter. `buying_power` is what one contract needs
    /// of the account (`None` when it cannot be worked out) and `net_liquidation` the net
    /// liquidation value of the account; each is only needed by the rule that uses it.
    /// - `Fixed`: the count.
    /// - `PremiumAllocation`: the budget divided by the cost of one contract (the price times 100),
    ///   rounded down.
    /// - `PctOfNetLiq`: the percent of the net liquidation value divided by the buying power of one
    ///   contract, rounded down.
    ///
    /// The result is limited to `max` (if not 0) and raised to `min`.
    #[must_use]
    pub fn quantity(&self, entry_price: Decimal, buying_power: Option<Decimal>, net_liquidation: Option<Decimal>) -> Option<u32> {
        let (contracts, max, min) = match self {
            Self::Fixed { count } => return (*count > 0).then_some(*count),
            Self::PremiumAllocation { budget, max, min } => {
                let cost = entry_price.abs().checked_mul(Decimal::ONE_HUNDRED)?;
                (budget.checked_div(cost)?, *max, *min)
            }
            Self::PctOfNetLiq { percent, max, min } => {
                let allocated = percent.checked_div(Decimal::ONE_HUNDRED)?.checked_mul(net_liquidation?)?;
                (allocated.checked_div(buying_power?)?, *max, *min)
            }
        };
        let mut quantity = contracts.floor().to_u32()?;
        if max > 0 && quantity > max {
            quantity = max;
        }
        if quantity < min {
            quantity = min;
        }
        (quantity > 0).then_some(quantity)
    }
    /// The rule as the grid shows it: the number, `$` and the budget, or the percent with `%`.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Fixed { count } => count.to_string(),
            Self::PremiumAllocation { budget, .. } => format!("${}", budget.normalize()),
            Self::PctOfNetLiq { percent, .. } => format!("{}%", percent.normalize()),
        }
    }
}
/// One schedule: enter `template_id` with `quantity` contracts at `hour`:`minute` New York time on
/// the chosen days.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    /// The id (1, 2, ...), unique in the file; 0 for one that is not saved yet.
    pub id: u32,
    /// A name for the grid; may be empty.
    pub name: String,
    /// The hour of the day, 0 to 23, New York time (the time of the exchange, so it does not move
    /// with the time zone of the computer).
    pub hour: u8,
    /// The minute, 0 to 59.
    pub minute: u8,
    /// The second of the minute, 0 to 59.
    pub second: u8,
    /// How many minutes after the time the schedule may still enter (the window); 0 = only at the time.
    pub expiration_minutes: u32,
    /// The template (`TradeTemplate::TradeTemplateID`) that is entered.
    pub template_id: i32,
    /// How many contracts the entry has.
    pub quantity: QuantityRule,
    /// After this many tries the alternate template is used (and, without one, the run is cancelled); 0 = no
    /// limit, tries go on until the window ends.
    pub max_attempts: u32,
    /// The template used once `max_attempts` is reached (`id`); 0 = none.
    pub alternate_template_id: i32,
    /// The schedule that must have entered its trade today before this one runs (`Schedule::id`); 0 = none.
    pub linked_schedule_id: u32,
    /// The account the order goes to (`IB:` prefix not included), e.g. `DU1234567`.
    pub account: String,
    /// A switched-off schedule is kept but never runs.
    pub active: bool,
    /// Report it when a run of this schedule expires without an entry.
    pub notify_on_expiry: bool,
    /// The days it runs on; none chosen means Monday to Friday.
    pub days: Days,
    /// Days it does not run, as typed: dates as YYYY-MM-DD, DD-MM-YYYY or DD.MM.YYYY, separated by commas or line
    /// breaks; anything that is no date (a comment such as FOMC) is ignored.
    pub blackout_dates: String,
    /// If this is not empty the schedule runs only on these days (same format); the days and the
    /// blackout days do not count then.
    pub exact_dates: String,
    /// The day of the last run, `YYYY-MM-DD` (New York); `None` if it never ran. A schedule runs
    /// at most once per day.
    pub last_run: Option<String>,
    /// The condition that must be true before the trade is entered (`Condition::id`); 0 = none.
    pub condition_id: u32,
    /// The template entered instead if the condition is false (`id`); 0 = none, then the run is cancelled.
    pub condition_failure_template_id: i32,
    /// The set the schedule belongs to: 0 = default, 1 to 4 = alternate sets (see `GROUPS`).
    pub group: u8,
}

/// The id a new schedule gets: one above the highest in `list`.
#[must_use]
pub fn next_id(list: &[Schedule]) -> u32 {
    list.iter().map(|s| s.id).max().unwrap_or(0).saturating_add(1)
}

/// Stores `schedule`: a known id replaces that schedule, id 0 (or an unknown one) is added with a new id.
/// Returns the id it has now.
pub fn upsert(list: &mut Vec<Schedule>, mut schedule: Schedule) -> u32 {
    if let Some(slot) = list.iter_mut().find(|s| s.id == schedule.id && schedule.id != 0) {
        *slot = schedule;
        return slot.id;
    }
    schedule.id = next_id(list);
    let id = schedule.id;
    list.push(schedule);
    id
}

/// The dates in `text`: entries separated by commas or line breaks; what is no date is ignored.
#[must_use]
pub fn parse_dates(text: &str) -> Vec<NaiveDate> {
    text.split(['\r', '\n', ','])
        .map(str::trim)
        .filter_map(|entry| ["%Y-%m-%d", "%d-%m-%Y", "%d.%m.%Y"].iter().find_map(|format| NaiveDate::parse_from_str(entry, format).ok()))
        .collect()
}

/// Why a schedule does not run on a day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// The schedule is switched off.
    Inactive,
    /// The exchange is closed: its English and German holiday name.
    Holiday(&'static str, &'static str),
    /// The day is a blackout day, not one of the chosen days, or not one of the exact dates.
    NotThisDay,
    /// The time of day does not exist (a number out of range, or the hour that daylight saving time skips).
    InvalidTime,
}

/// What a schedule does on one day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// It runs: it may enter from `start` until `end` (UTC).
    Runs {
        /// The beginning of the window.
        start: DateTime<Utc>,
        /// The end of the window (start plus the expiration minutes).
        end: DateTime<Utc>,
    },
    /// It does not run, with the reason.
    Skipped(Skip),
}

impl Schedule {
    /// The time as `HH:MM:SS` (24 hours, New York) for the grid.
    #[must_use]
    pub fn time_label(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// Whether the schedule is meant for `day` by its days and dates (not by the calendar of the
    /// exchange): the exact dates if there are any, else not on a blackout day, else on the chosen
    /// days (Monday to Friday if none is chosen).
    #[must_use]
    pub fn runs_on(&self, day: NaiveDate) -> bool {
        if !self.exact_dates.trim().is_empty() {
            return parse_dates(&self.exact_dates).contains(&day);
        }
        if parse_dates(&self.blackout_dates).contains(&day) {
            return false;
        }
        if self.days.is_empty() {
            Days::WEEKDAYS.contains(day.weekday())
        } else {
            self.days.contains(day.weekday())
        }
    }

    /// What the schedule does on `day` (a New York calendar day): nothing if it is switched off, on a
    /// holiday of the exchange or not meant for the day, else its window as UTC instants (the time of
    /// day is New York time, so the window follows daylight saving time).
    #[must_use]
    pub fn plan_for(&self, day: NaiveDate) -> Plan {
        if !self.active {
            return Plan::Skipped(Skip::Inactive);
        }
        if let Some((english, german)) = holiday(day) {
            return Plan::Skipped(Skip::Holiday(english, german));
        }
        if !self.runs_on(day) {
            return Plan::Skipped(Skip::NotThisDay);
        }
        let window =
            self.start_on(day).and_then(|start| start.checked_add_signed(Duration::minutes(i64::from(self.expiration_minutes))).map(|end| (start, end)));
        window.map_or(Plan::Skipped(Skip::InvalidTime), |(start, end)| Plan::Runs { start, end })
    }

    /// The instant the schedule's time of day (New York time) falls on `day`, whether or not the schedule
    /// runs that day; `None` for a time that does not exist.
    #[must_use]
    pub fn start_on(&self, day: NaiveDate) -> Option<DateTime<Utc>> {
        NaiveTime::from_hms_opt(u32::from(self.hour), u32::from(self.minute), u32::from(self.second))
            .and_then(|time| chrono_tz::America::New_York.from_local_datetime(&day.and_time(time)).earliest())
            .map(|local| local.with_timezone(&Utc))
    }
}
/// The schedules of the Live or the Paper Trading mode (see `storage::mode_dir`).
pub fn schedules_path(live: bool) -> io::Result<PathBuf> {
    Ok(mode_dir(live)?.join("schedules.json"))
}

/// Every saved schedule of the mode. Not in tests: they must not touch the real data folder.
#[must_use]
pub fn load_schedules(live: bool, set_aside_suffix: &str) -> Loaded<Schedule> {
    if cfg!(test) {
        return Loaded::default();
    }
    match schedules_path(live) {
        Ok(path) => load_from(&path, set_aside_suffix, "schedules"),
        Err(error) => Loaded {
            problem: Some(format!("the schedules folder cannot be found ({error}) - no schedules loaded, saving is blocked")),
            save_blocked: true,
            ..Loaded::default()
        },
    }
}

/// How many sets of schedules there are: the default set (0) and four alternate sets (1 to 4). Only the
/// schedules of the active set run; the others wait to be switched to (e.g. a set for FOMC days).
pub const GROUPS: u8 = 5;

/// The name of a set as the screens show it.
#[must_use]
pub fn group_name(group: u8) -> String {
    if group == 0 {
        "Default".to_string()
    } else {
        format!("Alternate {group}")
    }
}

/// The file with the number of the active set (per mode, next to `schedules.json`).
pub fn active_group_path(live: bool) -> io::Result<PathBuf> {
    Ok(mode_dir(live)?.join("schedule_set.json"))
}

/// The active set of the mode; the default set if nothing was saved or the file is not usable. Not in
/// tests: they must not touch the real data folder.
#[must_use]
pub fn load_active_group(live: bool, set_aside_suffix: &str) -> u8 {
    if cfg!(test) {
        return 0;
    }
    let Ok(path) = active_group_path(live) else {
        return 0;
    };
    load_from::<u8>(&path, set_aside_suffix, "schedule set").items.first().copied().filter(|group| *group < GROUPS).unwrap_or(0)
}

/// Saves the active set of the mode (not in tests).
pub fn save_active_group(live: bool, group: u8) -> io::Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    write_to(&active_group_path(live)?, &[group])
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn upsert_replaces_a_known_id_and_gives_a_new_one_its_own() {
        let mut list = Vec::new();
        assert_eq!(next_id(&list), 1);
        let first = upsert(&mut list, Schedule { name: "A".into(), ..Schedule::default() });
        let second = upsert(&mut list, Schedule { name: "B".into(), ..Schedule::default() });
        assert_eq!((first, second), (1, 2));
        assert_eq!(upsert(&mut list, Schedule { id: 1, name: "A2".into(), ..Schedule::default() }), 1);
        assert_eq!(list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["A2", "B"]);
        // An id that is not in the list (deleted meanwhile) is added under a new one, never reused.
        assert_eq!(upsert(&mut list, Schedule { id: 9, ..Schedule::default() }), 3);
        assert_eq!(next_id(&list), 4);
    }

    fn weekly() -> Schedule {
        Schedule {
            id: 1,
            name: "Morning".into(),
            template_id: 7,
            account: "DU1".into(),
            days: Days::NONE.with(Weekday::Mon, true).with(Weekday::Wed, true),
            hour: 9,
            minute: 35,
            quantity: QuantityRule::Fixed { count: 1 },
            active: true,
            ..Schedule::default()
        }
    }

    /// A file name in a folder of its own, and the guard that removes the folder when the test is over.
    fn temp_file(name: &str) -> (crate::test_support::TempDir, PathBuf) {
        let guard = crate::test_support::TempDir::new(&format!("schedule-{name}"));
        fs::create_dir_all(&guard).unwrap();
        let path = guard.join("schedules.json");
        (guard, path)
    }

    #[test]
    fn days_know_their_weekdays_and_never_the_weekend() {
        let days = Days::NONE.with(Weekday::Tue, true).with(Weekday::Fri, true);
        assert!(days.contains(Weekday::Tue) && days.contains(Weekday::Fri));
        assert!(!days.contains(Weekday::Mon) && !days.contains(Weekday::Sat) && !days.contains(Weekday::Sun));
        // The weekend cannot be switched on.
        assert_eq!(days.with(Weekday::Sat, true), days);
        assert_eq!(days.with(Weekday::Tue, false).label(), "Fri");
        assert_eq!(Days::WEEKDAYS.label(), "Mon-Fri");
    }

    #[test]
    fn days_are_stored_as_names_and_unknown_names_are_dropped() {
        let json = serde_json::to_string(&weekly()).unwrap();
        assert!(json.contains(r#""days":["Mon","Wed"]"#), "{json}");
        let back: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, weekly());
        let odd: Days = serde_json::from_str(r#"["fri","Sat","Mon","nonsense"]"#).unwrap();
        assert_eq!(odd.label(), "Mon, Fri");
    }

    #[test]
    fn schedules_are_read_back_as_they_were_written() {
        let (_guard, path) = temp_file("roundtrip");
        fs::write(&path, serde_json::to_string_pretty(&[weekly()]).unwrap()).unwrap();
        let loaded: Loaded<Schedule> = load_from(&path, "x", "schedules");
        assert_eq!(loaded, Loaded { items: vec![weekly()], problem: None, save_blocked: false });
    }

    #[test]
    fn a_missing_file_means_no_schedules_and_no_problem() {
        let (_guard, path) = temp_file("missing");
        assert_eq!(load_from::<Schedule>(&path, "x", "schedules"), Loaded::default());
    }

    #[test]
    fn an_unreadable_schedules_file_is_set_aside_instead_of_being_overwritten() {
        let (_guard, path) = temp_file("broken");
        fs::write(&path, "this is not json").unwrap();
        let loaded = load_from::<Schedule>(&path, "20261116-100000", "schedules");
        assert!(loaded.items.is_empty() && !loaded.save_blocked);
        assert!(loaded.problem.as_deref().is_some_and(|p| p.contains("set aside")), "{loaded:?}");
        assert!(!path.exists(), "the broken file is moved away");
        let aside = path.with_file_name("schedules.json.unreadable-20261116-100000");
        assert_eq!(fs::read_to_string(aside).unwrap(), "this is not json", "its content is kept");
    }

    #[test]
    fn a_file_that_can_neither_be_read_nor_set_aside_blocks_saving() {
        // A directory where the file should be: reading fails with an error that is not "not found".
        let (_guard, path) = temp_file("directory");
        fs::create_dir(&path).unwrap();
        let loaded: Loaded<Schedule> = load_from(&path, "x", "schedules");
        assert!(loaded.save_blocked, "{loaded:?}");
        assert!(loaded.problem.is_some());
    }

    #[test]
    fn a_file_with_missing_fields_still_loads_with_defaults() {
        let (_guard, path) = temp_file("sparse");
        fs::write(&path, r#"[{"id":3,"template_id":5}]"#).unwrap();
        let loaded: Loaded<Schedule> = load_from(&path, "x", "schedules");
        assert_eq!(loaded.items.len(), 1);
        assert_eq!((loaded.items[0].id, loaded.items[0].template_id, loaded.items[0].quantity.clone()), (3, 5, QuantityRule::Fixed { count: 0 }));
        assert!(!loaded.items[0].active, "a schedule from an incomplete file is not active");
    }
    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn utc(text: &str) -> DateTime<Utc> {
        chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap().and_utc()
    }

    #[test]
    fn dates_are_read_from_lines_and_commas_and_comments_are_ignored() {
        let text = "FOMC 2026, 2026-01-28, 2026-03-18, 17.06.2026\n  2026-09-18 ,\r\nOther Dates, 2026-10-09\n2026-02-30, nonsense";
        let dates = parse_dates(text);
        assert_eq!(dates, [day("2026-01-28"), day("2026-03-18"), day("2026-06-17"), day("2026-09-18"), day("2026-10-09")]);
        assert!(parse_dates("").is_empty() && parse_dates("   \n ").is_empty());
    }

    #[test]
    fn a_schedule_runs_on_its_chosen_days_and_never_at_the_weekend() {
        let s = weekly(); // Monday and Wednesday
        assert!(s.runs_on(day("2026-11-16")), "a Monday");
        assert!(s.runs_on(day("2026-11-18")), "a Wednesday");
        assert!(!s.runs_on(day("2026-11-17")), "a Tuesday");
        assert!(!s.runs_on(day("2026-11-21")), "a Saturday");
    }

    #[test]
    fn a_schedule_without_days_runs_monday_to_friday() {
        let s = Schedule { days: Days::NONE, ..weekly() };
        assert!(s.runs_on(day("2026-11-17")) && s.runs_on(day("2026-11-20")));
        assert!(!s.runs_on(day("2026-11-21")) && !s.runs_on(day("2026-11-22")));
    }

    #[test]
    fn blackout_days_stop_a_schedule_but_exact_dates_override_everything() {
        let blackout = Schedule { blackout_dates: "FOMC, 2026-11-16".into(), ..weekly() };
        assert!(!blackout.runs_on(day("2026-11-16")), "a blackout Monday");
        assert!(blackout.runs_on(day("2026-11-23")), "another Monday");
        // Exact dates: only these days, whatever the days and the blackout say.
        let exact = Schedule { exact_dates: "2026-11-17\n2026-11-16".into(), blackout_dates: "2026-11-16".into(), ..weekly() };
        assert!(exact.runs_on(day("2026-11-17")), "a Tuesday that is not a chosen day");
        assert!(exact.runs_on(day("2026-11-16")), "the blackout does not count");
        assert!(!exact.runs_on(day("2026-11-18")), "a chosen day that is not an exact date");
        // Exact dates with no date in them: the schedule never runs.
        let none = Schedule { exact_dates: "only a comment".into(), ..weekly() };
        assert!(!none.runs_on(day("2026-11-16")));
    }

    #[test]
    fn the_window_is_the_new_york_time_as_utc_and_follows_daylight_saving_time() {
        let s = Schedule { hour: 9, minute: 35, second: 20, expiration_minutes: 5, ..weekly() };
        // Winter: New York is five hours behind UTC.
        assert_eq!(s.plan_for(day("2026-11-16")), Plan::Runs { start: utc("2026-11-16 14:35:20"), end: utc("2026-11-16 14:40:20") });
        // Summer: four hours.
        assert_eq!(s.plan_for(day("2026-06-15")), Plan::Runs { start: utc("2026-06-15 13:35:20"), end: utc("2026-06-15 13:40:20") });
        // Without expiration minutes the window is the one second.
        let at_once = Schedule { expiration_minutes: 0, ..s };
        assert_eq!(at_once.plan_for(day("2026-11-16")), Plan::Runs { start: utc("2026-11-16 14:35:20"), end: utc("2026-11-16 14:35:20") });
    }

    #[test]
    fn a_schedule_does_not_run_when_it_is_off_the_exchange_is_closed_or_the_day_is_not_its_own() {
        let s = Schedule { days: Days::WEEKDAYS, ..weekly() };
        assert_eq!(Schedule { active: false, ..s.clone() }.plan_for(day("2026-11-16")), Plan::Skipped(Skip::Inactive));
        assert_eq!(s.plan_for(day("2026-11-26")), Plan::Skipped(Skip::Holiday("Thanksgiving", "Thanksgiving")));
        assert_eq!(s.plan_for(day("2026-11-21")), Plan::Skipped(Skip::NotThisDay), "a Saturday");
        assert_eq!(weekly().plan_for(day("2026-11-17")), Plan::Skipped(Skip::NotThisDay), "a Tuesday, not chosen");
    }

    #[test]
    fn a_time_that_does_not_exist_is_reported_not_guessed() {
        let s = Schedule { days: Days::WEEKDAYS, ..weekly() };
        assert_eq!(Schedule { hour: 24, ..s.clone() }.plan_for(day("2026-11-16")), Plan::Skipped(Skip::InvalidTime));
        assert_eq!(Schedule { second: 60, ..s.clone() }.plan_for(day("2026-11-16")), Plan::Skipped(Skip::InvalidTime));
        // The hour that daylight saving time skips (02:30 on Sunday 2026-03-08) does not exist.
        assert_eq!(Schedule { hour: 2, minute: 30, exact_dates: "2026-03-08".into(), ..s }.plan_for(day("2026-03-08")), Plan::Skipped(Skip::InvalidTime));
    }

    #[test]
    fn the_time_label_shows_the_seconds() {
        assert_eq!(Schedule { second: 7, ..weekly() }.time_label(), "09:35:07");
    }
    #[test]
    fn the_quantity_rule_is_shown_as_the_grid_shows_it() {
        assert_eq!(QuantityRule::Fixed { count: 3 }.label(), "3");
        assert_eq!(QuantityRule::PremiumAllocation { budget: Decimal::new(50000, 2), max: 5, min: 1 }.label(), "$500");
        assert_eq!(QuantityRule::PctOfNetLiq { percent: Decimal::new(250, 2), max: 0, min: 0 }.label(), "2.5%");
        assert_eq!(QuantityRule::default(), QuantityRule::Fixed { count: 0 });
    }

    #[test]
    fn every_quantity_rule_is_stored_and_read_back_exactly() {
        for rule in [
            QuantityRule::Fixed { count: 2 },
            QuantityRule::PremiumAllocation { budget: Decimal::new(50025, 2), max: 5, min: 1 },
            QuantityRule::PctOfNetLiq { percent: Decimal::new(125, 1), max: 0, min: 2 },
        ] {
            let schedule = Schedule { quantity: rule.clone(), ..weekly() };
            let json = serde_json::to_string(&schedule).unwrap();
            let back: Schedule = serde_json::from_str(&json).unwrap();
            assert_eq!(back.quantity, rule, "{json}");
        }
    }
    #[test]
    fn the_premium_allocation_buys_what_the_budget_pays_for_rounded_down() {
        let rule = QuantityRule::PremiumAllocation { budget: Decimal::new(500, 0), max: 5, min: 0 };
        // The example of the description: 500 dollars, a debit spread of 1.35 (135 dollars) -> 3 contracts.
        assert_eq!(rule.quantity(Decimal::new(135, 2), None, None), Some(3));
        // A credit of 2.20 (220 dollars) -> 2.
        assert_eq!(rule.quantity(Decimal::new(220, 2), None, None), Some(2));
        // The sign of the price does not matter.
        assert_eq!(rule.quantity(Decimal::new(-135, 2), None, None), Some(3));
        // Too dear for one contract: nothing, unless a minimum is set.
        assert_eq!(rule.quantity(Decimal::new(600, 2), None, None), None);
        let with_min = QuantityRule::PremiumAllocation { budget: Decimal::new(500, 0), max: 5, min: 1 };
        assert_eq!(with_min.quantity(Decimal::new(600, 2), None, None), Some(1), "the minimum is entered even if the budget is too small");
        // The maximum limits it.
        assert_eq!(rule.quantity(Decimal::new(10, 2), None, None), Some(5));
        let no_max = QuantityRule::PremiumAllocation { budget: Decimal::new(500, 0), max: 0, min: 0 };
        assert_eq!(no_max.quantity(Decimal::new(10, 2), None, None), Some(50), "0 means no limit");
        // A price of 0 cannot be divided by.
        assert_eq!(rule.quantity(Decimal::ZERO, None, None), None);
    }

    #[test]
    fn the_net_liquidation_rule_needs_the_buying_power_and_the_balance() {
        // 2 percent of 100 000 = 2 000; one contract needs 450: 4 contracts.
        let rule = QuantityRule::PctOfNetLiq { percent: Decimal::new(2, 0), max: 0, min: 0 };
        let (bp, net) = (Some(Decimal::new(450, 0)), Some(Decimal::new(100_000, 0)));
        assert_eq!(rule.quantity(Decimal::ONE, bp, net), Some(4));
        assert_eq!(rule.quantity(Decimal::ONE, None, net), None, "buying power that cannot be worked out: no trade");
        assert_eq!(rule.quantity(Decimal::ONE, bp, None), None);
        assert_eq!(rule.quantity(Decimal::ONE, Some(Decimal::ZERO), net), None);
        let limited = QuantityRule::PctOfNetLiq { percent: Decimal::new(2, 0), max: 3, min: 0 };
        assert_eq!(limited.quantity(Decimal::ONE, bp, net), Some(3));
        let raised = QuantityRule::PctOfNetLiq { percent: Decimal::new(2, 0), max: 0, min: 2 };
        assert_eq!(raised.quantity(Decimal::ONE, Some(Decimal::new(5000, 0)), net), Some(2), "0.4 contracts rounds down to 0, the minimum raises it");
    }

    #[test]
    fn a_fixed_quantity_is_the_count_and_zero_is_nothing() {
        assert_eq!(QuantityRule::Fixed { count: 3 }.quantity(Decimal::ONE, None, None), Some(3));
        assert_eq!(QuantityRule::Fixed { count: 0 }.quantity(Decimal::ONE, None, None), None);
    }
}
