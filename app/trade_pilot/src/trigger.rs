//! Triggers: a template trade that is entered when a signal arrives (a webhook), inside a window of the day. This module
//! holds what a trigger *is*: the record, its file (`triggers.json` per mode, written like the schedules) and the decision
//! whether a signal may start an entry now. What the entry does is the same as for a schedule: the trigger starts a run
//! (see `schedule_run`) that goes through the same attempts, checks and orders.

use crate::list_file::{load_from, Loaded};
use crate::market_calendar::holiday;
use crate::schedule::Schedule;
use crate::storage::mode_dir;
use chrono::{NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

/// One trigger. What is entered (template, account, quantity, attempts, condition) and on which days is that of a
/// schedule, whose fields it carries: the time of the schedule is the start of the window of the day, and its minutes of
/// expiration are how long a run started by a signal may take to get its entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Trigger {
    /// The entry, the days and the checks.
    #[serde(flatten)]
    pub base: Schedule,
    /// The end of the window of the day, New York time: a signal after it is ignored.
    pub end_hour: u8,
    /// The minute of the end of the window.
    pub end_minute: u8,
    /// How many entries a day the trigger may start (0 = no limit).
    pub max_executions: u32,
    /// How many of its trades may be open at once on the account (0 = no limit).
    pub max_open: u32,
    /// The strategy the trades are booked under; empty = the one of the template.
    pub strategy: String,
    /// What starts an entry: empty or "Webhook" (a signal arrives), or one of `ENTRY_TRIGGERS` that watches the one-minute bars of the SPX
    /// itself for an EMA crossover.
    pub entry: String,
}

/// The entry triggers a trigger can have: a webhook signal, or the crossing of two EMAs of the one-minute SPX bars.
pub const ENTRY_TRIGGERS: [&str; 7] = [
    "Webhook",
    "EMA5 Cross Over EMA20",
    "EMA5 Cross Under EMA20",
    "EMA5 Cross Over EMA40",
    "EMA5 Cross Under EMA40",
    "EMA20 Cross Over EMA40",
    "EMA20 Cross Under EMA40",
];

/// An EMA crossover a trigger waits for: the EMA of `fast` bars passes the EMA of `slow` bars, upwards (`over`) or downwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmaCross {
    pub fast: usize,
    pub slow: usize,
    pub over: bool,
}

impl EmaCross {
    /// Whether the average moved across the other between the bar before (`before`: fast, slow) and the last closed bar (`now`): over means it
    /// was at or below and is above now, under the other way round.
    #[must_use]
    pub fn crossed(&self, before: (Decimal, Decimal), now: (Decimal, Decimal)) -> bool {
        if self.over {
            before.0 <= before.1 && now.0 > now.1
        } else {
            before.0 >= before.1 && now.0 < now.1
        }
    }
}

/// The id a new trigger gets: one above the highest in `list`.
#[must_use]
pub fn next_id(list: &[Trigger]) -> u32 {
    list.iter().map(|t| t.base.id).max().unwrap_or(0).saturating_add(1)
}

/// Stores `trigger`: a known id replaces that trigger, id 0 (or an unknown one) is added with a new id. Returns the id it has now.
pub fn upsert(list: &mut Vec<Trigger>, mut trigger: Trigger) -> u32 {
    if let Some(slot) = list.iter_mut().find(|t| t.base.id == trigger.base.id && trigger.base.id != 0) {
        *slot = trigger;
        return slot.base.id;
    }
    trigger.base.id = next_id(list);
    let id = trigger.base.id;
    list.push(trigger);
    id
}

/// The triggers of the Live or the Paper Trading mode (see `storage::mode_dir`).
///
/// # Errors
///
/// The folder of the mode cannot be found.
pub fn triggers_path(live: bool) -> io::Result<PathBuf> {
    Ok(mode_dir(live)?.join("triggers.json"))
}

/// Every saved trigger of the mode. Not in tests: they must not touch the real data folder.
#[must_use]
pub fn load_triggers(live: bool, set_aside_suffix: &str) -> Loaded<Trigger> {
    if cfg!(test) {
        return Loaded::default();
    }
    match triggers_path(live) {
        Ok(path) => load_from(&path, set_aside_suffix, "triggers"),
        Err(error) => Loaded {
            problem: Some(format!("the triggers folder cannot be found ({error}) - no triggers loaded, saving is blocked")),
            save_blocked: true,
            ..Loaded::default()
        },
    }
}

