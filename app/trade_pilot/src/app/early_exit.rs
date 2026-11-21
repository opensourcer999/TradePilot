//! The early exit of a template (`exit_condition_id`): while a trade is open the condition is checked against the quotes of now;
//! once it is true the trade is closed like a timed exit that is due at once (limit order first or market order, as the
//! template's `exit_order_limit` says). Nothing is checked in the first two minutes of the session, while the quotes are
//! not current, or for a trade that has a close order on its way. The values a rule can look at about the trade
//! (`Trade_*`) are those of `condition::TradeValues`.

use super::{custom_held, live_legs, stored_record, App, Duration, StoredTrade};
use crate::condition::{evaluate, LegValues, Snapshot, TradeValues, Verdict};
use crate::storage::TimedExit;
use crate::trade_live::LiveLeg;
use chrono::{NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;

/// How often the early-exit conditions are looked at.
const CHECK_EVERY: Duration = Duration::from_secs(5);

fn dec_f64(value: f64) -> Option<Decimal> {
    Decimal::try_from(value).ok()
}

fn leg_values(leg: Option<&LiveLeg>) -> Option<LegValues> {
    let leg = leg?;
    let quote = leg.quote;
    let delta = quote.and_then(|quote| quote.delta).and_then(dec_f64).and_then(|delta| delta.checked_mul(Decimal::ONE_HUNDRED)).map(|delta| delta.abs());
    Some(LegValues { strike: leg.strike, ask: quote.and_then(|quote| quote.ask), delta })
}

impl App {
    /// What closing `trade` costs now (credit trades) and what it brings now (debit trades); a custom trade is valued by its own legs.
    pub(super) fn closing_quotes(&self, trade: &StoredTrade) -> (Option<Decimal>, Option<Decimal>) {
        if let Some(held) = custom_held(trade, &self.chains) {
            let value = crate::trade_live::custom_closing_value(&held);
            return (value.filter(|v| *v < Decimal::ZERO).map(|v| -v), value.filter(|v| *v > Decimal::ZERO));
        }
        let legs = live_legs(trade, &self.chains);
        (legs.as_ref().and_then(|legs| legs.current_ask(&trade.trade_type)), legs.as_ref().and_then(|legs| legs.current_bid(&trade.trade_type)))
    }

    /// What closing the open `trade` costs now, as the close dialog and the stop/target window show it: the ask of its legs; a custom
    /// trade that receives money when it is closed shows that amount. `None` while a quote is missing.
    pub(super) fn closing_price(&self, trade: &StoredTrade) -> Option<Decimal> {
        let (cost, proceeds) = self.closing_quotes(trade);
        cost.or(proceeds.filter(|_| trade.custom_legs.is_some()))
    }

    /// The value of the open `trade` for its stop adjustments: `(bid, ask)` as `current_bid` and `current_ask` give them; a custom trade is
    /// valued by its own legs. `None` while a quote is missing.
    pub(super) fn adjustment_values(&self, trade: &StoredTrade) -> Option<(Decimal, Decimal)> {
        if let Some(held) = custom_held(trade, &self.chains) {
            return crate::trade_live::custom_open_values(&held, trade.price_open >= Decimal::ZERO);
        }
        let live = live_legs(trade, &self.chains)?;
        Some((live.current_bid(&trade.trade_type)?, live.current_ask(&trade.trade_type)?))
    }

    /// The market and the open `trade` as the conditions see them.
    pub(super) fn trade_snapshot(&self, trade: &StoredTrade) -> Snapshot {
        let mut snapshot = self.condition_snapshot();
        snapshot.underlying = self
            .chains
            .underlyings
            .iter()
            .find(|underlying| underlying.symbol == trade.symbol)
            .and_then(|underlying| underlying.price)
            .and_then(dec_f64)
            .filter(|price| *price > Decimal::ZERO);
        let legs = live_legs(trade, &self.chains);
        let closing = self.closing_quotes(trade);
        let record = stored_record(trade, &self.chains);
        let profit = record.as_ref().filter(|record| record.profit_known).map(|record| record.current_profit);
        let total_delta = record.as_ref().and_then(|record| dec_f64(record.total_delta)).filter(|delta| !delta.is_zero());
        let held = custom_held(trade, &self.chains);
        let mut custom_deltas: [Option<Decimal>; 4] = [None; 4];
        for (slot, leg) in custom_deltas.iter_mut().zip(held.iter().flatten()) {
            *slot =
                leg.quote.and_then(|quote| quote.delta).and_then(dec_f64).and_then(|delta| delta.checked_mul(Decimal::ONE_HUNDRED)).map(|delta| delta.abs());
        }
        snapshot.account.clone_from(&trade.account);
        let premium = trade.price_open.checked_mul(Decimal::from(trade.multiplier)).map(|premium| premium.abs()).filter(|premium| !premium.is_zero());
        let profit_pct = profit
            .zip(premium)
            .and_then(|(profit, premium)| profit.checked_div(Decimal::from(trade.qty.max(1)))?.checked_div(premium)?.checked_mul(Decimal::ONE_HUNDRED));
        snapshot.trade = Some(TradeValues {
            profit,
            profit_pct,
            ask: closing.0,
            bid: closing.1,
            short_put: legs.as_ref().and_then(|legs| leg_values(legs.short_put.as_ref())),
            long_put: legs.as_ref().and_then(|legs| leg_values(legs.long_put.as_ref())),
            short_call: legs.as_ref().and_then(|legs| leg_values(legs.short_call.as_ref())),
            long_call: legs.as_ref().and_then(|legs| leg_values(legs.long_call.as_ref())),
            position_delta: total_delta.and_then(|total| total.checked_div(Decimal::from(trade.qty.max(1)))),
            total_delta,
            ratio_open: trade.origin.as_ref().and_then(|origin| origin.ratio_open.as_deref()).and_then(|text| text.parse().ok()),
            custom_deltas,
        });
        snapshot
    }

    /// Looks at the early-exit condition of every open trade whose template has one; `now` is New York time. A condition that
    /// is true starts the exit at once (see the module).
    pub(super) fn check_early_exits(&mut self, now: NaiveDateTime) {
        let instant = self.clock.instant();
        if !crate::clock::due(&mut self.watch.exit_checked, instant, CHECK_EVERY) {
            return;
        }
        let after_opening = NaiveTime::from_hms_opt(9, 32, 0).is_some_and(|time| now.time() >= time);
        if !after_opening || !self.market_is_open() || self.market_data_warning(instant, true).is_some() {
            return;
        }
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index).filter(|t| t.is_open() && t.close_order_id.is_none()).cloned() else { continue };
            let Some(template) = self.template_of(&trade).cloned() else {
                continue;
            };
            let due = trade.timed_exit.as_ref().and_then(|exit| NaiveDateTime::parse_from_str(&exit.at, "%Y-%m-%d %H:%M").ok()).is_some_and(|at| at <= now);
            if template.exit_condition_id <= 0 || due {
                continue;
            }
            let Some(condition) = self.conditions.list.iter().find(|c| i64::from(c.id) == i64::from(template.exit_condition_id)).cloned() else {
                continue;
            };
            let snapshot = self.trade_snapshot(&trade);
            let snapshot = self.with_bars(snapshot, &condition, &trade.symbol);
            let evaluation = evaluate(&condition, &snapshot);
            for line in &evaluation.lines {
                self.debug(&format!("Early exit {} {} {}: {line}", trade.symbol, trade.trade_type, trade.strikes));
            }
            if evaluation.verdict != Verdict::Pass {
                continue;
            }
            self.log(&format!(
                "Early exit of {} {} {}: condition {} is true - closing the trade",
                trade.symbol, trade.trade_type, trade.strikes, condition.name
            ));
            let mut updated = trade;
            updated.timed_exit = Some(TimedExit { at: now.format("%Y-%m-%d %H:%M").to_string(), limit: template.exit_flags.exit_order_limit });
            self.save_trade(index, updated);
        }
    }
}
