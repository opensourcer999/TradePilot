//! Trade entry from a template - the strike selection for Instant Order's PREVIEW. It covers
//! every trade type routed there: IronCondor(-Full), `IronFly`, Put/CallSpread,
//! Put/CallDebitSpread, LongIronCondor-Full, Long/ShortPut, Long/ShortCall, Long/ShortStrangle.
//! Calendars (`select_calendar`) and custom trades (`custom`) have their own selection.
//!
//! Pure trade logic (`coding_guideline.md` section 2): no I/O, no clock - quote age comes in as
//! flags. Prices and strikes are `Decimal` with checked arithmetic (section 3); deltas are greeks,
//! not money, and stay `f64`.
//!
//! The steps, filters and sort keys are applied in a fixed order, including a few quirks (each
//! marked as such). Long strikes are reused: a long the account holds without a market is
//! preferred as the long of a spread (`reusable_long`); selling a long left over by a stop is
//! `App::sell_leftover_longs`. Not implemented yet: this app's own trades of today (extra
//! conflict strikes) and quantity allocation. The opening order (`SellIB`, IB part) is
//! `build_entry_order` below.

use crate::pricing::{dec, fix_price};
use crate::templates::TradeTemplate;
use crate::trade_type::TradeType;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::cmp::Ordering;

pub mod custom;

/// One option of the chain as the selection sees it: an unknown bid or ask
/// is -1.
#[derive(Clone, Debug, PartialEq)]
pub struct Leg {
    /// TWS contract id.
    pub con_id: i32,
    /// Strike price.
    pub strike: Decimal,
    /// "P" or "C".
    pub right: String,
    /// Expiration, yyyyMMdd.
    pub expiry: String,
    /// Bid; -1 = none.
    pub bid: Decimal,
    /// Ask; -1 = none.
    pub ask: Decimal,
    /// TWS model delta (0 = none yet), -1..1.
    pub delta: f64,
    /// A live quote is subscribed (`MarketDataReqID > 0`).
    pub subscribed: bool,
    /// Open position of the trading account in this option (contracts, + long / - short).
    pub position: Decimal,
    /// The order of an open trade of this account that is working on this option in TWS, if any.
    pub working: WorkingSide,
    /// The last quote is older than 120 s (`Timestamp.AddSeconds(120) < Now`).
    pub outdated: bool,
}

/// The side of an order of this account that is working on an option in TWS: IB rejects an order on the other side of the
/// same US option contract (error 201), so such an option cannot be entered in the opposite direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkingSide {
    /// No order is working on the option.
    #[default]
    None,
    /// A buy order works (the exit of a short option).
    Buy,
    /// A sell order works (the exit of a long option).
    Sell,
}

fn minus_one() -> Decimal {
    Decimal::NEGATIVE_ONE
}

impl Leg {
    /// `OptionsContract.Mid`: (bid + ask) / 2 with a real bid, else the ask, else 0.
    #[must_use]
    pub fn mid(&self) -> Option<Decimal> {
        if self.bid > Decimal::ZERO && self.ask > Decimal::new(1, 1) {
            self.bid.checked_add(self.ask)?.checked_div(Decimal::TWO)
        } else if self.ask > Decimal::ZERO {
            Some(self.ask)
        } else {
            Some(Decimal::ZERO)
        }
    }

    /// `OptionsContract.WideBidAsk`.
    #[must_use]
    pub fn wide_bid_ask(&self, symbol: &str) -> bool {
        if self.bid > minus_one() && self.bid.abs() > self.ask.abs() {
            return true;
        }
        if !matches!(symbol, "SPX" | "ES" | "RUT" | "NDX") && self.bid == minus_one() && self.ask > Decimal::new(10, 2) {
            return true;
        }
        let limit = if self.bid > Decimal::from(25) {
            Decimal::from(5)
        } else if self.bid > Decimal::from(10) {
            Decimal::from(3)
        } else {
            Decimal::new(150, 2)
        };
        self.ask.checked_sub(self.bid).is_none_or(|spread| spread.abs() > limit)
    }

    /// `OptionsContract.DataStatus`.
    #[must_use]
    pub fn data_status(&self, symbol: &str) -> DataStatus {
        if !self.subscribed {
            return DataStatus::NotSubscribed;
        }
        let (low, high) = match symbol {
            "SPX" | "RUT" => (Decimal::new(80, 2), Decimal::from(50)),
            "NDX" => (Decimal::new(120, 2), Decimal::from(100)),
            _ => (Decimal::new(10, 2), Decimal::from(5)),
        };
        if (self.bid <= low && self.ask > Decimal::ZERO) || self.bid >= high {
            return DataStatus::LowPrice;
        }
        if self.outdated {
            return DataStatus::OutDated;
        }
        DataStatus::Current
    }
}

/// `OptionsContract.DataStatus`'s values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataStatus {
    /// No live quote subscribed.
    NotSubscribed,
    /// Very cheap or very expensive - its age is not judged.
    LowPrice,
    /// Older than 120 s.
    OutDated,
    /// Fresh.
    Current,
}

/// A vertical: a short and/or a long option of one side.
#[derive(Clone, Debug, PartialEq)]
pub struct Vertical {
    /// The sold option, if any.
    pub short: Option<Leg>,
    /// The bought option, if any.
    pub long: Option<Leg>,
    mid_distance: Decimal,
    otm: i64,
}

/// Sort key for an optional price: a price that could not be computed sorts last.
fn key(price: Option<Decimal>) -> Decimal {
    price.unwrap_or(Decimal::MIN)
}

impl Vertical {
    /// A vertical from its legs (`new VerticalTrade(shortLeg, longLeg)`).
    #[must_use]
    pub fn new(short: Option<Leg>, long: Option<Leg>) -> Self {
        Self { short, long, mid_distance: Decimal::ZERO, otm: 0 }
    }

    /// `VerticalTrade.Price`: credit (short - long), or a single leg's price. `None` only on
    /// arithmetic overflow.
    #[must_use]
    pub fn price(&self) -> Option<Decimal> {
        match (&self.short, &self.long) {
            (Some(s), Some(l)) => s.mid()?.checked_sub(l.mid()?),
            (None, Some(l)) => {
                if l.ask > Decimal::new(5, 1) {
                    l.mid()
                } else {
                    Some(l.ask)
                }
            }
            (Some(s), None) => s.mid(),
            (None, None) => Some(Decimal::ZERO),
        }
    }

    /// `VerticalTrade.ShortPrice`.
    pub fn short_price(&self) -> Option<Decimal> {
        self.short.as_ref().map_or(Some(Decimal::ZERO), Leg::mid)
    }

    /// `VerticalTrade.MidPrice`.
    #[must_use]
    pub fn mid_price(&self) -> Option<Decimal> {
        match (&self.short, &self.long) {
            (Some(s), Some(l)) => s.mid()?.checked_sub(l.mid()?),
            (None, Some(l)) => l.mid(),
            (Some(s), None) => s.mid(),
            (None, None) => Some(Decimal::ZERO),
        }
    }

    /// The short's delta, else the long's.
    #[must_use]
    pub fn delta(&self) -> f64 {
        self.short.as_ref().or(self.long.as_ref()).map_or(0.0, |l| l.delta)
    }

    fn strike_sort(&self) -> Decimal {
        self.short.as_ref().or(self.long.as_ref()).map_or(Decimal::ZERO, |l| l.strike)
    }

    /// Distance between the strikes (0 for a single leg).
    #[must_use]
    pub fn width(&self) -> Decimal {
        match (&self.short, &self.long) {
            (Some(s), Some(l)) => s.strike.checked_sub(l.strike).map_or(Decimal::MAX, |w| w.abs()),
            _ => Decimal::ZERO,
        }
    }

    /// "P" or "C".
    #[must_use]
    pub fn put_call(&self) -> &str {
        self.short.as_ref().or(self.long.as_ref()).map_or("", |l| l.right.as_str())
    }

    fn reused_long(&self) -> u8 {
        match &self.long {
            Some(l) if l.ask == Decimal::ZERO && l.position > Decimal::ZERO => 1,
            _ => 0,
        }
    }

    fn wide_bid_ask(&self, symbol: &str) -> bool {
        self.short.as_ref().is_some_and(|l| l.wide_bid_ask(symbol)) || self.long.as_ref().is_some_and(|l| l.wide_bid_ask(symbol))
    }

    /// `VerticalTrade.DataStatus`: the long's status, unless it is outdated - then the short's.
    fn data_status(&self, symbol: &str) -> DataStatus {
        let mut status = self.long.as_ref().map_or(DataStatus::Current, |l| l.data_status(symbol));
        if status != DataStatus::OutDated {
            if let Some(s) = &self.short {
                status = s.data_status(symbol);
            }
        }
        status
    }

    fn same_short(&self, other: &Self) -> bool {
        match (&self.short, &other.short) {
            (Some(a), Some(b)) => a.con_id == b.con_id && a.strike == b.strike && a.right == b.right,
            (None, None) => true,
            _ => false,
        }
    }

    /// The same vertical with long and short swapped (debit spreads).
    fn flipped(&self) -> Self {
        Self::new(self.long.clone(), self.short.clone())
    }
}

/// What PREVIEW shows: the chosen put/call verticals, the entry price and the log.
#[derive(Debug, Default)]
pub struct EntryOutcome {
    /// The chosen put side.
    pub put: Option<Vertical>,
    /// The chosen call side.
    pub call: Option<Vertical>,
    /// Limit price on the tick grid, after the Test Trade shift; `None` = no trade.
    pub entry_price: Option<Decimal>,
    /// The "Trade Details" text.
    pub preview: String,
    /// The trade log lines, in order.
    pub log: Vec<String>,
    /// The legs of a custom trade (the put and call sides are empty then).
    pub custom: Option<custom::CustomEntry>,
}

/// Everything the selection looks at.
pub struct EntryInput<'a> {
    /// The template to trade.
    pub template: &'a TradeTemplate,
    /// The underlying's current price (`Price[symbol]`).
    pub price: Decimal,
    /// Today's opening price, for `Offset from open` (`SPXOpen`); falls back to `price`.
    pub open_price: Option<Decimal>,
    /// Every option of the template's symbol and expiry.
    pub legs: Vec<Leg>,
    /// US regular session - quote checks only apply then.
    pub market_open: bool,
    /// The underlying's price is older than 60 s.
    pub underlying_outdated: bool,
    /// Instant Order's "Test Trade?": prices the order so that it does not fill.
    pub test_trade: bool,
}

/// The minimum option price - the "narrower long" candidates cost this.
fn close_adjustment(symbol: &str) -> Decimal {
    if matches!(symbol, "SPX" | "RUT" | "ES" | "NDX") {
        Decimal::new(5, 2)
    } else {
        Decimal::new(1, 2)
    }
}

fn fmt_dec(v: Decimal) -> String {
    v.normalize().to_string()
}

fn fmt_opt(v: Option<Decimal>) -> String {
    v.map_or_else(|| "?".to_string(), fmt_dec)
}

fn fmt_date(yyyymmdd: &str) -> String {
    chrono::NaiveDate::parse_from_str(yyyymmdd, "%Y%m%d").map_or_else(|_| yyyymmdd.to_string(), crate::dates::show_date)
}

fn cmp_f(a: f64, b: f64) -> Ordering {
    a.total_cmp(&b)
}

/// Sorted by price descending, then by width, the most common order.
fn by_price_desc_width(list: &mut [Vertical]) {
    list.sort_by(|a, b| key(b.price()).cmp(&key(a.price())).then(a.width().cmp(&b.width())));
}

/// The sides' targets and preferences, already normalized.
struct SideTarget<'a> {
    preference: &'a str,
    target_type: &'a str,
    min: Decimal,
    max: Decimal,
}

/// Sorts the candidate trades by the template's preference.
fn sort_trades(mut trades: Vec<Vertical>, side: &SideTarget, price: Decimal, open: Decimal) -> Vec<Vertical> {
    let target_type = side.target_type;
    let premium_like = matches!(target_type, "Premium" | "Short premium" | "Offset from open" | "Offset from price" | "Percent OTM");
    let by_reused = |a: &Vertical, b: &Vertical| b.reused_long().cmp(&a.reused_long());
    match side.preference {
        "Highest premium" => {
            if premium_like {
                trades.sort_by(|a, b| key(b.price()).cmp(&key(a.price())).then(by_reused(a, b)).then(a.width().cmp(&b.width())));
            }
            if target_type == "Delta" {
                trades.sort_by(|a, b| cmp_f(b.delta().abs(), a.delta().abs()).then(by_reused(a, b)).then(a.width().cmp(&b.width())));
            }
        }
        "Middle premium" => {
            let half = side.max.checked_add(side.min).and_then(|s| s.checked_div(Decimal::TWO)).unwrap_or(Decimal::ZERO);
            let distance = |value: Option<Decimal>, target: Decimal| value.and_then(|v| v.checked_sub(target)).map_or(Decimal::MAX, |d| d.abs());
            match target_type {
                "Premium" => trades.iter_mut().for_each(|t| t.mid_distance = distance(t.price(), half)),
                "Short premium" => trades.iter_mut().for_each(|t| t.mid_distance = distance(t.short_price(), half)),
                "Delta" => {
                    let mid = if half > Decimal::ONE { half.checked_div(Decimal::ONE_HUNDRED).unwrap_or(half) } else { half };
                    for t in &mut trades {
                        t.mid_distance = distance(dec(t.delta().abs()), mid);
                    }
                }
                "Offset from open" | "Offset from price" | "Percent OTM" => {
                    let base = if target_type == "Offset from open" { open } else { price };
                    if target_type == "Percent OTM" {
                        let pct = half.checked_div(Decimal::ONE_HUNDRED).unwrap_or(Decimal::ZERO);
                        let above = Decimal::ONE.checked_add(pct).and_then(|f| base.checked_mul(f)).unwrap_or(base);
                        let below = Decimal::ONE.checked_sub(pct).and_then(|f| base.checked_mul(f)).unwrap_or(base);
                        for t in &mut trades {
                            let target = if t.put_call() == "P" { below } else { above };
                            t.mid_distance = distance(Some(t.strike_sort()), target);
                        }
                    } else {
                        let offset = if side.min == side.max { side.min } else { half };
                        let target = base.checked_add(offset).unwrap_or(base);
                        for t in &mut trades {
                            t.mid_distance = distance(Some(t.strike_sort()), target);
                        }
                    }
                }
                _ => {}
            }
            trades.sort_by(|a, b| {
                a.mid_distance.cmp(&b.mid_distance).then(by_reused(a, b)).then(key(b.price()).cmp(&key(a.price()))).then(a.width().cmp(&b.width()))
            });
        }
        "Lowest premium" => {
            if premium_like {
                trades.sort_by(|a, b| key(a.price()).cmp(&key(b.price())).then(by_reused(a, b)).then(a.width().cmp(&b.width())));
            }
            if target_type == "Delta" {
                trades.sort_by(|a, b| {
                    cmp_f(a.delta().abs(), b.delta().abs()).then(by_reused(a, b)).then(a.width().cmp(&b.width())).then(key(a.price()).cmp(&key(b.price())))
                });
            }
        }
        "Wider Spread" | "Widest" => {
            trades.sort_by(|a, b| b.width().cmp(&a.width()).then(by_reused(a, b)).then(key(b.price()).cmp(&key(a.price()))));
        }
        "Narrowest" | "Same width" => {
            trades.sort_by(|a, b| a.width().cmp(&b.width()).then(by_reused(a, b)).then(key(b.price()).cmp(&key(a.price()))));
        }
        "Furthest OTM" | "Nearest OTM" => {
            for t in &mut trades {
                // Rounds half to even.
                t.otm = price
                    .checked_sub(t.strike_sort())
                    .map(|d| d.abs().round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointNearestEven))
                    .and_then(|d| d.to_i64())
                    .unwrap_or(i64::MAX);
            }
            let most = side.preference == "Furthest OTM";
            trades.sort_by(|a, b| {
                let by_otm = if most { b.otm.cmp(&a.otm) } else { a.otm.cmp(&b.otm) };
                by_otm.then(by_reused(a, b)).then(key(b.price()).cmp(&key(a.price()))).then(a.width().cmp(&b.width()))
            });
        }
        _ => {}
    }
    trades
}

