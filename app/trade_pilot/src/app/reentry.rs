//! Re-entries: when a trade of a template with `reentry_stop`, `reentry_profit_target` or `reentry_close` ends the matching
//! way, a new entry is scheduled after the template's delay - with the template chosen for that case (or the same one) and,
//! if the template has a second one, a second entry. The entry is a run like those of the schedules (same attempts, same
//! window, same checks), so everything an entry has to pass is passed again. A chain of re-entries is limited by
//! `reentry_max_entries` and by the template's deadline; once the daily profit target is reached there are none.

use super::App;
use crate::schedule::{QuantityRule, Schedule};
use crate::schedule_run::{next_id, ReEntry, RunStatus, ScheduleRun};
use crate::storage::{StoredTrade, TradeOrigin};
use crate::templates::TradeTemplate;
use crate::trade_summary::TradeStatus;
use chrono::{DateTime, Utc};

/// How a trade ended, as far as re-entries are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ending {
    Stop,
    ProfitTarget,
    Close,
}

impl Ending {
    /// The ending of a trade that was finished with `status` by `how` ("stop 11", "profit target 12", "close order 13");
    /// `None` for one marked closed by hand.
    pub(super) fn of(status: TradeStatus, how: &str) -> Option<Self> {
        match status {
            TradeStatus::Stopped => Some(Self::Stop),
            TradeStatus::Closed if how.starts_with("profit target") => Some(Self::ProfitTarget),
            TradeStatus::Closed => Some(Self::Close),
            TradeStatus::Open | TradeStatus::Expired | TradeStatus::ClosedOther | TradeStatus::Split => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::ProfitTarget => "profit target",
            Self::Close => "close",
        }
    }

    /// Whether the template re-enters after this ending, and with which templates (0 = none chosen).
    fn templates(self, template: &TradeTemplate) -> Option<(i32, i32)> {
        match self {
            Self::Stop => template.reenter.re_enter_stop.then_some((template.reentry_stop_template_id, template.reentry_stop_template_id2)),
            Self::ProfitTarget => {
                template.reenter.re_enter_profit_target.then_some((template.reentry_profit_target_template_id, template.reentry_profit_target_template_id2))
            }
            Self::Close => template.reenter.re_enter_close.then_some((template.reentry_close_template_id, template.reentry_close_template_id2)),
        }
    }
}

/// The contracts of the re-entry of `trade`: a trade entered with a ratio is entered again with its base quantity.
fn reentry_quantity(trade: &StoredTrade, template: &TradeTemplate) -> u32 {
    let ratio = if trade.trade_type.contains("Put") {
        template.put_ratio
    } else if trade.trade_type.contains("Call") {
        template.call_ratio
    } else {
        1
    };
    let quantity = trade.qty;
    let ratio = u32::try_from(ratio).unwrap_or(1);
    if quantity > 1 && ratio > 1 {
        (quantity / ratio).max(1)
    } else {
        quantity
    }
}

/// The moment `hour`:`minute` of `day` in New York as a UTC instant; `None` for a time that does not exist.
fn new_york_instant(day: chrono::NaiveDate, hour: i32, minute: i32) -> Option<DateTime<Utc>> {
    let time = day.and_hms_opt(u32::try_from(hour).ok()?, u32::try_from(minute.max(0)).ok()?, 0)?;
    Some(time.and_local_timezone(chrono_tz::America::New_York).earliest()?.with_timezone(&Utc))
}

/// What one re-entry run enters.
struct ReEntryPlan<'a> {
    trade: &'a StoredTrade,
    origin: &'a TradeOrigin,
    template_id: i32,
    quantity: u32,
    start: DateTime<Utc>,
    label: &'a str,
    ending: Ending,
}

impl App {
    /// The short/long ratio of a trade at its entry (the asks of its short legs over the asks of its long legs),
    /// as text; `None` while a leg has no ask or the trade has no long leg.
    pub(super) fn entry_ratio(&self, planned: &crate::trade_entry::PlannedTrade) -> Option<String> {
        let ask = |leg: &Option<crate::trade_entry::LegRef>| -> Option<rust_decimal::Decimal> {
            match leg {
                None => Some(rust_decimal::Decimal::ZERO),
                Some(leg) => super::chain_quote(&self.chains, leg.con_id)?.ask.filter(|ask| *ask > rust_decimal::Decimal::ZERO),
            }
        };
        let legs = &planned.legs;
        let shorts = ask(&legs.short_call)?.checked_add(ask(&legs.short_put)?)?;
        let longs = ask(&legs.long_call)?.checked_add(ask(&legs.long_put)?)?;
        shorts.checked_div(longs.max(rust_decimal::Decimal::ZERO)).filter(|_| !longs.is_zero()).map(|ratio| ratio.normalize().to_string())
    }

