//! Trades rebuilt from the executions TWS reported. TWS keeps only the current trading day, so
//! the app stores every execution it sees (`storage::StoredExecution`); from that history this
//! module finds the trades again:
//!
//! - The opening orders this app sends carry the order reference "`TradePilot` `OrderID`: N
//!   (Opening)". The executions of one such order are its legs and fill prices: they give the
//!   strikes, the quantity and the entry price.
//! - A single option bought or sold with an Instant Order has no reference: its executions (an order id, no reference)
//!   are found again as a `ShortPut`, `ShortCall`, `LongPut` or `LongCall` trade, unless the order only buys back an
//!   option that a known open trade is short in.
//! - When an option expires worthless TWS books it as an execution at price 0 (no order, no
//!   reference): BOT closes a short, SLD a long. Such bookings close a rebuilt trade; bookings
//!   that belong to no known trade become a trade with an unknown entry price.
//!
//! Pure logic on `Decimal` (`coding_guideline.md` sections 2/3), no I/O and no clock.

use crate::pricing::dec;
use crate::records::{StoredExecution, StoredLeg, StoredTrade};
use crate::trade_type::TradeType;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// What a rebuild found: trades to add, and known open trades that have expired since.
#[derive(Debug, Default, PartialEq)]
pub struct Rebuilt {
    /// Trades the executions show that were not stored yet.
    pub new: Vec<StoredTrade>,
    /// Known open trades that expired in the meantime.
    pub updated: Vec<StoredTrade>,
    /// Known open trades whose opening order was filled for more contracts since they were recorded (see `grown_trades`).
    pub grown: Vec<StoredTrade>,
}

/// The reference of an order of this app: `TraPi-12` for an opening order, `TraPi-13 (SL)` for a stop, `TraPi-14 (TP)` for a profit target
/// (`kind` is "SL", "TP" or "Close"; empty for an opening order). TWS shows it in its Order Ref column.
#[must_use]
pub fn order_ref(id: i32, kind: &str) -> String {
    if kind.is_empty() {
        format!("TraPi-{id}")
    } else {
        format!("TraPi-{id} ({kind})")
    }
}

/// The order id in an opening reference: `TraPi-12`, or the shorter `TP-12` and the longer `TradePilot OrderID: 12 (Opening)` of earlier versions.
#[must_use]
pub fn opening_order_id(order_ref: &str) -> Option<i32> {
    if let Some(id) = order_ref.strip_prefix("TraPi-").or_else(|| order_ref.strip_prefix("TP-")) {
        return id.parse().ok();
    }
    let rest = order_ref.strip_prefix("TradePilot OrderID: ")?;
    let (id, tail) = rest.split_once(' ')?;
    (tail == "(Opening)" || tail.starts_with("(Opening) - ")).then(|| id.parse().ok()).flatten()
}

/// One option of a rebuilt trade.
#[derive(Clone, Debug)]
struct Leg {
    con_id: i32,
    strike: Decimal,
    put: bool,
    sold: bool,
    qty: Decimal,
    /// The average fill price per contract.
    price: Decimal,
    /// Expiration yyyyMMdd.
    expiry: String,
}

fn fmt(value: Decimal) -> String {
    value.normalize().to_string()
}

/// "yyyyMMdd HH:mm:ss Zone" as "yyyy-MM-dd" and "dd-MM-yyyy HH:mm:ss".
fn time_parts(time: &str) -> Option<(String, String)> {
    let mut parts = time.split_whitespace();
    let date = parts.next()?;
    let clock = parts.next()?;
    let (y, m, d) = (date.get(0..4)?, date.get(4..6)?, date.get(6..8)?);
    Some((format!("{y}-{m}-{d}"), format!("{d}-{m}-{y} {clock}")))
}

/// The legs of one order: fills of the same option and side are merged (quantity added, average
/// price weighted by quantity).
fn merge_legs(execs: &[&StoredExecution]) -> Option<Vec<Leg>> {
    let mut legs: Vec<Leg> = Vec::new();
    for e in execs {
        let (qty, price, strike) = (dec(e.quantity)?, dec(e.price)?, dec(e.strike)?);
        let sold = e.side == "SLD";
        match legs.iter_mut().find(|l| l.con_id == e.con_id && l.sold == sold && l.strike == strike) {
            Some(leg) => {
                let total = leg.qty.checked_add(qty)?;
                leg.price = leg.price.checked_mul(leg.qty)?.checked_add(price.checked_mul(qty)?)?.checked_div(total)?;
                leg.qty = total;
            }
            None => legs.push(Leg { con_id: e.con_id, strike, put: e.right == "P", sold, qty, price, expiry: e.expiry.clone() }),
        }
    }
    (!legs.is_empty()).then_some(legs)
}

/// Whether a sold and a bought leg of one side form a calendar: the same expiry is a vertical
/// (`Some(false)`), the sold leg expiring first a calendar (`Some(true)`); a sold leg that
/// expires later is a trade type this app does not enter (`None`).
fn is_calendar(sold: &Leg, bought: &Leg) -> Option<bool> {
    match sold.expiry.cmp(&bought.expiry) {
        std::cmp::Ordering::Equal => Some(false),
        std::cmp::Ordering::Less => Some(true),
        std::cmp::Ordering::Greater => None,
    }
}

