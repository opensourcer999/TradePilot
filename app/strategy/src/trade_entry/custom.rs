//! The strike selection of a custom trade: up to four legs, each with its own action
//! (buy or sell), right, ratio, expiration and a target that says how its strike is found - by delta, by premium, by a
//! distance from the underlying or from an earlier leg. The entry price is the mid price of the whole combo (a credit is
//! positive). Nothing is sent. Pure logic like the rest of `trade_entry`.

use super::{fmt_dec, DataStatus, EntryOutcome, Leg};
use crate::pricing::{dec, fix_price};
use crate::templates::TradeTemplate;
use rust_decimal::Decimal;

/// One selected leg of a custom trade with its ratio (positive = bought, negative = sold).
#[derive(Clone, Debug, PartialEq)]
pub struct CustomLeg {
    /// The option.
    pub leg: Leg,
    /// The ratio with its sign: a sold leg is negative.
    pub ratio: i32,
}

/// The legs a custom trade holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomEntry {
    /// The legs in the order of the template (leg 1 first).
    pub legs: Vec<CustomLeg>,
}

impl CustomEntry {
    /// The mid price of the combo: what it brings (a credit, positive) or costs (a debit, negative).
    #[must_use]
    pub fn mid_price(&self) -> Option<Decimal> {
        self.legs.iter().try_fold(Decimal::ZERO, |sum, leg| sum.checked_sub(leg.leg.mid()?.checked_mul(Decimal::from(leg.ratio))?))
    }

    /// What the position needs as buying power per contract of the trade: the worst loss at the expiration, in dollars (the premium counts:
    /// a credit lowers it, a debit is the loss). `None` when the risk is not defined or not worked out: more sold calls than bought ones
    /// (the loss grows without limit), more sold puts than bought ones (refused as well: the loss is large), legs of different expirations
    /// other than a debit trade whose sold legs expire first, or no loss at all.
    #[must_use]
    pub fn buying_power(&self) -> Option<Decimal> {
        let premium = self.mid_price()?;
        let same_expiry = self.legs.iter().all(|l| Some(&l.leg.expiry) == self.legs.first().map(|first| &first.leg.expiry));
        if !same_expiry {
            let first_sold = self.legs.iter().filter(|l| l.ratio < 0).map(|l| l.leg.expiry.as_str()).max()?;
            let first_bought = self.legs.iter().filter(|l| l.ratio > 0).map(|l| l.leg.expiry.as_str()).min()?;
            return (premium < Decimal::ZERO && first_sold <= first_bought)
                .then(|| premium.abs().checked_mul(Decimal::ONE_HUNDRED))
                .flatten()
                .filter(|bp| *bp > Decimal::ZERO);
        }
        let net = |right: &str| self.legs.iter().filter(|l| l.leg.right == right).map(|l| l.ratio).sum::<i32>();
        if net("C") < 0 || net("P") < 0 {
            return None;
        }
        let payoff = |price: Decimal| -> Option<Decimal> {
            self.legs.iter().try_fold(premium, |sum, l| {
                let intrinsic = if l.leg.right == "C" { price.checked_sub(l.leg.strike)? } else { l.leg.strike.checked_sub(price)? }.max(Decimal::ZERO);
                sum.checked_add(intrinsic.checked_mul(Decimal::from(l.ratio))?)
            })
        };
        let mut worst = payoff(Decimal::ZERO)?;
        for leg in &self.legs {
            worst = worst.min(payoff(leg.leg.strike)?);
        }
        Decimal::ZERO.checked_sub(worst)?.max(Decimal::ZERO).checked_mul(Decimal::ONE_HUNDRED).filter(|bp| *bp > Decimal::ZERO)
    }

    /// The absolute mid value of the sold legs and of the bought legs.
    fn short_and_long(&self) -> Option<(Decimal, Decimal)> {
        let side = |sold: bool| {
            self.legs
                .iter()
                .filter(|leg| (leg.ratio < 0) == sold)
                .try_fold(Decimal::ZERO, |sum, leg| sum.checked_add(leg.leg.mid()?.checked_mul(Decimal::from(leg.ratio))?))
                .map(|value| value.abs())
        };
        Some((side(true)?, side(false)?))
    }