/// Why a signal does not start an entry of a trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The trigger is switched off.
    Inactive,
    /// The exchange is closed today, or the trigger is not meant for this day.
    NotToday,
    /// The time is outside the window of the day.
    OutsideWindow,
    /// The trigger has started as many entries today as it may.
    MaxExecutions(u32),
    /// As many of its trades are open as it may have.
    MaxOpen(u32),
}

impl Refusal {
    /// The reason as the log says it.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Inactive => "the trigger is switched off".to_string(),
            Self::NotToday => "not a day of the trigger (or the exchange is closed)".to_string(),
            Self::OutsideWindow => "not within the time window of the trigger".to_string(),
            Self::MaxExecutions(max) => format!("the maximum of {max} entries today is reached"),
            Self::MaxOpen(max) => format!("{max} trades of the trigger are open already"),
        }
    }
}

impl Trigger {
    /// Whether a signal at `now` (New York time) may start an entry: `started_today` entries were started today and `open`
    /// trades of the trigger are open on its account.
    ///
    /// # Errors
    ///
    /// The reason it may not.
    pub fn check(&self, now: NaiveDateTime, started_today: usize, open: usize) -> Result<(), Refusal> {
        if !self.base.active {
            return Err(Refusal::Inactive);
        }
        if holiday(now.date()).is_some() || !self.base.runs_on(now.date()) {
            return Err(Refusal::NotToday);
        }
        let time_of = |hour: u8, minute: u8| NaiveTime::from_hms_opt(u32::from(hour), u32::from(minute), 0);
        let (start, end) = (time_of(self.base.hour, self.base.minute), time_of(self.end_hour, self.end_minute));
        if !start.zip(end).is_some_and(|(start, end)| now.time() >= start && now.time() < end) {
            return Err(Refusal::OutsideWindow);
        }
        if self.max_executions > 0 && started_today >= usize::try_from(self.max_executions).unwrap_or(usize::MAX) {
            return Err(Refusal::MaxExecutions(self.max_executions));
        }
        if self.max_open > 0 && open >= usize::try_from(self.max_open).unwrap_or(usize::MAX) {
            return Err(Refusal::MaxOpen(self.max_open));
        }
        Ok(())
    }

    /// The name of the entry trigger as the screens show it.
    #[must_use]
    pub fn entry_label(&self) -> &str {
        if self.entry.is_empty() {
            ENTRY_TRIGGERS[0]
        } else {
            &self.entry
        }
    }

    /// The EMA crossover this trigger waits for; `None` for a webhook trigger.
    #[must_use]
    pub fn ema_cross(&self) -> Option<EmaCross> {
        let words: Vec<&str> = self.entry.split_whitespace().collect();
        let length = |name: &str| name.strip_prefix("EMA").and_then(|digits| digits.parse::<usize>().ok());
        match words.as_slice() {
            [fast, "Cross", direction @ ("Over" | "Under"), slow] => Some(EmaCross { fast: length(fast)?, slow: length(slow)?, over: *direction == "Over" }),
            _ => None,
        }
    }

