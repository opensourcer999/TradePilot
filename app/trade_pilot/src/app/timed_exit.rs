//! The timed exit of a template (`exit_hour`/`exit_minute` or `exit_minutes_in_trade`): at the time worked out when the trade
//! was filled, the trade is closed. A template with `exit_order_limit` closes with a limit order that starts in the middle between
//! bid and ask and moves towards the market every three seconds (`next_limit`); when it has not filled within two minutes (or
//! there is no quote) it is replaced by a market order. Without `exit_order_limit` the close is a market order. The early exit
//! (condition) and a closing trigger end in the same timed exit. The window is five minutes from the exit time: a close that has
//! not been placed by then is given up and reported. The order is the one of a manual close (`place_close_order`).

use super::{mask_account, App, Decimal, Duration, StoredTrade};
use crate::trade_mail;
use crate::watch::ProgressiveClose;
use chrono::NaiveDateTime;

/// The window after the exit time in which the close is tried.
const WINDOW: chrono::Duration = chrono::Duration::minutes(5);
/// A limit close that has not filled by then is replaced by a market close.
const LIMIT_WAIT: Duration = Duration::from_mins(2);
/// Without a quote a limit close waits this long after the exit time, then the market order goes out.
const NO_QUOTE_WAIT: chrono::Duration = chrono::Duration::minutes(3);
/// How often a close that could not be placed is tried again.
const RETRY: Duration = Duration::from_secs(5);
/// How often the limit of a working limit close is moved a step towards the market.
const STEP_EVERY: Duration = Duration::from_secs(3);

impl App {
    /// One look at the open trades that have a timed exit; `now` is New York time.
    pub(super) fn check_timed_exits(&mut self, now: NaiveDateTime) {
        if !self.trades_stored.iter().any(|t| t.is_open() && t.timed_exit.is_some()) {
            return;
        }
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index).cloned().filter(StoredTrade::is_open) else { continue };
            let Some(exit) = trade.timed_exit.clone() else { continue };
            let label = trade.label();
            let Ok(at) = NaiveDateTime::parse_from_str(&exit.at, "%Y-%m-%d %H:%M") else {
                self.log(&format!("Timed exit of {label}: '{}' is not a time - dropped", exit.at));
                self.drop_timed_exit(index);
                continue;
            };
            if now < at {
                continue;
            }
            if now > at + WINDOW {
                let planned = at.format("%H:%M").to_string();
                self.log(&format!(
                    "WARNING: Timed exit of {label} planned for {planned} did not complete within 5 minutes - given up, close the trade by hand"
                ));
                self.notify_trade_mail(&trade_mail::close_aborted(&format!("{label} ({})", mask_account(&trade.account)), &planned), true, true);
                self.drop_timed_exit(index);
                continue;
            }
            if !self.market_is_open() {
                continue;
            }
            self.timed_exit_step(index, &trade, &exit, now - at);
        }
    }

    /// Forgets the timed exit of the trade at `index`: it has run out or cannot be read.
    fn drop_timed_exit(&mut self, index: usize) {
        if let Some(mut trade) = self.trades_stored.get(index).cloned() {
            trade.timed_exit = None;
            self.save_trade(index, trade);
        }
    }

    /// The close of a trade whose exit time has come, `late` after it: waits for a close order on its way, moves its limit a step
    /// towards the market, replaces a limit order that did not fill, or places the order.
    fn timed_exit_step(&mut self, index: usize, trade: &StoredTrade, exit: &crate::storage::TimedExit, late: chrono::Duration) {
        let now = self.clock.instant();
        if let Some(close_id) = trade.close_order_id {
            let waited_too_long = self.watch.timed_limit_sent.get(&close_id).is_some_and(|sent| now.saturating_duration_since(*sent) >= LIMIT_WAIT);
            if waited_too_long && !self.watch.timed_market.contains(&trade.key()) {
                self.log(&format!("Timed exit of {}: the limit order {close_id} did not fill - cancelling it, a market order follows", trade.label()));
                self.watch.timed_market.insert(trade.key());
                let _logged = self.submit_cancel(close_id);
            } else if !waited_too_long {
                self.step_progressive_close(index, trade, close_id);
            }
            return;
        }
        if self.watch.timed_tried.get(&trade.key()).is_some_and(|at| now.saturating_duration_since(*at) < RETRY) {
            return;
        }
        self.watch.timed_tried.insert(trade.key(), now);
        let (ask, bid) = self.closing_quotes(trade);
        let market = !exit.limit || self.watch.timed_market.contains(&trade.key()) || (ask.is_none() && late >= NO_QUOTE_WAIT);
        if !market && ask.is_none() {
            return; // a limit close waits for a quote
        }
        // A limit close starts in the middle between bid and ask (at the ask when there is no bid).
        let limit = if market { None } else { ask.map(|ask| mid_price(ask, bid)) };
        match self.place_close_order(index, limit) {
            Ok(order_id) => {
                self.log(&format!("Timed exit of {}: close order {order_id} sent", trade.label()));
                if let Some(price) = limit {
                    self.watch.timed_limit_sent.insert(order_id, now);
                    let fixed = crate::pricing::fix_price(price, &trade.symbol, crate::trade_exit::close_legs(trade).len()).unwrap_or(price);
                    self.watch.progressive_close.insert(trade.key(), ProgressiveClose { price: fixed, step: 0, moved: now });
                }
            }
            Err(reason) => self.log(&format!("Timed exit of {} not placed yet: {reason}", trade.label())),
        }
    }

    /// Moves the limit of the working close order `close_id` one step towards the market when `STEP_EVERY` has passed since the last move: a
    /// close that pays goes up towards the ask, a close that sells goes down towards the bid, never beyond it.
    fn step_progressive_close(&mut self, index: usize, trade: &StoredTrade, close_id: i32) {
        let now = self.clock.instant();
        let Some(state) = self.watch.progressive_close.get(&trade.key()).cloned() else { return };
        if now.saturating_duration_since(state.moved) < STEP_EVERY {
            return;
        }
        let (ask, bid) = self.closing_quotes(trade);
        let sells = crate::trade_exit::close_receives(trade);
        let far = if sells { bid } else { ask };
        let Some(far) = far else { return };
        let step = state.step.saturating_add(1);
        let wanted = next_limit(state.price, step, step_unit(&trade.symbol), !sells, far);
        let next = crate::pricing::fix_price(wanted, &trade.symbol, crate::trade_exit::close_legs(trade).len()).unwrap_or(wanted);
        if next != state.price {
            if let Err(reason) = self.move_close_limit(index, close_id, next) {
                self.log(&format!("Timed exit of {}: the close price could not be moved: {reason}", trade.label()));
                return;
            }
        }
        self.watch.progressive_close.insert(trade.key(), ProgressiveClose { price: next, step, moved: now });
    }
}

