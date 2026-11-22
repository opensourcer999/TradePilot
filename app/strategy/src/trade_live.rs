//! Open trades while they live: the current price to close, the running profit, the delta and
//! the settlement at expiration:
//! current ask and bid, current profit, delta and settlement.
//!
//! Pure logic (`coding_guideline.md` section 2): quotes and the underlying price come in as values.
//! Money is `Decimal` with checked arithmetic (section 3); deltas are greeks and stay `f64`.
//!
//! Covered: every trade type template entry opens (Put/CallSpread, Short/Long Put/Call, the debit
//! spreads, the strangles, `IronFly`, IronCondor-Full, LongIronCondor-Full, and the calendars
//! `PutCalendar`, `CallCalendar`, `DoubleCalendar`), and custom trades by their own legs (`custom_closing_value`). Not covered:
//! Straddle (not entered by this app). Commissions are not part of these values; the caller adds them (`fees`).
//! Contracts closed by profit-target levels are left to the caller:
//! it values only the contracts still open (`StoredTrade::remaining_qty`) and adds what the levels
//! made (`StoredTrade::realized_profit`).

use crate::trade_type::TradeType;
use rust_decimal::Decimal;

/// The live quote of one leg: a missing bid is -1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegQuote {
    /// Bid; -1 = none.
    pub bid: Decimal,
    /// Ask; `None` = no quote received yet.
    pub ask: Option<Decimal>,
    /// TWS model delta, if known.
    pub delta: Option<f64>,
}

/// One held leg: its strike and, once quotes arrive, its quote.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveLeg {
    /// Strike price.
    pub strike: Decimal,
    /// Latest quote; `None` = nothing received for this option yet.
    pub quote: Option<LegQuote>,
}

/// The legs of a trade (`ShortPut`, `LongPut`, `ShortCall`, `LongCall`) and its ratios.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TradeLegs {
    /// The sold put, if any.
    pub short_put: Option<LiveLeg>,
    /// The bought put, if any.
    pub long_put: Option<LiveLeg>,
    /// The sold call, if any.
    pub short_call: Option<LiveLeg>,
    /// The bought call, if any.
    pub long_call: Option<LiveLeg>,
    /// `Trade.put_ratio` (at least 1).
    pub put_ratio: u32,
    /// `Trade.call_ratio` (at least 1).
    pub call_ratio: u32,
}

fn minus_one() -> Decimal {
    Decimal::NEGATIVE_ONE
}

/// A leg's ask (`ShortPutAsk` ...): 0 without the leg. `None` when the leg is held but has no
/// ask yet - computing with 0 there would show the full premium as profit; this
/// app shows no value instead of a wrong one.
fn ask(leg: Option<&LiveLeg>) -> Option<Decimal> {
    match leg {
        None => Some(Decimal::ZERO),
        Some(l) => l.quote.and_then(|q| q.ask),
    }
}

/// A leg's bid (`ShortPutBid` ...): 0 without the leg, -1 while it has none.
fn bid(leg: Option<&LiveLeg>) -> Option<Decimal> {
    match leg {
        None => Some(Decimal::ZERO),
        Some(l) => l.quote.map(|q| q.bid),
    }
}

fn floor_zero(v: Decimal) -> Decimal {
    v.max(Decimal::ZERO)
}

/// A price that is really quoted: a leg whose ask is 0 has no market yet.
fn nonzero(v: Decimal) -> Option<Decimal> {
    (v != Decimal::ZERO).then_some(v)
}

/// What selling a leg at its bid brings: a leg without a bid (-1) is worth nothing.
fn sold_value(bid: Decimal) -> Decimal {
    if bid == minus_one() {
        Decimal::ZERO
    } else {
        bid
    }
}

impl TradeLegs {
    fn ratios(&self) -> (Decimal, Decimal) {
        (Decimal::from(self.put_ratio.max(1)), Decimal::from(self.call_ratio.max(1)))
    }

