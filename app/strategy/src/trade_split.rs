//! The full stop split: an `IronFly`, `IronCondor-Full` or `ShortStrangle` whose stop buys back both shorts as one order is split into
//! one trade per side once one short is nearly worthless. Each side then has a stop and a profit target of its own, as the sides of
//! an `IronCondor` have from the start.
//!
//! When: the trade is open, its template's stop type is `Short`, its stop order type is not `None`, it has no close order, and one
//! short is quoted with an ask above 0 and a bid at the split price or below (no bid counts as below). The split itself turns the
//! trade into its two sides:
//!
//! - the put side (`PutSpread`, for a strangle `ShortPut`) and the call side (`CallSpread` / `ShortCall`), with the legs of their side;
//! - the entry price of each side from the fills of its legs in the opening order (a split without them is not made);
//! - a ratio above 1 becomes ratio 1: the contracts are multiplied by it, the stop and the target divided by it;
//! - each side keeps the stop and the target price of the trade, the stop on its short, the target on all its legs;
//! - of the stop adjustments only those by an amount are kept, they mean the same price on each side;
//! - profit-target levels hit before stay on record on each side with the same contracts, their closing price shared out in the
//!   proportion of the entry prices;
//! - the exit time, the profit-target cancel time and where the trade came from are kept.
//!
//! The trade itself is kept with the status `Split`: hidden, it counts for nothing, its sides count instead.

use crate::pricing::fix_price;
use crate::records::{ExitLevels, StopAdjustments, StoredExecution, StoredTrade};
use crate::trade_exit::leg_prices;
use crate::trade_summary::TradeStatus;
use rust_decimal::Decimal;

/// The trade types that are split.
pub const SPLIT_TYPES: [&str; 3] = ["IronCondor-Full", "IronFly", "ShortStrangle"];

/// A leg's quote for the split: the bid (`None` while there is none) and the ask (`None` while there is none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShortQuote {
    /// The bid; `None` while there is none.
    pub bid: Option<Decimal>,
    /// The ask; `None` while there is none.
    pub ask: Option<Decimal>,
}

impl ShortQuote {
    /// Nearly worthless: quoted with an ask above 0 and a bid at `price` or below, or none at all.
    #[must_use]
    pub fn at_or_below(self, price: Decimal) -> bool {
        self.ask.is_some_and(|ask| ask > Decimal::ZERO) && self.bid.is_none_or(|bid| bid <= price)
    }
}

/// Whether the trade is one the split applies to at all: a split type, open, no close order, its template's stop type `Short` and a
/// stop order type other than `None` (`stop_wanted`).
#[must_use]
pub fn splittable(trade: &StoredTrade, stop_type: &str, stop_wanted: bool) -> bool {
    SPLIT_TYPES.contains(&trade.trade_type.as_str())
        && trade.is_open()
        && trade.close_order_id.is_none()
        && trade.custom_legs.is_none()
        && stop_type == "Short"
        && stop_wanted
        && trade.short_put.is_some()
        && trade.short_call.is_some()
}

/// Whether one of the shorts has fallen to the split price: `put` and `call` are the quotes of the short put and the short call.
#[must_use]
pub fn split_due(put: Option<ShortQuote>, call: Option<ShortQuote>, price: Decimal) -> bool {
    [put, call].into_iter().flatten().any(|quote| quote.at_or_below(price))
}

/// One side of a split trade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The puts.
    Put,
    /// The calls.
    Call,
}

impl Side {
    fn trade_type(self, whole: &str) -> &'static str {
        match (self, whole == "ShortStrangle") {
            (Self::Put, false) => "PutSpread",
            (Self::Put, true) => "ShortPut",
            (Self::Call, false) => "CallSpread",
            (Self::Call, true) => "ShortCall",
        }
    }

    fn right(self) -> &'static str {
        match self {
            Self::Put => "P",
            Self::Call => "C",
        }
    }
}

