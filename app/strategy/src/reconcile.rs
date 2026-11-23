//! The decision of the reconciliation with TWS (`coding_guideline.md` section 8): what to do about
//! the exit orders of one open trade, given the orders TWS reports as still working and the
//! executions seen. Pure logic - no `App`, no connection, no log: `App::reconcile_open_orders`
//! carries the decision out. It can be tested on its own, with plain data.

use crate::pricing::dec;
use crate::records::{StoredExecution, StoredTrade};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// What the reconciliation found for one trade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reconciliation {
    /// Everything the trade placed is working (or the trade is not open): nothing to do.
    Working,
    /// A stop adjustment waited for its old stop to be cancelled, but the old stop filled: no
    /// adjusted stop is placed, the trade is closed (at least in part).
    AdjustmentStopFilled {
        /// The old stop that filled.
        order_id: i32,
    },
    /// The old stop is gone, but orders cannot be placed now (the safety gate is shut): the
    /// adjusted stop keeps waiting for the next session and the trade has no stop meanwhile.
    AdjustmentBlocked,
    /// The old stop is gone and did not fill: the adjusted stop is placed now.
    PlaceAdjustedStop,
    /// The old stop still works: its cancel is sent again.
    CancelAgain {
        /// The old stop orders that still work.
        order_ids: Vec<i32>,
    },
    /// Exit orders of the trade are not working any more. Nothing is removed from the trade: such an
    /// order can also have been placed under another client id, which TWS does not report.
    Missing {
        /// Orders the executions show filled (the trade is closed, at least in part).
        filled: Vec<i32>,
        /// Orders that are simply not working any more (cancelled, expired, or under another client id).
        missing: Vec<i32>,
        /// One of the missing orders is a stop: the trade may have none.
        stop_gone: bool,
    },
}

/// The decision for `trade`. `working` are the order ids TWS reports as working, `executions` what
/// is known of the fills, `orders_allowed` whether orders may be placed now (the safety gate).
/// A trade that is not open is left alone.
#[must_use]
pub fn reconcile_trade(trade: &StoredTrade, working: &[i32], executions: &[StoredExecution], orders_allowed: bool) -> Reconciliation {
    if !trade.is_open() {
        return Reconciliation::Working;
    }
    let filled = |order_id: i32| executions.iter().any(|e| e.order_id == order_id);
    let old_stops: Vec<i32> = [trade.stop_order_id, trade.stop_limit_order_id].into_iter().flatten().collect();
    if trade.adjustments.as_ref().is_some_and(|a| a.replace_pending) {
        let still_working: Vec<i32> = old_stops.iter().copied().filter(|id| working.contains(id)).collect();
        // A fill comes first: whatever else the old stop looks like, the position is gone.
        return if let Some(order_id) = old_stops.iter().copied().find(|id| filled(*id)) {
            Reconciliation::AdjustmentStopFilled { order_id }
        } else if still_working.is_empty() && !orders_allowed {
            Reconciliation::AdjustmentBlocked
        } else if still_working.is_empty() {
            Reconciliation::PlaceAdjustedStop
        } else {
            Reconciliation::CancelAgain { order_ids: still_working }
        };
    }
    let gone: Vec<i32> = trade.exit_order_ids().into_iter().filter(|id| !working.contains(id)).collect();
    if gone.is_empty() {
        return Reconciliation::Working;
    }
    let (filled_ids, missing): (Vec<i32>, Vec<i32>) = gone.into_iter().partition(|id| filled(*id));
    let stop_gone = old_stops.iter().any(|id| missing.contains(id));
    Reconciliation::Missing { filled: filled_ids, missing, stop_gone }
}

