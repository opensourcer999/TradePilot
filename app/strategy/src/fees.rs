//! The commissions TWS charged for the executions of a trade: its opening fills and the fills of its stop, profit
//! target and close orders. Plain logic, no `App`.

use crate::records::{StoredExecution, StoredTrade};
use rust_decimal::Decimal;
use std::collections::HashMap;

/// What a trade cost in fees, in dollars. `None` where TWS has not reported the commission of every fill that counts
/// (or there is no fill of that kind), so a missing figure is never taken for 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TradeFees {
    /// The fills of the opening order that belong to the trade's legs.
    pub open: Option<Decimal>,
    /// The fills of the exit and close orders that belong to the trade's legs.
    pub close: Option<Decimal>,
}

impl TradeFees {
    /// Opening and closing fees together; `None` when neither is known.
    #[must_use]
    pub fn total(&self) -> Option<Decimal> {
        match (self.open, self.close) {
            (None, None) => None,
            (open, close) => open.unwrap_or(Decimal::ZERO).checked_add(close.unwrap_or(Decimal::ZERO)),
        }
    }
}

/// The sum of the commissions of `fills`; `None` without a fill or when one has no commission yet.
fn commission_of<S: std::hash::BuildHasher>(fills: &[&StoredExecution], commissions: &HashMap<String, Decimal, S>) -> Option<Decimal> {
    if fills.is_empty() {
        return None;
    }
    fills.iter().try_fold(Decimal::ZERO, |sum, fill| sum.checked_add(*commissions.get(&fill.exec_id)?))
}

/// The fees of `trade`. A put side and a call side entered with one order share its executions: each trade counts the
/// fills of its own legs only.
#[must_use]
pub fn trade_fees<S: std::hash::BuildHasher>(trade: &StoredTrade, executions: &[StoredExecution], commissions: &HashMap<String, Decimal, S>) -> TradeFees {
    let legs: Vec<i32> = [&trade.short_put, &trade.long_put, &trade.short_call, &trade.long_call].into_iter().flatten().map(|leg| leg.con_id).collect();
    let mut exit_orders = trade.exit_order_ids();
    exit_orders.extend(trade.close_order_id);
    let of_legs = |wanted: &dyn Fn(i32) -> bool| -> Vec<&StoredExecution> {
        executions.iter().filter(|fill| wanted(fill.order_id) && legs.contains(&fill.con_id)).collect()
    };
    TradeFees {
        open: commission_of(&of_legs(&|order| order == trade.order_id), commissions),
        close: commission_of(&of_legs(&|order| exit_orders.contains(&order)), commissions),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn fill(exec_id: &str, order_id: i32, con_id: i32) -> StoredExecution {
        StoredExecution {
            exec_id: exec_id.to_string(),
            time: "20261116 10:00:00 America/New_York".to_string(),
            account: "DU1".to_string(),
            symbol: "SPX".to_string(),
            con_id,
            order_id,
            order_ref: String::new(),
            sec_type: "OPT".to_string(),
            expiry: "20261116".to_string(),
            right: "P".to_string(),
            strike: 7300.0,
            side: "BOT".to_string(),
            quantity: 1.0,
            price: 1.0,
        }
    }

    fn trade() -> StoredTrade {
        let mut trade: StoredTrade = serde_json::from_str(
            r#"{"day":"2026-11-16","opened":"16-11-2026 10:00:00","account":"DU1","symbol":"SPX","trade_type":"PutSpread","strikes":"7300/7290 P",
                "order_id":10,"qty":1,"multiplier":100,"price_open":"1.5","delta":0.0,
                "short_put":{"con_id":1,"strike":"7300"},"long_put":{"con_id":2,"strike":"7290"}}"#,
        )
        .unwrap();
        trade.stop_order_id = Some(11);
        trade
    }

    #[test]
    fn the_fees_of_the_opening_and_the_exit_fills_are_added_per_trade() {
        let fills = [fill("a", 10, 1), fill("b", 10, 2), fill("c", 11, 1), fill("d", 11, 2), fill("other", 10, 99)];
        let commissions: HashMap<String, Decimal> =
            [("a", "0.65"), ("b", "0.65"), ("c", "0.70"), ("d", "0.70"), ("other", "5")].map(|(id, c)| (id.to_string(), d(c))).into();
        let fees = trade_fees(&trade(), &fills, &commissions);
        assert_eq!((fees.open, fees.close), (Some(d("1.30")), Some(d("1.40"))), "the fill of a leg this trade does not hold is not counted");
        assert_eq!(fees.total(), Some(d("2.70")));
    }

    #[test]
    fn a_fill_without_a_commission_yet_makes_the_figure_unknown_not_zero() {
        let fills = [fill("a", 10, 1), fill("b", 10, 2)];
        let commissions: HashMap<String, Decimal> = [("a".to_string(), d("0.65"))].into();
        let fees = trade_fees(&trade(), &fills, &commissions);
        assert_eq!((fees.open, fees.close, fees.total()), (None, None, None));
        assert_eq!(trade_fees(&trade(), &[], &HashMap::new()), TradeFees::default());
    }
}