/// The trade `trade` split into its put side and its call side (see the module), or why it cannot be split. `executions` are the
/// executions known, for the fill prices of the legs.
///
/// # Errors
///
/// The reason as text: a type that is not split, a short missing, or the fills of a leg not known.
pub fn split_sides(trade: &StoredTrade, executions: &[StoredExecution]) -> Result<[StoredTrade; 2], String> {
    if !SPLIT_TYPES.contains(&trade.trade_type.as_str()) || trade.short_put.is_none() || trade.short_call.is_none() {
        return Err("only an IronFly, IronCondor-Full or ShortStrangle with both shorts can be split".to_string());
    }
    let put = side_trade(trade, Side::Put);
    let call = side_trade(trade, Side::Call);
    let open_of = |side: &StoredTrade| {
        let prices = leg_prices(side, executions);
        let short = prices.short?;
        short.checked_sub(prices.long.unwrap_or(Decimal::ZERO))
    };
    let (Some(put_open), Some(call_open)) = (open_of(&put), open_of(&call)) else {
        return Err("the fill prices of the legs of the opening order are not known".to_string());
    };
    let total = put_open.checked_add(call_open).filter(|total| *total != Decimal::ZERO);
    Ok([finish_side(trade, put, Side::Put, put_open, total), finish_side(trade, call, Side::Call, call_open, total)])
}

/// The legs and the shape of one side, before its prices are known.
fn side_trade(trade: &StoredTrade, side: Side) -> StoredTrade {
    let mut part = trade.clone();
    match side {
        Side::Put => {
            part.short_call = None;
            part.long_call = None;
            part.call_ratio = 1;
        }
        Side::Call => {
            part.short_put = None;
            part.long_put = None;
            part.put_ratio = 1;
        }
    }
    // The fill prices are wanted per option: the ratio goes into the contracts (`finish_side`).
    part.put_ratio = 1;
    part.call_ratio = 1;
    part.trade_type = side.trade_type(&trade.trade_type).to_string();
    let (short, long) = match side {
        Side::Put => (&part.short_put, &part.long_put),
        Side::Call => (&part.short_call, &part.long_call),
    };
    let strikes: Vec<&str> = [short, long].into_iter().flatten().map(|leg| leg.strike.as_str()).collect();
    part.strikes = format!("{} {}", strikes.join("/"), side.right());
    part
}

/// Divides a price of the whole trade by the side's ratio and puts it on the price grid of `legs` legs.
fn per_side(price: Option<Decimal>, ratio: u32, symbol: &str, legs: usize) -> Option<Decimal> {
    let divided = price?.checked_div(Decimal::from(ratio.max(1)))?;
    fix_price(divided, symbol, legs).map(|p| p.normalize())
}

/// The side `part` with its entry price `open`, its contracts, stop, target, levels and adjustments; `total` is the entry price of
/// both sides together (the share of the closing prices of levels hit before).
fn finish_side(trade: &StoredTrade, mut part: StoredTrade, side: Side, open: Decimal, total: Option<Decimal>) -> StoredTrade {
    let ratio = match side {
        Side::Put => trade.put_ratio.max(1),
        Side::Call => trade.call_ratio.max(1),
    };
    let short = match side {
        Side::Put => part.short_put.as_ref().map(|leg| leg.con_id),
        Side::Call => part.short_call.as_ref().map(|leg| leg.con_id),
    };
    let side_legs = [&part.short_put, &part.long_put, &part.short_call, &part.long_call].iter().filter(|leg| leg.is_some()).count();
    part.put_ratio = 1;
    part.call_ratio = 1;
    part.qty = trade.qty.saturating_mul(ratio);
    part.price_open = open.normalize();
    part.delta = 0.0;
    part.status = TradeStatus::Open.as_str().to_string();
    part.price_close = None;
    part.stop_order_id = None;
    part.stop_limit_order_id = None;
    part.profit_order_id = None;
    part.close_order_id = None;
    part.oca_group = None;
    part.stop_price = per_side(trade.stop_now(), ratio, &trade.symbol, 1);
    let target = trade.exit_levels.as_ref().and_then(|levels| levels.prices.get(usize::from(levels.level)).cloned().flatten()).and_then(|p| p.parse().ok());
    part.profit_price = per_side(target.or(trade.profit_price), ratio, &trade.symbol, side_legs);
    part.exit_levels = trade.exit_levels.as_ref().and_then(|levels| side_levels(levels, ratio, open, total, part.profit_price));
    part.adjustments = trade.adjustments.as_ref().map(|adjustments| side_adjustments(adjustments, short, part.stop_price, &trade.symbol, ratio));
    part
}