/// What the option fills of `order_id` closed: the contracts (its smallest leg) and the price per contract, positive when the fills
/// brought money and negative when they cost it - the sign of `StoredTrade::price_close`. A fill that TWS reported twice counts once.
/// `None` without option fills of the order or with a price that cannot be computed.
#[must_use]
pub fn fill_of_order(executions: &[StoredExecution], order_id: i32) -> Option<(u32, Decimal)> {
    let mut seen: Vec<&str> = Vec::new();
    // Per option: the contracts filled and the money they brought (sold) or cost (bought).
    let mut legs: Vec<(i32, Decimal, Decimal)> = Vec::new();
    for e in executions.iter().filter(|e| e.order_id == order_id && e.sec_type == "OPT") {
        if seen.contains(&e.exec_id.as_str()) {
            continue;
        }
        seen.push(e.exec_id.as_str());
        let (qty, price) = (dec(e.quantity)?, dec(e.price)?);
        let amount = price.checked_mul(qty)?;
        let value = if e.side == "SLD" { amount } else { -amount };
        match legs.iter_mut().find(|(con_id, _, _)| *con_id == e.con_id) {
            Some(leg) => {
                leg.1 = leg.1.checked_add(qty)?;
                leg.2 = leg.2.checked_add(value)?;
            }
            None => legs.push((e.con_id, qty, value)),
        }
    }
    let units = legs.iter().map(|(_, qty, _)| *qty).min()?;
    let contracts = u32::try_from(units.to_i64()?).ok().filter(|q| *q > 0)?;
    let mut total = Decimal::ZERO;
    for (_, _, value) in &legs {
        total = total.checked_add(*value)?;
    }
    let price = total.checked_div(units)?.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    Some((contracts, price.normalize()))
}