/// The trade type of a set of legs (`None` for a set this app does not enter).
fn trade_type(legs: &[Leg]) -> Option<TradeType> {
    let find = |put: bool, sold: bool| legs.iter().find(|l| l.put == put && l.sold == sold);
    let (sp, lp, sc, lc) = (find(true, true), find(true, false), find(false, true), find(false, false));
    Some(match (sp, lp, sc, lc) {
        (Some(sp), Some(lp), Some(sc), Some(lc)) => match (is_calendar(sp, lp)?, is_calendar(sc, lc)?) {
            (true, true) => TradeType::DoubleCalendar,
            (false, false) => TradeType::IronCondorFull,
            _ => return None,
        },
        (Some(s), Some(l), None, None) => {
            if is_calendar(s, l)? {
                TradeType::PutCalendar
            } else if s.strike > l.strike {
                TradeType::PutSpread
            } else {
                TradeType::PutDebitSpread
            }
        }
        (None, None, Some(s), Some(l)) => {
            if is_calendar(s, l)? {
                TradeType::CallCalendar
            } else if s.strike < l.strike {
                TradeType::CallSpread
            } else {
                TradeType::CallDebitSpread
            }
        }
        (Some(_), None, Some(_), None) => TradeType::ShortStrangle,
        (None, Some(_), None, Some(_)) => TradeType::LongStrangle,
        (Some(_), None, None, None) => TradeType::ShortPut,
        (None, None, Some(_), None) => TradeType::ShortCall,
        (None, Some(_), None, None) => TradeType::LongPut,
        (None, None, None, Some(_)) => TradeType::LongCall,
        _ => return None,
    })
}

/// "6650/6640 P" per side (short first, then long), the put side before the call side.
fn strikes_text(legs: &[Leg]) -> String {
    let side = |put: bool| {
        let parts: Vec<String> = [true, false].iter().filter_map(|sold| legs.iter().find(|l| l.put == put && l.sold == *sold)).map(|l| fmt(l.strike)).collect();
        (!parts.is_empty()).then(|| format!("{} {}", parts.join("/"), if put { "P" } else { "C" }))
    };
    [side(true), side(false)].into_iter().flatten().collect::<Vec<_>>().join("  ")
}

fn stored_leg(leg: Option<&Leg>) -> Option<StoredLeg> {
    leg.map(|l| StoredLeg { con_id: l.con_id, strike: fmt(l.strike) })
}

/// The legs of a custom trade: every leg the order filled, with its ratio to the trade's quantity (`unit`) and the sign of a sold one.
fn custom_legs_of(legs: &[Leg], unit: Decimal) -> Option<Vec<crate::records::CustomLegStored>> {
    legs.iter()
        .map(|l| {
            let ratio = i32::try_from(l.qty.checked_div(unit)?.round().to_i64()?).ok()?.max(1);
            Some(crate::records::CustomLegStored {
                con_id: l.con_id,
                right: if l.put { "P" } else { "C" }.to_string(),
                strike: fmt(l.strike),
                expiry: l.expiry.clone(),
                ratio: if l.sold { -ratio } else { ratio },
            })
        })
        .collect()
}

/// A custom trade as the grid shows it: `-1 6690P +1 6680P`.
fn custom_strikes_text(legs: &[crate::records::CustomLegStored]) -> String {
    legs.iter().map(|l| format!("{:+} {}{}", l.ratio, l.strike, l.right)).collect::<Vec<_>>().join(" ")
}

/// One opening order as a trade (status "Open"); `None` for a price that cannot be computed. A leg combination without a standard
/// type is a custom trade when the order is one of this app (`allow_custom`), else nothing: an order without the reference of the app
/// (one that closes a calendar by hand, say) must not turn into a trade.
fn opening_trade(order_id: i32, execs: &[&StoredExecution], allow_custom: bool) -> Option<StoredTrade> {
    let first = execs.first()?;
    let legs = merge_legs(execs)?;
    // A set of legs this app has no standard type for is a custom trade; one that fits a standard type is rebuilt as that type.
    let kind = trade_type(&legs);
    if kind.is_none() && !allow_custom {
        return None;
    }
    // The trade's quantity is its smallest leg; a bigger leg is a ratio.
    let unit = legs.iter().map(|l| l.qty).min()?;
    let qty = u32::try_from(unit.to_i64()?).ok().filter(|q| *q > 0)?;
    let ratio = |put: bool| -> Option<u32> {
        let biggest = legs.iter().filter(|l| l.put == put).map(|l| l.qty).max().unwrap_or(unit);
        u32::try_from(biggest.checked_div(unit)?.round().to_i64()?).ok().map(|r| r.max(1))
    };
    let mut credit = Decimal::ZERO;
    for l in &legs {
        let value = l.price.checked_mul(l.qty)?;
        credit = if l.sold { credit.checked_add(value)? } else { credit.checked_sub(value)? };
    }
    let price_open = credit.checked_div(unit)?.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    let custom_legs = if kind.is_none() { Some(custom_legs_of(&legs, unit)?) } else { None };
    let (day, opened) = time_parts(&first.time)?;
    let pick = |put: bool, sold: bool| stored_leg(legs.iter().find(|l| l.put == put && l.sold == sold));
    Some(StoredTrade {
        day,
        opened,
        account: first.account.clone(),
        symbol: first.symbol.clone(),
        trade_type: kind.map_or_else(|| "Custom".to_string(), |kind| kind.to_string()),
        strikes: custom_legs.as_ref().map_or_else(|| strikes_text(&legs), |custom| custom_strikes_text(custom)),
        order_id,
        qty,
        multiplier: 100,
        price_open: price_open.normalize(),
        delta: 0.0,
        short_put: custom_legs.is_none().then(|| pick(true, true)).flatten(),
        long_put: custom_legs.is_none().then(|| pick(true, false)).flatten(),
        short_call: custom_legs.is_none().then(|| pick(false, true)).flatten(),
        long_call: custom_legs.is_none().then(|| pick(false, false)).flatten(),
        put_ratio: if custom_legs.is_some() { 1 } else { ratio(true)? },
        call_ratio: if custom_legs.is_some() { 1 } else { ratio(false)? },
        // A calendar's trade expiry is its near (sold) leg's.
        expiry: legs.iter().filter(|l| l.sold).map(|l| l.expiry.clone()).min().unwrap_or_else(|| first.expiry.clone()),
        status: crate::trade_summary::TradeStatus::Open.as_str().to_string(),
        price_close: None,
        stop_order_id: None,
        profit_order_id: None,
        stop_limit_order_id: None,
        exit_levels: None,
        adjustments: None,
        oca_group: None,
        close_order_id: None,
        stop_price: None,
        profit_price: None,
        entry_unknown: false,
        strategy: String::new(),
        timed_exit: None,
        profit_cancel: None,
        origin: None,
        custom_legs,
    })
}