/// The levels of a side: those hit before stay on record (contracts times the ratio, the closing price in the share of the side's
/// entry price), the rest is one level at the target. `None` when none was hit.
fn side_levels(levels: &ExitLevels, ratio: u32, open: Decimal, total: Option<Decimal>, target: Option<Decimal>) -> Option<ExitLevels> {
    if levels.closed.is_empty() {
        return None;
    }
    let share = total.and_then(|total| open.checked_div(total)).unwrap_or(Decimal::ONE);
    let closed: Vec<(u32, String)> = levels
        .closed
        .iter()
        .map(|(qty, price)| {
            let price = price.parse::<Decimal>().ok().and_then(|p| p.checked_mul(share)).map(|p| p.round_dp(4).normalize().to_string());
            (qty.saturating_mul(ratio), price.unwrap_or_else(|| "0".to_string()))
        })
        .collect();
    let hit = closed.len().min(3);
    let mut qty = [0_u32; 4];
    for (slot, (contracts, _)) in qty.iter_mut().zip(&closed) {
        *slot = *contracts;
    }
    let mut prices: [Option<String>; 4] = Default::default();
    if let Some(slot) = prices.get_mut(hit) {
        *slot = target.map(|p| p.to_string());
    }
    Some(ExitLevels { qty, level: u8::try_from(hit).unwrap_or(3), order_ids: [None; 4], closed, prices, legs: Vec::new(), outside_rth: levels.outside_rth })
}