/// The price in the middle between the ask and the bid; the ask when there is no bid.
fn mid_price(ask: Decimal, bid: Option<Decimal>) -> Decimal {
    bid.and_then(|bid| ask.checked_add(bid)).map_or(ask, |sum| sum / Decimal::TWO)
}

/// The size of one step of a progressive close: 0.05, for the ETFs SPY and QQQ 0.01.
fn step_unit(symbol: &str) -> Decimal {
    if matches!(symbol, "SPY" | "QQQ") {
        Decimal::new(1, 2)
    } else {
        Decimal::new(5, 2)
    }
}

/// The limit after step number `step` (1 for the first): from `price` by one unit for each of the first three steps, then by two, three,
/// four units ... Upwards when the close pays (`up`), else downwards; never beyond `far`, the quote on the other side (the ask when going
/// up, the bid when going down).
fn next_limit(price: Decimal, step: u32, unit: Decimal, up: bool, far: Decimal) -> Decimal {
    let units = if step <= 3 { 1 } else { step.saturating_sub(2) };
    let change = unit.checked_mul(Decimal::from(units)).unwrap_or(unit);
    if up {
        price.checked_add(change).map_or(far, |next| next.min(far))
    } else {
        price.checked_sub(change).map_or(far, |next| next.max(far))
    }
}

impl App {
    /// The profit target of an open trade whose cancel time has come is taken out of TWS: its working orders are cancelled
    /// (the stop stays). `now` is New York time.
    pub(super) fn check_profit_target_cancels(&mut self, now: NaiveDateTime) {
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index).cloned().filter(StoredTrade::is_open) else { continue };
            let Some(at) = trade.profit_cancel.as_deref().and_then(|text| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").ok()) else {
                if trade.profit_cancel.is_some() {
                    self.drop_profit_cancel(index);
                }
                continue;
            };
            if now < at || !self.market_is_open() {
                continue;
            }
            let ids = super::attach::working_target_ids(&trade);
            if ids.is_empty() {
                // Nothing working yet or any more: a marker that is a while old has nothing left to do.
                if now - at >= chrono::Duration::minutes(2) {
                    self.drop_profit_cancel(index);
                }
                continue;
            }
            self.log(&format!("Cancel expired profit target of {}: orders {ids:?}", trade.label()));
            for id in ids {
                if self.submit_cancel(id).is_ok() {
                    self.watch.user_cancelled.insert(id);
                }
            }
            self.drop_profit_cancel(index);
        }
    }

    /// Forgets the profit-target cancel time of the trade at `index`.
    fn drop_profit_cancel(&mut self, index: usize) {
        if let Some(mut trade) = self.trades_stored.get(index).cloned() {
            trade.profit_cancel = None;
            self.save_trade(index, trade);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn the_limit_moves_by_one_unit_for_three_steps_then_by_two_three_four_units() {
        let (unit, far) = (d("0.05"), d("5"));
        let mut price = d("1.00");
        let mut seen = Vec::new();
        for step in 1..=6 {
            price = next_limit(price, step, unit, true, far);
            seen.push(price.to_string());
        }
        assert_eq!(seen, ["1.05", "1.10", "1.15", "1.25", "1.40", "1.60"], "+0.05 three times, then +0.10, +0.15, +0.20");
        // ETFs step by 0.01.
        assert_eq!(next_limit(d("1.00"), 1, step_unit("SPY"), true, far), d("1.01"));
        assert_eq!(step_unit("QQQ"), d("0.01"));
        assert_eq!(step_unit("SPX"), d("0.05"));
    }

    #[test]
    fn the_limit_never_goes_beyond_the_quote_on_the_other_side_and_a_sale_goes_down() {
        assert_eq!(next_limit(d("0.38"), 1, d("0.05"), true, d("0.40")), d("0.40"), "stops at the ask");
        assert_eq!(next_limit(d("0.40"), 2, d("0.05"), true, d("0.40")), d("0.40"));
        assert_eq!(next_limit(d("1.00"), 1, d("0.05"), false, d("0.90")), d("0.95"), "a sale goes down towards the bid");
        assert_eq!(next_limit(d("0.92"), 1, d("0.05"), false, d("0.90")), d("0.90"), "and stops at it");
    }

    #[test]
    fn a_close_starts_in_the_middle_between_bid_and_ask_or_at_the_ask_without_a_bid() {
        assert_eq!(mid_price(d("0.40"), Some(d("0.20"))), d("0.30"));
        assert_eq!(mid_price(d("0.40"), None), d("0.40"));
    }
}
