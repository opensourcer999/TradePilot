//! Option positions TWS holds that no open trade of this app accounts for: opened by hand in TWS on an earlier day, by another
//! API client, or by an instance of this app on another computer with its own trade list. Long options kept for reuse also show here, as their trade
//! is booked closed. Also the working orders of other API clients that touch the contracts of a trade: while one works, this app
//! sends no exit order of its own for that trade. And the trade such positions become when the person takes them over
//! (`import_trade`). Pure logic: the positions, orders and trades come in as arguments.

use crate::pricing::dec;
use crate::records::{StoredExecution, StoredTrade};
use crate::trade_entry::Action;
use crate::trade_exit::close_legs;
use broker::{OpenOrderInfo, Position};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// Why the chosen positions cannot be taken over as one trade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportRefusal {
    /// No position is chosen.
    NoLegs,
    /// The positions are on more than one account.
    Accounts,
    /// The positions are on more than one underlying.
    Symbols,
    /// A position has no multiplier, or the positions have different ones.
    Multiplier,
    /// TWS sent no average cost for a position, so no entry price can be proposed.
    AverageCost,
    /// The legs do not make a trade (a quantity that is no whole number of contracts, say).
    NotATrade,
}

/// The order id of the next trade taken over from positions: below every id given so far. Taken-over trades have negative ids, which
/// TWS never gives an order, so they never meet the id of a real order (`trade_rebuild::rebuild` tells trades apart by it).
#[must_use]
pub fn next_import_id(trades: &[StoredTrade]) -> Option<i32> {
    trades.iter().map(|t| t.order_id).filter(|id| *id < 0).min().unwrap_or(0).checked_sub(1)
}

/// The trade the `chosen` positions make, with the id `order_id` (`next_import_id`) and opened at `time` ("yyyyMMdd HH:mm:ss"). Each
/// position is a leg with the contracts it has without a trade; the trade is built as one from executions is
/// (`trade_rebuild::opening_trade`): its type, strikes, ratios and quantity from the legs, its entry price from their average cost
/// divided by the multiplier (what TWS shows as the average price), a sold leg as a credit. The person confirms or changes that price
/// before the trade is stored. The multiplier is the positions' own (5 for an option on the DAX, say).
///
/// # Errors
///
/// Why the positions do not make one trade (`ImportRefusal`).
pub fn import_trade(positions: &[Position], chosen: &[UnmanagedPosition], order_id: i32, time: &str) -> Result<StoredTrade, ImportRefusal> {
    let legs: Vec<(&Position, Decimal)> = chosen.iter().filter_map(|u| positions.get(u.position).map(|p| (p, u.quantity))).collect();
    let (first, _) = legs.first().ok_or(ImportRefusal::NoLegs)?;
    if legs.iter().any(|(p, _)| p.account != first.account) {
        return Err(ImportRefusal::Accounts);
    }
    if legs.iter().any(|(p, _)| p.symbol != first.symbol) {
        return Err(ImportRefusal::Symbols);
    }
    let multiplier = first.multiplier.and_then(dec).filter(|m| *m > Decimal::ZERO).ok_or(ImportRefusal::Multiplier)?;
    if legs.iter().any(|(p, _)| p.multiplier.and_then(dec) != Some(multiplier)) {
        return Err(ImportRefusal::Multiplier);
    }
    let mut fills = Vec::with_capacity(legs.len());
    for (position, quantity) in &legs {
        let cost = position.average_cost.and_then(dec).ok_or(ImportRefusal::AverageCost)?;
        let price = cost.checked_div(multiplier).and_then(|p| p.to_f64()).ok_or(ImportRefusal::AverageCost)?;
        fills.push(StoredExecution {
            exec_id: format!("import-{}", position.contract_id.trim()),
            time: time.to_string(),
            account: position.account.clone(),
            symbol: position.symbol.clone(),
            con_id: position.contract_id.trim().parse().map_err(|_| ImportRefusal::NotATrade)?,
            order_id,
            order_ref: String::new(),
            sec_type: position.security_type.clone(),
            expiry: position.expiry.clone(),
            right: position.right.clone(),
            strike: position.strike,
            side: if quantity.is_sign_negative() { "SLD" } else { "BOT" }.to_string(),
            quantity: quantity.abs().to_f64().ok_or(ImportRefusal::NotATrade)?,
            price,
        });
    }
    let fills: Vec<&StoredExecution> = fills.iter().collect();
    let mut trade = crate::trade_rebuild::opening_trade(order_id, &fills, true).ok_or(ImportRefusal::NotATrade)?;
    trade.multiplier = multiplier.to_u32().filter(|m| Decimal::from(*m) == multiplier).ok_or(ImportRefusal::Multiplier)?;
    Ok(trade)
}

