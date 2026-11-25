//! The watchers behind the warning mails: when a condition has held long enough to be reported, and how often it is
//! reported again. Plain logic on `Instant`s and `Decimal`s, no `App`.

use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Today's profit when it has reached the daily target, else `None`. The target counts only while there is an
/// open trade (`open` is known and not 0 - an unquoted open trade makes it unknown) and open + closed lies above it.
#[must_use]
pub fn profit_target_total(target: Decimal, open: Option<Decimal>, closed: Decimal) -> Option<Decimal> {
    let open = open.filter(|open| *open != Decimal::ZERO)?;
    let total = open.checked_add(closed)?;
    (target > Decimal::ZERO && total > target).then_some(total)
}

/// The daily profit target: reached once a day, after it has been met for the delay.
#[derive(Debug, Default)]
pub struct ProfitTargetWatch {
    hit_day: Option<String>,
    met_since: Option<Instant>,
}

impl ProfitTargetWatch {
    /// The target was hit on `day` (yyyyMMdd).
    #[must_use]
    pub fn is_hit(&self, day: &str) -> bool {
        self.hit_day.as_deref() == Some(day)
    }

    /// One look: `met` says whether the target is met now. True exactly once per day - when it has been met without a
    /// break for `delay` (a delay of 0 hits at once).
    pub fn step(&mut self, day: &str, now: Instant, met: bool, delay: Duration) -> bool {
        if self.is_hit(day) {
            return false;
        }
        if !met {
            self.met_since = None;
            return false;
        }
        let since = *self.met_since.get_or_insert(now);
        if now.saturating_duration_since(since) < delay {
            return false;
        }
        self.hit_day = Some(day.to_string());
        self.met_since = None;
        true
    }
}

/// Conditions that must hold for a while before they are reported, and then are reported again only after a pause.
#[derive(Debug, Default)]
pub struct Sustained {
    since: HashMap<String, Instant>,
    sent: HashMap<String, Instant>,
}

impl Sustained {
    /// One look at the condition `key`: true when it has been `active` for `hold` and was not reported within `repeat`.
    pub fn fire(&mut self, key: &str, active: bool, now: Instant, hold: Duration, repeat: Duration) -> bool {
        if !active {
            self.since.remove(key);
            return false;
        }
        let since = *self.since.entry(key.to_string()).or_insert(now);
        if now.saturating_duration_since(since) < hold {
            return false;
        }
        if self.sent.get(key).is_some_and(|at| now.saturating_duration_since(*at) < repeat) {
            return false;
        }
        self.sent.insert(key.to_string(), now);
        true
    }
}

/// What the watchers remember between two looks.
#[derive(Debug, Default)]
pub struct WatchState {
    pub profit_target: ProfitTargetWatch,
    pub sustained: Sustained,
    /// The order that closed each finished trade (by `StoredTrade::key`): a different exit order of the same trade filling
    /// afterwards is an over fill.
    pub closed_by: HashMap<String, i32>,
    /// Over fills reported (the exit order ids).
    pub over_fills: HashSet<i32>,
    /// Open short ETF/stock options reported (contract ids) on `short_day`.
    pub short_reported: HashSet<i32>,
    pub short_day: String,
    /// When the watchers last looked.
    pub checked: Option<Instant>,
    /// TWS messages whose values did not match what the app knows for sure (by what they were about, e.g. "portfolio 123"): each is
    /// reported once, its values are not used.
    pub mismatch_reported: HashSet<String>,
    /// Trades (by `StoredTrade::key`) whose timed exit uses a market order: the limit order did not fill in time.
    pub timed_market: HashSet<String>,
    /// When a timed exit last tried to place its order, by trade (`StoredTrade::key`).
    pub timed_tried: HashMap<String, Instant>,
    /// When the limit order of a timed exit was sent, by its order id.
    pub timed_limit_sent: HashMap<i32, Instant>,
    /// The limit close of a timed exit that is moved towards the market step by step, by trade (`StoredTrade::key`).
    pub progressive_close: HashMap<String, ProgressiveClose>,
    /// When the early-exit conditions were last looked at.
    pub exit_checked: Option<Instant>,
    /// Exit orders the person (or a template time) asked to cancel: their end is expected, not a warning.
    pub user_cancelled: HashSet<i32>,
    /// The exit the person is asked to confirm deleting: the trade (index) and whether it is the stop (else the profit target).
    pub delete_asked: Option<(usize, bool)>,
    /// Stops and profit targets sent again with a new price, by order id: the new price goes on the trade only once TWS
    /// confirms the change, so a modify of an order that has ended meanwhile never shows a stop that does not exist.
    pub exit_changes: HashMap<i32, Decimal>,
    /// When a stop conversion was last started, by trade (`StoredTrade::key`).
    pub conversion_tried: HashMap<String, Instant>,
    /// Trades (by `StoredTrade::key`) whose full stop split has started, as they were then: their stop and target prices are
    /// gone from the trade once the orders are cancelled, the sides take them from here.
    pub split_pending: HashMap<String, crate::storage::StoredTrade>,
    /// When a cancel of the split was last sent, by trade and step (`<trade key>/<step>`).
    pub split_tried: HashMap<String, Instant>,
    /// When the opening orders of the last minute were sent (for the limit of new orders per minute).
    pub entries_sent: Vec<Instant>,
    /// The New York day (yyyyMMdd) on which today's loss reached its limit: no new position is opened on it any more.
    pub loss_stop_day: Option<String>,
}

