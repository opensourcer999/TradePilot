//! The stop conversions of Settings > Trading Features and Settings > General > Market Time: the stop of an open credit trade closes
//! other legs as the market moves.
//!
//! - "Convert Vertical Stop to Short": a long the stop sells that has no bid, or one at the price or below, is taken out of the stop;
//!   it would bring next to nothing and can hold up the fill.
//! - "Convert 0 DTE Stops to Short": on the day of the expiration, from the time set on, the stop buys back the shorts only.
//! - "Convert Short Stop to Vertical": a long the template's stop type wants to sell, which was left out because it had no bid (or by the
//!   conversion above), is put back once its bid is 0.10 or more (and above the price of the conversion to Short) - on any day only
//!   before the time of the 0-DTE conversion.
//!
//! The stop keeps its price. A conversion cancels the working stop and, once TWS has confirmed that, places the stop on the new legs
//! (the same path as a stop adjustment, `place_replacement_stop`).

use super::{chain_quote, trade_exit, App, Decimal, Instant, StoredTrade, TimeField};
use std::time::Duration;

/// A trade whose conversion could not be started is not tried again before this pause.
const CONVERSION_RETRY: Duration = Duration::from_secs(30);

/// What the conversions of the settings ask for at this moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ConversionRules {
    /// "Convert Vertical Stop to Short" with its bid price.
    pub vertical_to_short: Option<Decimal>,
    /// "Convert 0 DTE Stops to Short" is on, the trade expires today and the time has come.
    pub zero_dte_short: bool,
    /// "Convert Short Stop to Vertical" is on.
    pub short_to_vertical: bool,
}

/// A leg of a stop as a trade stores it: contract id, ratio, and whether it is sold (a long).
pub(super) type StopLeg = (i32, i32, bool);

/// The bid a long needs before "Convert Short Stop to Vertical" puts it back into the stop.
const BACK_TO_VERTICAL_BID: Decimal = Decimal::from_parts(10, 0, 0, false, 2);

/// The legs a stop is to close from now on, and why; `None` when it stays as it is. `stop_legs` are the legs it closes now and
/// `wanted_longs` the longs the template's stop type sells (contract id, ratio, sold). `quote` is a leg's bid: `None` while the leg
/// is not quoted, `Some(None)` while it is quoted without a bid.
pub(super) fn converted_legs(
    stop_legs: &[StopLeg],
    wanted_longs: &[StopLeg],
    quote: &dyn Fn(i32) -> Option<Option<Decimal>>,
    rules: ConversionRules,
) -> Option<(Vec<StopLeg>, &'static str)> {
    let bid = |con_id: i32| quote(con_id).flatten();
    let is_long = |leg: &(i32, i32, bool)| leg.2;
    let shorts: Vec<(i32, i32, bool)> = stop_legs.iter().filter(|leg| !is_long(leg)).copied().collect();
    if shorts.is_empty() {
        return None;
    }
    if rules.zero_dte_short {
        return stop_legs.iter().any(is_long).then_some((shorts, "0-DTE stop converted to Short"));
    }
    if let Some(price) = rules.vertical_to_short {
        // A quoted long without any bid cannot be sold at all, so it goes as well.
        let cheap = |leg: &(i32, i32, bool)| is_long(leg) && quote(leg.0).is_some_and(|bid| bid.is_none_or(|bid| bid <= price));
        if stop_legs.iter().any(cheap) {
            let kept: Vec<(i32, i32, bool)> = stop_legs.iter().filter(|leg| !cheap(leg)).copied().collect();
            return Some((kept, "Vertical stop converted to Short (the long has no bid, or one at the conversion price or below)"));
        }
    }
    if rules.short_to_vertical {
        // Above the price of the conversion to Short too, or the next look would take it out again.
        let enough = |bid: Decimal| bid >= BACK_TO_VERTICAL_BID && rules.vertical_to_short.is_none_or(|price| bid > price);
        let back: Vec<(i32, i32, bool)> =
            wanted_longs.iter().filter(|leg| !stop_legs.iter().any(|held| held.0 == leg.0) && bid(leg.0).is_some_and(enough)).copied().collect();
        if !back.is_empty() {
            let mut legs = stop_legs.to_vec();
            legs.extend(back);
            return Some((legs, "Short stop converted back to Vertical (the long has a bid again)"));
        }
    }
    None
}

/// Whether the 0-DTE conversion applies to a trade now: switched on, the trade expires `today` (yyyyMMdd, New York) and the New York
/// time `now` is at or past the time of the setting (hour and minute, 24 h).
pub(super) fn zero_dte_due(enabled: bool, expiry: &str, today: &str, now: chrono::NaiveTime, at: (i32, i32)) -> bool {
    let Some(from) = u32::try_from(at.0).ok().zip(u32::try_from(at.1).ok()).and_then(|(hour, minute)| chrono::NaiveTime::from_hms_opt(hour, minute, 0)) else {
        return false;
    };
    enabled && expiry == today && now >= from
}