/// An option that expired worthless: TWS books it at price 0 without an order.
fn is_expiry_booking(e: &StoredExecution) -> bool {
    e.sec_type == "OPT" && e.price == 0.0 && e.order_ref.is_empty() && e.order_id == 0
}

fn booked(trade_leg: Option<&StoredLeg>, bookings: &[&StoredExecution]) -> bool {
    match trade_leg {
        None => true,
        Some(leg) => bookings.iter().any(|b| b.con_id == leg.con_id),
    }
}

/// Marks an open trade as expired when every one of its options was booked as expired.
fn expire_if_booked(trade: &mut StoredTrade, bookings: &[&StoredExecution]) -> bool {
    if let Some(custom) = trade.custom_legs.clone().filter(|_| trade.is_open()) {
        if !custom.is_empty() && custom.iter().all(|leg| bookings.iter().any(|b| b.con_id == leg.con_id)) {
            trade.status = crate::trade_summary::TradeStatus::Expired.as_str().to_string();
            trade.price_close = Some(Decimal::ZERO);
            return true;
        }
        return false;
    }
    let has_legs = trade.short_put.is_some() || trade.long_put.is_some() || trade.short_call.is_some() || trade.long_call.is_some();
    let all = booked(trade.short_put.as_ref(), bookings)
        && booked(trade.long_put.as_ref(), bookings)
        && booked(trade.short_call.as_ref(), bookings)
        && booked(trade.long_call.as_ref(), bookings);
    if trade.is_open() && has_legs && all {
        trade.status = crate::trade_summary::TradeStatus::Expired.as_str().to_string();
        trade.price_close = Some(Decimal::ZERO);
        return true;
    }
    false
}

/// Trades for expiry bookings that belong to no known trade: shorts (BOT) are paired with the
/// nearest long (SLD) of the same right on the same expiry; the entry price stays unknown.
fn orphan_trades(bookings: &[&StoredExecution], known: &[StoredTrade]) -> Vec<StoredTrade> {
    let in_known = |con_id: i32| {
        known.iter().any(|t| {
            [&t.short_put, &t.long_put, &t.short_call, &t.long_call].into_iter().flatten().any(|l| l.con_id == con_id)
                || t.custom_legs.iter().flatten().any(|l| l.con_id == con_id)
        })
    };
    // (symbol, expiry, right) -> the unmatched bookings
    let mut groups: BTreeMap<(String, String, String), Vec<&StoredExecution>> = BTreeMap::new();
    for b in bookings.iter().filter(|b| !in_known(b.con_id)) {
        groups.entry((b.symbol.clone(), b.expiry.clone(), b.right.clone())).or_default().push(b);
    }
    let mut trades = Vec::new();
    for ((symbol, expiry, right), group) in groups {
        let put = right == "P";
        let mut shorts: Vec<&StoredExecution> = group.iter().copied().filter(|b| b.side == "BOT").collect();
        let mut longs: Vec<&StoredExecution> = group.iter().copied().filter(|b| b.side == "SLD").collect();
        // The spread's long is on the far side of the short: below it for puts, above for calls.
        let far = |s: &StoredExecution, l: &StoredExecution| if put { l.strike < s.strike } else { l.strike > s.strike };
        let mut pairs: Vec<(&StoredExecution, Option<&StoredExecution>)> = Vec::new();
        shorts.sort_by(|a, b| a.strike.total_cmp(&b.strike));
        for s in shorts.drain(..) {
            let nearest = longs
                .iter()
                .enumerate()
                .filter(|(_, l)| far(s, l))
                .min_by(|(_, a), (_, b)| (a.strike - s.strike).abs().total_cmp(&(b.strike - s.strike).abs()))
                .map(|(i, _)| i);
            pairs.push((s, nearest.map(|i| longs.remove(i))));
        }
        for (s, l) in pairs {
            let legs: Vec<&StoredExecution> = std::iter::once(s).chain(l).collect();
            if let Some(trade) = orphan_from(&symbol, &expiry, put, &legs) {
                trades.push(trade);
            }
        }
        for l in longs {
            if let Some(trade) = orphan_from(&symbol, &expiry, put, &[l]) {
                trades.push(trade);
            }
        }
    }
    trades
}