    /// The strikes as the grid and the log show them, e.g. `-1 7000P +1 6990P`.
    #[must_use]
    pub fn strikes_text(&self) -> String {
        self.legs.iter().map(|leg| format!("{:+} {}{}", leg.ratio, fmt_dec(leg.leg.strike), leg.leg.right)).collect::<Vec<_>>().join(" ")
    }

    /// The delta of the position per contract as `Trade.Delta` has it for the other types: the sum of ratio times delta times
    /// 100; 0 as soon as a leg has no delta yet.
    #[must_use]
    pub fn delta(&self) -> f64 {
        if self.legs.iter().any(|leg| leg.leg.delta == 0.0) {
            return 0.0;
        }
        self.legs.iter().map(|leg| f64::from(leg.ratio) * leg.leg.delta).sum::<f64>() * 100.0
    }
}

/// Everything the selection looks at.
pub struct CustomInput<'a> {
    /// The template to trade.
    pub template: &'a TradeTemplate,
    /// The underlying's current price.
    pub price: Decimal,
    /// Today's opening price of the underlying (for `Offset from open`); the price if unknown.
    pub open_price: Option<Decimal>,
    /// Every option of the symbol in the expirations of `expiries`.
    pub legs: Vec<Leg>,
    /// The expiration (yyyyMMdd) of each of the four legs; unused legs are empty.
    pub expiries: [String; 4],
    /// US regular session - quote checks only apply then.
    pub market_open: bool,
    /// The underlying's price is older than 60 s.
    pub underlying_outdated: bool,
    /// Instant Order's "Test Trade?".
    pub test_trade: bool,
}

/// What the template says about one leg.
struct LegSpec<'a> {
    action: &'a str,
    right: &'a str,
    ratio: i32,
    target: Decimal,
    target_type: &'a str,
}

/// The four legs of a template (the ones whose action is not "None" are used).
fn specs(t: &TradeTemplate) -> Option<[LegSpec<'_>; 4]> {
    let spec = |action, right, ratio, target: f64, target_type| Some(LegSpec { action, right, ratio, target: dec(target)?, target_type });
    Some([
        spec(t.custom_leg1_action.as_str(), t.custom_leg1_put_call.as_str(), t.custom_leg1_ratio, t.custom_leg1_target, t.custom_leg1_target_type.as_str())?,
        spec(t.custom_leg2_action.as_str(), t.custom_leg2_put_call.as_str(), t.custom_leg2_ratio, t.custom_leg2_target, t.custom_leg2_target_type.as_str())?,
        spec(t.custom_leg3_action.as_str(), t.custom_leg3_put_call.as_str(), t.custom_leg3_ratio, t.custom_leg3_target, t.custom_leg3_target_type.as_str())?,
        spec(t.custom_leg4_action.as_str(), t.custom_leg4_put_call.as_str(), t.custom_leg4_ratio, t.custom_leg4_target, t.custom_leg4_target_type.as_str())?,
    ])
}

