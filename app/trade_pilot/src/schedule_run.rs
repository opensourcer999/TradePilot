//! The daily runs of the schedules: one run per
//! schedule and New York day, with its window as UTC instants and its status. The runs are kept in
//! `schedule_runs.json` so a restart does not create a second run for the day or enter again.
//! Everything here is plain logic on values; the clock is passed in.

use crate::list_file::{load_from, Loaded};
use crate::schedule::{Plan, Schedule};
use crate::storage::mode_dir;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

/// Where a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    /// Waiting for its window, or for the next attempt inside it.
    Waiting,
    /// Held by the user: no attempt is made until it is resumed; the window still runs out.
    Paused,
    /// An entry is being made.
    Working,
    /// The trade was entered and filled.
    Complete,
    /// The trade was entered but only partly filled.
    Partial,
    /// The window ended without an entry.
    Expired,
    /// Stopped: switched off, a limit of attempts, an error or cancelled by the user.
    Cancelled,
}

impl RunStatus {
    /// Whether the run has not begun and not ended: it is waiting or paused.
    #[must_use]
    pub fn is_open(self) -> bool {
        matches!(self, Self::Waiting | Self::Paused)
    }
}

/// One run of a schedule on one day.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleRun {
    /// The id (1, 2, ...), unique in the file.
    pub id: u32,
    /// The schedule it belongs to (`Schedule::id`).
    pub schedule_id: u32,
    /// The New York day of the run.
    pub day: NaiveDate,
    /// The beginning of the window.
    pub window_start: DateTime<Utc>,
    /// The end of the window.
    pub window_end: DateTime<Utc>,
    /// Where the run stands.
    pub status: RunStatus,
    /// How many times an entry was tried.
    pub attempts: u32,
    /// When the last attempt began.
    pub started: Option<DateTime<Utc>>,
    /// When the run reached its end (Complete, Expired, Cancelled).
    pub finished: Option<DateTime<Utc>>,
    /// What happened, for the task log.
    pub notes: String,
    /// Set for the re-entry of a trade that ended (see `ReEntry`); such a run belongs to no schedule of the list.
    #[serde(default)]
    pub reentry: Option<ReEntry>,
    /// Set for the entry a trigger started; such a run belongs to no schedule of the list either.
    #[serde(default)]
    pub trigger: Option<TriggerRun>,
}

/// How long after the start of its window a run can still begin, however short the window is: a window of 0 minutes is a
/// single moment, which the app, checking a few times a second, would otherwise always miss.
const START_GRACE_SECONDS: i64 = 5;

impl ScheduleRun {
    /// The last moment the run can begin: the end of its window, but no earlier than `START_GRACE_SECONDS` after its start.
    #[must_use]
    pub fn last_moment(&self) -> DateTime<Utc> {
        self.window_end.max(self.window_start + Duration::seconds(START_GRACE_SECONDS))
    }
}

/// The entry of a trigger: what it enters (the schedule part of the trigger) and under which strategy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerRun {
    /// The trigger (`Trigger::base.id`).
    pub trigger_id: u32,
    /// The entry as a schedule would give it.
    pub schedule: Schedule,
    /// The strategy the trade is booked under; empty = that of the template.
    pub strategy: String,
}

/// A re-entry: what the run enters, and where it stands in the chain of re-entries of the first trade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReEntry {
    /// The entry as a schedule would give it: the template, the account and the number of contracts; its window length.
    pub schedule: Schedule,
    /// The order id of the first entry of the chain.
    pub root: i32,
    /// How many re-entries this one makes in the chain (1 for the first).
    pub entries: u32,
    /// The order id of the trade whose end caused it.
    pub parent_order: i32,
}

/// How many days the runs are kept (the grid and the task log show the last week).
const KEEP_DAYS: i64 = 7;
/// A run is tried again no sooner than this many seconds after its last attempt began.
const RETRY_AFTER_SECONDS: i64 = 5;

/// The runs of the Live or the Paper Trading mode (see `storage::mode_dir`).
pub fn runs_path(live: bool) -> io::Result<PathBuf> {
    Ok(mode_dir(live)?.join("schedule_runs.json"))
}

/// Every saved run of the mode. Not in tests: they must not touch the real data folder.
#[must_use]
pub fn load_runs(live: bool, set_aside_suffix: &str) -> Loaded<ScheduleRun> {
    if cfg!(test) {
        return Loaded::default();
    }
    match runs_path(live) {
        Ok(path) => load_from(&path, set_aside_suffix, "schedule runs"),
        Err(error) => Loaded {
            problem: Some(format!("the schedule runs folder cannot be found ({error}) - no runs loaded, saving is blocked")),
            save_blocked: true,
            ..Loaded::default()
        },
    }
}