/// One orphan trade from the bookings of its legs (a short BOT, a long SLD).
fn orphan_from(symbol: &str, expiry: &str, put: bool, legs: &[&StoredExecution]) -> Option<StoredTrade> {
    let first = legs.first()?;
    let short = legs.iter().find(|b| b.side == "BOT");
    let long = legs.iter().find(|b| b.side == "SLD");
    let leg = |e: Option<&&StoredExecution>| e.and_then(|e| Some(StoredLeg { con_id: e.con_id, strike: fmt(dec(e.strike)?) }));
    let (short_leg, long_leg) = (leg(short), leg(long));
    let qty = u32::try_from(dec(first.quantity)?.round().to_i64()?).ok().filter(|q| *q > 0)?;
    let kind = match (short.is_some(), long.is_some(), put) {
        (true, true, true) => TradeType::PutSpread,
        (true, true, false) => TradeType::CallSpread,
        (true, false, true) => TradeType::ShortPut,
        (true, false, false) => TradeType::ShortCall,
        (false, true, true) => TradeType::LongPut,
        _ => TradeType::LongCall,
    };
    let parts: Vec<String> = [short, long].into_iter().flatten().filter_map(|e| dec(e.strike)).map(fmt).collect();
    let (day, _) = time_parts(&format!("{expiry} 00:00:00"))?;
    let (sp, lp, sc, lc) = if put { (short_leg.clone(), long_leg.clone(), None, None) } else { (None, None, short_leg, long_leg) };
    Some(StoredTrade {
        day,
        opened: "unknown".to_string(),
        account: first.account.clone(),
        symbol: symbol.to_string(),
        trade_type: kind.to_string(),
        strikes: format!("{} {}", parts.join("/"), if put { "P" } else { "C" }),
        order_id: 0,
        qty,
        multiplier: 100,
        price_open: Decimal::ZERO,
        delta: 0.0,
        short_put: sp,
        long_put: lp,
        short_call: sc,
        long_call: lc,
        put_ratio: 1,
        call_ratio: 1,
        expiry: expiry.to_string(),
        status: "Expired".to_string(),
        price_close: Some(Decimal::ZERO),
        stop_order_id: None,
        profit_order_id: None,
        stop_limit_order_id: None,
        exit_levels: None,
        adjustments: None,
        oca_group: None,
        close_order_id: None,
        stop_price: None,
        profit_price: None,
        entry_unknown: true,
        strategy: String::new(),
        timed_exit: None,
        profit_cancel: None,
        origin: None,
        custom_legs: None,
    })
}

/// Whether an order is only a buy-back of options that a known, still open trade is short in: such an order closes
/// that trade, it does not open a new one.
fn closes_a_known_short(group: &[&StoredExecution], known: &[StoredTrade]) -> bool {
    let short_in_known = |con_id: i32| {
        known.iter().filter(|t| t.is_open()).any(|t| {
            [&t.short_put, &t.short_call].into_iter().flatten().any(|leg| leg.con_id == con_id && con_id != 0)
                || t.custom_legs.iter().flatten().any(|leg| leg.ratio < 0 && leg.con_id == con_id && con_id != 0)
        })
    };
    !group.is_empty() && group.iter().all(|e| e.side == "BOT" && short_in_known(e.con_id))
}

/// The known open trades of this app whose opening order has been filled for more contracts than the trade says: fills that came in after
/// the trade was recovered from the executions (an order that was only partly filled when the app connected). Each is the stored trade with
/// the quantity and the entry price of all the fills. Trades are matched by their order and their legs; nothing new is made here.
#[must_use]
pub fn grown_trades(execs: &[StoredExecution], known: &[StoredTrade]) -> Vec<StoredTrade> {
    let mut groups: BTreeMap<i32, Vec<&StoredExecution>> = BTreeMap::new();
    for e in execs.iter().filter(|e| e.sec_type == "OPT") {
        if let Some(id) = opening_order_id(&e.order_ref) {
            groups.entry(id).or_default().push(e);
        }
    }
    known
        .iter()
        .filter(|trade| trade.is_open() && !trade.entry_unknown)
        .filter_map(|trade| {
            let built = opening_trade(trade.order_id, groups.get(&trade.order_id)?, true)?;
            let same = built.trade_type == trade.trade_type && built.strikes == trade.strikes && built.expiry == trade.expiry;
            (same && built.qty > trade.qty).then(|| StoredTrade { qty: built.qty, price_open: built.price_open, ..trade.clone() })
        })
        .collect()
}