/// An option position, or the part of it, that no open trade holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnmanagedPosition {
    /// Index of the position in the slice given to `unmanaged_positions`.
    pub position: usize,
    /// Contracts without a trade, with the sign of the position (negative = short); never 0 and never more than the position.
    pub quantity: Decimal,
}

/// The contracts the open `trades` hold, per account and contract id: the open quantity times the leg's ratio, negative for a sold leg.
fn held_by_trades(trades: &[StoredTrade]) -> Option<HashMap<(String, i32), Decimal>> {
    let mut held: HashMap<(String, i32), Decimal> = HashMap::new();
    for trade in trades.iter().filter(|t| t.is_open()) {
        let open = Decimal::from(trade.remaining_qty());
        for leg in close_legs(trade) {
            let contracts = open.checked_mul(Decimal::from(leg.ratio))?;
            // The closing order buys back what the trade sold.
            let signed = match leg.action {
                Action::Buy => -contracts,
                Action::Sell => contracts,
            };
            let sum = held.entry((trade.account.clone(), leg.con_id)).or_default();
            *sum = sum.checked_add(signed)?;
        }
    }
    Some(held)
}

/// The option positions that the open `trades` do not account for, in the order of `positions`. A trade that holds more of a contract
/// than TWS reports leaves nothing over: that is a missing position, which the position check of the trades reports. A position on the
/// other side of what the trades hold is unmanaged in full. `None` when a quantity cannot be read or the sums overflow.
#[must_use]
pub fn unmanaged_positions(positions: &[Position], trades: &[StoredTrade]) -> Option<Vec<UnmanagedPosition>> {
    let held = held_by_trades(trades)?;
    let mut found = Vec::new();
    for (index, position) in positions.iter().enumerate().filter(|(_, p)| p.security_type == "OPT") {
        let quantity = dec(position.quantity)?;
        if quantity.is_zero() {
            continue;
        }
        let Ok(con_id) = position.contract_id.trim().parse::<i32>() else { continue };
        let by_trades = held.get(&(position.account.clone(), con_id)).copied().unwrap_or_default();
        let rest = quantity.checked_sub(by_trades)?;
        let same_side = rest.is_sign_negative() == quantity.is_sign_negative();
        if rest.is_zero() || !same_side {
            continue;
        }
        let quantity = if rest.abs() > quantity.abs() { quantity } else { rest };
        found.push(UnmanagedPosition { position: index, quantity });
    }
    Some(found)
}