/// Where a progressive limit close stands: the limit it was sent or last moved to (the price as typed: what the close pays, or for a
/// sold long what it asks), the steps made so far, and when the last move was.
#[derive(Clone, Debug)]
pub struct ProgressiveClose {
    pub price: Decimal,
    pub step: u32,
    pub moved: Instant,
}

/// Whether a credit trade's stop has been passed: what it costs to close is above the stop price (and quoted).
#[must_use]
pub fn beyond_stop(stop: Decimal, close_cost: Decimal) -> bool {
    stop > Decimal::ZERO && close_cost > stop
}

/// The percent beyond the stop typed in the settings: a number from 0 up (a comma taken as the decimal point), else `None`.
#[must_use]
pub fn beyond_stop_percent(text: &str) -> Option<Decimal> {
    crate::pricing::parse_decimal(text).filter(|pct| *pct >= Decimal::ZERO)
}

/// Whether a credit trade is far enough past its stop to be closed: what it costs to close is at least `percent` above the stop
/// (0 = any cost above the stop).
#[must_use]
pub fn far_beyond_stop(stop: Decimal, close_cost: Decimal, percent: Decimal) -> bool {
    let Some(limit) = percent.checked_div(Decimal::ONE_HUNDRED).and_then(|share| share.checked_add(Decimal::ONE)).and_then(|factor| stop.checked_mul(factor))
    else {
        return false;
    };
    beyond_stop(stop, close_cost) && close_cost >= limit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn the_target_needs_an_open_trade_a_quote_and_a_total_above_it() {
        assert_eq!(profit_target_total(d("1000"), Some(d("300")), d("800")), Some(d("1100")));
        assert_eq!(profit_target_total(d("1000"), Some(d("200")), d("800")), None, "exactly at the target is not above it");
        assert_eq!(profit_target_total(d("1000"), Some(d("0")), d("2000")), None, "no open trade");
        assert_eq!(profit_target_total(d("1000"), None, d("2000")), None, "an open trade without a quote");
        assert_eq!(profit_target_total(d("0"), Some(d("5")), d("5")), None, "0 = no target");
    }

    #[test]
    fn the_target_is_hit_once_after_the_delay_and_a_break_starts_it_again() {
        let t0 = Instant::now();
        let second = |n: u64| t0 + Duration::from_secs(n);
        let mut watch = ProfitTargetWatch::default();
        let delay = Duration::from_secs(15);
        assert!(!watch.step("20261116", second(0), true, delay));
        assert!(!watch.step("20261116", second(10), true, delay));
        assert!(!watch.step("20261116", second(12), false, delay), "a break");
        assert!(!watch.step("20261116", second(14), true, delay), "counted again from here");
        assert!(!watch.step("20261116", second(28), true, delay));
        assert!(watch.step("20261116", second(29), true, delay));
        assert!(watch.is_hit("20261116") && !watch.step("20261116", second(60), true, delay), "once");
        assert!(!watch.is_hit("20261117"), "a new day starts fresh");
        assert!(watch.step("20261117", second(100), true, Duration::ZERO), "no delay: at once");
    }

    #[test]
    fn a_condition_is_reported_after_it_held_and_again_only_after_the_pause() {
        let t0 = Instant::now();
        let second = |n: u64| t0 + Duration::from_secs(n);
        let (hold, repeat) = (Duration::from_mins(1), Duration::from_mins(15));
        let mut sustained = Sustained::default();
        assert!(!sustained.fire("a", true, second(0), hold, repeat));
        assert!(!sustained.fire("a", true, second(59), hold, repeat));
        assert!(sustained.fire("a", true, second(60), hold, repeat));
        assert!(!sustained.fire("a", true, second(200), hold, repeat), "paused");
        assert!(sustained.fire("a", true, second(961), hold, repeat), "after the pause");
        assert!(!sustained.fire("a", false, second(970), hold, repeat), "it passed");
        assert!(!sustained.fire("a", true, second(971), hold, repeat), "holds again from the start");
        assert!(!sustained.fire("b", true, second(0), hold, repeat), "every key is its own");
    }

    #[test]
    fn a_stop_is_passed_when_closing_costs_more_than_the_stop() {
        assert!(beyond_stop(d("0.90"), d("1.20")));
        assert!(!beyond_stop(d("0.90"), d("0.90")));
        assert!(!beyond_stop(d("0"), d("1")), "no stop");
    }

    #[test]
    fn a_trade_is_closed_beyond_its_stop_only_once_the_cost_is_the_percent_above_it() {
        assert!(far_beyond_stop(d("1.00"), d("1.10"), d("10")), "exactly 10 % above");
        assert!(!far_beyond_stop(d("1.00"), d("1.09"), d("10")));
        assert!(far_beyond_stop(d("1.00"), d("1.01"), d("0")), "0 % = any cost above the stop");
        assert!(!far_beyond_stop(d("1.00"), d("1.00"), d("0")), "at the stop is not beyond it");
        assert!(!far_beyond_stop(d("0"), d("5"), d("10")), "no stop");
        assert_eq!(beyond_stop_percent(" 12,5 "), Some(d("12.5")));
        assert_eq!(beyond_stop_percent("-1"), None);
        assert_eq!(beyond_stop_percent(""), None);
    }
}