    /// `Trade.CurrentAsk`: what closing a credit trade costs now (buy the shorts at the ask, sell
    /// the longs at the bid). `None` = a quote is missing or the type is not covered.
    #[must_use]
    pub fn current_ask(&self, trade_type: &str) -> Option<Decimal> {
        let (sp, lp, sc, lc) = (self.short_put.as_ref(), self.long_put.as_ref(), self.short_call.as_ref(), self.long_call.as_ref());
        let (pr, cr) = self.ratios();
        let kind = TradeType::parse(trade_type)?;
        match kind {
            TradeType::ShortPut => Some(floor_zero(ask(sp)?)),
            TradeType::ShortCall => Some(floor_zero(ask(sc)?)),
            TradeType::ShortStrangle => Some(floor_zero(ask(sc)?.checked_add(ask(sp)?)?)),
            TradeType::PutSpread | TradeType::CallSpread => {
                let (short, long) = if kind == TradeType::PutSpread { (sp, lp) } else { (sc, lc) };
                let long_bid = bid(long)?;
                let value = if long_bid == minus_one() { ask(short)? } else { ask(short)?.checked_sub(long_bid)? };
                Some(floor_zero(value))
            }
            TradeType::PutDebitSpread | TradeType::CallDebitSpread => {
                let (short, long) = if kind == TradeType::PutDebitSpread { (sp, lp) } else { (sc, lc) };
                let short_bid = bid(short)?;
                let value = if short_bid == minus_one() { ask(long)? } else { ask(long)?.checked_sub(short_bid)? };
                Some(floor_zero(value))
            }
            TradeType::LongPut => Some(floor_zero(ask(lp)?)),
            TradeType::LongCall => Some(floor_zero(ask(lc)?)),
            TradeType::LongStrangle => Some(floor_zero(ask(lc)?.checked_add(ask(lp)?)?)),
            TradeType::IronCondorFull | TradeType::IronFly => {
                // A long without a bid is left out; no floor at 0 here.
                let mut value = ask(sc)?.checked_mul(cr)?.checked_add(ask(sp)?.checked_mul(pr)?)?;
                let (lcb, lpb) = (bid(lc)?, bid(lp)?);
                if lcb != minus_one() {
                    value = value.checked_sub(lcb.checked_mul(cr)?)?;
                }
                if lpb != minus_one() {
                    value = value.checked_sub(lpb.checked_mul(pr)?)?;
                }
                Some(value)
            }
            TradeType::LongIronCondorFull => {
                let longs = ask(lc)?.checked_mul(cr)?.checked_add(ask(lp)?.checked_mul(pr)?)?;
                let shorts = bid(sc)?.checked_mul(cr)?.checked_add(bid(sp)?.checked_mul(pr)?)?;
                Some(floor_zero(longs.checked_sub(shorts)?))
            }
            // A calendar is valued like a credit trade: the cost to close is the near leg bought
            // back at the ask minus the far leg sold at the bid (negative while the far leg is worth
            // more), so the profit is the entry price (a debit is negative) minus that cost. A leg
            // without an ask gives no value instead of a wrong one.
            TradeType::PutCalendar | TradeType::CallCalendar => {
                let (short, long) = if kind == TradeType::PutCalendar { (sp, lp) } else { (sc, lc) };
                nonzero(ask(short)?)?;
                nonzero(ask(long)?)?;
                ask(short)?.checked_sub(sold_value(bid(long)?))
            }
            TradeType::DoubleCalendar => {
                for leg in [sc, sp, lc, lp] {
                    nonzero(ask(leg)?)?;
                }
                let shorts = ask(sc)?.checked_mul(cr)?.checked_add(ask(sp)?.checked_mul(pr)?)?;
                let longs = sold_value(bid(lc)?).checked_mul(cr)?.checked_add(sold_value(bid(lp)?).checked_mul(pr)?)?;
                shorts.checked_sub(longs)
            }
            TradeType::IronCondor | TradeType::Straddle | TradeType::Custom => None,
        }
    }