/// The working orders of other API clients (`orders`) on the account of `trade` that touch one of its contracts: an option order on
/// one of them, or a combo with a leg on one of them. A combo whose legs could not be read counts when it is on the trade's symbol, as
/// nothing shows that it leaves the trade alone. A second stop next to such an order is not in its OCA group: if both filled, the
/// second fill would open a new position.
#[must_use]
pub fn other_client_orders_on<'a>(trade: &StoredTrade, orders: &'a [OpenOrderInfo]) -> Vec<&'a OpenOrderInfo> {
    let contracts: Vec<i32> = close_legs(trade).iter().map(|leg| leg.con_id).collect();
    orders
        .iter()
        .filter(|order| order.account.trim() == trade.account.trim())
        .filter(|order| match (order.sec_type.as_str(), &order.combo_legs) {
            ("OPT", _) => contracts.contains(&order.con_id),
            ("BAG", Some(legs)) => legs.iter().any(|leg| contracts.contains(&leg.con_id)),
            ("BAG", None) => order.symbol == trade.symbol,
            _ => false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{CustomLegStored, ExitLevels, StoredLeg};

    fn option(account: &str, con_id: i32, quantity: f64) -> Position {
        Position {
            account: account.into(),
            contract_id: con_id.to_string(),
            symbol: "SPX".into(),
            security_type: "OPT".into(),
            expiry: "20261218".into(),
            strike: 6650.0,
            right: "P".into(),
            multiplier: Some(100.0),
            quantity,
            average_cost: Some(120.0),
        }
    }

    /// A put spread of `qty` contracts: short put 1, long put 2, on account DU1.
    fn put_spread(qty: u32) -> StoredTrade {
        let mut trade: StoredTrade = serde_json::from_str(
            r#"{"day":"2026-11-13","opened":"","account":"DU1","symbol":"SPX","trade_type":"PutSpread","strikes":"6650/6640 P",
                "order_id":10,"qty":1,"multiplier":100,"price_open":"1.2","delta":0.0,"status":"Open"}"#,
        )
        .unwrap();
        trade.qty = qty;
        trade.short_put = Some(StoredLeg { con_id: 1, strike: "6650".into() });
        trade.long_put = Some(StoredLeg { con_id: 2, strike: "6640".into() });
        trade
    }

    fn quantities(found: &[UnmanagedPosition]) -> Vec<(usize, Decimal)> {
        found.iter().map(|u| (u.position, u.quantity)).collect()
    }

    #[test]
    fn positions_a_trade_holds_in_full_are_managed() {
        let positions = [option("DU1", 1, -2.0), option("DU1", 2, 2.0)];
        assert_eq!(unmanaged_positions(&positions, &[put_spread(2)]), Some(vec![]));
    }

    #[test]
    fn a_position_without_a_trade_is_unmanaged() {
        let positions = [option("DU1", 7, -3.0)];
        assert_eq!(quantities(&unmanaged_positions(&positions, &[put_spread(2)]).unwrap()), vec![(0, Decimal::from(-3))]);
    }

    #[test]
    fn contracts_beyond_what_the_trades_hold_are_unmanaged() {
        let positions = [option("DU1", 1, -5.0), option("DU1", 2, 2.0)];
        assert_eq!(quantities(&unmanaged_positions(&positions, &[put_spread(2)]).unwrap()), vec![(0, Decimal::from(-3))]);
    }

    #[test]
    fn a_trade_that_holds_more_than_tws_leaves_nothing_but_the_other_side_is_unmanaged() {
        let positions = [option("DU1", 1, -1.0), option("DU1", 2, -1.0)];
        let found = quantities(&unmanaged_positions(&positions, &[put_spread(2)]).unwrap());
        assert_eq!(found, vec![(1, Decimal::NEGATIVE_ONE)], "the short of contract 2 is no trade's, the missing part of contract 1 is no position");
    }

    #[test]
    fn a_trade_of_another_account_does_not_hold_the_position() {
        let positions = [option("DU2", 1, -2.0)];
        assert_eq!(quantities(&unmanaged_positions(&positions, &[put_spread(2)]).unwrap()), vec![(0, Decimal::from(-2))]);
    }

    #[test]
    fn closed_trades_hold_nothing() {
        let closed = StoredTrade { status: "Closed".into(), ..put_spread(2) };
        let positions = [option("DU1", 1, -2.0)];
        assert_eq!(quantities(&unmanaged_positions(&positions, &[closed]).unwrap()), vec![(0, Decimal::from(-2))]);
    }

    #[test]
    fn contracts_closed_by_profit_levels_are_no_longer_held_by_the_trade() {
        let mut trade = put_spread(3);
        trade.exit_levels = Some(ExitLevels { level: 1, closed: vec![(2, "0.1".into())], ..ExitLevels::default() });
        let positions = [option("DU1", 1, -3.0), option("DU1", 2, 1.0)];
        assert_eq!(quantities(&unmanaged_positions(&positions, &[trade]).unwrap()), vec![(0, Decimal::from(-2))]);
    }

    #[test]
    fn the_ratio_of_a_leg_counts() {
        let trade = StoredTrade { put_ratio: 2, ..put_spread(1) };
        let positions = [option("DU1", 1, -2.0), option("DU1", 2, 2.0)];
        assert_eq!(unmanaged_positions(&positions, &[trade]), Some(vec![]));
    }

    #[test]
    fn the_legs_of_a_custom_trade_count_with_their_ratio_and_sign() {
        let leg = |con_id: i32, ratio: i32| CustomLegStored { con_id, right: "P".into(), strike: "6650".into(), expiry: "20261218".into(), ratio };
        let trade =
            StoredTrade { trade_type: "Custom".into(), short_put: None, long_put: None, custom_legs: Some(vec![leg(1, -1), leg(2, 2)]), ..put_spread(2) };
        let positions = [option("DU1", 1, -2.0), option("DU1", 2, 5.0)];
        assert_eq!(quantities(&unmanaged_positions(&positions, &[trade]).unwrap()), vec![(1, Decimal::ONE)]);
    }

    #[test]
    fn stocks_and_closed_positions_are_left_out() {
        let stock = Position { security_type: "STK".into(), ..option("DU1", 9, 100.0) };
        let positions = [stock, option("DU1", 8, 0.0)];
        assert_eq!(unmanaged_positions(&positions, &[]), Some(vec![]));
    }

    #[test]
    fn an_unreadable_contract_id_is_left_out_and_an_unreadable_quantity_gives_nothing() {
        let bad_id = Position { contract_id: "x".into(), ..option("DU1", 1, -1.0) };
        assert_eq!(unmanaged_positions(&[bad_id], &[]), Some(vec![]));
        assert_eq!(unmanaged_positions(&[option("DU1", 1, f64::NAN)], &[]), None);
    }

    /// A working order of another client, as TWS reported one entered by hand (client 0, order id 0, its perm id).
    fn other_order(sec_type: &str, con_id: i32, legs: Option<Vec<(i32, &str)>>) -> OpenOrderInfo {
        OpenOrderInfo {
            order_id: 0,
            con_id,
            symbol: "SPX".into(),
            sec_type: sec_type.into(),
            expiry: String::new(),
            strike: 0.0,
            right: String::new(),
            action: "BUY".into(),
            total_quantity: 1.0,
            order_type: "STP".into(),
            limit_price: None,
            aux_price: Some(5.0),
            tif: "GTC".into(),
            oca_group: String::new(),
            account: "DU1".into(),
            order_ref: String::new(),
            client_id: 0,
            perm_id: 622_296_617,
            combo_legs: legs.map(|legs| legs.into_iter().map(|(con_id, action)| broker::OpenOrderLeg { con_id, ratio: 1, action: action.into() }).collect()),
            trail_stop_price: None,
        }
    }

    #[test]
    fn an_option_order_of_another_client_on_a_leg_of_the_trade_touches_it() {
        let orders = [other_order("OPT", 1, None), other_order("OPT", 9, None)];
        let found = other_client_orders_on(&put_spread(2), &orders);
        assert_eq!(found.iter().map(|o| o.con_id).collect::<Vec<_>>(), vec![1], "the short put 1, not contract 9");
    }

    #[test]
    fn a_combo_of_another_client_touches_the_trade_through_one_of_its_legs() {
        let spread = put_spread(2);
        assert_eq!(other_client_orders_on(&spread, &[other_order("BAG", 0, Some(vec![(2, "SELL"), (8, "BUY")]))]).len(), 1);
        assert_eq!(other_client_orders_on(&spread, &[other_order("BAG", 0, Some(vec![(7, "SELL"), (8, "BUY")]))]).len(), 0);
    }

    #[test]
    fn a_combo_whose_legs_are_unknown_counts_on_the_same_symbol_only() {
        let spread = put_spread(2);
        assert_eq!(other_client_orders_on(&spread, &[other_order("BAG", 0, None)]).len(), 1, "nothing shows that it leaves the trade alone");
        let other_symbol = OpenOrderInfo { symbol: "RUT".into(), ..other_order("BAG", 0, None) };
        assert_eq!(other_client_orders_on(&spread, &[other_symbol]).len(), 0);
    }

    #[test]
    fn orders_of_another_account_or_of_stocks_do_not_touch_the_trade() {
        let spread = put_spread(2);
        let elsewhere = OpenOrderInfo { account: "DU2".into(), ..other_order("OPT", 1, None) };
        let stock = other_order("STK", 1, None);
        assert_eq!(other_client_orders_on(&spread, &[elsewhere, stock]).len(), 0);
    }

    fn chosen(quantities: &[(usize, i64)]) -> Vec<UnmanagedPosition> {
        quantities.iter().map(|(position, quantity)| UnmanagedPosition { position: *position, quantity: Decimal::from(*quantity) }).collect()
    }

    /// A short put 6650 at an average cost of 120 (1.20 a contract) and a long put 6640 at 50, two contracts each.
    fn put_spread_positions() -> Vec<Position> {
        let short = Position { strike: 6650.0, average_cost: Some(120.0), ..option("DU1", 1, -2.0) };
        let long = Position { strike: 6640.0, average_cost: Some(50.0), ..option("DU1", 2, 2.0) };
        vec![short, long]
    }

    #[test]
    fn positions_taken_over_become_a_trade_with_the_average_cost_as_entry_price() {
        let trade = import_trade(&put_spread_positions(), &chosen(&[(0, -2), (1, 2)]), -1, "20261127 10:15:00").unwrap();
        assert_eq!((trade.trade_type.as_str(), trade.strikes.as_str(), trade.qty, trade.multiplier), ("PutSpread", "6650/6640 P", 2, 100));
        assert_eq!(trade.price_open, Decimal::new(7, 1), "1.20 received for the short, 0.50 paid for the long");
        assert_eq!((trade.order_id, trade.status.as_str(), trade.entry_unknown), (-1, "Open", false));
        assert_eq!((trade.day.as_str(), trade.opened.as_str(), trade.expiry.as_str()), ("2026-11-27", "27-11-2026 10:15:00", "20261218"));
        assert_eq!(trade.short_put.as_ref().map(|l| l.con_id), Some(1));
        assert_eq!(trade.long_put.as_ref().map(|l| l.con_id), Some(2));
    }

    #[test]
    fn the_multiplier_of_the_positions_is_the_trades() {
        // As TWS reported a DAX option: an average cost of 685.70 at a multiplier of 5 is 137.14 a contract.
        let dax = Position { symbol: "DAX".into(), strike: 24100.0, multiplier: Some(5.0), average_cost: Some(685.7), ..option("DU1", 7, 1.0) };
        let trade = import_trade(&[dax], &chosen(&[(0, 1)]), -2, "20261127 10:15:00").unwrap();
        assert_eq!((trade.trade_type.as_str(), trade.multiplier, trade.price_open), ("LongPut", 5, Decimal::new(-13714, 2)));
    }

    #[test]
    fn only_the_contracts_without_a_trade_are_taken_over() {
        let trade = import_trade(&put_spread_positions(), &chosen(&[(0, -1), (1, 1)]), -1, "20261127 10:15:00").unwrap();
        assert_eq!(trade.qty, 1);
    }

    #[test]
    fn positions_that_do_not_make_one_trade_are_refused() {
        let positions = put_spread_positions();
        assert_eq!(import_trade(&positions, &[], -1, "20261127 10:15:00"), Err(ImportRefusal::NoLegs));
        let mut other = positions.clone();
        other[1].account = "DU2".into();
        assert_eq!(import_trade(&other, &chosen(&[(0, -2), (1, 2)]), -1, "20261127 10:15:00"), Err(ImportRefusal::Accounts));
        let mut other = positions.clone();
        other[1].symbol = "RUT".into();
        assert_eq!(import_trade(&other, &chosen(&[(0, -2), (1, 2)]), -1, "20261127 10:15:00"), Err(ImportRefusal::Symbols));
        let mut other = positions.clone();
        other[1].multiplier = Some(10.0);
        assert_eq!(import_trade(&other, &chosen(&[(0, -2), (1, 2)]), -1, "20261127 10:15:00"), Err(ImportRefusal::Multiplier));
        let mut other = positions.clone();
        other[0].multiplier = None;
        assert_eq!(import_trade(&other, &chosen(&[(0, -2)]), -1, "20261127 10:15:00"), Err(ImportRefusal::Multiplier));
        let mut other = positions;
        other[0].average_cost = None;
        assert_eq!(import_trade(&other, &chosen(&[(0, -2), (1, 2)]), -1, "20261127 10:15:00"), Err(ImportRefusal::AverageCost));
    }

    #[test]
    fn taken_over_trades_count_down_from_minus_one_and_are_named_after_their_id() {
        assert_eq!(next_import_id(&[]), Some(-1));
        let ids = |id: i32| StoredTrade { order_id: id, ..put_spread(1) };
        assert_eq!(next_import_id(&[ids(12), ids(-1), ids(-3)]), Some(-4));
        assert_eq!(crate::trade_rebuild::import_ref(-3).as_deref(), Some("TraPi-Imp-3"));
        assert_eq!(crate::trade_rebuild::trade_ref(-3), "TraPi-Imp-3");
        assert_eq!(crate::trade_rebuild::import_ref(12), None);
        assert_eq!(crate::trade_rebuild::trade_ref(12), "TraPi-12");
    }
}