/// The option for one leg, `relative` being the price or strike its target counts from. `None` when no
/// option fits or the one that fits collides with an open position.
fn select_leg(spec: &LegSpec<'_>, expiry: &str, relative: Decimal, legs: &[Leg], symbol: &str, allow_conflict: bool, log: &mut Vec<String>) -> Option<Leg> {
    // Strikes the account holds the other way round in this expiration: selling a bought strike (or buying a sold one) conflicts.
    let conflicts: Vec<Decimal> = legs
        .iter()
        .filter(|l| l.right == spec.right && l.expiry == expiry)
        .filter(|l| (spec.action == "Buy" && l.position < Decimal::ZERO) || (spec.action == "Sell" && l.position > Decimal::ZERO))
        .map(|l| l.strike)
        .collect();
    let mut candidates: Vec<&Leg> = legs
        .iter()
        .filter(|l| l.right == spec.right && l.expiry == expiry && l.ask > Decimal::NEGATIVE_ONE && l.subscribed)
        .filter(|l| !l.wide_bid_ask(symbol))
        .collect();
    candidates.sort_by_key(|l| std::cmp::Reverse(l.strike));
    let closest = |distance: &dyn Fn(&Leg) -> Option<Decimal>| -> Option<&Leg> {
        candidates.iter().copied().filter_map(|l| distance(l).map(|d| (d, l))).min_by(|a, b| a.0.cmp(&b.0)).map(|(_, l)| l)
    };
    let chosen: Option<&Leg> = match spec.target_type {
        "Fixed premium" => {
            let distance = |l: &Leg| l.mid().and_then(|mid| mid.abs().checked_sub(spec.target)).map(|value| value.abs());
            // Equal distances: a put takes the higher strike (the list is highest first), a call the lower one.
            let mut ranked: Vec<&Leg> = candidates.clone();
            if spec.right != "P" {
                ranked.reverse();
            }
            ranked.iter().copied().filter_map(|l| distance(l).map(|d| (d, l))).min_by(|a, b| a.0.cmp(&b.0)).map(|(_, l)| l)
        }
        "Delta" => {
            let wanted = spec.target.checked_div(Decimal::ONE_HUNDRED)?;
            // A delta of 0 is one TWS has not sent yet, not a real far-out option: it must not win a small target.
            closest(&|l: &Leg| dec(l.delta).filter(|delta| !delta.is_zero()).and_then(|delta| delta.abs().checked_sub(wanted)).map(|value| value.abs()))
        }
        "Offset from leg 1" | "Offset from leg 2" | "Offset from leg 3" => {
            let strike = relative.checked_add(spec.target)?;
            candidates.iter().copied().find(|l| l.strike == strike)
        }
        "Offset from price" | "Offset from open" => {
            let wanted = relative.checked_add(spec.target)?;
            closest(&|l: &Leg| l.strike.checked_sub(wanted).map(|value| value.abs()))
        }
        "Percent OTM" => {
            let mut wanted = spec.target.checked_div(Decimal::ONE_HUNDRED)?;
            if spec.right == "P" {
                wanted = -wanted;
            }
            closest(&|l: &Leg| l.strike.checked_div(relative)?.checked_sub(Decimal::ONE)?.checked_sub(wanted).map(|value| value.abs()))
        }
        _ => None,
    };
    let chosen = chosen?;
    if !allow_conflict && conflicts.contains(&chosen.strike) {
        log.push(format!(
            "Selected strike {} {} {} {} conflicts with currently open position",
            spec.action,
            fmt_dec(chosen.strike),
            chosen.right,
            chosen.expiry
        ));
        return None;
    }
    Some(chosen.clone())
}