    /// `Trade.CurrentBid`: what closing a debit trade brings now (sell the longs at the bid, buy
    /// the shorts at the ask). `None` = a quote is missing or the type is not covered.
    #[must_use]
    pub fn current_bid(&self, trade_type: &str) -> Option<Decimal> {
        let (sp, lp, sc, lc) = (self.short_put.as_ref(), self.long_put.as_ref(), self.short_call.as_ref(), self.long_call.as_ref());
        let (pr, cr) = self.ratios();
        let kind = TradeType::parse(trade_type)?;
        match kind {
            TradeType::ShortPut => Some(floor_zero(bid(sp)?)),
            TradeType::ShortCall => Some(floor_zero(bid(sc)?)),
            TradeType::ShortStrangle => Some(floor_zero(bid(sc)?.checked_add(bid(sp)?)?)),
            TradeType::PutSpread | TradeType::CallSpread => {
                let (short, long) = if kind == TradeType::PutSpread { (sp, lp) } else { (sc, lc) };
                let value = if bid(long)? == minus_one() { bid(short)? } else { bid(short)?.checked_sub(ask(long)?)? };
                Some(floor_zero(value))
            }
            TradeType::PutDebitSpread | TradeType::CallDebitSpread => {
                let (short, long) = if kind == TradeType::PutDebitSpread { (sp, lp) } else { (sc, lc) };
                Some(floor_zero(bid(long)?.checked_sub(ask(short)?)?))
            }
            TradeType::LongPut => Some(floor_zero(bid(lp)?)),
            TradeType::LongCall => Some(floor_zero(bid(lc)?)),
            TradeType::LongStrangle => {
                let (lpb, lcb) = (bid(lp)?, bid(lc)?);
                match (lpb == minus_one(), lcb == minus_one()) {
                    (true, true) => Some(Decimal::ZERO),
                    (true, false) => Some(lcb),
                    (false, true) => Some(lpb),
                    (false, false) => Some(floor_zero(lcb.checked_add(lpb)?)),
                }
            }
            TradeType::IronCondorFull | TradeType::IronFly => {
                let mut value = bid(sc)?.checked_mul(cr)?.checked_add(bid(sp)?.checked_mul(pr)?)?;
                let (lcb, lpb) = (bid(lc)?, bid(lp)?);
                if lcb != minus_one() {
                    value = value.checked_sub(ask(lc)?.checked_mul(cr)?)?;
                }
                if lpb != minus_one() {
                    value = value.checked_sub(ask(lp)?.checked_mul(pr)?)?;
                }
                Some(value)
            }
            TradeType::LongIronCondorFull => {
                let longs = bid(lc)?.checked_mul(cr)?.checked_add(bid(lp)?.checked_mul(pr)?)?;
                let shorts = ask(sc)?.checked_mul(cr)?.checked_add(ask(sp)?.checked_mul(pr)?)?;
                Some(floor_zero(longs.checked_sub(shorts)?))
            }
            // What the calendar would bring if it were opened now: the near legs sold at the bid,
            // the far legs bought at the ask.
            TradeType::PutCalendar | TradeType::CallCalendar => {
                let (short, long) = if kind == TradeType::PutCalendar { (sp, lp) } else { (sc, lc) };
                nonzero(ask(short)?)?;
                nonzero(ask(long)?)?;
                sold_value(bid(short)?).checked_sub(ask(long)?)
            }
            TradeType::DoubleCalendar => {
                for leg in [sc, sp, lc, lp] {
                    nonzero(ask(leg)?)?;
                }
                // Weighted by the ratios like `current_ask` (a plain sum would leave
                // them out here; with ratios of 1, as calendars are entered, both are the same).
                let shorts = sold_value(bid(sc)?).checked_mul(cr)?.checked_add(sold_value(bid(sp)?).checked_mul(pr)?)?;
                let longs = ask(lc)?.checked_mul(cr)?.checked_add(ask(lp)?.checked_mul(pr)?)?;
                shorts.checked_sub(longs)
            }
            TradeType::IronCondor | TradeType::Straddle | TradeType::Custom => None,
        }
    }

