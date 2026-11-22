//! Settings > Trading Features > "IronFly/IronCondor - Full Stop Split": carries out the split of `strategy::trade_split` on the open
//! trades.
//!
//! It goes in steps, each once TWS has confirmed the one before: first the stop (and stop-limit) of the trade is cancelled, then its
//! profit targets, then the trade is split into its two sides, and each side gets a profit target and a stop of its own. Once started
//! the split is finished even if the short has gained again meanwhile: the trade has no stop by then, and the sides carry the same
//! stop price. Nothing is sent while the safety gate is shut or TWS is not connected.

use super::attach::working_target_ids;
use super::stop_conversion::conversion_price;
use super::{chain_quote, fix_price, live_legs, trade_exit, App, Decimal, Instant, StopAdjustments, StoredTrade, TradeStatus};
use std::time::Duration;
use strategy::trade_split::{split_due, split_sides, splittable, ShortQuote};

/// A cancel of the split that TWS has not confirmed is sent again after this pause.
const SPLIT_RETRY: Duration = Duration::from_secs(30);
/// How far above the cost to close a side's target is put when the target is reached already.
const TARGET_ABOVE_ASK: Decimal = Decimal::from_parts(10, 0, 0, false, 2);

/// The target of a side: lowered to the cost to close plus 0.10 when that cost is below it already (it would fill at once), else as
/// it is. `None` without a target.
pub(super) fn side_target(target: Option<Decimal>, ask: Option<Decimal>) -> Option<Decimal> {
    let target = target?;
    match ask.filter(|ask| *ask > Decimal::ZERO) {
        Some(ask) if ask < target => ask.checked_add(TARGET_ABOVE_ASK),
        _ => Some(target),
    }
}

/// The step of a split that has started, by what the trade still has working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SplitStep {
    /// The stop (or stop-limit) is cancelled first.
    CancelStop,
    /// Then the profit targets.
    CancelTargets,
    /// No exit order is left: the trade is split.
    Split,
}

/// What comes next for a trade whose split has started.
pub(super) fn split_step(trade: &StoredTrade) -> SplitStep {
    if trade.stop_order_id.is_some() || trade.stop_limit_order_id.is_some() {
        SplitStep::CancelStop
    } else if !working_target_ids(trade).is_empty() {
        SplitStep::CancelTargets
    } else {
        SplitStep::Split
    }
}

impl App {
    /// Whether the full stop split of `trade` has started (its exit orders are left alone by the
    /// stop adjustments and conversions meanwhile).
    pub(super) fn split_started(&self, trade: &StoredTrade) -> bool {
        self.watch.split_pending.contains_key(&trade.key())
    }