/// Whether the New York time `now` lies before the time of day `at` (hour and minute, 24 h); an unusable time is never reached.
pub(super) fn before_time_of_day(now: chrono::NaiveTime, at: (i32, i32)) -> bool {
    u32::try_from(at.0)
        .ok()
        .zip(u32::try_from(at.1).ok())
        .and_then(|(hour, minute)| chrono::NaiveTime::from_hms_opt(hour, minute, 0))
        .is_none_or(|from| now < from)
}

/// The price typed for "Convert Vertical to Short": a number from 0 up (a comma taken as the decimal point).
pub(super) fn conversion_price(text: &str) -> Option<Decimal> {
    text.trim().replace(',', ".").parse::<Decimal>().ok().filter(|price| *price >= Decimal::ZERO)
}

impl App {
    /// The conversions of the settings that are switched on, for the trade expiring on `expiry`; New York time `now`.
    fn conversion_rules(&self, expiry: &str, now: chrono::NaiveDateTime) -> ConversionRules {
        let flags = self.settings.convert_flags;
        let at = TimeField {
            hour: self.settings.convert_0dte_hour.clone(),
            minute: self.settings.convert_0dte_minute.clone(),
            pm: self.settings.time_flags.convert_0dte_pm,
        }
        .to_24h();
        let today = now.format("%Y%m%d").to_string();
        let zero_dte_short = zero_dte_due(self.settings.time_flags.convert_0dte_enabled, expiry, &today, now.time(), at);
        ConversionRules {
            vertical_to_short: flags.convert_vertical_to_short_enabled.then(|| conversion_price(&self.settings.convert_vertical_to_short_price)).flatten(),
            zero_dte_short,
            short_to_vertical: flags.convert_short_to_vertical_enabled && before_time_of_day(now.time(), at),
        }
    }

    /// The legs a trade's stop may convert between: the longs its template's stop type sells, as the stop was placed at the fill.
    fn wanted_longs(&self, trade: &StoredTrade) -> Vec<(i32, i32, bool)> {
        let Some(template) = self.template_of(trade) else { return Vec::new() };
        trade_exit::exit_legs(trade, template, &|_| true).iter().map(super::ComboLegSpec::stored).filter(|leg| leg.2).collect()
    }