/// The legs as they are listed in the trade details.
fn display_trade(call: Option<&Vertical>, put: Option<&Vertical>) -> String {
    let mut text = String::new();
    for (label, v) in [("CALL:", call), ("PUT: ", put)] {
        let Some(v) = v else { continue };
        let line = match (&v.short, &v.long) {
            (Some(s), Some(l)) => {
                format!("{label}{} {} {} / {} {}", fmt_opt(v.price()), fmt_dec(s.strike), fmt_date(&s.expiry), fmt_dec(l.strike), fmt_date(&l.expiry))
            }
            (Some(s), None) => format!("{label}{} {} {} ", fmt_opt(v.price()), fmt_dec(s.strike), fmt_date(&s.expiry)),
            (None, Some(l)) => format!("{label}{} {} {}", fmt_opt(v.price()), fmt_dec(l.strike), fmt_date(&l.expiry)),
            (None, None) => continue,
        };
        text.push_str(&line);
        text.push('\n');
    }
    text
}

fn parse_widths(text: &str) -> Vec<Decimal> {
    text.split(',').map(str::trim).filter(|s| !s.is_empty()).filter_map(|s| s.parse::<Decimal>().ok()).collect()
}

/// `TradeTemplate.IsDebit` for a trade type (see `TradeType::is_debit`).
#[must_use]
pub fn is_debit(trade_type: &str) -> bool {
    crate::trade_type::is_debit(trade_type)
}

/// The buying power one contract of the selected trade needs, for what has a defined risk: a trade bought
/// for a debit needs what it costs (the price times 100); a credit vertical or iron condor needs the wider
/// side minus the credit, times 100. `None` for a trade that has no such figure (a single short option, a
/// strangle without long legs) - the same as "not worked out" for the percent-of-net-liquidation quantity.
#[must_use]
pub fn buying_power_per_contract(trade_type: &str, outcome: &EntryOutcome) -> Option<Decimal> {
    let price = outcome.entry_price?.abs();
    if let Some(custom) = &outcome.custom {
        // The worst loss at the expiration of the legs; `None` for a position whose risk is not defined.
        return custom.buying_power();
    }
    let risk = if is_debit(trade_type) {
        price
    } else {
        let mut widest = Decimal::ZERO;
        for side in [outcome.put.as_ref(), outcome.call.as_ref()].into_iter().flatten() {
            if side.short.is_none() || side.long.is_none() {
                return None;
            }
            widest = widest.max(side.width());
        }
        widest.checked_sub(price)?
    };
    risk.checked_mul(Decimal::ONE_HUNDRED).filter(|power| *power > Decimal::ZERO)
}
/// The trade types that are entered through the tranche entry (see
/// `TradeType::uses_tranche_entry`; "Straddle" is not among them - its entry returns
/// without entering anything).
pub fn uses_tranche_entry(trade_type: &str) -> bool {
    crate::trade_type::is(trade_type, TradeType::uses_tranche_entry)
}

/// The template's numbers as decimals (stored as `f64` in the template's JSON format).
struct Numbers {
    target_min: Decimal,
    target_max: Decimal,
    target_min_call: Option<Decimal>,
    target_max_call: Option<Decimal>,
    min_otm: Decimal,
    min_otm_call: Option<Decimal>,
    long_max_premium: Decimal,
    long_min_premium: Decimal,
    long_max_premium_call: Decimal,
    long_min_premium_call: Decimal,
    long_max_width: Option<Decimal>,
    long_max_width_call: Option<Decimal>,
    min_entry: Decimal,
    max_entry: Decimal,
}

fn numbers(t: &TradeTemplate) -> Option<Numbers> {
    let opt = |v: Option<f64>| -> Option<Option<Decimal>> { v.map_or(Some(None), |x| dec(x).map(Some)) };
    let cent = Decimal::new(1, 2);
    Some(Numbers {
        target_min: dec(t.target_min)?,
        target_max: dec(t.target_max)?,
        target_min_call: opt(t.target_min_call)?,
        target_max_call: opt(t.target_max_call)?,
        min_otm: dec(t.min_otm)?,
        min_otm_call: opt(t.min_otm_call)?,
        long_max_premium: opt(t.long_max_premium)?.unwrap_or(cent),
        long_min_premium: opt(t.long_min_premium)?.unwrap_or(cent),
        long_max_premium_call: opt(t.long_max_premium_call)?.unwrap_or(cent),
        long_min_premium_call: opt(t.long_min_premium_call)?.unwrap_or(cent),
        long_max_width: opt(t.long_max_width)?,
        long_max_width_call: opt(t.long_max_width_call)?,
        min_entry: dec(t.min_entry_price)?,
        max_entry: dec(t.max_entry_price)?,
    })
}

/// `x / 100` for delta targets given in percent (`if TargetMin >= 1 { /= 100 }`).
fn percent(x: Decimal) -> Decimal {
    x.checked_div(Decimal::ONE_HUNDRED).unwrap_or(x)
}

fn in_range(v: Option<Decimal>, lo: Decimal, hi: Decimal) -> bool {
    v.is_some_and(|v| v >= lo && v <= hi)
}

fn no_trade(mut out: EntryOutcome, reason: &str) -> EntryOutcome {
    out.log.push(reason.to_string());
    out.preview = "No Matching Trade Found".into();
    out.put = None;
    out.call = None;
    out.entry_price = None;
    out
}

/// Whether the entry price is inside the Min and Max Entry Price of the template and not 0; says why not in the log.
fn entry_allowed(entry: Decimal, n: &Numbers, log: &mut Vec<String>) -> bool {
    if n.min_entry > Decimal::ZERO && entry.abs() < n.min_entry {
        log.push(format!("Trade ({}) does not meet Min Entry Price: {}", fmt_dec(entry.abs()), fmt_dec(n.min_entry)));
        return false;
    }
    if n.max_entry > Decimal::ZERO && entry.abs() > n.max_entry {
        log.push(format!("Trade ({}) does not meet Max Entry Price: {}", fmt_dec(entry.abs()), fmt_dec(n.max_entry)));
        return false;
    }
    if entry == Decimal::ZERO {
        log.push("Trade cannot be entered for $0".into());
        return false;
    }
    true
}

/// The combined price on the single-leg grid (`fix_price` default), shifted for a test trade; `None` on an overflow.
fn grid_entry_price(entry: Decimal, input: &EntryInput, log: &mut Vec<String>) -> Option<Decimal> {
    let t = input.template;
    let entry = fix_price(entry, &t.underlying_symbol, 1)?;
    if input.test_trade {
        test_trade_price(entry, &t.trade_type, &t.underlying_symbol, log)
    } else {
        Some(entry)
    }
}
/// Finds the trade the template describes in the
/// current chain. Nothing is sent. The steps, each its own function below: the options of each
/// side (`side_options`), the verticals built from them (`side_verticals`), each side's target
/// filter (`put_targets`, `call_targets`), the filters over all trades (`filter_trades`), the
/// order of preference and the chosen sides (`choose_sides`), the entry price (`entry_of`) and
/// the quote checks while the market is open (`quote_problems`).
pub fn select_trade(input: &EntryInput) -> EntryOutcome {
    let t = input.template;
    let symbol = t.underlying_symbol.as_str();
    let trade_type = t.trade_type.as_str();
    let kind = |property: fn(TradeType) -> bool| crate::trade_type::is(trade_type, property);
    let price = input.price;
    let open = input.open_price.unwrap_or(price);
    let mut out = EntryOutcome::default();
    let Some(n) = numbers(t) else {
        return no_trade(out, "Trade - template has an invalid number (NaN/infinite)");
    };
    let (min_otm, min_otm_call) = min_otm_of(trade_type, symbol, &n);
    out.log.push(format!(
        "Selection criteria - trade type: {trade_type} Put Target: {} Min: {} Max: {} preference: {} Long Type: {} Call Target: {} Min: {} Max: {} preference: {} Long Type: {}",
        t.target_type,
        fmt_dec(n.target_min),
        fmt_dec(n.target_max),
        t.preference,
        t.long_type,
        t.target_type_call,
        fmt_opt(n.target_min_call),
        fmt_opt(n.target_max_call),
        t.preference_call,
        t.long_type_call
    ));
    let positions = OpenPositions::of(&input.legs);
    if input.legs.is_empty() {
        return no_trade(out, "Trade - No Quotes Found. Please check your connection and market data.");
    }

    let options = side_options(input, t, symbol, min_otm, min_otm_call);
    out.log.push(format!("Trade - Load Contracts   Puts: {}  Calls: {}", options.puts.len(), options.calls.len()));
    let no_delta = input.legs.iter().filter(|l| l.subscribed && l.ask > minus_one() && l.delta == 0.0).count();
    if no_delta > 0 {
        out.log.push(format!("Trade - {no_delta} subscribed option(s) have no delta yet - a delta target cannot count them"));
    }
    let mut max_width = Decimal::ZERO;
    let mut candidates: Vec<Vertical> = Vec::new();
    let put_longs =
        LongChoice { long_type: &t.long_type, widths: parse_widths(&t.long_width), min_premium: n.long_min_premium, max_premium: n.long_max_premium };
    side_candidates(trade_type, &options.puts, &options.narrow_puts, &put_longs, true, &mut max_width, &mut candidates);
    out.log.push(format!("Trade - Load Contracts Puts Complete ({} possible trades)", candidates.len()));
    let call_longs = LongChoice {
        long_type: &t.long_type_call,
        widths: parse_widths(&t.long_width_call),
        min_premium: n.long_min_premium_call,
        max_premium: n.long_max_premium_call,
    };
    side_candidates(trade_type, &options.calls, &options.narrow_calls, &call_longs, false, &mut max_width, &mut candidates);
    out.log.push(format!("Trade - Load Contracts Call Complete ({} possible trades)", candidates.len()));

    // The strike the offset/percentage targets refer to: the long for debit trades, else the short.
    let use_long_leg = is_debit(trade_type) && !kind(TradeType::is_debit_spread);
    let mut put = put_targets(&candidates, t, &n, price, open, use_long_leg, &mut out.log);
    let mut call = call_targets(&candidates, t, &n, price, open, use_long_leg, &mut out.log);
    if t.long_type_call == "Premium cap" {
        if let Some(limit) = n.long_max_width_call {
            call.list.retain(|v| v.width() <= limit);
            by_price_desc_width(&mut call.list);
            max_width = max_width.max(limit);
            out.log.push(format!("Trade - Filter Call Trades on max premium/max width complete ({} possible call trades)", call.list.len()));
        }
    }
    if t.long_type == "Premium cap" {
        if let Some(limit) = n.long_max_width {
            put.list.retain(|v| v.width() <= limit);
            by_price_desc_width(&mut put.list);
            max_width = max_width.max(limit);
            out.log.push(format!("Trade - Filter Put Trades on max premium/max width complete ({} possible put trades)", put.list.len()));
        }
    }
    // The widest long (`num4`) only matters to the long reuse, which is not implemented yet.
    let _ = max_width;

    let mut trades: Vec<Vertical> = put.list.clone();
    trades.extend(call.list.iter().cloned());
    let trades = filter_trades(trades, t, &n, symbol, price, (min_otm, min_otm_call), &positions, put.target_type, &mut out.log);
    let trades = without_working_order_conflicts(trades, &mut out.log);
    let (put_leg, call_leg) = choose_sides(trades, t, &n, price, open, &put, &call);
    let Some((put_leg, call_leg, entry)) = entry_of(trade_type, t, put_leg, call_leg) else {
        return no_trade(out, "No Matching Trade Found");
    };
    let Some(entry) = entry else {
        return no_trade(out, "Trade - entry price could not be computed (overflow)");
    };
    let trade_text = display_trade(call_leg.as_ref(), put_leg.as_ref());
    if !entry_allowed(entry, &n, &mut out.log) {
        return no_trade(out, "No Matching Trade Found");
    }
    out.log.push("Trade - Trade Found".into());

    if input.market_open && quote_problems(input, trade_type, symbol, put_leg.as_ref(), call_leg.as_ref(), &mut out.log) {
        out.log.push("Quotes are out of date or too wide: trade not entered.".into());
        out.preview = format!("{trade_text}Quotes are out of date or too wide: trade not entered.");
        out.put = put_leg;
        out.call = call_leg;
        return out;
    }

    let Some(entry) = grid_entry_price(entry, input, &mut out.log) else {
        return no_trade(out, "Trade - entry price could not be computed (overflow)");
    };
    out.entry_price = Some(entry);
    out.put = put_leg;
    out.call = call_leg;
    out.preview = format!("{symbol}  Price: {}\n{trade_text}", fmt_dec(price));
    out
}
/// Min OTM of the put and the call side; an Iron Fly without one sits just inside the money.
fn min_otm_of(trade_type: &str, symbol: &str, n: &Numbers) -> (Decimal, Option<Decimal>) {
    let (mut min_otm, mut min_otm_call) = (n.min_otm, n.min_otm_call);
    if trade_type == "IronFly" {
        let default = if matches!(symbol, "SPX" | "ES") { Decimal::new(-25, 1) } else { Decimal::new(-5, 1) };
        if min_otm == Decimal::ZERO {
            min_otm = default;
        }
        if min_otm_call.is_none_or(|v| v == Decimal::ZERO) {
            min_otm_call = Some(default);
        }
    }
    (min_otm, min_otm_call)
}

/// The strikes this account already holds in this expiry, short and long, per side.
struct OpenPositions {
    short_puts: Vec<Decimal>,
    short_calls: Vec<Decimal>,
    long_puts: Vec<Decimal>,
    long_calls: Vec<Decimal>,
}

impl OpenPositions {
    fn of(legs: &[Leg]) -> Self {
        let strikes_where = |right: &str, short: bool| -> Vec<Decimal> {
            legs.iter().filter(|l| l.right == right && if short { l.position < Decimal::ZERO } else { l.position > Decimal::ZERO }).map(|l| l.strike).collect()
        };
        Self {
            short_puts: strikes_where("P", true),
            short_calls: strikes_where("C", true),
            long_puts: strikes_where("P", false),
            long_calls: strikes_where("C", false),
        }
    }
}

/// The options each side can use: OTM by at least Min OTM, quoted, subscribed, no wide bid/ask
/// (puts from the highest strike down, calls from the lowest up), and per side up to five
/// "narrower long" candidates - the cheapest options - unless the template disables them.
struct SideOptions {
    puts: Vec<Leg>,
    calls: Vec<Leg>,
    narrow_puts: Vec<Leg>,
    narrow_calls: Vec<Leg>,
}

fn side_options(input: &EntryInput, t: &TradeTemplate, symbol: &str, min_otm: Decimal, min_otm_call: Option<Decimal>) -> SideOptions {
    let price = input.price;
    let usable = |q: &&Leg| q.ask > minus_one() && q.subscribed && !q.wide_bid_ask(symbol);
    let put_limit = price.checked_sub(min_otm);
    let mut puts: Vec<Leg> = input.legs.iter().filter(|q| q.right == "P" && put_limit.is_some_and(|lim| q.strike <= lim)).filter(usable).cloned().collect();
    puts.sort_by_key(|p| std::cmp::Reverse(p.strike));
    // A call side without Min OTM (null) has no candidates.
    let call_limit = min_otm_call.and_then(|m| price.checked_add(m));
    let mut calls: Vec<Leg> = input.legs.iter().filter(|q| q.right == "C" && call_limit.is_some_and(|lim| q.strike >= lim)).filter(usable).cloned().collect();
    calls.sort_by_key(|c| c.strike);
    let min_price = close_adjustment(symbol);
    let cheapest = |list: &[Leg], disabled: bool| -> Vec<Leg> {
        if disabled {
            Vec::new()
        } else {
            list.iter().filter(|o| o.ask == min_price).take(5).cloned().collect()
        }
    };
    let narrow_puts = cheapest(&puts, t.long_flags.disable_narrower_long);
    let narrow_calls = cheapest(&calls, t.long_flags.disable_narrower_long_call);
    SideOptions { puts, calls, narrow_puts, narrow_calls }
}

/// How the long of a side's verticals is chosen (`long_type`): at a fixed width, or by premium.
struct LongChoice<'a> {
    long_type: &'a str,
    widths: Vec<Decimal>,
    min_premium: Decimal,
    max_premium: Decimal,
}

/// A long this account holds that has no market any more but can be reused.
fn reusable_long(q: &Leg) -> bool {
    q.ask == Decimal::ZERO && q.bid == minus_one() && q.position > Decimal::ZERO
}

/// The candidates of one side (`put` or call) for the trade type: verticals for the types built
/// from them, single longs or single shorts for the others.
fn side_candidates(trade_type: &str, options: &[Leg], narrow: &[Leg], longs: &LongChoice, put: bool, max_width: &mut Decimal, candidates: &mut Vec<Vertical>) {
    let kind = |property: fn(TradeType) -> bool| crate::trade_type::is(trade_type, property);
    let side = if put { TradeType::has_put_side } else { TradeType::has_call_side };
    if kind(if put { TradeType::builds_put_verticals } else { TradeType::builds_call_verticals }) {
        side_verticals(options, narrow, longs, put, max_width, candidates);
    }
    if kind(TradeType::buys_only) && kind(side) {
        candidates.extend(options.iter().map(|o| Vertical::new(None, Some(o.clone()))));
    }
    if kind(TradeType::sells_only) && kind(side) {
        candidates.extend(options.iter().map(|o| Vertical::new(Some(o.clone()), None)));
    }
}