    /// The origin recorded on a trade entered by the order `order_id` of the run `run_id` (if any) with `template`: the place in
    /// its chain of re-entries and how long a re-entry after it may take.
    pub(super) fn entry_origin(&self, run_id: Option<u32>, order_id: i32, template: &TradeTemplate) -> TradeOrigin {
        let run = run_id.and_then(|id| self.schedule.runs.iter().find(|run| run.id == id));
        let window = run.map_or(0, |run| u32::try_from((run.window_end - run.window_start).num_minutes()).unwrap_or(0));
        let trigger = run.and_then(|run| run.trigger.as_ref()).map_or(0, |trigger| trigger.trigger_id);
        let (root, entries) = run.and_then(|run| run.reentry.as_ref()).map_or((order_id, 0), |reentry| (reentry.root, reentry.entries));
        TradeOrigin { template_id: template.id, root, entries, window_minutes: if window == 0 { 5 } else { window }, trigger, ratio_open: None }
    }

    /// A trade ended by `how` with `status`: schedules its re-entries if its template asks for them.
    pub(super) fn schedule_reentries(&mut self, trade: &StoredTrade, status: TradeStatus, how: &str) {
        let Some(ending) = Ending::of(status, how) else { return };
        let Some(origin) = trade.origin.clone() else { return };
        let Some(template) = self.live_template(origin.template_id).cloned() else {
            return;
        };
        let Some((first, second)) = ending.templates(&template) else {
            return;
        };
        let label = trade.label();
        let skipped = |why: &str| format!("Re-entry after the {} of {label} skipped: {why}", ending.name());
        if self.daily_target_reached_today() {
            self.log(&skipped("the daily profit target is reached"));
            return;
        }
        // One re-entry per entry order: the legs of a strangle that end one after the other count once.
        if self.schedule.runs.iter().any(|run| run.reentry.as_ref().is_some_and(|reentry| reentry.parent_order == trade.order_id)) {
            return;
        }
        let start = self.clock.utc() + chrono::Duration::minutes(i64::from(template.reentry_delay.max(0)));
        if template.reentry_expiration_hour > 0 {
            let deadline = new_york_instant(self.clock.new_york().date(), template.reentry_expiration_hour, template.reentry_expiration_minute);
            if deadline.is_some_and(|deadline| start > deadline) {
                self.log(&skipped(&format!(
                    "it would come after the deadline {:02}:{:02}",
                    template.reentry_expiration_hour, template.reentry_expiration_minute
                )));
                return;
            }
        }
        if i64::from(origin.entries) >= i64::from(template.reentry_max_entries) {
            self.log(&skipped(&format!("the maximum of {} re-entries is reached", template.reentry_max_entries)));
            return;
        }
        let quantity = reentry_quantity(trade, &template);
        // The first template falls back to the trade's own; the second is only entered when it is set.
        let templates = [if first > 0 { first } else { origin.template_id }, second];
        for template_id in templates.into_iter().filter(|id| *id > 0) {
            self.add_reentry_run(&ReEntryPlan { trade, origin: &origin, template_id, quantity, start, label: &label, ending });
        }
        self.save_schedule_runs();
    }

    /// Adds the waiting run that enters a template again after the end of a trade.
    fn add_reentry_run(&mut self, plan: &ReEntryPlan<'_>) {
        let window = plan.origin.window_minutes.max(1);
        let schedule = Schedule {
            name: "Re-entry".to_string(),
            template_id: plan.template_id,
            account: plan.trade.account.clone(),
            quantity: QuantityRule::Fixed { count: plan.quantity },
            expiration_minutes: window,
            active: true,
            ..Schedule::default()
        };
        let id = next_id(&self.schedule.runs);
        let entries = plan.origin.entries.saturating_add(1);
        self.schedule.runs.push(ScheduleRun {
            id,
            schedule_id: 0,
            day: self.clock.new_york().date(),
            window_start: plan.start,
            window_end: plan.start + chrono::Duration::minutes(i64::from(window)),
            status: RunStatus::Waiting,
            attempts: 0,
            started: None,
            finished: None,
            notes: String::new(),
            reentry: Some(ReEntry { schedule, root: plan.origin.root, entries, parent_order: plan.trade.order_id }),
            trigger: None,
        });
        let wait = (plan.start - self.clock.utc()).num_seconds().max(0);
        self.log(&format!(
            "Schedule run {id}: re-entry {entries} after the {} of {} scheduled - template {} x{} in {}m {:02}s",
            plan.ending.name(),
            plan.label,
            plan.template_id,
            plan.quantity,
            wait / 60,
            wait % 60
        ));
    }
}