    /// Looks at the stop of every open credit trade and starts the conversion the settings ask for (see the module). Nothing is touched
    /// while the safety gate is shut or TWS is not connected: a cancelled stop could not be replaced. `now` is New York time.
    pub(super) fn check_stop_conversions(&mut self, now: chrono::NaiveDateTime, instant: Instant) {
        let flags = self.settings.convert_flags;
        let any = flags.convert_vertical_to_short_enabled || flags.convert_short_to_vertical_enabled || self.settings.time_flags.convert_0dte_enabled;
        if !any || !self.trading_allowed() || self.tws_cmd_sender.is_none() {
            return;
        }
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index).cloned() else { continue };
            let Some(adjustments) = trade.adjustments.clone() else { continue };
            let convertible = trade.is_open()
                && trade.price_open > Decimal::ZERO
                && trade.custom_legs.is_none()
                && trade.close_order_id.is_none()
                && !adjustments.replace_pending
                && !self.split_started(trade.order_id)
                && adjustments.rel.is_none()
                && !adjustments.legs.is_empty()
                && (trade.stop_order_id.is_some() || trade.stop_limit_order_id.is_some())
                && trade_exit::StopOrderType::parse(&adjustments.order_type).is_some_and(|kind| kind != trade_exit::StopOrderType::None);
            if !convertible || self.watch.conversion_tried.get(&trade.order_id).is_some_and(|at| instant.saturating_duration_since(*at) < CONVERSION_RETRY) {
                continue;
            }
            let rules = self.conversion_rules(&trade.expiry, now);
            let wanted = self.wanted_longs(&trade);
            let chains = &self.chains;
            // Quoted once an ask or a bid has come; a bid of -1 is none.
            let quote = |con_id: i32| {
                chain_quote(chains, con_id).filter(|q| q.ask.is_some() || q.bid >= Decimal::ZERO).map(|q| Some(q.bid).filter(|bid| *bid >= Decimal::ZERO))
            };
            let Some((legs, reason)) = converted_legs(&adjustments.legs, &wanted, &quote, rules) else { continue };
            self.watch.conversion_tried.insert(trade.order_id, instant);
            self.start_stop_conversion(index, &trade, legs, reason);
        }
    }

    /// Cancels the working stop of the trade at `index` so that it is placed again on `legs`.
    fn start_stop_conversion(&mut self, index: usize, trade: &StoredTrade, legs: Vec<(i32, i32, bool)>, reason: &str) {
        let label = trade.label();
        let Some(mut next) = trade.adjustments.clone() else { return };
        self.log(&format!("{label}: {reason} - the stop at {} is placed again on {} leg(s)", next.stop.as_deref().unwrap_or("-"), legs.len()));
        let mut cancelled = true;
        for order_id in [trade.stop_order_id, trade.stop_limit_order_id].into_iter().flatten() {
            self.log(&format!("Cancel {order_id} (stop conversion)"));
            cancelled &= self.submit_cancel(order_id).is_ok();
        }
        if !cancelled {
            self.log(&format!("{label}: the old stop could not be cancelled - the stop stays as it is"));
            return;
        }
        next.legs = legs;
        next.replace_pending = true;
        let mut updated = trade.clone();
        updated.adjustments = Some(next);
        self.save_trade(index, updated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    const SHORT: (i32, i32, bool) = (1, 1, false);
    const LONG: (i32, i32, bool) = (2, 1, true);

    /// The long quoted with `long` as its bid (`None` = quoted without a bid), every other leg at 1.50.
    fn bids(long: Option<&'static str>) -> impl Fn(i32) -> Option<Option<Decimal>> {
        move |con_id| if con_id == LONG.0 { Some(long.map(d)) } else { Some(Some(d("1.50"))) }
    }

    #[test]
    fn a_vertical_stop_drops_a_long_without_a_bid_or_with_one_at_the_price() {
        let rules = ConversionRules { vertical_to_short: Some(d("0.05")), ..ConversionRules::default() };
        assert_eq!(converted_legs(&[SHORT, LONG], &[LONG], &bids(Some("0.05")), rules).map(|(legs, _)| legs), Some(vec![SHORT]));
        assert_eq!(converted_legs(&[SHORT, LONG], &[LONG], &bids(Some("0.06")), rules), None, "still worth more");
        assert_eq!(converted_legs(&[SHORT, LONG], &[LONG], &bids(None), rules).map(|(legs, _)| legs), Some(vec![SHORT]), "quoted without a bid");
        let unquoted = |con_id: i32| (con_id != LONG.0).then_some(Some(d("1.50")));
        assert_eq!(converted_legs(&[SHORT, LONG], &[LONG], &unquoted, rules), None, "not quoted: nothing is decided on it");
    }

    #[test]
    fn on_the_expiration_day_after_the_time_the_stop_buys_back_the_shorts_only() {
        let rules = ConversionRules { zero_dte_short: true, short_to_vertical: true, ..ConversionRules::default() };
        assert_eq!(converted_legs(&[SHORT, LONG], &[LONG], &bids(Some("1.00")), rules).map(|(legs, _)| legs), Some(vec![SHORT]));
        assert_eq!(converted_legs(&[SHORT], &[LONG], &bids(Some("1.00")), rules), None, "a short stop stays short, even with a bid");
    }

    #[test]
    fn a_short_stop_takes_its_long_back_once_the_bid_is_ten_cents_and_above_the_conversion_price() {
        let rules = ConversionRules { vertical_to_short: Some(d("0.05")), short_to_vertical: true, ..ConversionRules::default() };
        assert_eq!(converted_legs(&[SHORT], &[LONG], &bids(Some("0.10")), rules).map(|(legs, _)| legs), Some(vec![SHORT, LONG]));
        assert_eq!(converted_legs(&[SHORT], &[LONG], &bids(Some("0.09")), rules), None, "below 0.10");
        let high = ConversionRules { vertical_to_short: Some(d("0.20")), short_to_vertical: true, ..ConversionRules::default() };
        assert_eq!(converted_legs(&[SHORT], &[LONG], &bids(Some("0.20")), high), None, "at the price it would only be taken out again");
        assert_eq!(converted_legs(&[SHORT], &[], &bids(Some("0.10")), rules), None, "the template's stop type wants no long");
        let off = ConversionRules { vertical_to_short: Some(d("0.05")), ..ConversionRules::default() };
        assert_eq!(converted_legs(&[SHORT], &[LONG], &bids(Some("0.10")), off), None, "switched off");
    }

    #[test]
    fn a_stop_without_a_short_is_never_converted() {
        let rules = ConversionRules { zero_dte_short: true, ..ConversionRules::default() };
        assert_eq!(converted_legs(&[LONG], &[LONG], &bids(Some("0.01")), rules), None);
    }

    #[test]
    fn the_zero_dte_time_counts_on_the_expiration_day_only() {
        let at = |text: &str| chrono::NaiveTime::parse_from_str(text, "%H:%M").unwrap();
        assert!(zero_dte_due(true, "20261120", "20261120", at("15:00"), (15, 0)));
        assert!(!zero_dte_due(true, "20261120", "20261120", at("14:59"), (15, 0)));
        assert!(!zero_dte_due(true, "20261121", "20261120", at("15:30"), (15, 0)), "expires later");
        assert!(!zero_dte_due(false, "20261120", "20261120", at("15:30"), (15, 0)), "switched off");
        assert!(before_time_of_day(at("14:59"), (15, 0)));
        assert!(!before_time_of_day(at("15:00"), (15, 0)));
        assert_eq!(conversion_price(" 0,05 "), Some(d("0.05")));
        assert_eq!(conversion_price("-1"), None);
    }
}