/// The verticals of one side: each option as the short, with a long further out of the money -
/// at each width (and the narrower cheap longs within the widest width so far), or by premium
/// (up to 15 longs, the nearest first). "Further out" is down for puts, up for calls; `max_width`
/// grows with the widths seen and carries over from the put side to the call side.
fn side_verticals(options: &[Leg], narrow: &[Leg], longs: &LongChoice, put: bool, max_width: &mut Decimal, candidates: &mut Vec<Vertical>) {
    // How far `to` lies out of the money from `from` (positive = further out).
    let outward = |from: Decimal, to: Decimal| if put { from.checked_sub(to) } else { to.checked_sub(from) };
    for short in options {
        if longs.long_type == "Width" {
            for &w in &longs.widths {
                *max_width = (*max_width).max(w);
                let long_strike = if put { short.strike.checked_sub(w) } else { short.strike.checked_add(w) };
                let at_width = options.iter().find(|o| Some(o.strike) == long_strike && o.ask >= Decimal::ZERO);
                for cheap in narrow {
                    let narrower = match at_width {
                        Some(o) => outward(cheap.strike, o.strike).is_some_and(|d| d > Decimal::ZERO),
                        None => outward(short.strike, cheap.strike).is_some_and(|d| d <= *max_width),
                    };
                    if narrower && outward(short.strike, cheap.strike).is_some_and(|d| d > Decimal::ZERO) {
                        candidates.push(Vertical::new(Some(short.clone()), Some(cheap.clone())));
                    }
                }
                if let Some(o) = at_width {
                    candidates.push(Vertical::new(Some(short.clone()), Some(o.clone())));
                }
            }
        }
        if longs.long_type == "Premium cap" {
            let mut chosen: Vec<&Leg> = options.iter().filter(|o| (o.ask <= longs.max_premium && o.ask >= longs.min_premium) || reusable_long(o)).collect();
            if put {
                chosen.sort_by_key(|l| std::cmp::Reverse(l.strike));
            } else {
                chosen.sort_by_key(|l| l.strike);
            }
            for l in chosen.into_iter().take(15) {
                if outward(short.strike, l.strike).is_some_and(|d| d > Decimal::ZERO) {
                    candidates.push(Vertical::new(Some(short.clone()), Some(l.clone())));
                }
            }
        }
    }
}

/// The candidates of one side (`right`) that `keep` accepts, highest price first.
fn kept_side(candidates: &[Vertical], right: &str, keep: &dyn Fn(&Vertical) -> bool) -> Vec<Vertical> {
    let mut kept: Vec<Vertical> = candidates.iter().filter(|v| v.put_call() == right && keep(v)).cloned().collect();
    by_price_desc_width(&mut kept);
    kept
}

/// The strike a strike target refers to: the long for a debit trade (`use_long_leg`), else the short.
fn target_leg_strike(v: &Vertical, use_long_leg: bool) -> Option<Decimal> {
    if use_long_leg {
        v.long.as_ref().map(|l| l.strike)
    } else {
        v.short.as_ref().map(|s| s.strike)
    }
}

fn abs_delta(v: &Vertical) -> Option<Decimal> {
    dec(v.delta().abs())
}

/// One side after its target filter: the trades left, the target type and the bounds as used
/// (a delta target given in percent is turned into a fraction).
struct SideTargets<'a> {
    list: Vec<Vertical>,
    target_type: &'a str,
    min: Decimal,
    max: Decimal,
}

/// The put side's target filter (`target_type`, `target_min`/`target_max`).
fn put_targets<'a>(
    candidates: &[Vertical],
    t: &'a TradeTemplate,
    n: &Numbers,
    price: Decimal,
    open: Decimal,
    use_long_leg: bool,
    log: &mut Vec<String>,
) -> SideTargets<'a> {
    let target_type = if t.target_type.is_empty() { "Premium" } else { t.target_type.as_str() };
    let (mut target_min, mut target_max) = (n.target_min, n.target_max);
    let mut put_list: Vec<Vertical> = candidates.iter().filter(|v| v.put_call() == "P").cloned().collect();
    let put_not_mid = target_min != target_max || t.preference != "Middle premium";
    match target_type {
        "Premium" if put_not_mid => {
            put_list = kept_side(candidates, "P", &|v| in_range(v.price(), target_min, target_max));
            log.push(format!("Trade - Filter Put Trades on premium complete ({} possible put trades)", put_list.len()));
        }
        "Short premium" if put_not_mid => {
            put_list = candidates.iter().filter(|v| v.put_call() == "P" && in_range(v.short_price(), target_min, target_max)).cloned().collect();
            put_list.sort_by(|a, b| key(b.short_price()).cmp(&key(a.short_price())).then(a.width().cmp(&b.width())));
            log.push(format!("Trade - Filter Put Trades on short premium complete ({} possible put trades)", put_list.len()));
        }
        "Delta" if put_not_mid => {
            if target_min >= Decimal::ONE {
                target_min = percent(target_min);
            }
            if target_max > Decimal::ONE {
                target_max = percent(target_max);
            }
            put_list = candidates.iter().filter(|v| v.put_call() == "P" && in_range(abs_delta(v), target_min, target_max)).cloned().collect();
            put_list.sort_by(|a, b| cmp_f(b.delta(), a.delta()).then(a.width().cmp(&b.width())));
            log.push(format!("Trade - Filter Put Trades on delta complete ({} possible put trades)", put_list.len()));
        }
        "Offset from open" | "Offset from price" if target_min != target_max => {
            let base = if target_type == "Offset from open" { open } else { price };
            let (lo, hi) = (base.checked_add(target_min), base.checked_add(target_max));
            put_list = kept_side(candidates, "P", &|v| match (lo, hi) {
                (Some(lo), Some(hi)) => in_range(target_leg_strike(v, use_long_leg), lo, hi),
                _ => false,
            });
            log.push(format!("Trade - Filter Put Trades on {target_type} complete ({} possible put trades)", put_list.len()));
        }
        "Percent OTM" if target_min != target_max => {
            let factor = |pct: Decimal| Decimal::ONE.checked_sub(percent(pct)).and_then(|f| price.checked_mul(f));
            // The bounds get Target Min/Max added once more.
            let lo = factor(target_max).and_then(|v| v.checked_add(target_min));
            let hi = factor(target_min).and_then(|v| v.checked_add(target_max));
            put_list = kept_side(candidates, "P", &|v| match (lo, hi) {
                (Some(lo), Some(hi)) => in_range(target_leg_strike(v, use_long_leg), lo, hi),
                _ => false,
            });
            log.push(format!("Trade - Filter Put Trades on {target_type} complete ({} possible put trades)", put_list.len()));
        }
        _ => {}
    }
    SideTargets { list: put_list, target_type, min: target_min, max: target_max }
}

/// The call side's target filter (`target_type_call`, `target_min_call`/`target_max_call`); a call
/// target that is not set (null) accepts nothing, as every comparison with it is false.
fn call_targets<'a>(
    candidates: &[Vertical],
    t: &'a TradeTemplate,
    n: &Numbers,
    price: Decimal,
    open: Decimal,
    use_long_leg: bool,
    log: &mut Vec<String>,
) -> SideTargets<'a> {
    let target_type_call = if t.target_type_call.is_empty() { "Premium" } else { t.target_type_call.as_str() };
    let call_bounds = n.target_min_call.zip(n.target_max_call);
    let (mut target_min_call, mut target_max_call) = call_bounds.unwrap_or((Decimal::ZERO, Decimal::ZERO));
    let call_equal_bounds = call_bounds.is_some_and(|(lo, hi)| lo == hi);
    let call_not_mid = !call_equal_bounds || t.preference_call != "Middle premium";
    let mut call_list: Vec<Vertical> = candidates.iter().filter(|v| v.put_call() == "C").cloned().collect();
    let bounds_set = call_bounds.is_some();
    match target_type_call {
        "Premium" if call_not_mid => {
            call_list = kept_side(candidates, "C", &|v| bounds_set && in_range(v.price(), target_min_call, target_max_call));
            log.push(format!("Trade - Filter Call Trades on premium complete ({} possible call trades)", call_list.len()));
        }
        "Short premium" if call_not_mid => {
            call_list = candidates
                .iter()
                .filter(|v| bounds_set && v.put_call() == "C" && in_range(v.short_price(), target_min_call, target_max_call))
                .cloned()
                .collect();
            call_list.sort_by(|a, b| key(b.short_price()).cmp(&key(a.short_price())).then(a.width().cmp(&b.width())));
            log.push(format!("Trade - Filter Call Trades on short premium complete ({} possible call trades)", call_list.len()));
        }
        "Delta" if call_not_mid => {
            if target_min_call >= Decimal::ONE {
                target_min_call = percent(target_min_call);
            }
            if target_max_call > Decimal::ONE {
                target_max_call = percent(target_max_call);
            }
            call_list =
                candidates.iter().filter(|v| bounds_set && v.put_call() == "C" && in_range(abs_delta(v), target_min_call, target_max_call)).cloned().collect();
            call_list.sort_by(|a, b| cmp_f(b.delta(), a.delta()).then(a.width().cmp(&b.width())));
            log.push(format!("Trade - Filter Call Trades on delta complete ({} possible call trades)", call_list.len()));
        }
        "Offset from open" | "Offset from price" if !call_equal_bounds => {
            let base = if target_type_call == "Offset from open" { open } else { price };
            let (lo, hi) = (base.checked_add(target_min_call), base.checked_add(target_max_call));
            call_list = kept_side(candidates, "C", &|v| match (bounds_set, lo, hi) {
                (true, Some(lo), Some(hi)) => in_range(target_leg_strike(v, use_long_leg), lo, hi),
                _ => false,
            });
            log.push(format!("Trade - Filter Call Trades on {target_type_call} complete ({} possible call trades)", call_list.len()));
        }
        "Percent OTM" if !call_equal_bounds => {
            let factor = |pct: Decimal| Decimal::ONE.checked_add(percent(pct)).and_then(|f| price.checked_mul(f));
            let lo = factor(target_min_call).and_then(|v| v.checked_add(target_min_call));
            let hi = factor(target_max_call).and_then(|v| v.checked_add(target_max_call));
            call_list = kept_side(candidates, "C", &|v| match (bounds_set, lo, hi) {
                (true, Some(lo), Some(hi)) => in_range(target_leg_strike(v, use_long_leg), lo, hi),
                _ => false,
            });
            log.push(format!("Trade - Filter Call Trades on {target_type_call} complete ({} possible call trades)", call_list.len()));
        }
        _ => {}
    }
    SideTargets { list: call_list, target_type: target_type_call, min: target_min_call, max: target_max_call }
}

/// The filters over the trades of both sides: Min/Max Entry Price (single-side types), the
/// Iron Fly's short strike, strikes already held (unless conflicts are allowed) and, for a
/// premium target with a highest/lowest preference, one trade per short strike and price.
#[allow(clippy::too_many_arguments)]
fn filter_trades(
    mut trades: Vec<Vertical>,
    t: &TradeTemplate,
    n: &Numbers,
    symbol: &str,
    price: Decimal,
    (min_otm, min_otm_call): (Decimal, Option<Decimal>),
    positions: &OpenPositions,
    target_type: &str,
    log: &mut Vec<String>,
) -> Vec<Vertical> {
    let trade_type = t.trade_type.as_str();
    let kind = |property: fn(TradeType) -> bool| crate::trade_type::is(trade_type, property);
    if matches!(
        trade_type,
        "PutSpread" | "ShortCall" | "LongCall" | "ShortPut" | "PutCalendar" | "PutDebitSpread" | "LongPut" | "CallCalendar" | "CallSpread" | "CallDebitSpread"
    ) {
        if n.min_entry > Decimal::ZERO {
            trades.retain(|v| v.price().is_some_and(|p| p >= n.min_entry));
            log.push(format!("Trade - Filter Trades on min entry price complete ({} possible trades)", trades.len()));
        }
        if n.max_entry > Decimal::ZERO {
            trades.retain(|v| v.price().is_some_and(|p| p <= n.max_entry));
            log.push(format!("Trade - Filter Trades on max entry price complete ({} possible trades)", trades.len()));
        }
    }
    if trade_type == "IronFly" {
        let two = Decimal::TWO;
        let center = if min_otm_call == Some(min_otm) {
            price.checked_mul(two)
        } else {
            price.checked_sub(min_otm).zip(min_otm_call.and_then(|m| m.checked_add(price))).and_then(|(a, b)| a.checked_add(b))
        };
        // Rounds half to even.
        let even = |d: Decimal| d.round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointNearestEven);
        let short_strike = center.and_then(|c| {
            if matches!(symbol, "ES" | "SPX") {
                c.checked_div(Decimal::TEN).map(even).and_then(|r| r.checked_mul(Decimal::from(5)))
            } else {
                c.checked_div(two).map(even)
            }
        });
        trades.retain(|v| v.short.as_ref().is_some_and(|s| Some(s.strike) == short_strike));
        by_price_desc_width(&mut trades);
        log.push(format!("Trade - Filter Trades on IronFly short strike complete ({} possible trades)", trades.len()));
    }
    if !t.exit_flags.allow_strike_conflict {
        let p = positions;
        let has = |list: &[Decimal], leg: &Option<Leg>, right: &str| leg.as_ref().is_some_and(|l| l.right == right && list.contains(&l.strike));
        if kind(TradeType::is_debit_spread) {
            trades.retain(|x| {
                !(has(&p.long_puts, &x.long, "P")
                    || has(&p.long_calls, &x.long, "C")
                    || has(&p.short_puts, &x.short, "P")
                    || has(&p.short_calls, &x.short, "C"))
            });
        } else {
            if !kind(TradeType::sells_only) {
                // Never buy what is short already...
                trades.retain(|x| !(has(&p.short_puts, &x.long, "P") || has(&p.short_calls, &x.long, "C")));
            }
            if !kind(TradeType::buys_only) {
                // ...nor sell what is long already.
                trades.retain(|x| !(has(&p.long_puts, &x.short, "P") || has(&p.long_calls, &x.short, "C")));
            }
        }
        log.push(format!("Trade - Filter Trades on open positions complete ({} possible trades)", trades.len()));
    }
    // Same short strike and same price: keep only the first (the narrowest after the sort).
    if target_type == "Premium" && matches!(t.preference.as_str(), "Highest premium" | "Lowest premium") {
        let mut unique: Vec<Vertical> = Vec::new();
        for v in trades {
            if unique.last().is_none_or(|prev: &Vertical| !v.same_short(prev) || v.price() != prev.price()) {
                unique.push(v);
            }
        }
        trades = unique;
        log.push(format!("Trade - Filter Trades on same short strike and price complete ({} possible trades)", trades.len()));
    }
    log.push(format!(
        "Trade - {} possible Call trades, {} possible Put trades",
        trades.iter().filter(|v| v.put_call() == "C").count(),
        trades.iter().filter(|v| v.put_call() == "P").count()
    ));
    trades
}