    /// `Trade.CurrentProfit` of an open trade, without commissions: a credit trade earns what it
    /// was sold for minus what closing costs now; a debit trade (negative `price_open`) what
    /// closing brings minus what it cost. `None` = no quote yet.
    #[must_use]
    pub fn current_profit(&self, trade_type: &str, price_open: Decimal, qty: u32, multiplier: u32) -> Option<Decimal> {
        // Valued like a credit trade: what it was sold for (a calendar's debit is negative) minus
        // the cost to close; like a debit trade: what closing brings minus what it cost.
        let per_contract = match TradeType::parse(trade_type)? {
            TradeType::PutSpread
            | TradeType::CallSpread
            | TradeType::ShortPut
            | TradeType::ShortCall
            | TradeType::ShortStrangle
            | TradeType::IronFly
            | TradeType::IronCondorFull
            | TradeType::PutCalendar
            | TradeType::CallCalendar
            | TradeType::DoubleCalendar => price_open.checked_sub(self.current_ask(trade_type)?)?,
            TradeType::LongPut
            | TradeType::LongCall
            | TradeType::PutDebitSpread
            | TradeType::CallDebitSpread
            | TradeType::LongStrangle
            | TradeType::LongIronCondorFull => price_open.checked_add(self.current_bid(trade_type)?)?,
            // Not entered by this app: no live figure.
            TradeType::IronCondor | TradeType::Straddle | TradeType::Custom => return None,
        };
        per_contract.checked_mul(Decimal::from(qty))?.checked_mul(Decimal::from(multiplier))
    }

    /// `Trade.Delta` now: (shortPut - longPut + shortCall - longCall) * -100; 0 while a held leg
    /// has no delta.
    #[must_use]
    pub fn delta(&self) -> f64 {
        let legs = [self.short_put.as_ref(), self.long_put.as_ref(), self.short_call.as_ref(), self.long_call.as_ref()];
        let mut deltas = [0.0_f64; 4];
        for (slot, leg) in deltas.iter_mut().zip(legs) {
            if let Some(leg) = leg {
                match leg.quote.and_then(|q| q.delta) {
                    Some(d) if d != 0.0 => *slot = d,
                    _ => return 0.0,
                }
            }
        }
        let [sp, lp, sc, lc] = deltas;
        (sp - lp + sc - lc) * -100.0
    }

    /// At expiration: every leg is worth its intrinsic value at
    /// the underlying's closing price. Returns `Trade.PriceClose` per contract - 0 when everything
    /// expires worthless, negative when a credit trade has to pay, positive when a long pays out.
    /// `None` for an implausible price (0 < price < 50000) or on overflow.
    #[must_use]
    pub fn settle(&self, underlying_close: Decimal) -> Option<Decimal> {
        if underlying_close <= Decimal::ZERO || underlying_close >= Decimal::from(50_000) {
            return None;
        }
        let (pr, cr) = self.ratios();
        let put_value = |leg: Option<&LiveLeg>| leg.map_or(Decimal::ZERO, |l| floor_zero(l.strike - underlying_close));
        let call_value = |leg: Option<&LiveLeg>| leg.map_or(Decimal::ZERO, |l| floor_zero(underlying_close - l.strike));
        let shorts = put_value(self.short_put.as_ref()).checked_mul(pr)?.checked_add(call_value(self.short_call.as_ref()).checked_mul(cr)?)?;
        let longs = put_value(self.long_put.as_ref()).checked_mul(pr)?.checked_add(call_value(self.long_call.as_ref()).checked_mul(cr)?)?;
        // `PriceClose = (shorts - longs) * -1`.
        longs.checked_sub(shorts)
    }
}