    /// One look at every open trade the split applies to (see the module).
    pub(super) fn check_stop_splits(&mut self, instant: Instant) {
        if !self.settings.convert_flags.iron_fly_stop_split_enabled || !self.trading_allowed() || self.tws_cmd_sender.is_none() {
            return;
        }
        let Some(price) = conversion_price(&self.settings.iron_fly_stop_split_price) else { return };
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index).cloned() else { continue };
            let started = self.split_started(&trade);
            if started && !trade.is_open() {
                // Closed meanwhile (its profit target filled before it was cancelled, say): nothing is left to split.
                if !trade.is_split() {
                    self.log(&format!("Full stop split of {}: the trade ended meanwhile - not split", trade.label()));
                }
                self.watch.split_pending.remove(&trade.key());
                continue;
            }
            if !started && !self.split_wanted(&trade, price) {
                continue;
            }
            if !started {
                self.log(&format!("Full stop split of {}: a short is at a bid of {price} or below - cancelling the stop", trade.label()));
                self.watch.split_pending.insert(trade.key(), trade.clone());
            }
            self.split_tick(index, &trade, instant);
        }
    }

    /// Whether the split of `trade` is to start now: it is one the split applies to, a short is at the price, and the fills of its
    /// legs are known (a split that could not be finished would leave the trade without a stop).
    fn split_wanted(&self, trade: &StoredTrade, price: Decimal) -> bool {
        let Some(template) = self.template_of(trade) else { return false };
        let stop_wanted = trade_exit::StopOrderType::parse(&template.stop_order_type).is_some_and(|kind| kind != trade_exit::StopOrderType::None);
        if !splittable(trade, &template.stop_type, stop_wanted) {
            return false;
        }
        let quote = |leg: Option<&crate::storage::StoredLeg>| {
            let quote = chain_quote(&self.chains, leg?.con_id)?;
            Some(ShortQuote { bid: Some(quote.bid).filter(|bid| *bid >= Decimal::ZERO), ask: quote.ask })
        };
        split_due(quote(trade.short_put.as_ref()), quote(trade.short_call.as_ref()), price) && split_sides(trade, &self.exec_history).is_ok()
    }

    /// The next step of a split that has started.
    fn split_tick(&mut self, index: usize, trade: &StoredTrade, instant: Instant) {
        let step = split_step(trade);
        let prefix = format!("{}/", trade.key());
        if step == SplitStep::Split {
            self.watch.split_tried.retain(|key, _| !key.starts_with(&prefix));
            self.split_trade(index, trade);
            return;
        }
        let key = format!("{prefix}{step:?}");
        if self.watch.split_tried.get(&key).is_some_and(|at| instant.saturating_duration_since(*at) < SPLIT_RETRY) {
            return;
        }
        self.watch.split_tried.insert(key, instant);
        let ids: Vec<i32> = match step {
            SplitStep::CancelStop => [trade.stop_order_id, trade.stop_limit_order_id].into_iter().flatten().collect(),
            SplitStep::CancelTargets | SplitStep::Split => working_target_ids(trade),
        };
        for id in ids {
            self.log(&format!("Cancel {id} (full stop split of {})", trade.label()));
            if self.submit_cancel(id).is_ok() {
                self.watch.user_cancelled.insert(id);
            }
        }
    }

    /// Splits the trade at `index`, whose exit orders are all gone, into its two sides, and places the exits of each side.
    fn split_trade(&mut self, index: usize, trade: &StoredTrade) {
        let label = trade.label();
        let snapshot = self.watch.split_pending.remove(&trade.key());
        // The prices of the cancelled stop and target are taken from the trade as it was when the split started.
        let mut basis = trade.clone();
        if let Some(before) = snapshot {
            basis.stop_price = before.stop_price;
            basis.profit_price = before.profit_price;
            basis.adjustments = before.adjustments;
            if let (Some(levels), Some(old)) = (basis.exit_levels.as_mut(), before.exit_levels) {
                levels.prices = old.prices;
            }
        } else {
            self.log(&format!("WARNING: full stop split of {label}: the prices of its old stop and target are not known (TradePilot was restarted)"));
        }
        let sides = match split_sides(&basis, &self.exec_history) {
            Ok(sides) => sides,
            Err(reason) => {
                self.log(&format!("WARNING: full stop split of {label} NOT made: {reason} - the trade has NO stop and NO target"));
                self.notify_missing_stop(&[format!("{label}: the full stop split could not be made - the trade has no stop")]);
                return;
            }
        };
        let mut split = trade.clone();
        split.status = TradeStatus::Split.as_str().to_string();
        self.save_trade(index, split);
        let names: Vec<String> = sides.iter().map(|side| format!("{} at {}", side.label(), side.price_open)).collect();
        self.log(&format!("Trade split: {label} into {}", names.join(" and ")));
        let template = self.template_of(trade).cloned();
        for mut side in sides {
            if side.adjustments.is_none() {
                side.adjustments = template.as_ref().map(|template| fresh_stop_record(template, &side));
            }
            self.store_new_trade(&side);
            self.trades_stored.push(side);
            self.pending.trade_log_dirty = true;
            self.place_side_exits(self.trades_stored.len() - 1);
        }
        self.refresh_trades(self.clock.instant(), true);
    }

    /// The profit target and then the stop of a side just split off (the stop joins the target's OCA group).
    fn place_side_exits(&mut self, index: usize) {
        let Some(side) = self.trades_stored.get(index).cloned() else { return };
        let label = side.label();
        let legs = trade_exit::close_legs(&side);
        let ask = live_legs(&side, &self.chains).and_then(|legs| legs.current_ask(&side.trade_type));
        let target = side_target(side.profit_price, ask).and_then(|price| fix_price(price, &side.symbol, legs.len()));
        if target != side.profit_price {
            self.log(&format!(
                "{label}: the target is lowered to {} (the side costs {} to close now)",
                target.map_or_else(|| "-".to_string(), |t| t.normalize().to_string()),
                ask.map_or_else(|| "-".to_string(), |a| a.normalize().to_string())
            ));
        }
        let mut cleared = side.clone();
        cleared.profit_price = None;
        if let (Some(target), Some(contract), Some(first_id)) = (target, trade_exit::exit_contract(&side, &legs), self.next_order_id) {
            self.place_attached_exits(index, &cleared, contract, None, Some(target), first_id, &legs);
            self.record_side_level(index);
        } else {
            self.log(&format!("WARNING: {label}: NO profit target placed (no target price, or the order cannot be built)"));
        }
        if self.trades_stored.get(index).and_then(|t| t.adjustments.as_ref()).is_some_and(|a| a.stop.is_some()) {
            self.place_replacement_stop(index);
        } else {
            self.log(&format!("WARNING: {label}: NO stop placed (its price is not known)"));
            self.notify_missing_stop(&[format!("{label}: no stop could be placed after the full stop split")]);
        }
    }

    /// A side with levels hit before: its new target is the order of the level that is next, so its fill ends the side.
    fn record_side_level(&mut self, index: usize) {
        let Some(trade) = self.trades_stored.get(index).cloned() else { return };
        let (Some(mut levels), Some(id)) = (trade.exit_levels.clone(), trade.profit_order_id) else { return };
        let slot = usize::from(levels.level);
        if let Some(entry) = levels.order_ids.get_mut(slot) {
            *entry = Some(id);
        }
        if let Some(qty) = levels.qty.get_mut(slot) {
            *qty = trade.remaining_qty();
        }
        let mut updated = trade;
        updated.exit_levels = Some(levels);
        self.save_trade(index, updated);
    }
}