/// The trades in the order of each side's preference, and the put and call side chosen from them.
fn choose_sides(
    mut trades: Vec<Vertical>,
    t: &TradeTemplate,
    n: &Numbers,
    price: Decimal,
    open: Decimal,
    put: &SideTargets,
    call: &SideTargets,
) -> (Option<Vertical>, Option<Vertical>) {
    let trade_type = t.trade_type.as_str();
    let kind = |property: fn(TradeType) -> bool| crate::trade_type::is(trade_type, property);
    let preference = if !t.preference.is_empty() {
        t.preference.as_str()
    } else if t.list_flags.lower_target {
        "Lowest premium"
    } else {
        "Highest premium"
    };
    let put_side = SideTarget { preference, target_type: put.target_type, min: put.min, max: put.max };
    let call_side = SideTarget { preference: t.preference_call.as_str(), target_type: call.target_type, min: call.min, max: call.max };
    let same_targets =
        trade_type == "IronCondor" && preference == t.preference_call && n.target_min_call == Some(n.target_min) && n.target_max_call == Some(n.target_max);
    if same_targets {
        trades = sort_trades(trades, &put_side, price, open);
    } else {
        let puts_sorted = sort_trades(trades.iter().filter(|v| v.put_call() == "P").cloned().collect(), &put_side, price, open);
        // Without a call preference the call list keeps its earlier order.
        let calls_sorted = if t.preference_call.is_empty() {
            call.list.clone()
        } else {
            sort_trades(trades.iter().filter(|v| v.put_call() == "C").cloned().collect(), &call_side, price, open)
        };
        let both = !puts_sorted.is_empty() && !calls_sorted.is_empty();
        trades = puts_sorted;
        trades.extend(calls_sorted);
        if preference == "Same width" && t.preference_call == "Same width" && both {
            trades = sort_trades(trades, &put_side, price, open);
        }
    }

    let mut put_leg: Option<Vertical> = None;
    let mut call_leg: Option<Vertical> = None;
    let ic = matches!(trade_type, "IronCondor" | "IronCondor-Full");
    if put.target_type == "Premium" && same_targets && matches!(preference, "Highest premium" | "Lowest premium") && ic {
        for v in &trades {
            let needs_put = put_leg.is_none() && kind(TradeType::builds_put_verticals) && !is_debit(trade_type);
            let needs_call = call_leg.is_none() && kind(TradeType::builds_call_verticals) && !is_debit(trade_type);
            if !(needs_put || needs_call) {
                break;
            }
            // Every further put replaces the previous one until a call comes.
            match v.put_call() {
                "P" => put_leg = Some(v.clone()),
                "C" => call_leg = Some(v.clone()),
                _ => {}
            }
        }
    } else if preference == "Same width" && ic {
        for v in &trades {
            match v.put_call() {
                "P" => put_leg = Some(v.clone()),
                "C" => call_leg = Some(v.clone()),
                _ => {}
            }
            if put_leg.as_ref().zip(call_leg.as_ref()).is_some_and(|(p, c)| p.width() == c.width()) {
                break;
            }
        }
    } else {
        put_leg = trades.iter().find(|v| v.put_call() == "P").cloned();
        call_leg = trades.iter().find(|v| v.put_call() == "C").cloned();
    }
    (put_leg, call_leg)
}

/// The entry price of the chosen sides for the trade type: `None` when the type found no trade;
/// else the sides as entered (a debit spread's vertical is flipped: bought long, sold short) and
/// the price (`None` inside on overflow).
#[allow(clippy::type_complexity)]
fn entry_of(
    trade_type: &str,
    t: &TradeTemplate,
    mut put: Option<Vertical>,
    mut call: Option<Vertical>,
) -> Option<(Option<Vertical>, Option<Vertical>, Option<Decimal>)> {
    let mut found = false;
    let mut entry: Option<Decimal> = None;
    if matches!(trade_type, "IronCondor" | "IronCondor-Full" | "IronFly" | "ShortStrangle" | "LongStrangle") {
        if let (Some(c), Some(p)) = (&call, &put) {
            found = true;
            entry = if t.call_ratio > 1 || t.put_ratio > 1 {
                let c_part = c.price().and_then(|v| v.checked_mul(Decimal::from(t.call_ratio)));
                let p_part = p.price().and_then(|v| v.checked_mul(Decimal::from(t.put_ratio)));
                c_part.zip(p_part).and_then(|(a, b)| a.checked_add(b))
            } else {
                c.price().zip(p.price()).and_then(|(a, b)| a.checked_add(b))
            };
        }
    }
    if matches!(trade_type, "PutSpread" | "LongPut" | "ShortPut") {
        if let Some(p) = &put {
            found = true;
            entry = if trade_type == "LongPut" { p.mid_price() } else { p.price() };
        }
    }
    if matches!(trade_type, "CallSpread" | "LongCall" | "ShortCall") {
        if let Some(c) = &call {
            found = true;
            entry = if trade_type == "LongCall" { c.mid_price() } else { c.price() };
        }
    }
    if trade_type == "PutDebitSpread" {
        if let Some(p) = put.take() {
            let flipped = p.flipped();
            entry = flipped.price();
            put = Some(flipped);
            found = true;
        }
    }
    if trade_type == "CallDebitSpread" {
        if let Some(c) = call.take() {
            let flipped = c.flipped();
            entry = flipped.price();
            call = Some(flipped);
            found = true;
        }
    }
    if trade_type == "LongIronCondor-Full" {
        if let (Some(p), Some(c)) = (&put, &call) {
            let (fp, fc) = (p.flipped(), c.flipped());
            entry = fc.price().zip(fp.price()).and_then(|(a, b)| a.checked_add(b));
            put = Some(fp);
            call = Some(fc);
            found = true;
        }
    }
    found.then_some((put, call, entry))
}

/// The quote checks while the market is open: an outdated underlying, an outdated quote of a
/// side's relevant leg, a wide bid/ask. Each problem is logged; `true` when there is any.
fn quote_problems(input: &EntryInput, trade_type: &str, symbol: &str, put: Option<&Vertical>, call: Option<&Vertical>, log: &mut Vec<String>) -> bool {
    let mut abort = false;
    if input.underlying_outdated {
        log.push(format!("{symbol} Outdated Quote"));
        abort = true;
    }
    let short_side_types = matches!(
        trade_type,
        "IronCondor" | "PutSpread" | "CallSpread" | "IronFly" | "IronCondor-Full" | "DoubleCalendar" | "ShortStrangle" | "ShortPut" | "ShortCall"
    );
    let long_side_types = crate::trade_type::is(trade_type, TradeType::closes_longs_always);
    for (name, v) in [("Put", put), ("Call", call)] {
        let Some(v) = v else { continue };
        let leg_present = if short_side_types { v.short.is_some() } else { long_side_types && v.long.is_some() };
        if leg_present && v.data_status(symbol) == DataStatus::OutDated {
            log.push(format!("{name} Outdated Quote"));
            abort = true;
        }
        if v.wide_bid_ask(symbol) {
            log.push(format!("Wide Bid/Ask on {name}"));
            abort = true;
        }
    }
    abort
}

/// Instant Order's "Test Trade?": moves the limit away from the market so the order does not
/// fill (a credit up, a debit down). `None` only on arithmetic overflow; a symbol without a known
/// step keeps its price.
fn test_trade_price(entry: Decimal, trade_type: &str, symbol: &str, log: &mut Vec<String>) -> Option<Decimal> {
    log.push("TEST TRADE MODE - Uncheck 'Test Trade' on the Instant Trade page to use live pricing.".into());
    let debit_type = matches!(
        trade_type,
        "DoubleCalendar"
            | "PutDebitSpread"
            | "CallCalendar"
            | "LongStrangle"
            | "PutCalendar"
            | "LongCall"
            | "LongPut"
            | "CallDebitSpread"
            | "LongIronCondor-Full"
    );
    let step = if matches!(symbol, "SPX" | "ES" | "NDX") {
        Some((Decimal::from(5), Decimal::new(10, 2)))
    } else if matches!(symbol, "SPY" | "QQQ" | "XSP" | "AAPL" | "MSFT" | "IWM" | "GLD" | "TSLA" | "NVDA" | "AMZN" | "GOOGL" | "META" | "IBIT" | "RUT" | "SPCX")
    {
        Some((Decimal::new(5, 1), Decimal::new(1, 2)))
    } else {
        None
    };
    let Some((shift, floor)) = step else { return Some(entry) };
    if !debit_type {
        entry.checked_add(shift)
    } else if entry < Decimal::ZERO {
        if entry < -shift {
            entry.checked_add(shift)
        } else {
            Some(-floor)
        }
    } else if entry > shift {
        entry.checked_sub(shift)
    } else {
        Some(floor)
    }
}

// ------------------------------------------------------------------------------------------
// Calendars: CallCalendar, PutCalendar and DoubleCalendar.
// ------------------------------------------------------------------------------------------

/// The calendar trade types (`TradeType::is_calendar`).
pub fn uses_calendar_entry(trade_type: &str) -> bool {
    crate::trade_type::is(trade_type, TradeType::is_calendar)
}

/// The four expirations (yyyyMMdd) a calendar needs: the sold near leg and the bought far leg of
/// each side. A side the trade type does not use is ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalendarExpiries {
    /// Expiration of the sold put (`short_put_dte`).
    pub short_put: String,
    /// Expiration of the bought put (`long_put_dte`).
    pub long_put: String,
    /// Expiration of the sold call (`short_call_dte`).
    pub short_call: String,
    /// Expiration of the bought call (`long_call_dte`).
    pub long_call: String,
}

/// Everything the calendar selection looks at: like [`EntryInput`], but `legs` holds the options
/// of every expiration in `expiries`.
pub struct CalendarInput<'a> {
    /// The template to trade.
    pub template: &'a TradeTemplate,
    /// The underlying's current price (`Price[symbol]`).
    pub price: Decimal,
    /// Every option of the template's symbol in the expirations of `expiries`.
    pub legs: Vec<Leg>,
    /// The expirations of the four legs.
    pub expiries: CalendarExpiries,
    /// US regular session - quote checks only apply then.
    pub market_open: bool,
    /// The underlying's price is older than 60 s.
    pub underlying_outdated: bool,
    /// Instant Order's "Test Trade?".
    pub test_trade: bool,
}

/// A candidate's distance from the target (`MidDistance`) is below this for a Delta target.
fn delta_window() -> Decimal {
    Decimal::new(7, 2)
}

/// Finds the calendar the template describes in
/// the current chains - a near leg is sold and the far leg at the strike `target` points above it
/// is bought, per side. Nothing is sent.
///
/// Deliberate behaviour: the open-position conflict check compares the sold strikes with
/// the positions of the far expiration and the bought strikes with those of the near expiration.
#[must_use]
pub fn select_calendar(input: &CalendarInput) -> EntryOutcome {
    let t = input.template;
    let symbol = t.underlying_symbol.as_str();
    let trade_type = t.trade_type.as_str();
    let price = input.price;
    let ex = &input.expiries;
    let mut out = EntryOutcome::default();
    let (puts, calls) = match trade_type {
        "DoubleCalendar" => (true, true),
        "PutCalendar" => (true, false),
        "CallCalendar" => (false, true),
        other => return no_trade(out, &format!("Trade - {other} is not a calendar")),
    };
    let Some(numbers) = calendar_numbers(t) else {
        return no_trade(out, "Trade - template has an invalid number (NaN/infinite)");
    };
    let CalendarNumbers { short_put_target, long_put_target, short_call_target, long_call_target, min_entry, max_entry, min_ratio, max_ratio } = numbers;
    out.log.push(calendar_criteria(t, &numbers));

    if input.legs.is_empty() {
        return no_trade(out, "Trade - No Quotes Found. Please check your connection and market data.");
    }

    let positions = calendar_positions(&input.legs, ex);

    let CalendarLists { put_shorts, put_longs, call_shorts, call_longs } = calendar_lists(&input.legs, ex);
    out.log.push(format!(
        "Trade - Load Contracts   Short Puts: {}  Short Calls: {}  Long Puts: {}  Long Calls: {}",
        put_shorts.len(),
        call_shorts.len(),
        put_longs.len(),
        call_longs.len()
    ));
    if (calls && (call_shorts.len() < 5 || call_longs.len() < 5)) || (puts && (put_shorts.len() < 5 || put_longs.len() < 5)) {
        out.log.push("Trade - Missing quotes detected. Refreshing market data. Fewer than 5 strikes loaded".into());
        out.preview = "No Matching Trade Found (fewer than 5 strikes loaded - please wait for the quotes and preview again)".into();
        return out;
    }

    let mut candidates: Vec<Vertical> = Vec::new();
    if puts && t.long_put_target_type == "Spread width" {
        candidates.extend(calendar_pairs(&put_shorts, &put_longs, long_put_target));
    }
    out.log.push(format!("Trade - Load Contracts Puts Complete ({} possible trades)", candidates.len()));
    if calls && t.long_call_target_type == "Spread width" {
        candidates.extend(calendar_pairs(&call_shorts, &call_longs, long_call_target));
    }
    out.log.push(format!("Trade - Load Contracts Call Complete ({} possible trades)", candidates.len()));
    if !t.exit_flags.allow_strike_conflict {
        drop_conflicting_calendars(&mut candidates, &positions, &mut out.log);
        candidates = without_working_order_conflicts(candidates, &mut out.log);
    }
    out.log.push(format!("Trade - Filter Trades on open positions complete ({} possible trades)", candidates.len()));
    out.log.push(format!(
        "Trade - {} possible Call trades, {} possible Put trades",
        candidates.iter().filter(|v| v.put_call() == "C").count(),
        candidates.iter().filter(|v| v.put_call() == "P").count()
    ));

    rank_calendar_candidates(&mut candidates, t, price, short_put_target, short_call_target, &mut out.log);
    let put = candidates.iter().find(|v| v.put_call() == "P").cloned();
    let call = candidates.iter().find(|v| v.put_call() == "C").cloned();
    out.log.push("Selecting the trade".into());

    // The side(s) the trade type needs must be there; the entry price is their sum (negative = debit).
    let sides: Vec<&Vertical> = match (trade_type, &put, &call) {
        ("DoubleCalendar", Some(p), Some(c)) => vec![c, p],
        ("PutCalendar", Some(p), _) => vec![p],
        ("CallCalendar", _, Some(c)) => vec![c],
        _ => Vec::new(),
    };
    let trade_text = display_trade(call.as_ref().filter(|_| calls), put.as_ref().filter(|_| puts));
    let sum = |f: &dyn Fn(&Vertical) -> Option<Decimal>| sides.iter().try_fold(Decimal::ZERO, |acc, v| acc.checked_add(f(v)?));
    let (Some(entry), Some(short_sum), Some(long_sum)) =
        (sum(&Vertical::price), sum(&Vertical::short_price), sum(&|v: &Vertical| v.long.as_ref().map_or(Some(Decimal::ZERO), Leg::mid)))
    else {
        return no_trade(out, "Trade - entry price could not be computed (overflow)");
    };
    let mut found = !sides.is_empty();
    found &= calendar_within_limits(entry, short_sum, long_sum, (min_entry, max_entry, min_ratio, max_ratio), &mut out.log);
    if !found {
        return no_trade(out, "No Matching Trade Found");
    }
    if entry == Decimal::ZERO {
        out.log.push("Trade cannot be entered for $0".into());
        return no_trade(out, "No Matching Trade Found");
    }
    out.log.push("Trade - Trade Found".into());

    // ---- Quote checks while the market is open ----
    if input.market_open && calendar_quotes_bad(input, put.as_ref().filter(|_| puts), call.as_ref().filter(|_| calls), &mut out.log) {
        out.log.push("Quotes are out of date: trade not entered.".into());
        out.preview = format!("{trade_text}Quotes are out of date: trade not entered.");
        out.put = put.filter(|_| puts);
        out.call = call.filter(|_| calls);
        return out;
    }

    let Some(entry) = calendar_entry_price(entry, input, &mut out.log) else {
        return no_trade(out, "Trade - entry price could not be computed (overflow)");
    };
    out.entry_price = Some(entry);
    out.put = put.filter(|_| puts);
    out.call = call.filter(|_| calls);
    out.preview = format!("{symbol}  Price: {}\n{trade_text}", fmt_dec(price));
    out
}

/// The quoted, subscribed options of each leg's expiration.
struct CalendarLists<'a> {
    put_shorts: Vec<&'a Leg>,
    put_longs: Vec<&'a Leg>,
    call_shorts: Vec<&'a Leg>,
    call_longs: Vec<&'a Leg>,
}

/// The quoted, subscribed options of each leg's expiration, puts from the highest strike down,
/// calls from the lowest strike up.
fn calendar_lists<'a>(legs: &'a [Leg], ex: &CalendarExpiries) -> CalendarLists<'a> {
    let list = |right: &str, expiry: &str| -> Vec<&'a Leg> {
        let mut found: Vec<&Leg> = legs.iter().filter(|l| l.right == right && l.expiry == expiry && l.ask > minus_one() && l.subscribed).collect();
        found.sort_by(|a, b| if right == "P" { b.strike.cmp(&a.strike) } else { a.strike.cmp(&b.strike) });
        found
    };
    CalendarLists {
        put_shorts: list("P", &ex.short_put),
        put_longs: list("P", &ex.long_put),
        call_shorts: list("C", &ex.short_call),
        call_longs: list("C", &ex.long_call),
    }
}
/// The line of the log that says what the template asks for.
fn calendar_criteria(t: &TradeTemplate, n: &CalendarNumbers) -> String {
    format!(
        "Trade criteria ({}) - Trade Type: {} Short Call: {} {} Short Put: {} {} Long Call: {} {} Long Put: {} {}",
        t.underlying_symbol,
        t.trade_type,
        fmt_dec(n.short_call_target),
        t.short_call_target_type,
        fmt_dec(n.short_put_target),
        t.short_put_target_type,
        fmt_dec(n.long_call_target),
        t.long_call_target_type,
        fmt_dec(n.long_put_target),
        t.long_put_target_type
    )
}
/// The numbers of the template a calendar needs: the targets of the four legs and the limits of the entry.
struct CalendarNumbers {
    short_put_target: Decimal,
    long_put_target: Decimal,
    short_call_target: Decimal,
    long_call_target: Decimal,
    min_entry: Decimal,
    max_entry: Decimal,
    min_ratio: Decimal,
    max_ratio: Decimal,
}