/// `Trade.ProfitLossNet` once closed, without commissions: `(PriceOpen + PriceClose) * Qty *
/// Multiplier`. `None` only on overflow.
#[must_use]
pub fn profit_loss(price_open: Decimal, price_close: Decimal, qty: u32, multiplier: u32) -> Option<Decimal> {
    price_open.checked_add(price_close)?.checked_mul(Decimal::from(qty))?.checked_mul(Decimal::from(multiplier))
}

/// A leg of a custom trade as it is held now: its ratio (positive = bought, negative = sold) and its latest quote.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CustomHeld {
    /// The ratio with its sign.
    pub ratio: i32,
    /// The latest quote; `None` = nothing received for this option yet.
    pub quote: Option<LegQuote>,
}

/// What closing a custom position brings now (positive) or costs (negative), per contract of the trade: the bought legs are sold at
/// their bid (a leg without a bid is worth nothing), the sold legs are bought back at their ask. `None` while a sold leg has no ask or a
/// bought leg has no quote at all - computing with 0 there would show a wrong profit.
#[must_use]
pub fn custom_closing_value(legs: &[CustomHeld]) -> Option<Decimal> {
    let mut value = Decimal::ZERO;
    for held in legs {
        let quote = held.quote?;
        let ratio = Decimal::from(held.ratio.abs());
        let part = if held.ratio < 0 { Decimal::ZERO.checked_sub(quote.ask?.checked_mul(ratio)?)? } else { quote.bid.max(Decimal::ZERO).checked_mul(ratio)? };
        value = value.checked_add(part)?;
    }
    Some(value)
}

/// The value of a custom position for the stop adjustments, as `current_bid` and `current_ask` give it for the other types: `(bid, ask)`.
/// A credit position (`credit`) is valued at what opening it would bring (shorts at the bid, longs at the ask, not below 0) and at what closing
/// costs (shorts at the ask, longs at the bid); a debit position at what closing brings and at what opening costs. `None` while a quote is missing.
#[must_use]
pub fn custom_open_values(legs: &[CustomHeld], credit: bool) -> Option<(Decimal, Decimal)> {
    let (mut short_bid, mut short_ask, mut long_bid, mut long_ask) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for held in legs {
        let quote = held.quote?;
        let ratio = Decimal::from(held.ratio.abs());
        let (bid, ask) = (quote.bid.max(Decimal::ZERO).checked_mul(ratio)?, quote.ask?.checked_mul(ratio)?);
        if held.ratio < 0 {
            (short_bid, short_ask) = (short_bid.checked_add(bid)?, short_ask.checked_add(ask)?);
        } else {
            (long_bid, long_ask) = (long_bid.checked_add(bid)?, long_ask.checked_add(ask)?);
        }
    }
    let (bid, ask) = if credit {
        (short_bid.checked_sub(long_ask)?, short_ask.checked_sub(long_bid)?)
    } else {
        (long_bid.checked_sub(short_ask)?, long_ask.checked_sub(short_bid)?)
    };
    Some((bid.max(Decimal::ZERO), ask.max(Decimal::ZERO)))
}