/// The trades the stored executions show that `known` (every stored trade) does not have yet,
/// and the known open trades that expired in the meantime.
#[must_use]
pub fn rebuild(execs: &[StoredExecution], known: &[StoredTrade]) -> Rebuilt {
    let mut groups: BTreeMap<i32, Vec<&StoredExecution>> = BTreeMap::new();
    for e in execs.iter().filter(|e| e.sec_type == "OPT") {
        if let Some(id) = opening_order_id(&e.order_ref) {
            groups.entry(id).or_default().push(e);
        }
    }
    let bookings: Vec<&StoredExecution> = execs.iter().filter(|e| is_expiry_booking(e)).collect();
    // Instant Orders of a single option carry no reference of this app: their executions are told apart by an order
    // id (an expiry booking has none) and found again like an opening order.
    let mut instant: BTreeMap<i32, Vec<&StoredExecution>> = BTreeMap::new();
    for e in execs.iter().filter(|e| e.sec_type == "OPT" && e.order_ref.trim().is_empty() && e.order_id > 0) {
        instant.entry(e.order_id).or_default().push(e);
    }

    let mut new: Vec<StoredTrade> = groups
        .iter()
        .filter(|(id, _)| !known.iter().any(|t| t.order_id == **id && !t.entry_unknown))
        .filter_map(|(id, group)| opening_trade(*id, group, true))
        .collect();
    new.extend(
        instant
            .iter()
            .filter(|(id, group)| !known.iter().any(|t| t.order_id == **id && !t.entry_unknown) && !closes_a_known_short(group, known))
            .filter_map(|(id, group)| opening_trade(*id, group, false)),
    );
    let mut updated = Vec::new();
    for trade in known.iter().filter(|t| !t.entry_unknown) {
        let mut copy = trade.clone();
        if expire_if_booked(&mut copy, &bookings) {
            updated.push(copy);
        }
    }
    for trade in &mut new {
        expire_if_booked(trade, &bookings);
    }
    let all: Vec<StoredTrade> = known.iter().cloned().chain(new.iter().cloned()).collect();
    for orphan in orphan_trades(&bookings, &all) {
        let duplicate = known.iter().any(|t| t.entry_unknown && t.expiry == orphan.expiry && t.strikes == orphan.strikes && t.trade_type == orphan.trade_type);
        if !duplicate {
            new.push(orphan);
        }
    }
    let grown = grown_trades(execs, known);
    Rebuilt { new, updated, grown }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn exec(id: &str, time: &str, order: i32, reference: &str, con: i32, right: &str, strike: f64, side: &str, qty: f64, price: f64) -> StoredExecution {
        StoredExecution {
            exec_id: id.into(),
            time: time.into(),
            account: "DU1".into(),
            symbol: "SPX".into(),
            con_id: con,
            order_id: order,
            order_ref: reference.into(),
            sec_type: "OPT".into(),
            expiry: "20261113".into(),
            right: right.into(),
            strike,
            side: side.into(),
            quantity: qty,
            price,
        }
    }

    const OPEN: &str = "TradePilot OrderID: 7 (Opening)";
    const T: &str = "20261113 21:30:05 Europe/Berlin";

    fn put_spread_entry() -> Vec<StoredExecution> {
        vec![exec("a", T, 7, OPEN, 11, "P", 7535.0, "SLD", 1.0, 1.4), exec("b", T, 7, OPEN, 12, "P", 7530.0, "BOT", 1.0, 0.65)]
    }

    #[test]
    fn an_order_of_legs_without_a_standard_type_is_rebuilt_as_a_custom_trade_and_expires_with_its_bookings() {
        let reference = "TradePilot OrderID: 9 (Opening) - 2.CUSTOM IC";
        // Sell the 6690 put, buy two 6680 puts, sell the 6720 call: no standard type.
        let execs = [
            exec("a", T, 9, reference, 21, "P", 6690.0, "SLD", 1.0, 2.0),
            exec("b", T, 9, reference, 22, "P", 6680.0, "BOT", 2.0, 0.8),
            exec("c", T, 9, reference, 23, "C", 6720.0, "SLD", 1.0, 1.3),
        ];
        let rebuilt = rebuild(&execs, &[]);
        let [trade] = rebuilt.new.as_slice() else { panic!("one trade expected: {rebuilt:?}") };
        assert_eq!((trade.trade_type.as_str(), trade.qty, trade.order_id), ("Custom", 1, 9));
        assert_eq!(trade.strikes, "-1 6690P +2 6680P -1 6720C");
        // 2.0 + 1.3 - 2 * 0.8 = 1.7 per contract of the trade.
        assert_eq!(trade.price_open, Decimal::new(17, 1));
        assert_eq!(
            trade.custom_legs.as_ref().map(|legs| legs.iter().map(|l| (l.con_id, l.right.as_str(), l.ratio)).collect::<Vec<_>>()),
            Some(vec![(21, "P", -1), (22, "P", 2), (23, "C", -1)])
        );
        assert!(trade.short_put.is_none() && trade.long_put.is_none() && trade.short_call.is_none() && trade.long_call.is_none());
        assert!(trade.is_open());
        // The legs expire worthless: TWS books each of them at 0, and the trade is closed at 0.
        let mut all = execs.to_vec();
        for (id, con, right, strike, side) in [("x", 21, "P", 6690.0, "BOT"), ("y", 22, "P", 6680.0, "SLD"), ("z", 23, "C", 6720.0, "BOT")] {
            all.push(exec(id, T, 0, "", con, right, strike, side, 1.0, 0.0));
        }
        let expired = rebuild(&all, &[]);
        assert_eq!(expired.new.len(), 1, "the bookings of its legs are no extra trades: {:?}", expired.new);
        assert_eq!((expired.new[0].status.as_str(), expired.new[0].price_close), ("Expired", Some(Decimal::ZERO)));
        // A known open custom trade is closed by the bookings as well; one booking short of all legs leaves it open.
        let known = vec![trade.clone()];
        let with_known = rebuild(&all, &known);
        assert_eq!((with_known.updated.len(), with_known.new.len()), (1, 0), "the known trade expires, no orphan trade appears");
        assert_eq!(rebuild(&all[..5], &known).updated, []);
        // The same legs of a standard type (a put spread) are still a put spread.
        let spread = [
            exec("p", T, 4, "TradePilot OrderID: 4 (Opening) - PCS", 31, "P", 6690.0, "SLD", 1.0, 2.0),
            exec("q", T, 4, "TradePilot OrderID: 4 (Opening) - PCS", 32, "P", 6680.0, "BOT", 1.0, 0.8),
        ];
        assert_eq!(rebuild(&spread, &[]).new[0].trade_type, "PutSpread");
    }

    #[test]
    fn an_order_without_the_reference_of_the_app_and_without_a_standard_type_is_no_trade() {
        // A calendar closed by hand in TWS: the near put bought back, the far put sold - order 55, no reference.
        let mut far = exec("f", T, 55, "", 12, "P", 7535.0, "SLD", 1.0, 3.0);
        far.expiry = "20261120".into();
        let closing = [exec("n", T, 55, "", 11, "P", 7535.0, "BOT", 1.0, 2.0), far];
        assert_eq!(rebuild(&closing, &[]).new, [] as [StoredTrade; 0]);
    }

    #[test]
    fn late_fills_of_the_opening_order_make_a_recovered_trade_grow_to_all_its_contracts() {
        let early = [exec("a", T, 4, "TraPi-4", 31, "P", 6690.0, "SLD", 4.0, 2.0), exec("b", T, 4, "TraPi-4", 32, "P", 6680.0, "BOT", 4.0, 0.8)];
        let trade = rebuild(&early, &[]).new.remove(0);
        assert_eq!(trade.qty, 4);
        assert!(grown_trades(&early, std::slice::from_ref(&trade)).is_empty(), "nothing new: nothing changes");
        let late = [
            early[0].clone(),
            early[1].clone(),
            exec("c", T, 4, "TraPi-4", 31, "P", 6690.0, "SLD", 1.0, 2.5),
            exec("d", T, 4, "TraPi-4", 32, "P", 6680.0, "BOT", 1.0, 1.0),
        ];
        let grown = grown_trades(&late, std::slice::from_ref(&trade));
        assert_eq!(grown.len(), 1);
        assert_eq!((grown[0].qty, grown[0].price_open.to_string()), (5, "1.26".to_string()), "the average of all five contracts");
        assert_eq!((grown[0].strikes.as_str(), grown[0].order_id), (trade.strikes.as_str(), 4));
        assert_eq!(rebuild(&late, std::slice::from_ref(&trade)).grown.len(), 1, "a rebuild reports it as well");
        let closed = StoredTrade { status: "Expired".to_string(), ..trade };
        assert!(grown_trades(&late, &[closed]).is_empty(), "a trade that is over is left alone");
    }
    #[test]
    fn an_order_reference_is_the_short_prefix_and_the_order_id_and_old_references_are_still_read() {
        assert_eq!(order_ref(12, ""), "TraPi-12");
        assert_eq!((order_ref(13, "SL"), order_ref(14, "TP")), ("TraPi-13 (SL)".to_string(), "TraPi-14 (TP)".to_string()));
        assert_eq!(opening_order_id("TraPi-13 (SL)"), None);
        assert_eq!(opening_order_id("TraPi-12"), Some(12));
        assert_eq!(opening_order_id("TraPi-12 (Opening)"), None);
        assert_eq!(opening_order_id("TraPi-x"), None);
        assert_eq!(opening_order_id("TP-12"), Some(12), "the shorter form of earlier versions");
        assert_eq!(opening_order_id("TradePilot OrderID: 12 (Opening)"), Some(12));
        assert_eq!(opening_order_id("TradePilot OrderID: 12 (Opening) - PCS"), Some(12));
        assert_eq!(opening_order_id("TradePilot OrderID: 12 (STP) - PCS"), None);
        assert_eq!(opening_order_id("TradePilot OrderID: 12 (Opening)x"), None);
    }

    #[test]
    fn the_opening_reference_is_recognised() {
        assert_eq!(opening_order_id("TradePilot OrderID: 7 (Opening)"), Some(7));
        assert_eq!(opening_order_id("TradePilot OrderID: 7 (Close)"), None);
        assert_eq!(opening_order_id("TradePilot OrderID: x (Opening)"), None);
        assert_eq!(opening_order_id(""), None);
    }

    #[test]
    fn an_opening_order_becomes_a_trade_with_its_legs_and_entry_price() {
        let found = rebuild(&put_spread_entry(), &[]);
        assert_eq!(found.updated, [] as [crate::records::StoredTrade; 0]);
        let [trade] = found.new.as_slice() else { panic!("one trade expected, got {}", found.new.len()) };
        assert_eq!((trade.trade_type.as_str(), trade.strikes.as_str(), trade.qty, trade.order_id), ("PutSpread", "7535/7530 P", 1, 7));
        // 1.40 sold - 0.65 bought = 0.75 credit.
        assert_eq!((trade.price_open.normalize().to_string().as_str(), trade.status.as_str(), trade.expiry.as_str()), ("0.75", "Open", "20261113"));
        assert_eq!((trade.day.as_str(), trade.opened.as_str()), ("2026-11-13", "13-11-2026 21:30:05"));
        assert_eq!(trade.short_put.as_ref().map(|l| l.con_id), Some(11));
        assert_eq!(trade.long_put.as_ref().map(|l| l.con_id), Some(12));
        assert!(!trade.entry_unknown);
    }

    #[test]
    fn an_instant_order_of_a_single_option_is_found_again_without_a_reference() {
        // Order 31: sold 1 put, no reference of this app; order 0 (an expiry booking style id) is not an order of the app.
        let execs = vec![exec("a", T, 31, "", 21, "P", 7400.0, "SLD", 1.0, 0.15), exec("b", T, 0, "", 22, "P", 7300.0, "BOT", 1.0, 0.0)];
        let found = rebuild(&execs, &[]);
        let instant: Vec<&StoredTrade> = found.new.iter().filter(|t| t.order_id == 31).collect();
        let [trade] = instant.as_slice() else { panic!("one instant trade expected, got {found:?}") };
        assert_eq!((trade.trade_type.as_str(), trade.strikes.as_str(), trade.qty, trade.order_id), ("ShortPut", "7400 P", 1, 31));
        assert_eq!(trade.price_open.normalize().to_string(), "0.15");
        assert!(!trade.entry_unknown);
        // A bought option is a debit trade.
        let bought = rebuild(&[exec("c", T, 32, "", 23, "C", 7600.0, "BOT", 2.0, 1.1)], &[]).new;
        let [long] = bought.as_slice() else { panic!("one trade expected") };
        assert_eq!((long.trade_type.as_str(), long.price_open.normalize().to_string().as_str(), long.qty), ("LongCall", "-1.1", 2));
        // Known already (recorded when it was filled): nothing new.
        assert_eq!(rebuild(&execs[..1], &found.new).new.iter().filter(|t| t.order_id == 31).count(), 0);
        // Orders with a reference of this app (stop, close) and orders without an id are never taken for an entry.
        let others =
            [exec("d", T, 33, "TradePilot OrderID: 33 (STP)", 21, "P", 7400.0, "BOT", 1.0, 0.4), exec("e", T, 0, "", 21, "P", 7400.0, "SLD", 1.0, 0.1)];
        assert_eq!(rebuild(&others, &[]).new.iter().filter(|t| !t.entry_unknown).count(), 0);
    }

    #[test]
    fn buying_back_a_short_option_of_an_open_trade_is_no_new_trade() {
        let known = rebuild(&[exec("a", T, 31, "", 21, "P", 7400.0, "SLD", 1.0, 0.15)], &[]).new;
        // The same option is bought back by hand with order 40: a closing order.
        let back = [exec("b", T, 40, "", 21, "P", 7400.0, "BOT", 1.0, 0.05)];
        assert_eq!(rebuild(&back, &known).new.len(), 0);
        // Another option bought is a new trade.
        let other = [exec("c", T, 41, "", 99, "P", 7390.0, "BOT", 1.0, 0.05)];
        assert_eq!(rebuild(&other, &known).new.len(), 1);
    }

    #[test]
    fn partial_fills_of_one_leg_are_merged_by_quantity_weighted_price() {
        let mut execs = vec![exec("a", T, 7, OPEN, 11, "P", 7535.0, "SLD", 1.0, 1.0), exec("c", T, 7, OPEN, 11, "P", 7535.0, "SLD", 3.0, 1.2)];
        execs.extend([exec("b", T, 7, OPEN, 12, "P", 7530.0, "BOT", 4.0, 0.5)]);
        let [trade] = rebuild(&execs, &[]).new.try_into().unwrap_or_else(|_| panic!("one trade expected"));
        // (1*1.0 + 3*1.2) / 4 = 1.15 sold; 0.50 bought: 0.65 per contract, 4 contracts.
        assert_eq!((trade.qty, trade.price_open.normalize().to_string().as_str()), (4, "0.65"));
    }

    #[test]
    fn expiry_bookings_close_the_rebuilt_trade_at_zero() {
        let mut execs = put_spread_entry();
        execs.push(exec("x", "20261114 03:46:40 Europe/Berlin", 0, "", 11, "P", 7535.0, "BOT", 1.0, 0.0));
        execs.push(exec("y", "20261114 03:46:40 Europe/Berlin", 0, "", 12, "P", 7530.0, "SLD", 1.0, 0.0));
        let found = rebuild(&execs, &[]);
        let [trade] = found.new.as_slice() else { panic!("one trade expected, got {}", found.new.len()) };
        assert_eq!((trade.status.as_str(), trade.price_close.map(|p| p.normalize().to_string()).as_deref()), ("Expired", Some("0")));
    }

    #[test]
    fn a_known_open_trade_is_marked_expired_and_not_added_twice() {
        let known = rebuild(&put_spread_entry(), &[]).new;
        let mut execs = put_spread_entry();
        execs.push(exec("x", "20261114 03:46:40 Europe/Berlin", 0, "", 11, "P", 7535.0, "BOT", 1.0, 0.0));
        execs.push(exec("y", "20261114 03:46:40 Europe/Berlin", 0, "", 12, "P", 7530.0, "SLD", 1.0, 0.0));
        let found = rebuild(&execs, &known);
        assert!(found.new.is_empty(), "the order is already stored");
        assert_eq!(found.updated.len(), 1);
        assert_eq!(found.updated.first().map(|t| t.status.as_str()), Some("Expired"));
        // Running it again on the updated trade changes nothing.
        let again = rebuild(&execs, &found.updated);
        assert!(again.new.is_empty() && again.updated.is_empty());
    }

    #[test]
    fn expiry_bookings_alone_give_a_spread_with_an_unknown_entry_price() {
        // What TWS held on 14-11-2026: only the expiry of a put spread.
        let execs = vec![
            exec("x", "20261114 03:46:40 Europe/Berlin", 0, "", 11, "P", 7535.0, "BOT", 1.0, 0.0),
            exec("y", "20261114 03:46:40 Europe/Berlin", 0, "", 12, "P", 7530.0, "SLD", 1.0, 0.0),
        ];
        let found = rebuild(&execs, &[]);
        let [trade] = found.new.as_slice() else { panic!("one trade expected, got {}", found.new.len()) };
        assert_eq!((trade.trade_type.as_str(), trade.strikes.as_str(), trade.qty), ("PutSpread", "7535/7530 P", 1));
        assert_eq!((trade.status.as_str(), trade.entry_unknown, trade.day.as_str()), ("Expired", true, "2026-11-13"));
        // Seen again later: not added twice.
        assert_eq!(rebuild(&execs, &found.new).new, [] as [crate::records::StoredTrade; 0]);
    }

    #[test]
    fn an_iron_condor_is_one_trade_and_a_single_short_is_a_short() {
        let execs = vec![
            exec("a", T, 7, OPEN, 11, "P", 7535.0, "SLD", 1.0, 1.0),
            exec("b", T, 7, OPEN, 12, "P", 7525.0, "BOT", 1.0, 0.4),
            exec("c", T, 7, OPEN, 13, "C", 7600.0, "SLD", 1.0, 1.1),
            exec("d", T, 7, OPEN, 14, "C", 7610.0, "BOT", 1.0, 0.5),
        ];
        let [ic] = rebuild(&execs, &[]).new.try_into().unwrap_or_else(|_| panic!("one trade expected"));
        assert_eq!(
            (ic.trade_type.as_str(), ic.strikes.as_str(), ic.price_open.normalize().to_string().as_str()),
            ("IronCondor-Full", "7535/7525 P  7600/7610 C", "1.2")
        );
        let single = vec![exec("s", T, 8, "TradePilot OrderID: 8 (Opening)", 21, "C", 7650.0, "SLD", 2.0, 0.9)];
        let [short] = rebuild(&single, &[]).new.try_into().unwrap_or_else(|_| panic!("one trade expected"));
        assert_eq!((short.trade_type.as_str(), short.qty, short.price_open.normalize().to_string().as_str()), ("ShortCall", 2, "0.9"));
    }

    #[test]
    fn a_calendar_is_recognized_by_its_two_expirations() {
        let far = |mut e: StoredExecution| {
            e.expiry = "20261120".into();
            e
        };
        let put = vec![exec("a", T, 7, OPEN, 11, "P", 7535.0, "SLD", 1.0, 2.5), far(exec("b", T, 7, OPEN, 12, "P", 7535.0, "BOT", 1.0, 3.5))];
        let [cal] = rebuild(&put, &[]).new.try_into().unwrap_or_else(|_| panic!("one trade expected"));
        assert_eq!((cal.trade_type.as_str(), cal.price_open.normalize().to_string().as_str(), cal.expiry.as_str()), ("PutCalendar", "-1", "20261113"));
        let call = vec![exec("a", T, 7, OPEN, 11, "C", 7600.0, "SLD", 1.0, 2.5), far(exec("b", T, 7, OPEN, 12, "C", 7600.0, "BOT", 1.0, 3.5))];
        assert_eq!(rebuild(&call, &[]).new.first().map(|t| t.trade_type.as_str()), Some("CallCalendar"));
        let double = vec![
            exec("a", T, 7, OPEN, 11, "P", 7535.0, "SLD", 1.0, 2.5),
            far(exec("b", T, 7, OPEN, 12, "P", 7535.0, "BOT", 1.0, 3.5)),
            exec("c", T, 7, OPEN, 13, "C", 7600.0, "SLD", 1.0, 2.5),
            far(exec("d", T, 7, OPEN, 14, "C", 7600.0, "BOT", 1.0, 3.5)),
        ];
        let [dbl] = rebuild(&double, &[]).new.try_into().unwrap_or_else(|_| panic!("one trade expected"));
        assert_eq!((dbl.trade_type.as_str(), dbl.price_open.normalize().to_string().as_str()), ("DoubleCalendar", "-2"));
        // A sold far leg against a bought near leg is no standard type: it is a custom trade (a reverse diagonal).
        let reversed = vec![far(exec("a", T, 7, OPEN, 11, "P", 7535.0, "SLD", 1.0, 3.5)), exec("b", T, 7, OPEN, 12, "P", 7535.0, "BOT", 1.0, 2.5)];
        let [custom] = rebuild(&reversed, &[]).new.try_into().unwrap_or_else(|_| panic!("one trade expected"));
        assert_eq!((custom.trade_type.as_str(), custom.strikes.as_str(), custom.price_open), ("Custom", "-1 7535P +1 7535P", Decimal::ONE));
    }

    #[test]
    fn a_calendar_stays_open_until_both_legs_are_booked_expired() {
        let far = |mut e: StoredExecution| {
            e.expiry = "20261120".into();
            e
        };
        let entry = vec![exec("a", T, 7, OPEN, 11, "P", 7535.0, "SLD", 1.0, 2.5), far(exec("b", T, 7, OPEN, 12, "P", 7535.0, "BOT", 1.0, 3.5))];
        let known = rebuild(&entry, &[]).new;
        // The near leg expires worthless: TWS books the short back at 0 (BOT).
        let near_booking = StoredExecution { order_ref: String::new(), order_id: 0, price: 0.0, ..exec("x", T, 0, "", 11, "P", 7535.0, "BOT", 1.0, 0.0) };
        let mut execs = entry.clone();
        execs.push(near_booking.clone());
        assert!(rebuild(&execs, &known).updated.is_empty(), "the far leg is still alive");
        let far_booking = StoredExecution { expiry: "20261120".into(), ..exec("y", T, 0, "", 12, "P", 7535.0, "SLD", 1.0, 0.0) };
        execs.push(far_booking);
        let updated = rebuild(&execs, &known).updated;
        assert_eq!(updated.first().map(|t| t.status.as_str()), Some("Expired"));
    }

    #[test]
    fn executions_of_orders_with_a_reference_of_the_app_other_than_an_opening_are_ignored() {
        // A close order of the app has its own reference and opens nothing; the order without a reference is an
        // Instant Order and is found as a trade.
        let close = exec("w", T, 5, "TradePilot OrderID: 5 (Close)", 31, "P", 7000.0, "BOT", 1.0, 1.0);
        assert_eq!(rebuild(std::slice::from_ref(&close), &[]), Rebuilt::default());
        let found = rebuild(&[exec("z", T, 99, "", 31, "P", 7000.0, "SLD", 1.0, 2.0), close], &[]);
        assert_eq!(found.new.iter().map(|t| (t.order_id, t.trade_type.as_str())).collect::<Vec<_>>(), [(99, "ShortPut")]);
    }
}