/// The numbers of the template; `None` when one of them is not a finite number.
fn calendar_numbers(t: &TradeTemplate) -> Option<CalendarNumbers> {
    Some(CalendarNumbers {
        short_put_target: dec(t.short_put_target)?,
        long_put_target: dec(t.long_put_target)?,
        short_call_target: dec(t.short_call_target)?,
        long_call_target: dec(t.long_call_target)?,
        min_entry: dec(t.min_entry_price)?,
        max_entry: dec(t.max_entry_price)?,
        min_ratio: dec(t.min_entry_sl_ratio)?,
        max_ratio: dec(t.max_entry_sl_ratio)?,
    })
}

/// Open positions of the account (strikes only), see the quirk of `select_calendar`.
fn calendar_positions(legs: &[Leg], ex: &CalendarExpiries) -> CalendarPositions {
    let strikes_where = |right: &str, expiry: &str, short: bool| -> Vec<Decimal> {
        legs.iter()
            .filter(|l| l.right == right && l.expiry == expiry && if short { l.position < Decimal::ZERO } else { l.position > Decimal::ZERO })
            .map(|l| l.strike)
            .collect()
    };
    CalendarPositions {
        short_put: strikes_where("P", &ex.long_put, true),
        short_call: strikes_where("C", &ex.long_call, true),
        long_put: strikes_where("P", &ex.short_put, false),
        long_call: strikes_where("C", &ex.short_call, false),
    }
}

/// Every near leg with a bid, paired with the far leg `Spread width` points above its strike.
fn calendar_pairs(shorts: &[&Leg], longs: &[&Leg], width: Decimal) -> Vec<Vertical> {
    shorts
        .iter()
        .filter(|s| s.bid > Decimal::ZERO)
        .filter_map(|s| {
            let target = s.strike.checked_add(width)?;
            let long = longs.iter().find(|l| l.strike == target && l.ask >= Decimal::ZERO)?;
            Some(Vertical::new(Some((*s).clone()), Some((*long).clone())))
        })
        .collect()
}

/// The entry price on the price grid, shifted for a test trade; `None` on an overflow.
fn calendar_entry_price(entry: Decimal, input: &CalendarInput, log: &mut Vec<String>) -> Option<Decimal> {
    let t = input.template;
    let entry = fix_price(entry, &t.underlying_symbol, 1)?;
    if input.test_trade {
        test_trade_price(entry, &t.trade_type, &t.underlying_symbol, log)
    } else {
        Some(entry)
    }
}
/// Drops the candidates that would sell an option a buy order of this account works on, or buy one a sell order works on:
/// TWS would reject them with error 201.
fn without_working_order_conflicts(mut candidates: Vec<Vertical>, log: &mut Vec<String>) -> Vec<Vertical> {
    candidates.retain(|v| {
        let sells_into_a_buy = v.short.as_ref().is_some_and(|leg| leg.working == WorkingSide::Buy);
        let buys_into_a_sell = v.long.as_ref().is_some_and(|leg| leg.working == WorkingSide::Sell);
        !sells_into_a_buy && !buys_into_a_sell
    });
    log.push(format!("Trade - Filter Trades on working orders complete ({} possible trades)", candidates.len()));
    candidates
}

/// The sold strikes of the open positions (see the quirk of `select_calendar)`: what a candidate must not collide with.
struct CalendarPositions {
    short_put: Vec<Decimal>,
    short_call: Vec<Decimal>,
    long_put: Vec<Decimal>,
    long_call: Vec<Decimal>,
}

/// Drops the candidates whose strikes collide with an open position of the account.
fn drop_conflicting_calendars(candidates: &mut Vec<Vertical>, positions: &CalendarPositions, log: &mut Vec<String>) {
    log.push(format!("Trade - Filter Trades on open positions precheck ({} possible trades)", candidates.len()));
    candidates.retain(|v| {
        let long_blocked =
            v.long.as_ref().is_some_and(|l| if l.right == "P" { positions.short_put.contains(&l.strike) } else { positions.short_call.contains(&l.strike) });
        let short_blocked =
            v.short.as_ref().is_some_and(|s| if s.right == "P" { positions.long_put.contains(&s.strike) } else { positions.long_call.contains(&s.strike) });
        !long_blocked && !short_blocked
    });
}

/// Measures the distance of each candidate's sold strike from the target of its side and orders the
/// candidates: closest first, then the cheapest debit, then the narrowest.
fn rank_calendar_candidates(
    candidates: &mut Vec<Vertical>,
    t: &TradeTemplate,
    price: Decimal,
    short_put_target: Decimal,
    short_call_target: Decimal,
    log: &mut Vec<String>,
) {
    // The distance of each candidate's sold strike from the template's target. A Delta target is
    // given in percent; a put's delta is negative, so it is added.
    let percent_or_ratio = |target: Decimal| if target >= Decimal::ONE { target.checked_div(Decimal::ONE_HUNDRED).unwrap_or(target) } else { target };
    for (right, target_type, target) in [("P", t.short_put_target_type.as_str(), short_put_target), ("C", t.short_call_target_type.as_str(), short_call_target)]
    {
        let is_put = right == "P";
        if target_type == "Delta" {
            let wanted = percent_or_ratio(target);
            for v in candidates.iter_mut().filter(|v| v.put_call() == right) {
                let delta = dec(v.delta()).unwrap_or(Decimal::ZERO);
                let distance = if is_put { delta.checked_add(wanted) } else { delta.checked_sub(wanted) };
                v.mid_distance = if v.delta() == 0.0 { Decimal::from(99) } else { distance.map_or(Decimal::MAX, |d| d.abs()) };
            }
            candidates.retain(|v| v.mid_distance < delta_window() || v.put_call() != right);
            log.push(format!("Trade - Filter Trades on delta complete ({} possible trades)", candidates.len()));
        }
        if target_type == "Points OTM" {
            for v in candidates.iter_mut().filter(|v| v.put_call() == right) {
                let strike = v.short.as_ref().map_or(Decimal::ZERO, |s| s.strike);
                let level = if is_put { price.checked_sub(target) } else { price.checked_add(target) };
                v.mid_distance = level.and_then(|l| l.checked_sub(strike)).map_or(Decimal::MAX, |d| d.abs());
            }
        }
    }
    // Closest to the target first, then the cheapest debit (the price nearest 0), then the narrowest.
    candidates.sort_by(|a, b| a.mid_distance.cmp(&b.mid_distance).then(key(b.price()).cmp(&key(a.price()))).then(a.width().cmp(&b.width())));
}

/// The entry price and the short/long ratio against the limits of the template; each miss is logged.
#[allow(clippy::too_many_arguments)]
fn calendar_within_limits(entry: Decimal, short_sum: Decimal, long_sum: Decimal, limits: (Decimal, Decimal, Decimal, Decimal), log: &mut Vec<String>) -> bool {
    let (min_entry, max_entry, min_ratio, max_ratio) = limits;
    let mut found = true;
    if min_entry > Decimal::ZERO && entry.abs() < min_entry {
        log.push(format!("Trade price ({}) does not meet Min Entry Price: {}", fmt_dec(entry.abs()), fmt_dec(min_entry)));
        found = false;
    }
    if max_entry > Decimal::ZERO && entry.abs() > max_entry {
        log.push(format!("Trade price ({}) does not meet Max Entry Price: {}", fmt_dec(entry.abs()), fmt_dec(max_entry)));
        found = false;
    }
    if found && long_sum != Decimal::ZERO {
        // The short/long price ratio of the combined trade.
        if let Some(ratio) = short_sum.abs().checked_div(long_sum.abs()) {
            log.push(format!("Trade Short Long ratio: {}  (short price: {} long price: {}", fmt_dec(ratio), fmt_dec(short_sum.abs()), fmt_dec(long_sum.abs())));
            if min_ratio > Decimal::ZERO && ratio < min_ratio {
                log.push(format!("Trade Short Long ratio ({}) does not meet Min ratio: {}", fmt_dec(ratio), fmt_dec(min_ratio)));
                found = false;
            }
            if max_ratio > Decimal::ZERO && ratio > max_ratio {
                log.push(format!("Trade Short Long ratio ({}) does not meet Max ratio: {}", fmt_dec(ratio), fmt_dec(max_ratio)));
                found = false;
            }
        }
    }
    found
}

/// Quote checks while the market is open: whether the trade has to be aborted for an outdated or wide quote.
fn calendar_quotes_bad(input: &CalendarInput, put: Option<&Vertical>, call: Option<&Vertical>, log: &mut Vec<String>) -> bool {
    let symbol = input.template.underlying_symbol.as_str();
    let mut abort = false;
    if input.underlying_outdated {
        log.push(format!("{symbol} Outdated Quote"));
        abort = true;
    }
    for (name, v) in [("Put", put), ("Call", call)] {
        let Some(v) = v else { continue };
        if v.data_status(symbol) == DataStatus::OutDated || v.wide_bid_ask(symbol) {
            log.push(format!("{name} Outdated or wide Quote"));
            abort = true;
        }
    }
    abort
}
// ------------------------------------------------------------------------------------------
// The opening order (`SellIB`, IB part) and the trades it opens.
// ------------------------------------------------------------------------------------------

/// Whether an order or a combo leg buys or sells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Buys ("BUY").
    Buy,
    /// Sells ("SELL").
    Sell,
}

impl Action {
    /// The action as TWS writes it: "BUY" or "SELL".
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }

    /// `Sell` when `sell`, else `Buy`.
    #[must_use]
    pub fn of(sell: bool) -> Self {
        if sell {
            Self::Sell
        } else {
            Self::Buy
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One leg of a combo order (`ComboLeg`: `ConId`, Ratio, Action; the exchange is the order's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComboLegSpec {
    /// TWS contract id.
    pub con_id: i32,
    /// `template.put_ratio` / `call_ratio` (0 counts as 1).
    pub ratio: i32,
    /// Buy (long) or sell (short).
    pub action: Action,
}

impl ComboLegSpec {
    /// A leg as a trade stores it: (contract id, ratio, sold).
    #[must_use]
    pub fn from_stored(&(con_id, ratio, sell): &(i32, i32, bool)) -> Self {
        Self { con_id, ratio, action: Action::of(sell) }
    }

    /// The leg as a trade stores it (see `from_stored`).
    #[must_use]
    pub fn stored(&self) -> (i32, i32, bool) {
        (self.con_id, self.ratio, self.action == Action::Sell)
    }
}

/// A trade the opening order creates (status 0). The selection
/// creates one per side for `IronCondor`, Put/CallSpread and the single-leg types, one for the
/// whole position for `IronFly`, IronCondor-Full, the strangles and LongIronCondor-Full.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedTrade {
    /// `Trade.trade_type` (an `IronCondor`'s sides become "`PutSpread`" / "`CallSpread`").
    pub trade_type: String,
    /// `Trade.Qty`.
    pub qty: u32,
    /// Strikes for the log, e.g. "6650/6640 P".
    pub strikes: String,
    /// This trade's part of the combo's fill price: 1 when the order opens only this trade. For
    /// an `IronCondor`'s two sides it is each side's share of the selected prices - the selection
    /// could take the exact per-leg prices from TWS's executions instead (not implemented yet).
    pub price_share: Decimal,
    /// `Trade.Delta` at entry: (shortPut - longPut + shortCall - longCall) * -100, 0 when a leg
    /// has no delta yet.
    pub delta: f64,
    /// The options this trade holds, for live marks and the settlement at expiration.
    pub legs: PlannedLegs,
    /// The legs of a custom trade (empty for every other type).
    pub custom: Vec<CustomLegRef>,
}

/// One leg of a custom trade as the trade keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomLegRef {
    /// TWS contract id.
    pub con_id: i32,
    /// "P" or "C".
    pub right: String,
    /// Strike price.
    pub strike: Decimal,
    /// Expiration, yyyyMMdd.
    pub expiry: String,
    /// The ratio with its sign (a sold leg is negative).
    pub ratio: i32,
}

/// One option a trade holds (`ShortPutContractID` + `ShortPut` ...).
#[derive(Clone, Debug, PartialEq)]
pub struct LegRef {
    /// TWS contract id.
    pub con_id: i32,
    /// Strike price.
    pub strike: Decimal,
}

/// The legs of a planned trade and its ratios (`Trade.put_ratio` / `call_ratio`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlannedLegs {
    /// The sold put, if any.
    pub short_put: Option<LegRef>,
    /// The bought put, if any.
    pub long_put: Option<LegRef>,
    /// The sold call, if any.
    pub short_call: Option<LegRef>,
    /// The bought call, if any.
    pub long_call: Option<LegRef>,
    /// Put ratio (at least 1).
    pub put_ratio: u32,
    /// Call ratio (at least 1).
    pub call_ratio: u32,
    /// Expiration yyyyMMdd (all legs of these trade types share it).
    pub expiry: String,
}

fn planned_legs(put: Option<&Vertical>, call: Option<&Vertical>, put_ratio: u32, call_ratio: u32) -> PlannedLegs {
    let leg = |l: Option<&Leg>| l.map(|l| LegRef { con_id: l.con_id, strike: l.strike });
    let expiry = [put, call].iter().flatten().find_map(|v| v.short.as_ref().or(v.long.as_ref()).map(|l| l.expiry.clone())).unwrap_or_default();
    PlannedLegs {
        short_put: leg(put.and_then(|v| v.short.as_ref())),
        long_put: leg(put.and_then(|v| v.long.as_ref())),
        short_call: leg(call.and_then(|v| v.short.as_ref())),
        long_call: leg(call.and_then(|v| v.long.as_ref())),
        put_ratio: put_ratio.max(1),
        call_ratio: call_ratio.max(1),
        expiry,
    }
}

/// The opening order of a template trade, before it is turned into TWS's `Contract`/`Order`.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryOrder {
    /// `Contract.Symbol`.
    pub symbol: String,
    /// "BAG" for a combo, "OPT" when only one leg is left.
    pub sec_type: &'static str,
    /// `Contract.Exchange`: SMART, or Settings > Order Routing for SPX.
    pub exchange: String,
    /// Single leg only: trading class (SPXW, RUTW, NDXP, else the symbol).
    pub trading_class: String,
    /// Single leg only: expiration yyyyMMdd.
    pub expiry: String,
    /// Single leg only: "P"/"C".
    pub right: String,
    /// Single leg only: strike.
    pub strike: Option<Decimal>,
    /// Buy for a combo (credit = negative limit); single leg: sell a short, buy a long.
    pub action: Action,
    /// `Order.TotalQuantity`.
    pub qty: u32,
    /// `Order.LmtPrice` (negative = credit combo).
    pub limit: Decimal,
    /// The combo legs, in the order long put, short put, long call, short call.
    pub legs: Vec<ComboLegSpec>,
    /// The trades this order opens.
    pub trades: Vec<PlannedTrade>,
    /// The log lines ("Long Put Contract ...", "Open ... `OrderID` ...").
    pub log: Vec<String>,
}

impl EntryOrder {
    /// `Trade.PriceOpen` (positive = credit received) from TWS's average fill price of this
    /// order: a combo is bought at a negative price for a credit; a single leg is sold for a
    /// credit or bought for a debit.
    #[must_use]
    pub fn price_open(&self, avg_fill_price: Decimal) -> Decimal {
        if self.sec_type == "OPT" && self.action == Action::Sell {
            avg_fill_price
        } else {
            -avg_fill_price
        }
    }
}

fn ratio(r: i32) -> i32 {
    if r == 0 {
        1
    } else {
        r
    }
}

/// `Trade.Delta` from the legs a trade holds.
fn trade_delta(put: Option<&Vertical>, call: Option<&Vertical>) -> f64 {
    let legs = [put.and_then(|v| v.short.as_ref()), put.and_then(|v| v.long.as_ref()), call.and_then(|v| v.short.as_ref()), call.and_then(|v| v.long.as_ref())];
    if legs.iter().flatten().any(|l| l.delta == 0.0) {
        return 0.0;
    }
    let delta = |l: Option<&Leg>| l.map_or(0.0, |l| l.delta);
    (delta(legs[0]) - delta(legs[1]) + delta(legs[2]) - delta(legs[3])) * -100.0
}

fn side_strikes(v: &Vertical) -> String {
    let parts: Vec<String> = [&v.short, &v.long].iter().filter_map(|l| l.as_ref().map(|l| fmt_dec(l.strike))).collect();
    format!("{} {}", parts.join("/"), v.put_call())
}