/// The stop record of a side: the stop on its short at the side's price, only the adjustments by an amount that are still to come.
fn side_adjustments(adjustments: &StopAdjustments, short: Option<i32>, stop: Option<Decimal>, symbol: &str, ratio: u32) -> StopAdjustments {
    let items = adjustments.items.iter().skip(usize::from(adjustments.level)).filter(|item| item.kind == "Amount").cloned().collect();
    let legs = adjustments.legs.iter().filter(|leg| Some(leg.0) == short).map(|leg| (leg.0, 1, leg.2)).collect();
    StopAdjustments {
        items,
        level: 0,
        stop: stop.map(|p| p.to_string()),
        legs,
        replace_pending: false,
        basis: adjustments.basis.as_deref().and_then(|b| per_side(b.parse().ok(), ratio, symbol, 1)).map(|p| p.to_string()),
        ..adjustments.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{StopAdjustment, StoredLeg};

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn leg(con_id: i32, strike: &str) -> StoredLeg {
        StoredLeg { con_id, strike: strike.to_string() }
    }

    /// An IronCondor-Full, 2 contracts at 3.00, stop 6.00, target 1.50, with its stop on both shorts.
    fn condor() -> StoredTrade {
        StoredTrade {
            day: "2026-11-20".into(),
            opened: "20-11-2026 10:00:00".into(),
            account: "DU1".into(),
            symbol: "SPX".into(),
            trade_type: "IronCondor-Full".into(),
            strikes: "5900/5890 P 6100/6110 C".into(),
            order_id: 40,
            qty: 2,
            multiplier: 100,
            price_open: d("3.00"),
            delta: 1.5,
            short_put: Some(leg(1, "5900")),
            long_put: Some(leg(2, "5890")),
            short_call: Some(leg(3, "6100")),
            long_call: Some(leg(4, "6110")),
            put_ratio: 1,
            call_ratio: 1,
            expiry: "20261120".into(),
            status: "Open".into(),
            price_close: None,
            stop_order_id: Some(41),
            profit_order_id: Some(42),
            stop_limit_order_id: None,
            exit_levels: None,
            adjustments: Some(StopAdjustments {
                order_type: "Market stop".into(),
                stop: Some("6".into()),
                legs: vec![(3, 1, false), (1, 1, false)],
                items: vec![
                    StopAdjustment { kind: "Profit percent".into(), value: Some(50.0), ..StopAdjustment::default() },
                    StopAdjustment { kind: "Amount".into(), value: Some(1.0), ..StopAdjustment::default() },
                ],
                ..StopAdjustments::default()
            }),
            oca_group: Some("TP-41".into()),
            close_order_id: None,
            stop_price: Some(d("6")),
            profit_price: Some(d("1.5")),
            entry_unknown: false,
            strategy: String::new(),
            timed_exit: None,
            profit_cancel: None,
            origin: None,
            custom_legs: None,
        }
    }

    fn fill(con_id: i32, side: &str, price: f64) -> StoredExecution {
        StoredExecution {
            exec_id: format!("e{con_id}"),
            time: String::new(),
            account: "DU1".into(),
            symbol: "SPX".into(),
            con_id,
            order_id: 40,
            order_ref: crate::trade_rebuild::order_ref(40, ""),
            sec_type: "OPT".into(),
            expiry: "20261120".into(),
            right: String::new(),
            strike: 0.0,
            side: side.into(),
            quantity: 2.0,
            price,
        }
    }

    fn fills() -> Vec<StoredExecution> {
        vec![fill(1, "SLD", 2.10), fill(2, "BOT", 0.30), fill(3, "SLD", 1.40), fill(4, "BOT", 0.20)]
    }

    #[test]
    fn a_short_at_the_price_with_an_ask_starts_the_split_no_bid_counts_as_below() {
        let quote = |bid: Option<&str>, ask: Option<&str>| Some(ShortQuote { bid: bid.map(d), ask: ask.map(d) });
        assert!(split_due(quote(Some("0.05"), Some("0.10")), quote(Some("2"), Some("2.1")), d("0.05")));
        assert!(split_due(quote(None, Some("0.05")), None, d("0.05")), "no bid at all");
        assert!(!split_due(quote(Some("0.06"), Some("0.10")), quote(Some("2"), Some("2.1")), d("0.05")));
        assert!(!split_due(quote(Some("0"), None), None, d("0.05")), "without an ask it is not quoted");
        assert!(!split_due(quote(Some("0"), Some("0")), None, d("0.05")), "an ask of 0 is none");
    }

    #[test]
    fn only_an_open_full_trade_with_a_short_stop_and_no_close_order_is_split() {
        let trade = condor();
        assert!(splittable(&trade, "Short", true));
        assert!(!splittable(&trade, "Vertical", true), "the stop type must be Short");
        assert!(!splittable(&trade, "Short", false), "stop order type None");
        assert!(!splittable(&StoredTrade { trade_type: "PutSpread".into(), ..trade.clone() }, "Short", true));
        assert!(!splittable(&StoredTrade { close_order_id: Some(9), ..trade.clone() }, "Short", true));
        assert!(!splittable(&StoredTrade { status: "Stopped".into(), ..trade.clone() }, "Short", true));
        assert!(!splittable(&StoredTrade { short_call: None, ..trade }, "Short", true));
    }

    #[test]
    fn a_condor_becomes_a_put_spread_and_a_call_spread_priced_from_their_fills() {
        let [put, call] = split_sides(&condor(), &fills()).unwrap();
        assert_eq!((put.trade_type.as_str(), put.strikes.as_str()), ("PutSpread", "5900/5890 P"));
        assert_eq!((call.trade_type.as_str(), call.strikes.as_str()), ("CallSpread", "6100/6110 C"));
        assert_eq!((put.price_open, call.price_open), (d("1.8"), d("1.2")), "2.10 - 0.30 and 1.40 - 0.20");
        assert!(put.short_call.is_none() && put.long_call.is_none() && call.short_put.is_none() && call.long_put.is_none());
        for side in [&put, &call] {
            assert_eq!(side.qty, 2);
            assert_eq!((side.stop_price, side.profit_price), (Some(d("6")), Some(d("1.5"))), "the trade's stop and target");
            assert_eq!((side.stop_order_id, side.profit_order_id, side.oca_group.as_deref()), (None, None, None), "placed anew");
            assert!(side.is_open());
            assert_eq!(side.order_id, 40, "the sides keep the opening order");
        }
        let put_stop = put.adjustments.unwrap();
        assert_eq!(put_stop.legs, [(1, 1, false)], "the put side's stop buys back the short put");
        assert_eq!(put_stop.items.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(), ["Amount"], "only the adjustment by an amount stays");
        assert_eq!(call.adjustments.unwrap().legs, [(3, 1, false)]);
    }

    #[test]
    fn a_strangle_becomes_a_short_put_and_a_short_call() {
        let strangle = StoredTrade { trade_type: "ShortStrangle".into(), long_put: None, long_call: None, ..condor() };
        let executions = vec![fill(1, "SLD", 2.0), fill(3, "SLD", 1.0)];
        let [put, call] = split_sides(&strangle, &executions).unwrap();
        assert_eq!((put.trade_type.as_str(), put.strikes.as_str(), put.price_open), ("ShortPut", "5900 P", d("2")));
        assert_eq!((call.trade_type.as_str(), call.strikes.as_str(), call.price_open), ("ShortCall", "6100 C", d("1")));
    }

    #[test]
    fn a_ratio_multiplies_the_contracts_and_divides_the_prices() {
        let ratio = StoredTrade { put_ratio: 2, ..condor() };
        let [put, call] = split_sides(&ratio, &fills()).unwrap();
        assert_eq!((put.qty, put.put_ratio, put.stop_price, put.profit_price), (4, 1, Some(d("3")), Some(d("0.75"))));
        assert_eq!((call.qty, call.stop_price), (2, Some(d("6"))), "the call side has ratio 1");
    }

    #[test]
    fn the_levels_hit_before_stay_on_each_side_with_their_share_of_the_closing_price() {
        let mut trade = condor();
        trade.qty = 4;
        trade.exit_levels = Some(ExitLevels {
            qty: [2, 2, 0, 0],
            level: 1,
            order_ids: [Some(42), Some(43), None, None],
            closed: vec![(2, "-1.5".into())],
            prices: [Some("1.5".into()), Some("1".into()), None, None],
            legs: Vec::new(),
            outside_rth: false,
        });
        let [put, call] = split_sides(&trade, &fills()).unwrap();
        let levels = put.exit_levels.clone().unwrap();
        assert_eq!(levels.closed, [(2, "-0.9".to_string())], "1.8 of 3.0 = 60 % of -1.5");
        assert_eq!((levels.level, levels.order_ids), (1, [None; 4]));
        assert_eq!(put.remaining_qty(), 2);
        assert_eq!(put.profit_price, Some(d("1")), "the target of the level that was next");
        assert_eq!(call.exit_levels.unwrap().closed, [(2, "-0.6".to_string())]);
    }

    #[test]
    fn without_the_fills_of_the_legs_or_of_another_type_there_is_no_split() {
        assert!(split_sides(&condor(), &fills()[..3]).is_err(), "the long call's fill is missing");
        assert!(split_sides(&StoredTrade { trade_type: "IronCondor".into(), ..condor() }, &fills()).is_err());
    }
}