/// The run of `schedule_id` on `day`, if there is one: the one still to come, else the last one made.
#[must_use]
pub fn run_of(runs: &[ScheduleRun], schedule_id: u32, day: NaiveDate) -> Option<&ScheduleRun> {
    let mut of_day = runs.iter().filter(|run| run.schedule_id == schedule_id && run.day == day && run.reentry.is_none() && run.trigger.is_none());
    of_day.clone().find(|run| run.status.is_open()).or_else(|| of_day.next_back())
}

/// The next free id of `runs`.
pub fn next_id(runs: &[ScheduleRun]) -> u32 {
    runs.iter().map(|run| run.id).max().unwrap_or(0).saturating_add(1)
}

/// Brings the runs of `today` (a New York day) in line with the schedules; `true` if anything changed.
/// - A schedule that runs today and has no run yet gets a waiting one, but only while its window is
///   not over (`now` is when it is made).
/// - A waiting run takes over the window of its schedule if that was edited.
/// - A waiting run of a schedule that does not run today any more (switched off, edited, deleted)
///   is removed; a run that has begun or ended is never touched.
/// - Runs of days before the last week are dropped.
pub fn setup_runs(runs: &mut Vec<ScheduleRun>, schedules: &[Schedule], today: NaiveDate, now: DateTime<Utc>) -> bool {
    let before = runs.len();
    let oldest = today - Duration::days(KEEP_DAYS);
    runs.retain(|run| run.day >= oldest);
    let mut changed = runs.len() != before;
    // A waiting run of today whose schedule is gone.
    let len = runs.len();
    runs.retain(|run| {
        run.day != today || !run.status.is_open() || run.reentry.is_some() || run.trigger.is_some() || schedules.iter().any(|s| s.id == run.schedule_id)
    });
    changed |= runs.len() != len;
    for schedule in schedules {
        let plan = schedule.plan_for(today);
        let of_today = |run: &ScheduleRun| run.schedule_id == schedule.id && run.day == today && run.reentry.is_none() && run.trigger.is_none();
        // The run still to come is the one an edit moves; without one, the runs that have ended decide.
        let existing = runs.iter().position(|run| of_today(run) && run.status.is_open());
        match (plan, existing) {
            (Plan::Runs { start, end }, None) => {
                // A run that ended without a trade does not hold back a run at another time of the day (the
                // schedule was moved); one that entered a trade, or ended at this very time, does.
                let blocked = runs
                    .iter()
                    .filter(|run| of_today(run))
                    .any(|run| !matches!(run.status, RunStatus::Expired | RunStatus::Cancelled) || run.window_start == start);
                if end > now && !blocked {
                    let id = next_id(runs);
                    runs.push(ScheduleRun {
                        id,
                        schedule_id: schedule.id,
                        day: today,
                        window_start: start,
                        window_end: end,
                        status: RunStatus::Waiting,
                        attempts: 0,
                        started: None,
                        finished: None,
                        notes: String::new(),
                        reentry: None,
                        trigger: None,
                    });
                    changed = true;
                }
            }
            (Plan::Runs { start, end }, Some(index)) => {
                if let Some(run) = runs.get_mut(index).filter(|run| run.status.is_open() && (run.window_start != start || run.window_end != end)) {
                    run.window_start = start;
                    run.window_end = end;
                    changed = true;
                }
            }
            (Plan::Skipped(_), Some(index)) => {
                if runs.get(index).is_some_and(|run| run.status.is_open()) {
                    runs.remove(index);
                    changed = true;
                }
            }
            (Plan::Skipped(_), None) => {}
        }
    }
    changed
}

/// The run to try next, if any: a waiting run whose window is open and that was not tried in the last
/// 5 seconds; the one tried longest ago first, then the one with the fewest attempts. The index in `runs`.
#[must_use]
pub fn next_run(runs: &[ScheduleRun], now: DateTime<Utc>) -> Option<usize> {
    runs.iter()
        .enumerate()
        .filter(|(_, run)| run.status == RunStatus::Waiting && run.window_start <= now && now <= run.last_moment())
        .filter(|(_, run)| run.started.is_none_or(|started| started + Duration::seconds(RETRY_AFTER_SECONDS) < now))
        .min_by_key(|(_, run)| (run.started, run.attempts))
        .map(|(index, _)| index)
}