/// The delta of a custom position per contract of the trade, as `Trade.Delta` has it for the other types: the sum of ratio times
/// delta times 100 (a long call is positive). 0 as soon as a leg has no delta yet.
#[must_use]
pub fn custom_delta(legs: &[CustomHeld]) -> f64 {
    let mut sum = 0.0;
    for held in legs {
        match held.quote.and_then(|quote| quote.delta) {
            Some(delta) if delta != 0.0 => sum += f64::from(held.ratio) * delta * 100.0,
            _ => return 0.0,
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[allow(clippy::unnecessary_wraps)] // the field it fills is an `Option`
    fn leg(strike: &str, bid: &str, ask: &str, delta: f64) -> Option<LiveLeg> {
        Some(LiveLeg { strike: d(strike), quote: Some(LegQuote { bid: d(bid), ask: Some(d(ask)), delta: Some(delta) }) })
    }

    #[allow(clippy::unnecessary_wraps)] // the field it fills is an `Option`
    fn unquoted(strike: &str) -> Option<LiveLeg> {
        Some(LiveLeg { strike: d(strike), quote: None })
    }

    fn put_spread() -> TradeLegs {
        TradeLegs {
            short_put: leg("6650", "1.00", "1.10", -0.15),
            long_put: leg("6640", "0.70", "0.80", -0.11),
            put_ratio: 1,
            call_ratio: 1,
            ..TradeLegs::default()
        }
    }

    #[test]
    fn a_credit_spread_earns_its_credit_minus_the_cost_to_close() {
        let legs = put_spread();
        // Buy the short back at 1.10, sell the long at 0.70: closing costs 0.40.
        assert_eq!(legs.current_ask("PutSpread"), Some(d("0.40")));
        // Sold for 1.00: (1.00 - 0.40) * 2 * 100.
        assert_eq!(legs.current_profit("PutSpread", d("1.00"), 2, 100), Some(d("120.00")));
        // Trade.Delta: (-0.15 - -0.11) * -100 = 4.
        assert!((legs.delta() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_long_without_a_bid_is_left_out_of_the_closing_cost() {
        let mut legs = put_spread();
        legs.long_put = Some(LiveLeg { strike: d("6640"), quote: Some(LegQuote { bid: minus_one(), ask: Some(d("0.05")), delta: Some(-0.01) }) });
        assert_eq!(legs.current_ask("PutSpread"), Some(d("1.10")));
    }

    #[test]
    fn no_quote_yet_means_no_profit_figure_instead_of_a_wrong_one() {
        let legs = TradeLegs { short_put: unquoted("6650"), long_put: leg("6640", "0.70", "0.80", -0.11), put_ratio: 1, call_ratio: 1, ..TradeLegs::default() };
        assert_eq!(legs.current_profit("PutSpread", d("1.00"), 1, 100), None);
        assert_eq!(legs.delta(), 0.0, "no delta while a leg has none");
    }

    #[test]
    fn a_debit_trade_earns_what_closing_brings_minus_what_it_cost() {
        let legs = TradeLegs {
            long_put: leg("6650", "1.40", "1.50", -0.2),
            short_put: leg("6640", "1.00", "1.10", -0.15),
            put_ratio: 1,
            call_ratio: 1,
            ..TradeLegs::default()
        };
        // Sell the long at 1.40, buy the short back at 1.10: 0.30. Paid 0.25.
        assert_eq!(legs.current_bid("PutDebitSpread"), Some(d("0.30")));
        assert_eq!(legs.current_profit("PutDebitSpread", d("-0.25"), 1, 100), Some(d("5.00")));
        let long_call = TradeLegs { long_call: leg("6750", "2.00", "2.20", 0.3), put_ratio: 1, call_ratio: 1, ..TradeLegs::default() };
        assert_eq!(long_call.current_profit("LongCall", d("-1.50"), 1, 100), Some(d("50.00")));
    }

    #[test]
    fn an_iron_condor_closes_all_four_legs() {
        let legs = TradeLegs {
            short_put: leg("6650", "1.00", "1.10", -0.15),
            long_put: leg("6640", "0.70", "0.80", -0.11),
            short_call: leg("6750", "0.90", "1.00", 0.14),
            long_call: leg("6760", "0.60", "0.70", 0.10),
            put_ratio: 1,
            call_ratio: 1,
        };
        // 1.00 + 1.10 - 0.60 - 0.70 = 0.80
        assert_eq!(legs.current_ask("IronCondor-Full"), Some(d("0.80")));
        assert_eq!(legs.current_profit("IronCondor-Full", d("1.50"), 1, 100), Some(d("70.00")));
        // (-0.15 + 0.11 + 0.14 - 0.10) * -100 = 0 (delta neutral)
        assert!(legs.delta().abs() < 1e-9);
    }

    #[test]
    fn settlement_pays_the_intrinsic_value_at_the_close() {
        let legs = put_spread();
        // Above both strikes: worthless, the whole credit is kept.
        assert_eq!(legs.settle(d("6700")), Some(d("0")));
        assert_eq!(profit_loss(d("1.00"), d("0"), 1, 100), Some(d("100.00")));
        // Between the strikes: the short put is 5 in the money.
        assert_eq!(legs.settle(d("6645")), Some(d("-5")));
        assert_eq!(profit_loss(d("1.00"), d("-5"), 1, 100), Some(d("-400.00")));
        // Below both: the loss is capped at the width (10).
        assert_eq!(legs.settle(d("6600")), Some(d("-10")));
        // A long call pays out.
        let long_call = TradeLegs { long_call: leg("6750", "0", "0", 0.0), put_ratio: 1, call_ratio: 1, ..TradeLegs::default() };
        assert_eq!(long_call.settle(d("6760")), Some(d("10")));
        assert_eq!(profit_loss(d("-1.50"), d("10"), 1, 100), Some(d("850.00")));
        // An implausible underlying price settles nothing.
        assert_eq!(legs.settle(d("0")), None);
        assert_eq!(legs.settle(d("50000")), None);
    }

    #[test]
    fn types_this_app_does_not_enter_have_no_live_figures() {
        assert_eq!(put_spread().current_profit("Custom", d("1"), 1, 100), None);
    }

    /// A put calendar: the near 6650 put sold, the far one bought.
    fn put_calendar() -> TradeLegs {
        TradeLegs {
            short_put: leg("6650", "2.40", "2.50", -0.25),
            long_put: leg("6650", "3.40", "3.50", -0.27),
            put_ratio: 1,
            call_ratio: 1,
            ..TradeLegs::default()
        }
    }

    #[test]
    fn a_calendar_is_valued_like_a_credit_trade_with_a_negative_entry() {
        let cal = put_calendar();
        // Closing: buy the near put at 2.50, sell the far one at 3.40 -> brings 0.90 (cost -0.90).
        assert_eq!(cal.current_ask("PutCalendar"), Some(d("-0.9")));
        // Opened now it would cost the far ask 3.50 minus the near bid 2.40.
        assert_eq!(cal.current_bid("PutCalendar"), Some(d("-1.1")));
        // Paid 1.00 (entry -1.00): closing for +0.90 loses 0.10 per contract.
        assert_eq!(cal.current_profit("PutCalendar", d("-1"), 2, 100), Some(d("-20")));
        // A calendar that gained: the far put is now worth 4.00.
        let gained = TradeLegs { long_put: leg("6650", "4.00", "4.10", -0.27), ..put_calendar() };
        assert_eq!(gained.current_profit("PutCalendar", d("-1"), 1, 100), Some(d("50")));
    }

    #[test]
    fn a_double_calendar_adds_both_sides_and_a_leg_without_a_quote_gives_no_figure() {
        let double = TradeLegs {
            short_put: leg("6650", "2.40", "2.50", -0.25),
            long_put: leg("6650", "3.40", "3.50", -0.27),
            short_call: leg("6750", "1.40", "1.50", 0.2),
            long_call: leg("6750", "2.40", "2.50", 0.22),
            put_ratio: 1,
            call_ratio: 1,
        };
        assert_eq!(double.current_ask("DoubleCalendar"), Some(d("-1.8")));
        assert_eq!(double.current_profit("DoubleCalendar", d("-2"), 1, 100), Some(d("-20")));
        // Opened now: 2.40 + 1.40 sold, 3.50 + 2.50 bought.
        assert_eq!(double.current_bid("DoubleCalendar"), Some(d("-2.2")));
        // Both prices weigh each side by its ratio: two put calendars to one call calendar.
        let two_puts = TradeLegs { put_ratio: 2, ..double.clone() };
        assert_eq!(two_puts.current_ask("DoubleCalendar"), Some(d("-2.7")), "2.50 x 2 + 1.50 - (3.40 x 2 + 2.40)");
        assert_eq!(two_puts.current_bid("DoubleCalendar"), Some(d("-3.3")), "2.40 x 2 + 1.40 - (3.50 x 2 + 2.50)");
        let unquoted_far = TradeLegs { long_call: unquoted("6750"), ..double.clone() };
        assert_eq!(unquoted_far.current_profit("DoubleCalendar", d("-2"), 1, 100), None);
        let no_ask = TradeLegs { short_call: leg("6750", "-1", "0", 0.2), ..double };
        assert_eq!(no_ask.current_ask("DoubleCalendar"), None, "an ask of 0 is no market");
    }
    #[test]
    fn a_custom_position_is_valued_for_the_stop_adjustments_like_the_other_types() {
        let quote = |bid: &str, ask: &str| Some(LegQuote { bid: d(bid), ask: Some(d(ask)), delta: None });
        // A credit spread: sold 6690 put 2.00/2.10, bought 6670 put 0.80/0.90. Opening brings 2.00 - 0.90 = 1.10, closing costs 2.10 - 0.80 = 1.30.
        let spread = [CustomHeld { ratio: -1, quote: quote("2.00", "2.10") }, CustomHeld { ratio: 1, quote: quote("0.80", "0.90") }];
        assert_eq!(custom_open_values(&spread, true), Some((d("1.10"), d("1.30"))));
        // A debit position (the same legs the other way round): closing brings 2.00 - 0.90 = 1.10, opening costs 2.10 - 0.80 = 1.30.
        let debit = [CustomHeld { ratio: 1, quote: quote("2.00", "2.10") }, CustomHeld { ratio: -1, quote: quote("0.80", "0.90") }];
        assert_eq!(custom_open_values(&debit, false), Some((d("1.10"), d("1.30"))));
        let missing = [CustomHeld { ratio: -1, quote: Some(LegQuote { bid: d("1"), ask: None, delta: None }) }];
        assert_eq!(custom_open_values(&missing, true), None);
    }

    #[test]
    fn a_custom_position_is_valued_by_selling_the_longs_at_the_bid_and_buying_the_shorts_at_the_ask() {
        let quote = |bid: &str, ask: &str, delta: f64| Some(LegQuote { bid: d(bid), ask: Some(d(ask)), delta: Some(delta) });
        // Sold 6690 put (ask 2.10), bought 6670 put (bid 0.80): closing costs 2.10 - 0.80 = 1.30.
        let spread = [CustomHeld { ratio: -1, quote: quote("2.00", "2.10", -0.22) }, CustomHeld { ratio: 1, quote: quote("0.80", "0.90", -0.10) }];
        assert_eq!(custom_closing_value(&spread), Some(d("-1.30")));
        // Ratios count: two bought at the bid.
        let double = [CustomHeld { ratio: 2, quote: quote("1.00", "1.10", 0.30) }];
        assert_eq!(custom_closing_value(&double), Some(d("2.00")));
        // A bought leg without a bid is worth nothing; a missing quote or a sold leg without an ask gives no value.
        let no_bid = [CustomHeld { ratio: 1, quote: Some(LegQuote { bid: d("-1"), ask: Some(d("0.05")), delta: None }) }];
        assert_eq!(custom_closing_value(&no_bid), Some(Decimal::ZERO));
        assert_eq!(custom_closing_value(&[CustomHeld { ratio: 1, quote: None }]), None);
        assert_eq!(custom_closing_value(&[CustomHeld { ratio: -1, quote: Some(LegQuote { bid: d("1"), ask: None, delta: None }) }]), None);
        // The delta follows the position: short put +22, long put -10.
        let delta = custom_delta(&spread);
        assert!((delta - 12.0).abs() < 1e-9, "{delta}");
        assert_eq!(custom_delta(&[CustomHeld { ratio: 1, quote: quote("1", "1.1", 0.0) }]), 0.0, "no delta yet");
    }
}