    /// The window of the day as text for the grid, e.g. `09:35 - 15:30`.
    #[must_use]
    pub fn window_label(&self) -> (String, String) {
        (format!("{:02}:{:02}", self.base.hour, self.base.minute), format!("{:02}:{:02}", self.end_hour, self.end_minute))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::Days;
    use chrono::NaiveDate;

    fn trigger() -> Trigger {
        Trigger {
            base: Schedule { id: 3, active: true, hour: 9, minute: 35, days: Days::WEEKDAYS, expiration_minutes: 5, ..Schedule::default() },
            end_hour: 15,
            end_minute: 30,
            ..Trigger::default()
        }
    }

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    #[test]
    fn a_signal_is_accepted_inside_the_window_of_a_day_of_the_trigger() {
        let t = trigger();
        assert_eq!(t.check(at(16, 10, 0), 0, 0), Ok(()), "Monday 10:00");
        assert_eq!(t.check(at(16, 9, 35), 0, 0), Ok(()), "the start counts");
        assert_eq!(t.check(at(16, 9, 34), 0, 0), Err(Refusal::OutsideWindow));
        assert_eq!(t.check(at(16, 15, 30), 0, 0), Err(Refusal::OutsideWindow), "the end does not");
        assert_eq!(t.check(at(21, 10, 0), 0, 0), Err(Refusal::NotToday), "Saturday");
        assert_eq!(t.check(at(26, 10, 0), 0, 0), Err(Refusal::NotToday), "Thanksgiving");
        let off = Trigger { base: Schedule { active: false, ..trigger().base }, ..trigger() };
        assert_eq!(off.check(at(16, 10, 0), 0, 0), Err(Refusal::Inactive));
    }

    #[test]
    fn the_limits_of_entries_and_open_trades_hold_back_a_signal_and_zero_means_no_limit() {
        let limited = Trigger { max_executions: 2, max_open: 1, ..trigger() };
        assert_eq!(limited.check(at(16, 10, 0), 1, 0), Ok(()));
        assert_eq!(limited.check(at(16, 10, 0), 2, 0), Err(Refusal::MaxExecutions(2)));
        assert_eq!(limited.check(at(16, 10, 0), 0, 1), Err(Refusal::MaxOpen(1)));
        assert_eq!(trigger().check(at(16, 10, 0), 99, 99), Ok(()));
        assert!(Refusal::MaxOpen(1).text().contains("open already"));
    }

    #[test]
    fn an_entry_trigger_names_its_ema_crossover_and_a_webhook_trigger_has_none() {
        assert_eq!(trigger().entry_label(), "Webhook");
        assert_eq!(trigger().ema_cross(), None);
        let over = Trigger { entry: "EMA5 Cross Over EMA20".into(), ..trigger() };
        assert_eq!(over.ema_cross(), Some(EmaCross { fast: 5, slow: 20, over: true }));
        let under = Trigger { entry: "EMA20 Cross Under EMA40".into(), ..trigger() };
        assert_eq!(under.ema_cross(), Some(EmaCross { fast: 20, slow: 40, over: false }));
        assert_eq!(under.entry_label(), "EMA20 Cross Under EMA40");
        for entry in ENTRY_TRIGGERS.iter().skip(1) {
            assert!(Trigger { entry: (*entry).into(), ..trigger() }.ema_cross().is_some(), "{entry}");
        }
        assert_eq!(Trigger { entry: "EMA5 Sideways EMA20".into(), ..trigger() }.ema_cross(), None);
    }

    #[test]
    fn an_average_crosses_over_when_it_was_at_or_below_the_other_and_is_above_it_now() {
        let d = |text: &str| text.parse::<Decimal>().unwrap();
        let over = EmaCross { fast: 5, slow: 20, over: true };
        assert!(over.crossed((d("99"), d("100")), (d("101"), d("100"))));
        assert!(over.crossed((d("100"), d("100")), (d("100.5"), d("100"))), "equal before counts as below");
        assert!(!over.crossed((d("101"), d("100")), (d("102"), d("100"))), "above before: nothing crossed");
        assert!(!over.crossed((d("99"), d("100")), (d("99.5"), d("100"))), "still below");
        let under = EmaCross { over: false, ..over };
        assert!(under.crossed((d("101"), d("100")), (d("99"), d("100"))));
        assert!(!under.crossed((d("99"), d("100")), (d("98"), d("100"))));
    }
    #[test]
    fn a_trigger_is_stored_flat_with_the_fields_of_its_schedule_and_ids_do_not_repeat() {
        let json = serde_json::to_string(&trigger()).unwrap();
        assert!(json.contains("\"id\":3") && json.contains("\"end_hour\":15") && !json.contains("\"base\""), "{json}");
        assert_eq!(serde_json::from_str::<Trigger>(&json).unwrap(), trigger());
        let mut list = Vec::new();
        assert_eq!(upsert(&mut list, trigger()), 1, "an id that is not in the list is replaced by the next one");
        let edited = Trigger { max_open: 2, ..list[0].clone() };
        assert_eq!(upsert(&mut list, edited), 1, "a known id replaces");
        assert_eq!((list.len(), list[0].max_open), (1, 2));
        assert_eq!(upsert(&mut list, Trigger::default()), 2);
    }
}