/// Which template a run enters with at its next attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateChoice {
    /// The template of the schedule.
    Main,
    /// The alternate template (the schedule's number of tries is reached).
    Alternate,
    /// No more tries: the run is cancelled.
    GiveUp,
}

/// The template for the next attempt of a run that has made `attempts` attempts: with a limit of tries, the
/// schedule's template is used until it is reached, then the alternate template for as many tries again; without an
/// alternate template, or when both are used up, the run is given up.
#[must_use]
pub fn choose_template(schedule: &Schedule, attempts: u32) -> TemplateChoice {
    let max = schedule.max_attempts;
    if max == 0 {
        return TemplateChoice::Main;
    }
    if (attempts >= max && schedule.alternate_template_id == 0) || attempts >= max.saturating_mul(2) {
        TemplateChoice::GiveUp
    } else if attempts >= max {
        TemplateChoice::Alternate
    } else {
        TemplateChoice::Main
    }
}

/// Whether the schedule may run today as far as its link goes: it has none, or the schedule it is linked
/// to has entered its trade (`Complete`) on `day`.
#[must_use]
pub fn linked_done(runs: &[ScheduleRun], schedule: &Schedule, day: NaiveDate) -> bool {
    schedule.linked_schedule_id == 0 || run_of(runs, schedule.linked_schedule_id, day).is_some_and(|run| run.status == RunStatus::Complete)
}
/// Ends every waiting or paused run whose window is over (`ScheduleRun::last_moment`): it becomes `Expired`. The ids of the runs that
/// expired now, so the caller can log and report them.
pub fn expire_runs(runs: &mut [ScheduleRun], now: DateTime<Utc>) -> Vec<u32> {
    let mut expired = Vec::new();
    for run in runs.iter_mut().filter(|run| run.status.is_open() && run.last_moment() < now) {
        run.status = RunStatus::Expired;
        run.finished = Some(now);
        expired.push(run.id);
    }
    expired
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{Days, QuantityRule};

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn utc(text: &str) -> DateTime<Utc> {
        chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap().and_utc()
    }

    /// 09:35:00 New York, five minutes of window, Monday to Friday (14:35 to 14:40 UTC in November).
    fn schedule(id: u32) -> Schedule {
        Schedule {
            id,
            template_id: 7,
            account: "DU1".into(),
            hour: 9,
            minute: 35,
            expiration_minutes: 5,
            quantity: QuantityRule::Fixed { count: 1 },
            active: true,
            days: Days::WEEKDAYS,
            ..Schedule::default()
        }
    }

    const MONDAY: &str = "2026-11-16";

    #[test]
    fn a_run_is_made_for_a_schedule_that_runs_today() {
        let mut runs = Vec::new();
        assert!(setup_runs(&mut runs, &[schedule(1), schedule(2)], day(MONDAY), utc("2026-11-16 13:00:00")));
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].id, runs[0].schedule_id, runs[0].status), (1, 1, RunStatus::Waiting));
        assert_eq!((runs[0].window_start, runs[0].window_end), (utc("2026-11-16 14:35:00"), utc("2026-11-16 14:40:00")));
        assert_eq!((runs[1].id, runs[1].schedule_id), (2, 2));
        // The second call changes nothing.
        assert!(!setup_runs(&mut runs, &[schedule(1), schedule(2)], day(MONDAY), utc("2026-11-16 13:00:01")));
    }

    #[test]
    fn no_run_is_made_once_the_window_is_over_but_one_is_while_it_is_open() {
        let mut late = Vec::new();
        assert!(!setup_runs(&mut late, &[schedule(1)], day(MONDAY), utc("2026-11-16 14:40:01")));
        assert!(late.is_empty(), "the window ended at 14:40:00");
        let mut inside = Vec::new();
        assert!(setup_runs(&mut inside, &[schedule(1)], day(MONDAY), utc("2026-11-16 14:38:00")), "the app starts inside the window");
        assert_eq!(inside[0].status, RunStatus::Waiting);
    }

    #[test]
    fn nothing_is_made_for_a_schedule_that_does_not_run_today() {
        let mut runs = Vec::new();
        let off = Schedule { active: false, ..schedule(1) };
        let holiday_day = day("2026-11-26");
        assert!(!setup_runs(&mut runs, &[off], day(MONDAY), utc("2026-11-16 13:00:00")));
        assert!(!setup_runs(&mut runs, &[schedule(1)], holiday_day, utc("2026-11-26 13:00:00")));
        assert_eq!(runs, Vec::<ScheduleRun>::new());
    }

    #[test]
    fn a_waiting_run_follows_an_edited_schedule_and_is_removed_when_the_schedule_no_longer_runs_today() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1)], day(MONDAY), utc("2026-11-16 13:00:00"));
        // The time is edited to 10:00.
        let edited = Schedule { hour: 10, minute: 0, ..schedule(1) };
        assert!(setup_runs(&mut runs, std::slice::from_ref(&edited), day(MONDAY), utc("2026-11-16 13:01:00")));
        assert_eq!(runs[0].window_start, utc("2026-11-16 15:00:00"));
        // Switched off: the waiting run goes.
        assert!(setup_runs(&mut runs, &[Schedule { active: false, ..edited }], day(MONDAY), utc("2026-11-16 13:02:00")));
        assert_eq!(runs, Vec::<ScheduleRun>::new());
        // Deleted schedule: the same.
        setup_runs(&mut runs, &[schedule(1)], day(MONDAY), utc("2026-11-16 13:03:00"));
        assert_eq!(runs.len(), 1);
        assert!(setup_runs(&mut runs, &[], day(MONDAY), utc("2026-11-16 13:04:00")));
        assert_eq!(runs, Vec::<ScheduleRun>::new());
    }

    #[test]
    fn a_run_that_has_begun_or_ended_is_never_touched_nor_made_again() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1)], day(MONDAY), utc("2026-11-16 13:00:00"));
        runs[0].status = RunStatus::Complete;
        let edited = Schedule { hour: 11, ..schedule(1) };
        assert!(!setup_runs(&mut runs, &[edited], day(MONDAY), utc("2026-11-16 15:00:00")), "a complete run keeps its window");
        assert_eq!((runs.len(), runs[0].window_start), (1, utc("2026-11-16 14:35:00")));
        // Switched off after it ran: the finished run stays in the record.
        assert!(!setup_runs(&mut runs, &[Schedule { active: false, ..schedule(1) }], day(MONDAY), utc("2026-11-16 15:00:00")));
        assert_eq!(runs.len(), 1);
        // A working run is kept too.
        runs[0].status = RunStatus::Working;
        assert!(!setup_runs(&mut runs, &[], day(MONDAY), utc("2026-11-16 15:00:00")) || runs.len() == 1);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn a_schedule_moved_after_its_run_ended_without_a_trade_gets_a_new_run_at_the_new_time() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1)], day(MONDAY), utc("2026-11-16 13:00:00"));
        expire_runs(&mut runs, utc("2026-11-16 14:41:00"));
        // Unchanged: no second run, also not after a restart.
        assert!(!setup_runs(&mut runs, &[schedule(1)], day(MONDAY), utc("2026-11-16 14:42:00")));
        // Moved to 10:00: the expired run stays in the record, a waiting one comes for 10:00.
        let moved = Schedule { hour: 10, minute: 0, ..schedule(1) };
        assert!(setup_runs(&mut runs, std::slice::from_ref(&moved), day(MONDAY), utc("2026-11-16 14:42:00")));
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].status, runs[1].status, runs[1].window_start), (RunStatus::Expired, RunStatus::Waiting, utc("2026-11-16 15:00:00")));
        assert_eq!(run_of(&runs, 1, day(MONDAY)).map(|run| run.id), Some(runs[1].id), "the run still to come is the schedule's run");
        // Moved again: the waiting run follows, no third one.
        let again = Schedule { hour: 10, minute: 30, ..schedule(1) };
        assert!(setup_runs(&mut runs, &[again], day(MONDAY), utc("2026-11-16 14:43:00")));
        assert_eq!((runs.len(), runs[1].window_start), (2, utc("2026-11-16 15:30:00")));
        // A cancelled run makes way for a moved schedule too; a complete one does not (no second trade).
        runs.truncate(1);
        runs[0].status = RunStatus::Cancelled;
        assert!(setup_runs(&mut runs, std::slice::from_ref(&moved), day(MONDAY), utc("2026-11-16 14:44:00")));
        runs.truncate(1);
        runs[0].status = RunStatus::Complete;
        assert!(!setup_runs(&mut runs, &[moved], day(MONDAY), utc("2026-11-16 14:44:00")));
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn runs_older_than_a_week_are_dropped() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1)], day("2026-11-09"), utc("2026-11-09 13:00:00"));
        assert_eq!(runs.len(), 1);
        assert!(setup_runs(&mut runs, &[], day("2026-11-17"), utc("2026-11-17 13:00:00")), "eight days later");
        assert_eq!(runs, Vec::<ScheduleRun>::new());
        let mut recent = Vec::new();
        setup_runs(&mut recent, &[schedule(1)], day("2026-11-10"), utc("2026-11-10 13:00:00"));
        assert!(!setup_runs(&mut recent, &[], day("2026-11-17"), utc("2026-11-17 13:00:00")) || recent.len() == 1);
        assert_eq!(recent.len(), 1, "seven days later it is still there");
    }

    #[test]
    fn a_window_of_zero_minutes_still_starts_in_the_moments_after_its_time_and_only_then_expires() {
        let mut runs = Vec::new();
        let at_once = Schedule { expiration_minutes: 0, ..schedule(1) };
        setup_runs(&mut runs, &[at_once], day(MONDAY), utc("2026-11-16 13:00:00"));
        // The app looks a few times a second: its first look comes a fraction after the start.
        let first_look = utc("2026-11-16 14:35:00") + Duration::milliseconds(318);
        assert!(expire_runs(&mut runs, first_look).is_empty(), "not over before it could start");
        assert_eq!(next_run(&runs, first_look), Some(0));
        assert_eq!(expire_runs(&mut runs, utc("2026-11-16 14:35:05")), Vec::<u32>::new(), "still inside the five seconds");
        assert_eq!(expire_runs(&mut runs, utc("2026-11-16 14:35:06")), [1], "a few seconds later it has expired");
    }

    #[test]
    fn only_a_waiting_run_whose_window_is_over_expires() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1), schedule(2), schedule(3)], day(MONDAY), utc("2026-11-16 13:00:00"));
        runs[1].status = RunStatus::Complete;
        runs[2].status = RunStatus::Working;
        assert!(expire_runs(&mut runs, utc("2026-11-16 14:40:00")).is_empty(), "at the end second it is still inside");
        let expired = expire_runs(&mut runs, utc("2026-11-16 14:40:01"));
        assert_eq!(expired, [1]);
        assert_eq!((runs[0].status, runs[0].finished), (RunStatus::Expired, Some(utc("2026-11-16 14:40:01"))));
        assert_eq!((runs[1].status, runs[2].status), (RunStatus::Complete, RunStatus::Working));
        assert!(expire_runs(&mut runs, utc("2026-11-16 15:00:00")).is_empty(), "once");
    }

    #[test]
    fn a_paused_run_is_never_tried_but_its_window_still_runs_out_and_a_new_window_reaches_it() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1)], day(MONDAY), utc("2026-11-16 13:00:00"));
        runs[0].status = RunStatus::Paused;
        assert_eq!(next_run(&runs, utc("2026-11-16 14:36:00")), None, "inside the window, but paused");
        runs[0].status = RunStatus::Waiting;
        assert_eq!(next_run(&runs, utc("2026-11-16 14:36:00")), Some(0));
        runs[0].status = RunStatus::Paused;
        // An edited window is taken over, a schedule that does not run today any more removes the run.
        let later = Schedule { hour: 10, ..schedule(1) };
        assert!(setup_runs(&mut runs, &[later], day(MONDAY), utc("2026-11-16 13:00:00")));
        assert_eq!((runs[0].status, runs[0].window_start), (RunStatus::Paused, utc("2026-11-16 15:35:00")));
        assert!(setup_runs(&mut runs, &[], day(MONDAY), utc("2026-11-16 13:00:00")));
        assert_eq!(runs, Vec::<ScheduleRun>::new());
        // The window ends while paused: the run expires.
        setup_runs(&mut runs, &[schedule(1)], day(MONDAY), utc("2026-11-16 13:00:00"));
        runs[0].status = RunStatus::Paused;
        assert_eq!(expire_runs(&mut runs, utc("2026-11-16 14:41:00")), [1]);
        assert_eq!(runs[0].status, RunStatus::Expired);
        assert!(RunStatus::Waiting.is_open() && RunStatus::Paused.is_open() && !RunStatus::Working.is_open() && !RunStatus::Expired.is_open());
    }

    #[test]
    fn a_run_is_found_by_its_schedule_and_day() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(4)], day(MONDAY), utc("2026-11-16 13:00:00"));
        assert!(run_of(&runs, 4, day(MONDAY)).is_some());
        assert!(run_of(&runs, 4, day("2026-11-17")).is_none() && run_of(&runs, 5, day(MONDAY)).is_none());
    }

    #[test]
    fn the_runs_are_stored_and_read_back_exactly() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1)], day(MONDAY), utc("2026-11-16 13:00:00"));
        let json = serde_json::to_string(&runs).unwrap();
        let back: Vec<ScheduleRun> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, runs, "{json}");
    }
    #[test]
    fn the_next_run_is_a_waiting_one_inside_its_window_not_tried_for_five_seconds() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1), schedule(2), schedule(3)], day(MONDAY), utc("2026-11-16 13:00:00"));
        assert_eq!(next_run(&runs, utc("2026-11-16 14:34:59")), None, "the window has not begun");
        assert_eq!(next_run(&runs, utc("2026-11-16 14:35:00")), Some(0), "at its first second");
        assert_eq!(next_run(&runs, utc("2026-11-16 14:40:00")), Some(0), "at its last second");
        assert_eq!(next_run(&runs, utc("2026-11-16 14:40:01")), None, "the window is over");
        // Run 1 was tried: the others come first, and it again 5 seconds later.
        runs[0].started = Some(utc("2026-11-16 14:35:10"));
        assert_eq!(next_run(&runs, utc("2026-11-16 14:35:11")), Some(1));
        runs[1].started = Some(utc("2026-11-16 14:35:11"));
        runs[2].started = Some(utc("2026-11-16 14:35:12"));
        assert_eq!(next_run(&runs, utc("2026-11-16 14:35:14")), None, "all were tried a moment ago");
        assert_eq!(next_run(&runs, utc("2026-11-16 14:35:16")), Some(0), "5 seconds after the last attempt, the oldest first");
        // Only a waiting run counts.
        runs[0].status = RunStatus::Working;
        assert_eq!(next_run(&runs, utc("2026-11-16 14:35:20")), Some(1));
    }

    #[test]
    fn the_run_with_fewer_attempts_goes_first_among_runs_tried_at_the_same_time() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1), schedule(2)], day(MONDAY), utc("2026-11-16 13:00:00"));
        runs[0].started = Some(utc("2026-11-16 14:35:00"));
        runs[1].started = Some(utc("2026-11-16 14:35:00"));
        runs[0].attempts = 4;
        runs[1].attempts = 1;
        assert_eq!(next_run(&runs, utc("2026-11-16 14:36:00")), Some(1));
    }

    #[test]
    fn the_alternate_template_takes_over_at_the_limit_of_tries_and_the_run_is_given_up_at_twice_the_limit() {
        let plain = schedule(1);
        assert_eq!(choose_template(&plain, 0), TemplateChoice::Main);
        assert_eq!(choose_template(&plain, 500), TemplateChoice::Main, "no limit of tries");
        let limited = Schedule { max_attempts: 3, ..schedule(1) };
        assert_eq!([0, 1, 2].map(|n| choose_template(&limited, n)), [TemplateChoice::Main; 3]);
        assert_eq!(choose_template(&limited, 3), TemplateChoice::GiveUp, "no alternate template");
        let with_alternate = Schedule { max_attempts: 3, alternate_template_id: 8, ..schedule(1) };
        assert_eq!([3, 4, 5].map(|n| choose_template(&with_alternate, n)), [TemplateChoice::Alternate; 3]);
        assert_eq!(choose_template(&with_alternate, 6), TemplateChoice::GiveUp);
    }

    #[test]
    fn a_linked_schedule_waits_until_the_one_it_is_linked_to_has_entered_its_trade() {
        let mut runs = Vec::new();
        setup_runs(&mut runs, &[schedule(1), Schedule { linked_schedule_id: 1, ..schedule(2) }], day(MONDAY), utc("2026-11-16 13:00:00"));
        let second = Schedule { linked_schedule_id: 1, ..schedule(2) };
        assert!(linked_done(&runs, &schedule(1), day(MONDAY)), "no link");
        assert!(!linked_done(&runs, &second, day(MONDAY)), "the first is still waiting");
        runs[0].status = RunStatus::Partial;
        assert!(!linked_done(&runs, &second, day(MONDAY)), "only a complete entry counts");
        runs[0].status = RunStatus::Complete;
        assert!(linked_done(&runs, &second, day(MONDAY)));
        assert!(!linked_done(&runs, &second, day("2026-11-17")), "another day: no run of the first schedule");
    }
}