/// The trades (Trade rows) an order opens: the whole combo for the trade types that are one trade, else one per side.
fn planned_trades(
    trade_type: &str,
    put: Option<&Vertical>,
    call: Option<&Vertical>,
    qty: u32,
    (put_ratio, call_ratio): (i32, i32),
) -> Result<Vec<PlannedTrade>, String> {
    let mut trades = Vec::new();
    let qty_put = qty.checked_mul(u32::try_from(put_ratio).unwrap_or(1)).ok_or("quantity too large")?;
    let qty_call = qty.checked_mul(u32::try_from(call_ratio).unwrap_or(1)).ok_or("quantity too large")?;
    let whole = matches!(trade_type, "IronFly" | "IronCondor-Full" | "LongStrangle" | "ShortStrangle" | "LongIronCondor-Full" | "DoubleCalendar");
    if whole {
        let strikes = [put, call].iter().flatten().map(|v| side_strikes(v)).collect::<Vec<_>>().join("  ");
        let legs = planned_legs(put, call, u32::try_from(put_ratio).unwrap_or(1), u32::try_from(call_ratio).unwrap_or(1));
        trades.push(PlannedTrade {
            trade_type: trade_type.to_string(),
            qty,
            strikes,
            price_share: Decimal::ONE,
            delta: trade_delta(put, call),
            legs,
            custom: Vec::new(),
        });
    } else {
        // Each side's share of the combo price (IronCondor), from the prices it was selected at.
        let side_price = |v: Option<&Vertical>| v.and_then(Vertical::price).map_or(Decimal::ZERO, |p| p.abs());
        let (pp, cp) = (side_price(put), side_price(call));
        let total = pp.checked_add(cp).ok_or("entry price could not be computed (overflow)")?;
        let both = put.is_some() && call.is_some();
        let share = |part: Decimal| -> Decimal {
            if !both {
                Decimal::ONE
            } else if total > Decimal::ZERO {
                part.checked_div(total).unwrap_or(Decimal::ONE)
            } else {
                Decimal::new(5, 1)
            }
        };
        if let Some(c) = call {
            let kept = matches!(trade_type, "LongCall" | "CallDebitSpread" | "ShortCall" | "CallCalendar");
            trades.push(PlannedTrade {
                trade_type: if kept { trade_type.to_string() } else { "CallSpread".to_string() },
                qty: qty_call,
                strikes: side_strikes(c),
                price_share: share(cp),
                delta: trade_delta(None, Some(c)),
                // A side's own trade has ratio 1, the ratio is in its quantity.
                legs: planned_legs(None, Some(c), 1, 1),
                custom: Vec::new(),
            });
        }
        if let Some(p) = put {
            let kept = matches!(trade_type, "LongPut" | "PutDebitSpread" | "ShortPut" | "PutCalendar");
            trades.push(PlannedTrade {
                trade_type: if kept { trade_type.to_string() } else { "PutSpread".to_string() },
                qty: qty_put,
                strikes: side_strikes(p),
                price_share: share(pp),
                delta: trade_delta(Some(p), None),
                legs: planned_legs(Some(p), None, 1, 1),
                custom: Vec::new(),
            });
        }
    }
    Ok(trades)
}

/// The trading class an option of `symbol` is traded in here: the weekly/PM-settled class for the
/// index options; `None` = let TWS pick the standard one. Without it, an SPX option expiring on a
/// monthly expiration day is ambiguous (SPX and SPXW share the date).
#[must_use]
pub fn option_trading_class(symbol: &str) -> Option<&'static str> {
    match symbol {
        "SPX" => Some("SPXW"),
        "RUT" => Some("RUTW"),
        "NDX" => Some("NDXP"),
        _ => None,
    }
}

/// The trading class of the options of `symbol` (`SPXW` for SPX ...); the symbol itself where it has none of its own.
pub fn trading_class(symbol: &str) -> String {
    option_trading_class(symbol).map_or_else(|| symbol.to_string(), str::to_string)
}

/// The opening order of a custom trade: the combo of its legs (a single leg is a plain option order), bought at the negative of
/// the mid price (a credit is a negative limit).
fn build_custom_order(symbol: &str, exchange: String, price: Decimal, qty: u32, custom: &custom::CustomEntry) -> Result<EntryOrder, String> {
    let mut log = Vec::new();
    let mut legs = Vec::new();
    let mut refs = Vec::new();
    for (number, leg) in custom.legs.iter().enumerate() {
        let l = &leg.leg;
        log.push(format!("Leg {} Contract {} {} {} {} {}/{}", number + 1, l.con_id, fmt_dec(l.strike), l.right, l.expiry, fmt_dec(l.bid), fmt_dec(l.ask)));
        legs.push(ComboLegSpec { con_id: l.con_id, ratio: leg.ratio.abs().max(1), action: Action::of(leg.ratio < 0) });
        refs.push(CustomLegRef { con_id: l.con_id, right: l.right.clone(), strike: l.strike, expiry: l.expiry.clone(), ratio: leg.ratio });
    }
    if legs.is_empty() {
        return Err("the selected trade has no legs".into());
    }
    let expiry = custom.legs.iter().map(|leg| leg.leg.expiry.clone()).min().unwrap_or_default();
    let mut order = EntryOrder {
        symbol: symbol.to_string(),
        sec_type: "BAG",
        exchange,
        trading_class: String::new(),
        expiry: String::new(),
        right: String::new(),
        strike: None,
        action: Action::Buy,
        qty,
        limit: -price,
        legs,
        trades: Vec::new(),
        log,
    };
    let (mut trade_qty, mut strikes, mut delta) = (qty, custom.strikes_text(), custom.delta());
    if let [only] = custom.legs.as_slice() {
        // One option is a plain option order. Its ratio is no combo ratio: it becomes the number of options, so the order buys
        // (or sells) `qty` times the ratio, the limit is the price of one option, and the trade holds that many options with ratio 1.
        let ratio = only.ratio.unsigned_abs().max(1);
        let per_option = price.abs().checked_div(Decimal::from(ratio)).ok_or("entry price could not be computed (overflow)")?;
        trade_qty = qty.checked_mul(ratio).ok_or("quantity too large")?;
        order.sec_type = "OPT";
        order.trading_class = trading_class(symbol);
        order.expiry.clone_from(&only.leg.expiry);
        order.right.clone_from(&only.leg.right);
        order.strike = Some(only.leg.strike);
        order.action = Action::of(only.ratio < 0);
        order.qty = trade_qty;
        order.limit = fix_price(per_option, symbol, 1).ok_or("entry price could not be computed (overflow)")?;
        order.legs.clear();
        let sign = only.ratio.signum();
        for leg in &mut refs {
            leg.ratio = sign;
        }
        strikes = format!("{sign:+} {}{}", fmt_dec(only.leg.strike), only.leg.right);
        delta /= f64::from(ratio);
    }
    let planned_legs = PlannedLegs { put_ratio: 1, call_ratio: 1, expiry, ..PlannedLegs::default() };
    order.trades.push(PlannedTrade {
        trade_type: "Custom".to_string(),
        qty: trade_qty,
        strikes,
        price_share: Decimal::ONE,
        delta,
        legs: planned_legs,
        custom: refs,
    });
    Ok(order)
}

/// `SellIB` for IB up to `placeOrder`: the combo (or single-leg) order for what `select_trade`
/// chose, and the trades it opens. `order_routing` is Settings > Order Routing (empty = SMART).
///
/// # Errors
///
/// The reason (as text) when the quantity is below 1, the symbol is an ES futures option, or the selected trade has no legs.
///
/// Not implemented yet: long-strike reuse (an `Ask == 0` long already held), the `DisplaySize`
/// randomised for 100+ contracts, cancelling stops of open trades on the same strikes
/// and ES futures options.
pub fn build_entry_order(template: &TradeTemplate, outcome: &EntryOutcome, qty: u32, order_routing: &str) -> Result<EntryOrder, String> {
    let symbol = template.underlying_symbol.trim().to_uppercase();
    let trade_type = template.trade_type.as_str();
    let price = outcome.entry_price.ok_or("no trade selected - nothing to enter")?;
    if qty == 0 {
        return Err("quantity must be at least 1".into());
    }
    if symbol == "ES" {
        return Err("ES futures options are not supported for template trades yet".into());
    }
    let (put_ratio, call_ratio) = (ratio(template.put_ratio), ratio(template.call_ratio));
    let put = outcome.put.as_ref();
    let call = outcome.call.as_ref();
    let exchange = if symbol == "SPX" && !order_routing.trim().is_empty() { order_routing.trim().to_string() } else { "SMART".to_string() };
    if let Some(custom) = &outcome.custom {
        return build_custom_order(&symbol, exchange, price, qty, custom);
    }
    let mut log = Vec::new();
    let mut legs = Vec::new();
    let mut single: Option<(&Leg, Action)> = None;
    for (v, r, name) in [(put, put_ratio, "Put"), (call, call_ratio, "Call")] {
        let Some(v) = v else { continue };
        if let Some(l) = &v.long {
            log.push(format!("Long {name} {} {} Contract {} {}/{}", fmt_dec(l.strike), l.expiry, l.con_id, fmt_dec(l.bid), fmt_dec(l.ask)));
            legs.push(ComboLegSpec { con_id: l.con_id, ratio: r, action: Action::Buy });
            single = Some((l, Action::Buy));
        }
        if let Some(s) = &v.short {
            log.push(format!("Short {name} {} {} Contract {} {}/{}", fmt_dec(s.strike), s.expiry, s.con_id, fmt_dec(s.bid), fmt_dec(s.ask)));
            legs.push(ComboLegSpec { con_id: s.con_id, ratio: r, action: Action::Sell });
            // The short leg is preferred for a one-leg order.
            single = Some((s, Action::Sell));
        }
    }
    if legs.is_empty() {
        return Err("the selected trade has no legs".into());
    }
    // `LmtPrice = price * -1` (a credit is a BUY of the combo at a negative price); a
    // LongStrangle's price already has the combo's sign.
    let mut limit = if trade_type == "LongStrangle" { price } else { -price };
    let mut order = EntryOrder {
        symbol: symbol.clone(),
        sec_type: "BAG",
        exchange,
        trading_class: String::new(),
        expiry: String::new(),
        right: String::new(),
        strike: None,
        action: Action::Buy,
        qty,
        limit,
        legs,
        trades: Vec::new(),
        log,
    };
    if order.legs.len() == 1 {
        let (leg, action) = single.ok_or("the selected trade has no legs")?;
        order.sec_type = "OPT";
        order.trading_class = trading_class(&symbol);
        order.expiry.clone_from(&leg.expiry);
        order.right.clone_from(&leg.right);
        order.strike = Some(leg.strike);
        order.action = action;
        limit = fix_price(limit.abs(), &symbol, 1).ok_or("entry price could not be computed (overflow)")?;
        order.limit = limit;
        order.legs.clear();
    }

    order.trades = planned_trades(trade_type, put, call, qty, (put_ratio, call_ratio))?;
    Ok(order)
}

/// The limit for the next fill attempt of an unfilled opening order -
/// the order is moved by the template's `fill_adjustment`
/// towards a fill (a credit combo's negative limit towards 0, a bought debit up, a sold single
/// leg down), onto the tick grid, but never past the template's own limits (target premium,
/// Min/Max Entry Price). Then the old limit is kept. `legs` is the number of combo legs (1 for a
/// single option), `action` the order's action.
///
/// One safety rule: the limit never reaches 0 or changes sign, so a credit
/// is never turned into a free or debit order. `None` only on arithmetic overflow.
#[must_use]
pub fn next_fill_limit(template: &TradeTemplate, symbol: &str, legs: usize, action: &str, limit: Decimal) -> Option<Decimal> {
    fill_limit_step(template, symbol, legs, action, limit, true)
}

/// `next_fill_limit` for a scheduled entry that has filled in part already: the rest is moved on beyond the template's limits
/// (target premium, Min/Max Entry Price), as the contracts filled so far are a position already and the rest belongs to it; the
/// limit still never reaches 0 or changes sign.
#[must_use]
pub fn next_fill_limit_partly_filled(template: &TradeTemplate, symbol: &str, legs: usize, action: &str, limit: Decimal) -> Option<Decimal> {
    fill_limit_step(template, symbol, legs, action, limit, false)
}