/// Finds the custom trade the template describes in the current chains.
/// Nothing is sent.
#[must_use]
pub fn select_custom(input: &CustomInput<'_>) -> EntryOutcome {
    let t = input.template;
    let symbol = t.underlying_symbol.as_str();
    let mut out = EntryOutcome::default();
    let fail = |mut out: EntryOutcome, text: &str| {
        out.log.push(text.to_string());
        out.preview = text.to_string();
        out
    };
    let (Some(specs), Some(min_entry), Some(max_entry), Some(min_ratio), Some(max_ratio)) =
        (specs(t), dec(t.min_entry_price), dec(t.max_entry_price), dec(t.min_entry_sl_ratio), dec(t.max_entry_sl_ratio))
    else {
        return fail(out, "Trade - template has an invalid number (NaN/infinite)");
    };
    out.log.push(format!("Selection criteria - trade type: {}", t.trade_type));
    for (number, spec) in specs.iter().enumerate().filter(|(_, spec)| spec.action != "None") {
        out.log.push(format!(
            "Trade criteria - Leg {}: {} {} {} {} {} {}",
            number + 1,
            spec.action,
            spec.right,
            spec.ratio,
            fmt_dec(spec.target),
            spec.target_type,
            input.expiries.get(number).map_or("", String::as_str)
        ));
    }
    if input.legs.is_empty() {
        return fail(out, "Trade - No Quotes Found. Please check your connection and market data.");
    }
    if specs.first().is_none_or(|spec| spec.action == "None") {
        return fail(out, "Trade - leg 1 of a custom trade must be bought or sold");
    }
    out.log.push("Selecting the legs".to_string());
    let open = input.open_price.unwrap_or(input.price);
    let mut chosen: Vec<CustomLeg> = Vec::new();
    let mut strikes: [Option<Decimal>; 3] = [None; 3];
    let mut relative = Decimal::ZERO;
    for (number, spec) in specs.iter().enumerate() {
        if spec.action == "None" {
            continue;
        }
        let expiry = input.expiries.get(number).map_or("", String::as_str);
        relative = match spec.target_type {
            "Offset from price" | "Percent OTM" => input.price,
            "Offset from open" => open,
            "Offset from leg 1" => strikes[0].unwrap_or(relative),
            "Offset from leg 2" => strikes[1].unwrap_or(relative),
            "Offset from leg 3" => strikes[2].unwrap_or(relative),
            _ => relative,
        };
        let Some(leg) = select_leg(spec, expiry, relative, &input.legs, symbol, t.exit_flags.allow_strike_conflict, &mut out.log) else {
            let text = format!("No Matching Trade: Leg {} Not Found", number + 1);
            out.log.push(text.clone());
            out.preview = text;
            return out;
        };
        if let Some(slot) = strikes.get_mut(number) {
            *slot = Some(leg.strike);
        }
        let ratio = if spec.action == "Sell" { -spec.ratio.abs().max(1) } else { spec.ratio.abs().max(1) };
        chosen.push(CustomLeg { leg, ratio });
    }
    finish_custom(input, CustomEntry { legs: chosen }, (min_entry, max_entry, min_ratio, max_ratio), out)
}

/// The legs and the mid price as the preview shows them.
fn describe(custom: &CustomEntry, entry: Decimal) -> String {
    let lines: Vec<String> = custom
        .legs
        .iter()
        .enumerate()
        .map(|(number, leg)| {
            format!(
                "Leg {}: {} {} {} {} {}  {}/{}  delta {:.2}",
                number + 1,
                if leg.ratio < 0 { "SELL" } else { "BUY" },
                leg.ratio.abs(),
                leg.leg.expiry,
                fmt_dec(leg.leg.strike),
                leg.leg.right,
                fmt_dec(leg.leg.bid),
                fmt_dec(leg.leg.ask),
                leg.leg.delta
            )
        })
        .collect();
    format!("{}\nMid price: {}\n", lines.join("\n"), fmt_dec(entry))
}

/// The entry price and the short/long ratio against the limits of the template; each miss is logged. A limit given as a negative
/// number is a debit: a negative minimum is the most the debit may cost, a negative maximum the least, and the log names them so.
fn within_limits(entry: Decimal, custom: &CustomEntry, limits: (Decimal, Decimal, Decimal, Decimal), log: &mut Vec<String>) -> bool {
    let (min_entry, max_entry, min_ratio, max_ratio) = limits;
    let mut found = true;
    if min_entry != Decimal::ZERO && entry < min_entry {
        log.push(if min_entry < Decimal::ZERO {
            format!("Trade price ({}) does not meet Max Entry Price: {}", fmt_dec(entry.abs()), fmt_dec(min_entry.abs()))
        } else {
            format!("Trade price ({}) does not meet Min Entry Price: {}", fmt_dec(entry), fmt_dec(min_entry))
        });
        found = false;
    }
    if max_entry != Decimal::ZERO && entry > max_entry {
        log.push(if max_entry < Decimal::ZERO {
            format!("Trade price ({}) does not meet Min Entry Price: {}", fmt_dec(entry.abs()), fmt_dec(max_entry.abs()))
        } else {
            format!("Trade price ({}) does not meet Max Entry Price: {}", fmt_dec(entry), fmt_dec(max_entry))
        });
        found = false;
    }
    let Some((sold, bought)) = custom.short_and_long().filter(|(_, bought)| *bought != Decimal::ZERO).filter(|_| found) else {
        return found;
    };
    let Some(ratio) = sold.checked_div(bought) else { return found };
    log.push(format!("Trade Short Long ratio: {}  (short price: {} long price: {})", fmt_dec(ratio), fmt_dec(sold), fmt_dec(bought)));
    if min_ratio > Decimal::ZERO && ratio < min_ratio {
        log.push(format!("Trade Short Long ratio ({}) does not meet Min ratio: {}", fmt_dec(ratio), fmt_dec(min_ratio)));
        found = false;
    }
    if max_ratio > Decimal::ZERO && ratio > max_ratio {
        log.push(format!("Trade Short Long ratio ({}) does not meet Max ratio: {}", fmt_dec(ratio), fmt_dec(max_ratio)));
        found = false;
    }
    found
}