/// The stop record of a side whose trade had none (it was placed before the legs of the stop were kept): the stop of the template on
/// the side's short.
fn fresh_stop_record(template: &super::TradeTemplate, side: &StoredTrade) -> StopAdjustments {
    let short = side.short_put.as_ref().or(side.short_call.as_ref()).map(|leg| leg.con_id);
    StopAdjustments {
        order_type: template.stop_order_type.clone(),
        stop: side.stop_price.map(|p| p.normalize().to_string()),
        trailing_offset: Some(template.stop_offset.to_string()),
        limit_offset: template.stop_limit_offset,
        market_offset: template.stop_limit_market_offset,
        legs: short.map(|con_id| vec![(con_id, 1, false)]).unwrap_or_default(),
        trigger_method: template.stop_trigger,
        outside_rth: template.order_flags.extended_hour_stop,
        ..StopAdjustments::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn a_target_that_is_reached_already_is_lowered_to_ten_cents_above_the_ask() {
        assert_eq!(side_target(Some(d("1.50")), Some(d("1.00"))), Some(d("1.10")));
        assert_eq!(side_target(Some(d("1.50")), Some(d("1.60"))), Some(d("1.50")), "not reached");
        assert_eq!(side_target(Some(d("1.50")), None), Some(d("1.50")), "no quote");
        assert_eq!(side_target(Some(d("1.50")), Some(d("0"))), Some(d("1.50")), "an ask of 0 is none");
        assert_eq!(side_target(None, Some(d("1.00"))), None);
    }
}