/// How many contracts the stop of `trade` covers when that is fewer than the trade holds: `Some((covered, held))`. `remaining` are the
/// contracts still to fill of the orders TWS has reported, by order id. A stop that was placed for the contracts known at that time stays
/// smaller when the trade grows by late fills. `None` when the trade is not open, a stop adjustment is under way, no stop of the trade
/// has been reported, or the stop covers every contract.
#[must_use]
pub fn stop_coverage<S: std::hash::BuildHasher>(trade: &StoredTrade, remaining: &HashMap<i32, f64, S>) -> Option<(u32, u32)> {
    if !trade.is_open() || trade.adjustments.as_ref().is_some_and(|a| a.replace_pending) {
        return None;
    }
    // The stop and the stop limit of one trade are alternatives (one cancels the other): the bigger of the two is what is covered.
    let reported = [trade.stop_order_id, trade.stop_limit_order_id].into_iter().flatten().filter_map(|id| remaining.get(&id).copied());
    let covered = reported.fold(None, |best: Option<f64>, quantity| Some(best.map_or(quantity, |best| best.max(quantity))))?;
    let covered = covered.round().to_u32()?;
    let held = trade.remaining_qty();
    (covered < held).then_some((covered, held))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{ExitLevels, StopAdjustments};

    /// An open put spread with stop #11 and profit target #12.
    fn trade() -> StoredTrade {
        let mut trade: StoredTrade = serde_json::from_str(
            r#"{"day":"2026-11-13","opened":"","account":"DU1","symbol":"SPX","trade_type":"PutSpread","strikes":"6650/6640 P",
                "order_id":10,"qty":2,"multiplier":100,"price_open":"0.3","delta":0.0}"#,
        )
        .unwrap();
        trade.stop_order_id = Some(11);
        trade.profit_order_id = Some(12);
        trade
    }

    fn fill_of(order_id: i32) -> StoredExecution {
        StoredExecution {
            exec_id: format!("e-{order_id}"),
            time: "20261113 15:00:00 US/Eastern".into(),
            account: "DU1".into(),
            symbol: "SPX".into(),
            con_id: 100,
            order_id,
            order_ref: String::new(),
            sec_type: "OPT".into(),
            expiry: String::new(),
            right: "P".into(),
            strike: 6650.0,
            side: "BOT".into(),
            quantity: 1.0,
            price: 0.9,
        }
    }

    fn waiting_for_cancel(mut trade: StoredTrade) -> StoredTrade {
        trade.adjustments = Some(StopAdjustments { replace_pending: true, ..StopAdjustments::default() });
        trade
    }

    #[test]
    fn a_stop_that_covers_fewer_contracts_than_the_trade_holds_is_reported() {
        let held = StoredTrade { qty: 5, ..trade() };
        let working = |quantity: f64| HashMap::from([(11, quantity), (12, 5.0)]);
        assert_eq!(stop_coverage(&held, &working(4.0)), Some((4, 5)), "the stop was placed for 4 of the 5 contracts");
        assert_eq!(stop_coverage(&held, &working(5.0)), None, "it covers all");
        assert_eq!(stop_coverage(&held, &HashMap::new()), None, "no state of the stop is known: nothing to say");
        let limit_too = StoredTrade { stop_limit_order_id: Some(13), ..held.clone() };
        assert_eq!(stop_coverage(&limit_too, &HashMap::from([(11, 4.0), (13, 5.0)])), None, "the bigger of stop and stop limit counts");
        let over = StoredTrade { status: "Expired".to_string(), ..held.clone() };
        assert_eq!(stop_coverage(&over, &working(1.0)), None, "a trade that is over has nothing to cover");
        let replacing = waiting_for_cancel(held);
        assert_eq!(stop_coverage(&replacing, &working(1.0)), None, "while the stop is replaced the figure is not final");
    }
    #[test]
    fn a_trade_whose_exit_orders_all_work_needs_nothing() {
        assert_eq!(reconcile_trade(&trade(), &[11, 12], &[], true), Reconciliation::Working);
        // Orders of other trades in the list do not matter.
        assert_eq!(reconcile_trade(&trade(), &[5, 11, 12, 99], &[], true), Reconciliation::Working);
        // A trade without any exit order has nothing to compare.
        let bare = StoredTrade { stop_order_id: None, profit_order_id: None, ..trade() };
        assert_eq!(reconcile_trade(&bare, &[], &[], true), Reconciliation::Working);
    }

    #[test]
    fn a_trade_that_is_not_open_is_left_alone() {
        let closed = StoredTrade { status: "Stopped".into(), ..trade() };
        assert_eq!(reconcile_trade(&closed, &[], &[fill_of(11)], true), Reconciliation::Working);
    }

    #[test]
    fn missing_exit_orders_are_split_into_filled_ones_and_vanished_ones() {
        // The stop vanished without a fill: the trade may have no stop.
        assert_eq!(reconcile_trade(&trade(), &[12], &[], true), Reconciliation::Missing { filled: vec![], missing: vec![11], stop_gone: true });
        // The profit target filled while nobody watched.
        assert_eq!(reconcile_trade(&trade(), &[11], &[fill_of(12)], true), Reconciliation::Missing { filled: vec![12], missing: vec![], stop_gone: false });
        // Both are gone: the target filled, the stop did not.
        assert_eq!(reconcile_trade(&trade(), &[], &[fill_of(12)], true), Reconciliation::Missing { filled: vec![12], missing: vec![11], stop_gone: true });
        // The fills of other orders say nothing about these.
        assert_eq!(reconcile_trade(&trade(), &[12], &[fill_of(99)], true), Reconciliation::Missing { filled: vec![], missing: vec![11], stop_gone: true });
    }

    #[test]
    fn the_orders_of_every_kind_are_compared_the_stop_limit_and_the_levels_too() {
        let mut t = trade();
        t.stop_limit_order_id = Some(13);
        t.exit_levels = Some(ExitLevels { order_ids: [Some(12), Some(14), None, None], ..ExitLevels::default() });
        assert_eq!(reconcile_trade(&t, &[11, 12, 13, 14], &[], true), Reconciliation::Working);
        // Only the stop-limit and the second level are gone.
        assert_eq!(reconcile_trade(&t, &[11, 12], &[], true), Reconciliation::Missing { filled: vec![], missing: vec![13, 14], stop_gone: true });
    }

    #[test]
    fn a_waiting_adjustment_places_its_stop_once_the_old_one_is_gone() {
        let t = waiting_for_cancel(trade());
        assert_eq!(reconcile_trade(&t, &[12], &[], true), Reconciliation::PlaceAdjustedStop);
        // ... but not while orders cannot be placed: it keeps waiting.
        assert_eq!(reconcile_trade(&t, &[12], &[], false), Reconciliation::AdjustmentBlocked);
    }

    #[test]
    fn a_waiting_adjustment_cancels_an_old_stop_that_still_works_again() {
        let t = waiting_for_cancel(trade());
        assert_eq!(reconcile_trade(&t, &[11, 12], &[], true), Reconciliation::CancelAgain { order_ids: vec![11] });
        // Cancelling needs no gate: it only reduces risk.
        assert_eq!(reconcile_trade(&t, &[11, 12], &[], false), Reconciliation::CancelAgain { order_ids: vec![11] });
        // Both old stops of a stop-limit pair that still work are cancelled.
        let mut pair = waiting_for_cancel(trade());
        pair.stop_limit_order_id = Some(13);
        assert_eq!(reconcile_trade(&pair, &[11, 13], &[], true), Reconciliation::CancelAgain { order_ids: vec![11, 13] });
    }

    #[test]
    fn an_old_stop_that_filled_while_the_adjustment_waited_gets_no_replacement() {
        let t = waiting_for_cancel(trade());
        assert_eq!(reconcile_trade(&t, &[12], &[fill_of(11)], true), Reconciliation::AdjustmentStopFilled { order_id: 11 });
        // The fill counts first: also while the old stop is still listed, and with the gate shut.
        assert_eq!(reconcile_trade(&t, &[11, 12], &[fill_of(11)], false), Reconciliation::AdjustmentStopFilled { order_id: 11 });
    }

    #[test]
    fn the_fills_of_a_closing_order_give_its_contracts_and_its_price_per_contract() {
        let leg = |id: &str, order_id: i32, con_id: i32, side: &str, qty: f64, price: f64| StoredExecution {
            exec_id: id.into(),
            con_id,
            side: side.into(),
            quantity: qty,
            price,
            ..fill_of(order_id)
        };
        // A credit spread bought back for 0.20: the short leg bought at 0.30, the long leg sold at 0.10.
        let spread = [leg("a", 8, 1, "BOT", 1.0, 0.3), leg("b", 8, 2, "SLD", 1.0, 0.1)];
        assert_eq!(fill_of_order(&spread, 8), Some((1, Decimal::new(-2, 1))));
        // The same fill reported twice, a fill of another order and a combo row are not counted.
        let mut noisy = spread.to_vec();
        noisy.push(leg("a", 8, 1, "BOT", 1.0, 0.3));
        noisy.push(leg("c", 9, 1, "BOT", 5.0, 9.0));
        noisy.push(StoredExecution { sec_type: "BAG".into(), ..leg("d", 8, 3, "BOT", 1.0, 0.2) });
        assert_eq!(fill_of_order(&noisy, 8), Some((1, Decimal::new(-2, 1))));
        // Two partial fills of two contracts: the price is per contract.
        let two = [leg("a", 8, 1, "BOT", 1.0, 0.3), leg("b", 8, 1, "BOT", 1.0, 0.5), leg("c", 8, 2, "SLD", 2.0, 0.1)];
        assert_eq!(fill_of_order(&two, 8), Some((2, Decimal::new(-3, 1))));
        // A single option sold to close brings its price.
        assert_eq!(fill_of_order(&[leg("a", 8, 1, "SLD", 1.0, 1.25)], 8), Some((1, Decimal::new(125, 2))));
        assert_eq!(fill_of_order(&spread, 99), None, "no fills of that order");
    }

    #[test]
    fn a_trade_with_an_adjustment_that_is_not_waiting_is_compared_like_any_other() {
        let mut t = trade();
        t.adjustments = Some(StopAdjustments { replace_pending: false, ..StopAdjustments::default() });
        assert_eq!(reconcile_trade(&t, &[11, 12], &[], true), Reconciliation::Working);
        assert_eq!(reconcile_trade(&t, &[12], &[], true), Reconciliation::Missing { filled: vec![], missing: vec![11], stop_gone: true });
    }
}