/// Whether a quote is too old or too wide to trade on while the market is open; each one is logged.
fn quotes_stale(input: &CustomInput<'_>, custom: &CustomEntry, log: &mut Vec<String>) -> bool {
    let symbol = input.template.underlying_symbol.as_str();
    let mut stale = input.underlying_outdated;
    if input.underlying_outdated {
        log.push(format!("{symbol} Outdated Quote"));
    }
    for leg in &custom.legs {
        if leg.leg.data_status(symbol) == DataStatus::OutDated || leg.leg.wide_bid_ask(symbol) {
            log.push(format!("Outdated Quote {} {} {}", fmt_dec(leg.leg.strike), leg.leg.right, leg.leg.expiry));
            stale = true;
        }
    }
    stale
}

/// The selected legs against the limits and the quotes, and the limit price of the order.
fn finish_custom(input: &CustomInput<'_>, custom: CustomEntry, limits: (Decimal, Decimal, Decimal, Decimal), mut out: EntryOutcome) -> EntryOutcome {
    let symbol = input.template.underlying_symbol.as_str();
    let fail = |mut out: EntryOutcome, text: &str| {
        out.log.push(text.to_string());
        out.preview = text.to_string();
        out
    };
    let Some(entry) = custom.mid_price() else {
        return fail(out, "Trade - entry price could not be computed (overflow)");
    };
    out.log.push(format!("Trade Found - Mid Price: {}", fmt_dec(entry)));
    let trade_text = describe(&custom, entry);
    if !within_limits(entry, &custom, limits, &mut out.log) || entry == Decimal::ZERO {
        if entry == Decimal::ZERO {
            out.log.push("Trade cannot be entered for $0".to_string());
        }
        out.log.push("No Matching Trade Found".to_string());
        out.preview = format!("{trade_text}No Matching Trade Found");
        return out;
    }
    out.log.push("Trade - Trade Found".to_string());
    if input.market_open && quotes_stale(input, &custom, &mut out.log) {
        out.log.push("Quotes are out of date: trade not entered.".to_string());
        out.preview = format!("{trade_text}Quotes are out of date: trade not entered.");
        out.custom = Some(custom);
        return out;
    }
    // The limit on the price grid; a test trade moves it away from the market (a debit is priced like a debit type).
    let Some(mut price) = fix_price(entry, symbol, custom.legs.len()) else {
        return fail(out, "Trade - entry price could not be computed (overflow)");
    };
    if input.test_trade {
        let kind = if entry < Decimal::ZERO { "LongCall" } else { "Custom" };
        let Some(moved) = super::test_trade_price(price, kind, symbol, &mut out.log) else {
            return fail(out, "Trade - entry price could not be computed (overflow)");
        };
        price = moved;
    }
    out.entry_price = Some(price);
    out.preview = format!("{symbol}  Price: {}\n{trade_text}", fmt_dec(input.price));
    out.custom = Some(custom);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trade_entry::WorkingSide;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn leg(con_id: i32, right: &str, strike: &str, bid: &str, ask: &str, delta: f64, expiry: &str) -> Leg {
        Leg {
            con_id,
            strike: d(strike),
            right: right.into(),
            expiry: expiry.into(),
            bid: d(bid),
            ask: d(ask),
            delta,
            subscribed: true,
            position: Decimal::ZERO,
            working: WorkingSide::None,
            outdated: false,
        }
    }

    /// Puts 6700 to 6650 and calls 6700 to 6750, bid 0.10 under the ask, today's expiry.
    fn chain() -> Vec<Leg> {
        let mut legs = Vec::new();
        for (k, (strike, ask, delta)) in [
            ("6700", "3.00", -0.30),
            ("6690", "2.10", -0.22),
            ("6680", "1.40", -0.15),
            ("6670", "0.90", -0.10),
            ("6660", "0.55", -0.06),
            ("6650", "0.30", -0.03),
        ]
        .into_iter()
        .enumerate()
        {
            let bid = d(ask) - d("0.10");
            legs.push(leg(100 + i32::try_from(k).unwrap(), "P", strike, &bid.to_string(), ask, delta, "20261118"));
        }
        for (k, (strike, ask, delta)) in
            [("6700", "2.80", 0.28), ("6710", "2.00", 0.20), ("6720", "1.30", 0.14), ("6730", "0.80", 0.09), ("6740", "0.45", 0.05), ("6750", "0.25", 0.03)]
                .into_iter()
                .enumerate()
        {
            let bid = d(ask) - d("0.10");
            legs.push(leg(200 + i32::try_from(k).unwrap(), "C", strike, &bid.to_string(), ask, delta, "20261118"));
        }
        legs
    }

    fn template(legs: [(&str, &str, i32, f64, &str); 4]) -> TradeTemplate {
        let mut t = TradeTemplate { underlying_symbol: "SPX".into(), trade_type: "Custom".into(), ..TradeTemplate::default() };
        let [first, second, third, fourth] = legs;
        (t.custom_leg1_action, t.custom_leg1_put_call, t.custom_leg1_ratio, t.custom_leg1_target, t.custom_leg1_target_type) =
            (first.0.into(), first.1.into(), first.2, first.3, first.4.into());
        (t.custom_leg2_action, t.custom_leg2_put_call, t.custom_leg2_ratio, t.custom_leg2_target, t.custom_leg2_target_type) =
            (second.0.into(), second.1.into(), second.2, second.3, second.4.into());
        (t.custom_leg3_action, t.custom_leg3_put_call, t.custom_leg3_ratio, t.custom_leg3_target, t.custom_leg3_target_type) =
            (third.0.into(), third.1.into(), third.2, third.3, third.4.into());
        (t.custom_leg4_action, t.custom_leg4_put_call, t.custom_leg4_ratio, t.custom_leg4_target, t.custom_leg4_target_type) =
            (fourth.0.into(), fourth.1.into(), fourth.2, fourth.3, fourth.4.into());
        t
    }

    fn input(t: &TradeTemplate, legs: Vec<Leg>) -> CustomInput<'_> {
        CustomInput {
            template: t,
            price: d("6705"),
            open_price: Some(d("6700")),
            legs,
            expiries: ["20261118".into(), "20261118".into(), "20261118".into(), "20261118".into()],
            market_open: false,
            underlying_outdated: false,
            test_trade: false,
        }
    }

    const NONE: (&str, &str, i32, f64, &str) = ("None", "P", 1, 0.0, "Delta");

    #[test]
    fn a_put_spread_is_found_by_delta_and_by_a_distance_from_the_first_leg() {
        // Sell the put nearest 0.22 delta, buy the one 20 points below it.
        let t = template([("Sell", "P", 1, 22.0, "Delta"), ("Buy", "P", 1, -20.0, "Offset from leg 1"), NONE, NONE]);
        let out = select_custom(&input(&t, chain()));
        let custom = out.custom.clone().expect("a trade is found");
        assert_eq!(custom.legs.iter().map(|l| (l.leg.con_id, l.ratio)).collect::<Vec<_>>(), [(101, -1), (103, 1)], "{:?}", out.log);
        // Credit: (2.00 + 2.10) / 2 ... mids are ask - 0.05: 2.05 - 0.85 = 1.20.
        assert_eq!(custom.mid_price(), Some(d("1.20")));
        assert_eq!(out.entry_price, Some(d("1.2")));
        assert_eq!(custom.strikes_text(), "-1 6690P +1 6670P");
        assert!(out.preview.contains("Leg 1: SELL 1 20261118 6690 P"), "{}", out.preview);
    }

    #[test]
    fn a_debit_trade_has_a_negative_entry_price_and_a_four_leg_trade_is_found_leg_by_leg() {
        // Iron condor by strikes from the underlying: sell the put 20 below, buy 10 further, sell the call 20 above, buy 10 further.
        let t = template([
            ("Sell", "P", 1, -20.0, "Offset from price"),
            ("Buy", "P", 1, -10.0, "Offset from leg 1"),
            ("Sell", "C", 1, 20.0, "Offset from price"),
            ("Buy", "C", 1, 10.0, "Offset from leg 3"),
        ]);
        let out = select_custom(&input(&t, chain()));
        let custom = out.custom.expect("found");
        assert_eq!(
            custom.legs.iter().map(|l| (l.leg.right.as_str(), l.leg.strike, l.ratio)).collect::<Vec<_>>(),
            [("P", d("6690"), -1), ("P", d("6680"), 1), ("C", d("6730"), -1), ("C", d("6740"), 1)]
        );
        // A long call only: a debit.
        let long = template([("Buy", "C", 2, 0.0, "Offset from price"), NONE, NONE, NONE]);
        let debit = select_custom(&input(&long, chain()));
        assert_eq!(
            debit.custom.as_ref().and_then(CustomEntry::mid_price),
            Some(d("-3.90")),
            "two contracts of the 6710 call at mid 1.95 (6700 and 6710 are equally near, the higher strike comes first): {:?}",
            debit.log
        );
        assert!(debit.entry_price.is_some_and(|p| p < Decimal::ZERO));
    }

    #[test]
    fn an_option_without_a_delta_yet_does_not_win_a_small_delta_target() {
        // The 6650 put (0.03) has no delta from TWS yet: 0.
        let mut legs = chain();
        for l in &mut legs {
            if l.con_id == 105 {
                l.delta = 0.0;
            }
        }
        let t = template([("Sell", "P", 1, 1.0, "Delta"), NONE, NONE, NONE]);
        let out = select_custom(&input(&t, legs));
        assert_eq!(out.custom.expect("found").legs[0].leg.con_id, 104, "the 6660 put (0.06) is the nearest real delta to 0.01");
    }

    #[test]
    fn premium_and_percentage_targets_find_their_strike() {
        let by_premium = template([("Sell", "C", 1, 1.25, "Fixed premium"), NONE, NONE, NONE]);
        let out = select_custom(&input(&by_premium, chain()));
        assert_eq!(out.custom.unwrap().legs[0].leg.strike, d("6720"), "mid 1.25 is the nearest");
        let by_percent = template([("Sell", "P", 1, 0.3, "Percent OTM"), NONE, NONE, NONE]);
        let out = select_custom(&input(&by_percent, chain()));
        assert_eq!(out.custom.unwrap().legs[0].leg.strike, d("6680"), "0.3 % below 6705 is 6684.9");
    }

    #[test]
    fn a_leg_that_is_not_found_a_conflict_and_the_limits_end_the_selection_with_their_reason() {
        let missing = template([("Sell", "P", 1, 22.0, "Delta"), ("Buy", "P", 1, -500.0, "Offset from leg 1"), NONE, NONE]);
        let out = select_custom(&input(&missing, chain()));
        assert!(out.custom.is_none() && out.preview == "No Matching Trade: Leg 2 Not Found", "{}", out.preview);
        // A bought 6690 put held by the account conflicts with selling it.
        let mut held = chain();
        for l in &mut held {
            if l.con_id == 101 {
                l.position = Decimal::ONE;
            }
        }
        let sell = template([("Sell", "P", 1, 22.0, "Delta"), NONE, NONE, NONE]);
        let out = select_custom(&input(&sell, held.clone()));
        assert!(out.custom.is_none() && out.log.iter().any(|l| l.contains("conflicts with currently open position")), "{:?}", out.log);
        let allowed = TradeTemplate { exit_flags: crate::templates::ExitSwitches { allow_strike_conflict: true, ..sell.exit_flags }, ..sell.clone() };
        assert!(select_custom(&input(&allowed, held)).custom.is_some());
        // Min entry price: the spread brings 1.20, the template wants 1.50.
        let spread =
            TradeTemplate { min_entry_price: 1.5, ..template([("Sell", "P", 1, 22.0, "Delta"), ("Buy", "P", 1, -20.0, "Offset from leg 1"), NONE, NONE]) };
        let out = select_custom(&input(&spread, chain()));
        assert!(out.entry_price.is_none() && out.log.iter().any(|l| l.contains("does not meet Min Entry Price: 1.5")), "{:?}", out.log);
        // Short/long ratio: 2.05 / 0.85 = 2.41; the template allows at most 2.
        let ratio =
            TradeTemplate { max_entry_sl_ratio: 2.0, ..template([("Sell", "P", 1, 22.0, "Delta"), ("Buy", "P", 1, -20.0, "Offset from leg 1"), NONE, NONE]) };
        let out = select_custom(&input(&ratio, chain()));
        assert!(out.entry_price.is_none() && out.log.iter().any(|l| l.contains("does not meet Max ratio")), "{:?}", out.log);
    }

    #[test]
    fn the_buying_power_is_the_worst_loss_at_expiration_and_undefined_risk_has_none() {
        let entry = |legs: Vec<(&str, &str, i32, &str)>| CustomEntry {
            legs: legs
                .into_iter()
                .enumerate()
                .map(|(i, (right, strike, ratio, mid))| CustomLeg {
                    leg: Leg { bid: d(mid), ask: d(mid), ..leg(i32::try_from(i).unwrap(), right, strike, mid, mid, 0.1, "20261118") },
                    ratio,
                })
                .collect(),
        };
        // A put credit spread, 10 wide, credit 2.05 - 0.85 = 1.20: 10 - 1.20 = 8.80 -> 880.
        assert_eq!(entry(vec![("P", "6690", -1, "2.05"), ("P", "6680", 1, "0.85")]).buying_power(), Some(d("880")));
        // An iron condor with two 10-wide wings: the risk is one wing less the whole credit (1.20 + 0.60).
        let condor = entry(vec![("P", "6690", -1, "2.05"), ("P", "6680", 1, "0.85"), ("C", "6720", -1, "1.30"), ("C", "6730", 1, "0.70")]);
        assert_eq!(condor.buying_power(), Some(d("820")), "10 - 1.80");
        // A debit position risks what it cost; a ratio spread with more sold than bought has no defined risk.
        assert_eq!(entry(vec![("C", "6700", 2, "1.95")]).buying_power(), Some(d("390")));
        assert_eq!(entry(vec![("P", "6690", -2, "2.05"), ("P", "6680", 1, "0.85")]).buying_power(), None, "excess short puts");
        assert_eq!(entry(vec![("C", "6700", -1, "2.80")]).buying_power(), None, "a naked call");
        assert_eq!(entry(vec![("P", "6690", -1, "2.05"), ("C", "6720", -1, "1.30"), ("C", "6730", 1, "0.70")]).buying_power(), None, "a naked put");
    }

    #[test]
    fn stale_quotes_abort_while_the_market_is_open_and_a_test_trade_moves_the_limit_away() {
        let t = template([("Sell", "P", 1, 22.0, "Delta"), ("Buy", "P", 1, -20.0, "Offset from leg 1"), NONE, NONE]);
        let mut stale = input(&t, chain());
        stale.market_open = true;
        stale.underlying_outdated = true;
        let out = select_custom(&stale);
        assert!(out.entry_price.is_none() && out.preview.contains("Quotes are out of date"), "{}", out.preview);
        let mut test = input(&t, chain());
        test.test_trade = true;
        let moved = select_custom(&test).entry_price.unwrap();
        assert!(moved > d("1.2"), "a credit is raised so that it does not fill: {moved}");
    }
}