/// The step of `next_fill_limit`; `bounded` keeps it within the template's limits.
fn fill_limit_step(template: &TradeTemplate, symbol: &str, legs: usize, action: &str, limit: Decimal, bounded: bool) -> Option<Decimal> {
    let n = numbers(template)?;
    let mut adjustment = dec(template.fill_adjustment)?.abs();
    // A positive limit that is SOLD (a single short option) is lowered to get filled.
    if limit > Decimal::ZERO && action.eq_ignore_ascii_case("SELL") {
        adjustment = -adjustment;
    }
    adjustment = fix_price(adjustment, symbol, legs)?;
    let mut next = fix_price(limit.checked_add(adjustment)?.round_dp(2), symbol, legs)?;
    if legs <= 1 {
        next = fix_price(next, symbol, legs)?;
        // The grid took the whole step back: go one more nickel in the same direction.
        if next == limit {
            let nickel = Decimal::new(5, 2);
            let moved = if adjustment > Decimal::ZERO {
                limit.checked_add(adjustment)?.checked_add(nickel)?
            } else {
                limit.checked_add(adjustment)?.checked_sub(nickel)?
            };
            next = fix_price(moved.round_dp(2), symbol, legs)?;
        }
    }
    // The template's limits; the Min/Max of a side count only when they differ, else 0 / 999.
    let wide = Decimal::from(999);
    let (put_min, put_max) = if n.target_max == n.target_min { (Decimal::ZERO, wide) } else { (n.target_min, n.target_max) };
    let (call_min, call_max) = if n.target_max_call == n.target_min_call {
        (Decimal::ZERO, wide)
    } else {
        (n.target_min_call.unwrap_or(Decimal::ZERO), n.target_max_call.unwrap_or(wide))
    };
    let price = next.abs();
    let put_premium = template.target_type.contains("Premium");
    let call_premium = template.target_type_call.contains("Premium");
    let below_min_entry = n.min_entry > Decimal::ZERO && price < n.min_entry;
    let beyond_max_entry = n.max_entry > Decimal::ZERO && price > n.max_entry;
    let outside_target = match template.trade_type.as_str() {
        "LongPut" | "PutDebitSpread" => put_premium && (price > put_max || beyond_max_entry),
        "LongCall" | "CallDebitSpread" => call_premium && (price > call_max || beyond_max_entry),
        "ShortPut" | "PutSpread" => put_premium && (price < put_min || below_min_entry),
        "ShortCall" | "CallSpread" => call_premium && (price < call_min || below_min_entry),
        other => !is_debit(other) && price < n.min_entry,
    };
    let crosses_zero = next.is_zero() || (next < Decimal::ZERO) != (limit < Decimal::ZERO);
    let beyond_limits = bounded && (outside_target || below_min_entry || beyond_max_entry);
    Some(if beyond_limits || crosses_zero { limit } else { next })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::ExitSwitches;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn leg(con_id: i32, right: &str, strike: Decimal, bid: Decimal, ask: Decimal, delta: f64) -> Leg {
        Leg {
            con_id,
            strike,
            right: right.into(),
            expiry: "20261113".into(),
            bid,
            ask,
            delta,
            subscribed: true,
            position: Decimal::ZERO,
            working: WorkingSide::None,
            outdated: false,
        }
    }

    /// A small SPX 0-DTE chain around 6700: 5-point strikes, premiums falling 0.15 (puts) and
    /// 0.125 (calls) per strike, bid 0.10 under the ask.
    fn chain() -> Vec<Leg> {
        let mut legs = Vec::new();
        let mut id = 1;
        for k in 0..20_i64 {
            let kd = Decimal::from(k);
            let strike = d("6690") - d("5") * kd;
            let ask = (d("3") - d("0.15") * kd).max(d("0.05"));
            let bid = ask - d("0.10");
            let bid = if bid > Decimal::ZERO { bid } else { minus_one() };
            let delta = -(0.30 - 0.015 * f64::from(i32::try_from(k).unwrap())).max(0.01);
            legs.push(leg(id, "P", strike, bid, ask, delta));
            id += 1;
        }
        for k in 0..20_i64 {
            let kd = Decimal::from(k);
            let strike = d("6710") + d("5") * kd;
            let ask = (d("2.5") - d("0.125") * kd).max(d("0.05"));
            let bid = ask - d("0.10");
            let bid = if bid > Decimal::ZERO { bid } else { minus_one() };
            let delta = (0.28 - 0.014 * f64::from(i32::try_from(k).unwrap())).max(0.01);
            legs.push(leg(id, "C", strike, bid, ask, delta));
            id += 1;
        }
        legs
    }

    fn ic_template() -> TradeTemplate {
        TradeTemplate {
            trade_type: "IronCondor".into(),
            underlying_symbol: "SPX".into(),
            target_type: "Premium".into(),
            target_min: 1.0,
            target_max: 1.5,
            preference: "Highest premium".into(),
            target_type_call: "Premium".into(),
            target_min_call: Some(1.0),
            target_max_call: Some(1.5),
            preference_call: "Highest premium".into(),
            long_type: "Width".into(),
            long_width: "10".into(),
            long_type_call: "Width".into(),
            long_width_call: "10".into(),
            min_otm_call: Some(0.0),
            ..TradeTemplate::default()
        }
    }

    fn short_put_delta() -> TradeTemplate {
        TradeTemplate {
            trade_type: "ShortPut".into(),
            underlying_symbol: "SPX".into(),
            target_type: "Delta".into(),
            target_min: 10.0,
            target_max: 20.0,
            preference: "Highest premium".into(),
            ..TradeTemplate::default()
        }
    }

    fn input(template: &TradeTemplate) -> EntryInput<'_> {
        EntryInput { template, price: d("6700"), open_price: None, legs: chain(), market_open: false, underlying_outdated: false, test_trade: false }
    }

    fn strikes(v: Option<&Vertical>) -> (Option<Decimal>, Option<Decimal>) {
        let v = v.as_ref().expect("side chosen");
        (v.short.as_ref().map(|l| l.strike), v.long.as_ref().map(|l| l.strike))
    }

    #[test]
    fn option_mid_and_wide_quotes_follow_the_rules() {
        assert_eq!(leg(1, "P", d("6600"), d("1.0"), d("1.2"), -0.1).mid(), Some(d("1.1")));
        assert_eq!(leg(1, "P", d("6600"), minus_one(), d("0.05"), 0.0).mid(), Some(d("0.05")), "no bid: the ask");
        assert_eq!(leg(1, "P", d("6600"), minus_one(), minus_one(), 0.0).mid(), Some(Decimal::ZERO), "no quote at all");
        assert!(leg(1, "P", d("6600"), d("1.0"), d("2.6"), 0.0).wide_bid_ask("SPX"), "spread 1.60 > 1.50");
        assert!(!leg(1, "P", d("6600"), d("1.0"), d("2.4"), 0.0).wide_bid_ask("SPX"));
        assert!(leg(1, "P", d("6600"), d("2.0"), d("1.0"), 0.0).wide_bid_ask("SPX"), "crossed: bid above ask");
        assert!(leg(1, "P", d("600"), minus_one(), d("0.2"), 0.0).wide_bid_ask("SPY"), "no bid on a non-index option");
    }

    #[test]
    fn an_iron_condor_outside_its_premium_target_finds_nothing() {
        // 10-wide credits in this chain: puts 0.30, calls 0.25 - below the 1.00-1.50 target.
        let t = ic_template();
        let out = select_trade(&input(&t));
        assert!(out.put.is_none() && out.entry_price.is_none() && out.preview == "No Matching Trade Found", "{:?}", out.log);
    }

    #[test]
    fn an_iron_condor_with_equal_targets_picks_by_the_rules() {
        let t = TradeTemplate { target_min: 0.2, target_max: 0.35, target_min_call: Some(0.2), target_max_call: Some(0.35), ..ic_template() };
        let out = select_trade(&input(&t));
        // Equal put/call targets: one list sorted by premium, walked until both sides are set.
        // Each further put (all 0.30) replaces the previous one until the first
        // call (0.25) comes - so the put is the last 0.30 spread (short 6605 / long 6595) and the
        // call the first 0.25 one (6710 / 6720).
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6605")), Some(d("6595"))), "{:?}", out.log);
        assert_eq!(strikes(out.call.as_ref()), (Some(d("6710")), Some(d("6720"))));
        assert_eq!(out.entry_price, Some(d("0.55")));
        assert_eq!(out.preview, "SPX  Price: 6700\nCALL:0.25 6710 2026-11-13 / 6720 2026-11-13\nPUT: 0.3 6605 2026-11-13 / 6595 2026-11-13\n");
    }

    #[test]
    fn single_short_puts_are_chosen_by_delta_and_priced_on_the_grid() {
        let t = short_put_delta();
        let out = select_trade(&input(&t));
        // Deltas: 0.30 - 0.015k; |delta| in [0.10, 0.20] -> k = 7..=13, highest |delta| = k=7: 6655.
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6655")), None), "{:?}", out.log);
        // Mid of k=7: ask 1.95, bid 1.85 -> 1.90 (on the 0.05 grid below 3.00).
        assert_eq!(out.entry_price, Some(d("1.9")));
        assert!(out.preview.starts_with("SPX  Price: 6700\nPUT: 1.9 6655 2026-11-13 "), "{}", out.preview);
    }

    #[test]
    fn a_put_spread_by_premium_with_width_and_mid_preference() {
        let t = TradeTemplate {
            trade_type: "PutSpread".into(),
            underlying_symbol: "SPX".into(),
            target_type: "Premium".into(),
            target_min: 0.6,
            target_max: 1.0,
            preference: "Middle premium".into(),
            long_type: "Width".into(),
            long_width: "20,30".into(),
            ..TradeTemplate::default()
        };
        let out = select_trade(&input(&t));
        // Credit per 5 points = 0.15: width 20 -> 0.60, width 30 -> 0.90. Mid target 0.80:
        // 0.90 (distance 0.10) beats 0.60 (0.20) -> the first 30-wide spread, 6690 / 6660.
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6690")), Some(d("6660"))), "{:?}", out.log);
        assert_eq!(out.entry_price, Some(d("0.9")));
    }

    #[test]
    fn strikes_conflicting_with_open_positions_are_skipped_unless_allowed() {
        let mut t = short_put_delta();
        let mut legs = chain();
        // Already long the 6655 put: selling it would close it -> the next one, 6650.
        if let Some(l) = legs.iter_mut().find(|l| l.right == "P" && l.strike == d("6655")) {
            l.position = Decimal::ONE;
        }
        let out = select_trade(&EntryInput { legs: legs.clone(), ..input(&t) });
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6650")), None));
        t.exit_flags.allow_strike_conflict = true;
        let out = select_trade(&EntryInput { legs, ..input(&t) });
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6655")), None));
    }

    #[test]
    fn an_option_an_exit_order_works_on_is_not_entered_on_the_other_side_even_if_conflicts_are_allowed() {
        let mut t = short_put_delta();
        t.exit_flags.allow_strike_conflict = true;
        let mut legs = chain();
        // A buy order (the exit of a short 6655 put) is working: selling that put would be rejected by TWS, so the next strike is used.
        if let Some(l) = legs.iter_mut().find(|l| l.right == "P" && l.strike == d("6655")) {
            l.working = WorkingSide::Buy;
        }
        let out = select_trade(&EntryInput { legs: legs.clone(), ..input(&t) });
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6650")), None), "{:?}", out.log);
        assert!(out.log.iter().any(|l| l.contains("Filter Trades on working orders")), "{:?}", out.log);
        // A sell order on that put does not stop another sell.
        if let Some(l) = legs.iter_mut().find(|l| l.right == "P" && l.strike == d("6655")) {
            l.working = WorkingSide::Sell;
        }
        let out = select_trade(&EntryInput { legs, ..input(&t) });
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6655")), None));
    }

    #[test]
    fn a_debit_spread_is_flipped_and_test_trade_shifts_the_price() {
        let t = TradeTemplate {
            trade_type: "PutDebitSpread".into(),
            underlying_symbol: "SPX".into(),
            target_type: "Premium".into(),
            target_min: 0.25,
            target_max: 0.35,
            preference: "Highest premium".into(),
            long_type: "Width".into(),
            long_width: "10".into(),
            ..TradeTemplate::default()
        };
        let out = select_trade(&input(&t));
        let put = out.put.as_ref().expect("put chosen");
        // Bought: the higher strike; sold: the lower one. Price = mid(short) - mid(long) < 0.
        assert!(put.short.as_ref().unwrap().strike < put.long.as_ref().unwrap().strike, "{:?}", out.log);
        assert_eq!(out.entry_price, Some(d("-0.3")));
        let test = select_trade(&EntryInput { test_trade: true, ..input(&t) });
        assert_eq!(test.entry_price, Some(d("-0.10")), "a small debit becomes -0.10 so it does not fill");
    }

    #[test]
    fn min_and_max_entry_price_and_zero_prices_reject_the_trade() {
        let t = TradeTemplate { min_entry_price: 5.0, ..short_put_delta() };
        assert!(select_trade(&input(&t)).entry_price.is_none(), "all candidates below 5.00");
        let t = TradeTemplate { max_entry_price: 0.5, ..short_put_delta() };
        assert!(select_trade(&input(&t)).entry_price.is_none(), "all candidates above 0.50");
        let template = short_put_delta();
        let empty = EntryInput { legs: Vec::new(), ..input(&template) };
        let out = select_trade(&empty);
        assert_eq!(out.preview, "No Matching Trade Found");
        assert!(out.log.iter().any(|l| l.contains("No Quotes Found")));
    }

    #[test]
    fn invalid_template_numbers_are_reported_not_used() {
        let t = TradeTemplate { target_min: f64::NAN, ..short_put_delta() };
        let out = select_trade(&input(&t));
        assert!(out.entry_price.is_none());
        assert!(out.log.iter().any(|l| l.contains("invalid number")));
    }

    #[test]
    fn outdated_quotes_abort_while_the_market_is_open() {
        let t = short_put_delta();
        let mut legs = chain();
        for l in &mut legs {
            l.outdated = true;
        }
        let out = select_trade(&EntryInput { legs: legs.clone(), market_open: true, ..input(&t) });
        assert!(out.entry_price.is_none());
        assert!(out.preview.ends_with("Quotes are out of date or too wide: trade not entered."), "{}", out.preview);
        // Outside market hours the age of a quote does not matter.
        assert!(select_trade(&EntryInput { legs, market_open: false, ..input(&t) }).entry_price.is_some());
    }

    #[test]
    fn iron_fly_centers_on_the_strike_nearest_the_price() {
        let mut legs = chain();
        // The ATM 6700 put and call the fly is built around.
        legs.push(leg(900, "P", d("6700"), d("6.9"), d("7.1"), -0.5));
        legs.push(leg(901, "C", d("6700"), d("6.9"), d("7.1"), 0.5));
        let t = TradeTemplate {
            trade_type: "IronFly".into(),
            underlying_symbol: "SPX".into(),
            target_type: "Premium".into(),
            target_min: 0.0,
            target_max: 20.0,
            preference: "Highest premium".into(),
            target_type_call: "Premium".into(),
            target_min_call: Some(0.0),
            target_max_call: Some(20.0),
            preference_call: "Highest premium".into(),
            long_type: "Width".into(),
            long_width: "20".into(),
            long_type_call: "Width".into(),
            long_width_call: "20".into(),
            ..TradeTemplate::default()
        };
        let out = select_trade(&EntryInput { legs, price: d("6701"), ..input(&t) });
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6700")), Some(d("6680"))), "{:?}", out.log);
        assert_eq!(strikes(out.call.as_ref()), (Some(d("6700")), Some(d("6720"))));
    }

    // ---- the opening order ----

    #[test]
    fn an_iron_condor_becomes_one_bag_bought_at_minus_its_credit_with_two_trades() {
        let t = TradeTemplate { target_min: 0.2, target_max: 0.35, target_min_call: Some(0.2), target_max_call: Some(0.35), ..ic_template() };
        let out = select_trade(&input(&t));
        let entry = out.entry_price.expect("trade selected");
        let order = build_entry_order(&t, &out, 2, "").unwrap();
        assert_eq!((order.sec_type, order.action.as_str(), order.exchange.as_str(), order.qty), ("BAG", "BUY", "SMART", 2));
        assert_eq!(order.limit, -entry, "credit combo: negative limit");
        let put = out.put.as_ref().unwrap();
        let call = out.call.as_ref().unwrap();
        let expected = vec![
            ComboLegSpec { con_id: put.long.as_ref().unwrap().con_id, ratio: 1, action: Action::Buy },
            ComboLegSpec { con_id: put.short.as_ref().unwrap().con_id, ratio: 1, action: Action::Sell },
            ComboLegSpec { con_id: call.long.as_ref().unwrap().con_id, ratio: 1, action: Action::Buy },
            ComboLegSpec { con_id: call.short.as_ref().unwrap().con_id, ratio: 1, action: Action::Sell },
        ];
        assert_eq!(order.legs, expected);
        // An IronCondor is recorded as its call spread and its put spread.
        let types: Vec<&str> = order.trades.iter().map(|t| t.trade_type.as_str()).collect();
        assert_eq!(types, ["CallSpread", "PutSpread"]);
        assert!(order.trades.iter().all(|t| t.qty == 2));
        let shares = order.trades.iter().try_fold(Decimal::ZERO, |acc, t| acc.checked_add(t.price_share)).unwrap();
        assert_eq!(shares.round_dp(10), Decimal::ONE, "the sides' shares make up the whole fill");
        // Filled at -1.40 (credit 1.40).
        assert_eq!(order.price_open(d("-1.4")), d("1.4"));
    }

    #[test]
    fn spx_combos_use_the_order_routing_setting() {
        let t = TradeTemplate { target_min: 0.2, target_max: 0.35, target_min_call: Some(0.2), target_max_call: Some(0.35), ..ic_template() };
        let out = select_trade(&input(&t));
        assert_eq!(build_entry_order(&t, &out, 1, "CBOE").unwrap().exchange, "CBOE");
        let spy = TradeTemplate { underlying_symbol: "SPY".into(), ..t.clone() };
        assert_eq!(build_entry_order(&spy, &out, 1, "CBOE").unwrap().exchange, "SMART");
    }

    #[test]
    fn a_single_short_put_is_a_plain_spxw_sell_on_the_tick_grid() {
        let t = short_put_delta();
        let out = select_trade(&input(&t));
        let order = build_entry_order(&t, &out, 1, "").unwrap();
        let short = out.put.as_ref().unwrap().short.as_ref().unwrap();
        assert_eq!((order.sec_type, order.action.as_str(), order.trading_class.as_str()), ("OPT", "SELL", "SPXW"));
        assert_eq!((order.strike, order.right.as_str(), order.expiry.as_str()), (Some(short.strike), "P", "20261113"));
        assert_eq!(order.legs, [] as [crate::trade_entry::ComboLegSpec; 0]);
        assert_eq!(order.limit, fix_price(out.entry_price.unwrap().abs(), "SPX", 1).unwrap());
        assert!(order.limit > Decimal::ZERO);
        assert_eq!(order.trades.len(), 1);
        assert_eq!(order.trades[0].trade_type, "ShortPut");
        assert_eq!(order.trades[0].price_share, Decimal::ONE);
        // A sold single leg filled at 2.10 is a 2.10 credit.
        assert_eq!(order.price_open(d("2.1")), d("2.1"));
        // Trade.Delta: short put delta x -100 (a short put is long delta).
        assert!((order.trades[0].delta - short.delta * -100.0).abs() < 1e-9);
    }

    #[test]
    fn a_debit_spread_is_bought_at_its_positive_debit() {
        let t = TradeTemplate {
            trade_type: "PutDebitSpread".into(),
            underlying_symbol: "SPX".into(),
            target_type: "Delta".into(),
            target_min: 10.0,
            target_max: 30.0,
            preference: "Highest premium".into(),
            long_type: "Width".into(),
            long_width: "10".into(),
            ..TradeTemplate::default()
        };
        let out = select_trade(&input(&t));
        let order = build_entry_order(&t, &out, 1, "").unwrap();
        assert_eq!(out.entry_price, Some(d("-0.3")));
        assert_eq!((order.sec_type, order.action.as_str(), order.limit), ("BAG", "BUY", d("0.3")));
        // Paid 0.30: a debit trade (negative PriceOpen).
        assert_eq!(order.price_open(d("0.3")), d("-0.3"));
        assert_eq!(order.trades[0].trade_type, "PutDebitSpread");
    }

    // ---- Fill attempts: the next limit of an unfilled order ----

    fn fill_template(trade_type: &str, target_type: &str) -> TradeTemplate {
        TradeTemplate {
            trade_type: trade_type.into(),
            underlying_symbol: "SPX".into(),
            target_type: target_type.into(),
            target_type_call: target_type.into(),
            fill_adjustment: 0.05,
            ..TradeTemplate::default()
        }
    }

    #[test]
    fn a_credit_combo_gives_up_a_little_credit_each_attempt() {
        let t = fill_template("PutSpread", "Delta");
        // BUY at -1.50 (a credit of 1.50): -1.45 is the next limit, then -1.40.
        assert_eq!(next_fill_limit(&t, "SPX", 2, "BUY", d("-1.50")), Some(d("-1.45")));
        assert_eq!(next_fill_limit(&t, "SPX", 2, "BUY", d("-1.45")), Some(d("-1.40")));
        // A debit combo pays a little more.
        let debit = fill_template("PutDebitSpread", "Delta");
        assert_eq!(next_fill_limit(&debit, "SPX", 2, "BUY", d("0.30")), Some(d("0.35")));
    }

    #[test]
    fn a_sold_single_option_is_lowered_and_the_tick_grid_is_respected() {
        let t = fill_template("ShortPut", "Delta");
        assert_eq!(next_fill_limit(&t, "SPX", 1, "SELL", d("2.10")), Some(d("2.05")));
        // Above 3.00 SPX trades in 0.10 steps: 3.20 - 0.05 = 3.15 falls to 3.10.
        assert_eq!(next_fill_limit(&t, "SPX", 1, "SELL", d("3.20")), Some(d("3.10")));
        // A bought single whose step the grid takes back moves one more nickel: 3.00 -> 3.10.
        let long = fill_template("LongPut", "Delta");
        assert_eq!(next_fill_limit(&long, "SPX", 1, "BUY", d("3.00")), Some(d("3.10")));
    }

    #[test]
    fn the_limit_never_moves_past_the_templates_own_limits() {
        // Short premium must stay at or above the template's minimum premium of 1.00.
        let t = TradeTemplate { target_min: 1.0, target_max: 1.5, ..fill_template("PutSpread", "Premium") };
        assert_eq!(next_fill_limit(&t, "SPX", 2, "BUY", d("-1.00")), Some(d("-1.00")), "0.95 would be below 1.00: kept");
        assert_eq!(
            next_fill_limit_partly_filled(&t, "SPX", 2, "BUY", d("-1.00")),
            Some(d("-0.95")),
            "filled in part already: the rest moves on beyond the Min Entry Price"
        );
        assert_eq!(next_fill_limit_partly_filled(&t, "SPX", 2, "BUY", d("-0.05")), Some(d("-0.05")), "but never to 0");
        assert_eq!(next_fill_limit(&t, "SPX", 2, "BUY", d("-1.20")), Some(d("-1.15")));
        // Min Entry Price holds for every type.
        let t = TradeTemplate { min_entry_price: 1.18, ..fill_template("PutSpread", "Delta") };
        assert_eq!(next_fill_limit(&t, "SPX", 2, "BUY", d("-1.20")), Some(d("-1.20")));
        // A debit trade must not pay more than Max Entry Price.
        let t = TradeTemplate { max_entry_price: 0.30, ..fill_template("PutDebitSpread", "Delta") };
        assert_eq!(next_fill_limit(&t, "SPX", 2, "BUY", d("0.30")), Some(d("0.30")));
        // An iron condor (default rule): not below Min Entry Price.
        let t = TradeTemplate { min_entry_price: 1.40, ..fill_template("IronCondor", "Premium") };
        assert_eq!(next_fill_limit(&t, "SPX", 4, "BUY", d("-1.40")), Some(d("-1.40")));
    }

    #[test]
    fn a_credit_is_never_turned_into_a_free_or_debit_order() {
        let t = fill_template("PutSpread", "Delta");
        // -0.05 + 0.05 would be 0: kept (a limit of 0 would be sent otherwise).
        assert_eq!(next_fill_limit(&t, "SPX", 2, "BUY", d("-0.05")), Some(d("-0.05")));
        let single = fill_template("ShortPut", "Delta");
        assert_eq!(next_fill_limit(&single, "SPX", 1, "SELL", d("0.05")), Some(d("0.05")));
    }

    #[test]
    fn no_selection_zero_quantity_and_es_are_refused() {
        let t = short_put_delta();
        let none = EntryOutcome::default();
        assert!(build_entry_order(&t, &none, 1, "").is_err());
        let out = select_trade(&input(&t));
        assert!(build_entry_order(&t, &out, 0, "").is_err());
        let es = TradeTemplate { underlying_symbol: "ES".into(), ..t };
        assert!(build_entry_order(&es, &out, 1, "").is_err());
    }

    // ---- calendars ----

    const NEAR: &str = "20261113";
    const FAR: &str = "20261120";

    /// The near chain (`chain()`) plus a far chain with the same strikes, every option $1.00 dearer.
    fn calendar_chain() -> Vec<Leg> {
        let mut legs = chain();
        let far: Vec<Leg> = legs
            .iter()
            .map(|l| {
                let ask = l.ask + d("1");
                let bid = ask - d("0.10");
                Leg { con_id: l.con_id + 100, expiry: FAR.into(), bid, ask, ..l.clone() }
            })
            .collect();
        legs.extend(far);
        legs
    }

    fn calendar_template(trade_type: &str) -> TradeTemplate {
        TradeTemplate {
            trade_type: trade_type.into(),
            underlying_symbol: "SPX".into(),
            short_put_target_type: "Delta".into(),
            short_put_target: 25.0,
            long_put_target_type: "Spread width".into(),
            long_put_target: 0.0,
            short_call_target_type: "Delta".into(),
            short_call_target: 20.0,
            long_call_target_type: "Spread width".into(),
            long_call_target: 0.0,
            ..TradeTemplate::default()
        }
    }

    fn calendar_input(template: &TradeTemplate) -> CalendarInput<'_> {
        CalendarInput {
            template,
            price: d("6700"),
            legs: calendar_chain(),
            expiries: CalendarExpiries { short_put: NEAR.into(), long_put: FAR.into(), short_call: NEAR.into(), long_call: FAR.into() },
            market_open: false,
            underlying_outdated: false,
            test_trade: false,
        }
    }

    #[test]
    fn a_put_calendar_sells_the_near_delta_strike_and_buys_the_far_one_at_the_same_strike() {
        let t = calendar_template("PutCalendar");
        let out = select_calendar(&calendar_input(&t));
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6675")), Some(d("6675"))), "{:?}", out.log);
        let put = out.put.as_ref().unwrap();
        assert_eq!((put.short.as_ref().unwrap().expiry.as_str(), put.long.as_ref().unwrap().expiry.as_str()), (NEAR, FAR));
        assert!(out.call.is_none());
        // Sold 2.50 (mid), bought 3.50: a debit of 1.00, shown as a negative price.
        assert_eq!(out.entry_price, Some(d("-1")));
        assert!(out.preview.contains("PUT:"), "{}", out.preview);
    }

    #[test]
    fn a_call_calendar_uses_the_call_side_only() {
        let t = calendar_template("CallCalendar");
        let out = select_calendar(&calendar_input(&t));
        assert_eq!(strikes(out.call.as_ref()), (Some(d("6740")), Some(d("6740"))), "{:?}", out.log);
        assert!(out.put.is_none());
        assert_eq!(out.entry_price, Some(d("-1")));
    }

    #[test]
    fn a_double_calendar_needs_both_sides_and_adds_their_prices() {
        let t = calendar_template("DoubleCalendar");
        let out = select_calendar(&calendar_input(&t));
        assert!(out.put.is_some() && out.call.is_some(), "{:?}", out.log);
        assert_eq!(out.entry_price, Some(d("-2")));
        // Without any far call quotes there is no double calendar.
        let mut input = calendar_input(&t);
        input.legs.retain(|l| !(l.right == "C" && l.expiry == FAR));
        let out = select_calendar(&input);
        assert!(out.entry_price.is_none(), "{:?}", out.log);
    }

    #[test]
    fn a_spread_width_puts_the_far_leg_that_many_points_above_the_near_strike() {
        let t = TradeTemplate { long_put_target: 5.0, ..calendar_template("PutCalendar") };
        let out = select_calendar(&calendar_input(&t));
        assert_eq!(strikes(out.put.as_ref()), (Some(d("6675")), Some(d("6680"))), "{:?}", out.log);
    }

    #[test]
    fn the_entry_price_and_the_short_long_ratio_limit_the_calendar() {
        let base = calendar_template("PutCalendar");
        let min_entry = TradeTemplate { min_entry_price: 1.5, ..base.clone() };
        let out = select_calendar(&calendar_input(&min_entry));
        assert!(out.entry_price.is_none() && out.log.iter().any(|l| l.contains("does not meet Min Entry Price")), "{:?}", out.log);
        let max_entry = TradeTemplate { max_entry_price: 0.5, ..base.clone() };
        assert!(select_calendar(&calendar_input(&max_entry)).entry_price.is_none());
        // Short 2.50 / long 3.50 = 0.714.
        let low_ratio = TradeTemplate { min_entry_sl_ratio: 0.8, ..base.clone() };
        let out = select_calendar(&calendar_input(&low_ratio));
        assert!(out.entry_price.is_none() && out.log.iter().any(|l| l.contains("does not meet Min ratio")), "{:?}", out.log);
        let high_ratio = TradeTemplate { max_entry_sl_ratio: 0.7, ..base.clone() };
        assert!(select_calendar(&calendar_input(&high_ratio)).entry_price.is_none());
        let fits = TradeTemplate { min_entry_sl_ratio: 0.7, max_entry_sl_ratio: 0.8, min_entry_price: 0.5, max_entry_price: 1.5, ..base };
        assert_eq!(select_calendar(&calendar_input(&fits)).entry_price, Some(d("-1")));
    }

    #[test]
    fn fewer_than_five_strikes_means_the_quotes_are_not_loaded_yet() {
        let t = calendar_template("PutCalendar");
        let mut input = calendar_input(&t);
        input.legs.retain(|l| !(l.right == "P" && l.expiry == FAR && l.strike < d("6680")));
        let out = select_calendar(&input);
        assert!(out.entry_price.is_none() && out.log.iter().any(|l| l.contains("Fewer than 5 strikes")), "{:?}", out.log);
    }

    #[test]
    fn a_held_near_long_blocks_its_strike_unless_conflicts_are_allowed() {
        let t = calendar_template("PutCalendar");
        let mut input = calendar_input(&t);
        // Long 1 of the near 6675 put (the quirk: longs are compared with the sold strikes).
        for l in input.legs.iter_mut().filter(|l| l.right == "P" && l.expiry == NEAR && l.strike == d("6675")) {
            l.position = Decimal::ONE;
        }
        let out = select_calendar(&input);
        assert_eq!(strikes(out.put.as_ref()).0, Some(d("6670")), "{:?}", out.log);
        let allowed = TradeTemplate { exit_flags: ExitSwitches { allow_strike_conflict: true, ..t.exit_flags }, ..t.clone() };
        let mut input = calendar_input(&allowed);
        for l in input.legs.iter_mut().filter(|l| l.right == "P" && l.expiry == NEAR && l.strike == d("6675")) {
            l.position = Decimal::ONE;
        }
        assert_eq!(strikes(select_calendar(&input).put.as_ref()).0, Some(d("6675")));
    }

    #[test]
    fn a_points_otm_target_picks_the_strike_closest_to_the_level() {
        let t = TradeTemplate { short_put_target_type: "Points OTM".into(), short_put_target: 52.0, ..calendar_template("PutCalendar") };
        // 6700 - 52 = 6648: the nearest 5-point strike is 6650.
        assert_eq!(strikes(select_calendar(&calendar_input(&t)).put.as_ref()).0, Some(d("6650")));
    }

    #[test]
    fn stale_or_missing_quotes_abort_a_calendar_while_the_market_is_open() {
        let t = calendar_template("PutCalendar");
        let mut input = calendar_input(&t);
        input.market_open = true;
        input.underlying_outdated = true;
        let out = select_calendar(&input);
        assert!(out.entry_price.is_none() && out.preview.contains("Quotes are out of date"), "{}", out.preview);
        let mut input = calendar_input(&t);
        input.market_open = true;
        for l in &mut input.legs {
            l.outdated = true;
        }
        assert!(select_calendar(&input).entry_price.is_none());
        let mut input = calendar_input(&t);
        input.market_open = true;
        assert!(select_calendar(&input).entry_price.is_some());
    }

    #[test]
    fn test_trade_moves_a_calendar_debit_towards_zero() {
        let t = calendar_template("PutCalendar");
        let mut input = calendar_input(&t);
        input.test_trade = true;
        // Debit 1.00 on SPX: the limit drops below the market (`entry + 5` for a credit, away for a debit).
        assert_eq!(select_calendar(&input).entry_price, Some(d("-0.1")));
    }

    #[test]
    fn a_calendar_order_buys_the_far_leg_and_sells_the_near_one_for_a_debit() {
        let t = calendar_template("PutCalendar");
        let out = select_calendar(&calendar_input(&t));
        let order = build_entry_order(&t, &out, 2, "").unwrap();
        assert_eq!((order.sec_type, order.action.as_str(), order.qty, order.limit), ("BAG", "BUY", 2, d("1")));
        let put = out.put.as_ref().unwrap();
        let (short_id, long_id) = (put.short.as_ref().unwrap().con_id, put.long.as_ref().unwrap().con_id);
        assert_eq!(
            order.legs,
            vec![ComboLegSpec { con_id: long_id, ratio: 1, action: Action::Buy }, ComboLegSpec { con_id: short_id, ratio: 1, action: Action::Sell }]
        );
        assert_eq!(order.trades.len(), 1);
        let trade = &order.trades[0];
        assert_eq!((trade.trade_type.as_str(), trade.qty, trade.legs.expiry.as_str()), ("PutCalendar", 2, NEAR));
        assert!(trade.legs.short_put.is_some() && trade.legs.long_put.is_some() && trade.legs.short_call.is_none());
    }

    #[test]
    fn a_double_calendar_is_one_trade_with_four_legs() {
        let t = calendar_template("DoubleCalendar");
        let out = select_calendar(&calendar_input(&t));
        let order = build_entry_order(&t, &out, 1, "").unwrap();
        assert_eq!((order.legs.len(), order.limit), (4, d("2")));
        assert_eq!(order.trades.len(), 1);
        assert_eq!(order.trades[0].trade_type, "DoubleCalendar");
        assert_eq!(order.legs.iter().filter(|l| l.action == Action::Buy).count(), 2);
    }

    #[test]
    fn only_the_three_calendars_use_the_calendar_entry() {
        for t in ["CallCalendar", "PutCalendar", "DoubleCalendar"] {
            assert!(uses_calendar_entry(t) && !uses_tranche_entry(t), "{t}");
        }
        assert!(!uses_calendar_entry("IronCondor") && !uses_calendar_entry("Custom"));
    }
    #[test]
    fn the_buying_power_of_a_credit_spread_is_its_width_minus_the_credit_and_of_a_debit_trade_its_cost() {
        let spread = Vertical::new(Some(leg(1, "P", d("6690"), d("1.85"), d("1.95"), -0.2)), Some(leg(2, "P", d("6680"), d("1.55"), d("1.65"), -0.16)));
        let credit = EntryOutcome { put: Some(spread.clone()), entry_price: Some(d("0.30")), ..EntryOutcome::default() };
        assert_eq!(buying_power_per_contract("PutSpread", &credit), Some(d("970")), "(10 - 0.30) x 100");
        // An iron condor: the wider of the two sides.
        let wide_call = Vertical::new(Some(leg(3, "C", d("6710"), d("1.50"), d("1.60"), 0.2)), Some(leg(4, "C", d("6725"), d("1.00"), d("1.10"), 0.1)));
        let condor = EntryOutcome { put: Some(spread), call: Some(wide_call), entry_price: Some(d("0.80")), ..EntryOutcome::default() };
        assert_eq!(buying_power_per_contract("IronCondor", &condor), Some(d("1420")), "(15 - 0.80) x 100");
        // A debit trade: what it costs, whatever the sign of its price.
        let debit = EntryOutcome { entry_price: Some(d("-1.35")), ..EntryOutcome::default() };
        assert_eq!(buying_power_per_contract("PutCalendar", &debit), Some(d("135")));
        // No figure: no trade price, a credit side with only one leg, a credit larger than the width.
        assert_eq!(buying_power_per_contract("PutSpread", &EntryOutcome::default()), None);
        let single = EntryOutcome {
            put: Some(Vertical::new(Some(leg(1, "P", d("6690"), d("1.85"), d("1.95"), -0.2)), None)),
            entry_price: Some(d("1.90")),
            ..EntryOutcome::default()
        };
        assert_eq!(buying_power_per_contract("ShortPut", &single), None);
        let narrow = Vertical::new(Some(leg(1, "P", d("6690"), d("1.85"), d("1.95"), -0.2)), Some(leg(2, "P", d("6685"), d("1.55"), d("1.65"), -0.16)));
        let absurd = EntryOutcome { put: Some(narrow), entry_price: Some(d("6")), ..EntryOutcome::default() };
        assert_eq!(buying_power_per_contract("PutSpread", &absurd), None);
    }
}
