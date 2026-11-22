//! Stop and profit-target orders for a trade that was just filled: their prices from the template,
//! the combo legs they close, and the TWS orders (linked in one OCA group, so the fill of one
//! cancels the other).
//!
//! Pure logic on `Decimal` (`coding_guideline.md` sections 2/3); `f64` appears only where an order
//! goes onto the TWS wire.
//!
//! Covered: credit and debit trades, the stop types `Market stop`, `Limit stop`, `Limit stop with OCO`, Trailing
//! and REL (with the ITM protection and the volatility guardrail as order conditions), the stop bases
//! (exact, average, short and long price), all four profit-target levels (the later ones are placed
//! one after the other) and the stop adjustments.
//!
//! Not implemented yet (reported to the user instead of being guessed): ES futures options.

use crate::pricing::{dec, fix_price};
use crate::records::{RelSettings, StopAdjustment, StopAdjustments, StoredExecution, StoredTrade};
use crate::templates::{OrderSwitches, TradeTemplate};
use crate::trade_entry::{Action, ComboLegSpec};
use crate::trade_type::TradeType;
use broker::{IbComboLeg, IbCondition, IbContract, IbOrder, OrderConditionFlags, OrderHandling};
use chrono::{Duration, NaiveDate};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

/// The prices of a trade's exit orders, per contract. A closing combo of a credit trade is bought
/// at a positive price; one of a debit trade at a negative price (what it brings); a single
/// option is always priced positive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExitPrices {
    /// `PriceStopTarget`: the price at which the stop triggers, for every stop type.
    pub stop: Option<Decimal>,
    /// `AuxPrice` of the market stop order (`STP`): the stop price itself for `Market stop`, the
    /// stop price plus the Stop Market Offset for the market backstop of `Limit stop with OCO`.
    pub market: Option<Decimal>,
    /// `LmtPrice` of the stop-limit order (`STP LMT`, triggered at `stop`), for `Limit stop` and
    /// `Limit stop with OCO`.
    pub limit: Option<Decimal>,
    /// The trailing amount of a Trailing stop (`TRAIL`, `AuxPrice`); `stop` is then its first
    /// stop price.
    pub trail: Option<Decimal>,
    /// A REL stop: its limit (`LmtPrice`) and offset (`AuxPrice`). It replaces the market stop
    /// (`market`, whose price is then the price condition of the REL order).
    pub rel: Option<(Decimal, Decimal)>,
    /// `PriceProfitTarget`, when a profit target is to be placed.
    pub profit: Option<Decimal>,
    /// What is not placed, and why (for the log).
    pub notes: Vec<String>,
}

impl ExitPrices {
    /// The prices as one line for the Debug log, "-" for what is not set.
    #[must_use]
    pub fn describe(&self) -> String {
        let show = |price: Option<Decimal>| price.map_or_else(|| "-".to_string(), |p| p.normalize().to_string());
        let rel = self.rel.map_or_else(|| "-".to_string(), |(limit, offset)| format!("{} offset {}", limit.normalize(), offset.normalize()));
        format!(
            "stop {}, market stop {}, stop limit {}, trailing {}, REL limit {rel}, profit target {}",
            show(self.stop),
            show(self.market),
            show(self.limit),
            show(self.trail),
            show(self.profit)
        )
    }
}

/// The kind of stop a template or a stop adjustment asks for (`stop_order_type`), by the names the
/// template files use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOrderType {
    /// No stop ("None", or nothing set).
    None,
    /// A market stop (`STP`).
    StopMarket,
    /// A stop-limit (`STP LMT`).
    StopLimit,
    /// A stop-limit with a market stop beyond it as the backstop ("`Limit stop with OCO`").
    StopLimitOco,
    /// A trailing stop (`TRAIL`).
    Trailing,
}

impl StopOrderType {
    /// The stop type of a name ("" counts as "None"). `None` for a type this app does not place
    /// (e.g. "TrailingPct"): such a stop is reported, never taken for another kind.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "" | "None" => Some(Self::None),
            "Market stop" => Some(Self::StopMarket),
            "Limit stop" => Some(Self::StopLimit),
            "Limit stop with OCO" => Some(Self::StopLimitOco),
            "Trailing" => Some(Self::Trailing),
            _ => None,
        }
    }

    /// The name the template files use.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::StopMarket => "Market stop",
            Self::StopLimit => "Limit stop",
            Self::StopLimitOco => "Limit stop with OCO",
            Self::Trailing => "Trailing",
        }
    }
}

fn dec_opt(value: Option<f64>) -> Option<Decimal> {
    value.and_then(dec)
}

/// The stop price from the entry price : the entry price plus the multiple, plus
/// the offset for a credit trade; a debit trade (negative entry price) loses the multiple of its
/// price instead. An "Amount" stop adds a fixed amount: to the credit, towards 0 for a debit.
fn raw_stop(template: &TradeTemplate, price_open: Decimal) -> Option<Decimal> {
    let multiple = dec(template.stop_multiple)?;
    let offset = dec(template.stop_offset)?;
    let debit = price_open < Decimal::ZERO;
    match (debit, template.stop_target_type == "Amount") {
        (false, true) => price_open.checked_add(multiple)?.checked_add(offset),
        (false, false) => price_open.checked_mul(multiple.checked_add(Decimal::ONE)?)?.checked_add(offset),
        (true, true) => price_open.checked_add(multiple.abs())?.checked_add(offset),
        (true, false) => price_open.checked_sub(price_open.checked_mul(multiple)?)?.checked_add(offset),
    }
}

/// The stop and profit-target prices for a trade entered at `price_open` per contract (positive =
/// credit, negative = debit). `legs` is the number of legs of the closing combo (the tick grid
/// depends on it).
#[cfg(test)]
#[must_use]
pub fn exit_prices(template: &TradeTemplate, symbol: &str, legs: usize, price_open: Decimal) -> ExitPrices {
    exit_prices_with_basis(template, symbol, legs, price_open, price_open)
}

/// `exit_prices` with the stop worked out from `basis` instead of the entry price
/// (see `stop_basis`); the profit targets and the credit/debit decision
/// still follow `price_open`.
#[must_use]
pub fn exit_prices_with_basis(template: &TradeTemplate, symbol: &str, legs: usize, price_open: Decimal, basis: Decimal) -> ExitPrices {
    let mut out = ExitPrices::default();
    if price_open == Decimal::ZERO {
        out.notes.push("the entry price is 0 - no stop or profit target placed".to_string());
        return out;
    }
    if symbol == "ES" {
        out.notes.push("stop and profit target for ES futures options are not supported yet".to_string());
        return out;
    }
    let debit = price_open < Decimal::ZERO;
    // A single option is priced positive; a debit combo's closing price is negative. A single leg
    // of a debit trade is its long, sold to close: exits that leave legs of a debit trade out are
    // refused before (`partial_debit_exit`).
    let single = legs <= 1;
    let on_wire = |price: Decimal| if debit && single { price.abs() } else { price };
    // The side a price must be on to be a valid stop or target: a combo of a debit trade is
    // bought at a negative price, everything else at a positive one.
    let valid = |price: Decimal| if debit && !single { price < Decimal::ZERO } else { price > Decimal::ZERO };

    // Stop (`PriceStopTarget`).
    match StopOrderType::parse(&template.stop_order_type) {
        Some(StopOrderType::None) => {}
        Some(kind @ (StopOrderType::StopMarket | StopOrderType::StopLimit | StopOrderType::StopLimitOco | StopOrderType::Trailing)) => {
            match raw_stop(template, basis).and_then(|raw| fix_price(raw, symbol, legs)).map(on_wire) {
                Some(stop) if valid(stop) => {
                    out.stop = Some(stop);
                    stop_orders(template, kind, symbol, legs, debit && single, stop, &mut out);
                    // A REL stop follows the market at an offset once its price condition holds; it
                    // exists for a single option only, a combo keeps its plain market stop.
                    if template.stop_flags.stop_rel && out.market.is_some() {
                        if single {
                            out.rel = Some(rel_prices(template.stop_rel_limit, template.stop_rel_offset, symbol, legs, stop));
                        } else {
                            out.notes.push("REL works for a single option only - this combo gets a plain market stop".to_string());
                        }
                    }
                }
                _ => out.notes.push("the stop price cannot be computed (or is on the wrong side of 0) - no stop placed".to_string()),
            }
        }
        None => out.notes.push(format!("stop order type '{}' is not supported yet - no stop placed", template.stop_order_type)),
    }

    // Profit target (`PriceProfitTarget`): the first level; the later levels are placed one after
    // the other when the one before is hit (`profit_level_price`).
    if !matches!(template.profit_target_type.as_str(), "" | "None") {
        match profit_level_price(template, symbol, legs, price_open, 0) {
            Some(price) => out.profit = Some(price),
            None => out.notes.push(format!("the profit target ({}) cannot be turned into a price - no profit target placed", template.profit_target_type)),
        }
    }
    out
}

/// The target value of profit-target level `level` (0-3): `profit_target`, `profit_target2` ...
fn level_target(template: &TradeTemplate, level: usize) -> Option<f64> {
    match level {
        0 => template.profit_target,
        1 => template.profit_target2,
        2 => template.profit_target3,
        3 => template.profit_target4,
        _ => None,
    }
}

/// The price of one profit-target level (`PriceProfitTarget` .. `PriceProfitTarget4`) for a trade
/// entered at `price_open` per contract (positive = credit, negative = debit): the entry price
/// less an amount, less or plus a percentage (a debit grows), or the exit price itself. A target
/// of exactly 0 on a credit trade becomes 0.05. `None` when the template has no target for this
/// level, or the price cannot be computed or is on the wrong side of 0.
#[must_use]
pub fn profit_level_price(template: &TradeTemplate, symbol: &str, legs: usize, price_open: Decimal, level: usize) -> Option<Decimal> {
    let debit = price_open < Decimal::ZERO;
    let single = legs <= 1;
    let target = dec_opt(level_target(template, level))?;
    let raw = match (template.profit_target_type.as_str(), debit) {
        ("Amount", _) => price_open.checked_sub(target)?,
        ("Percentage", false) => price_open.checked_mul(Decimal::ONE_HUNDRED.checked_sub(target)?)?.checked_div(Decimal::ONE_HUNDRED)?,
        ("Percentage", true) => price_open.checked_add(price_open.checked_mul(target)?.checked_div(Decimal::ONE_HUNDRED)?)?,
        ("Target price", false) => target,
        ("Target price", true) => -target.abs(),
        _ => return None,
    };
    let fixed = fix_price(raw, symbol, legs)?;
    // A single option is priced positive; a debit combo's closing price is negative.
    let price = if debit && single { fixed.abs() } else { fixed };
    if price == Decimal::ZERO && !debit {
        return Some(Decimal::new(5, 2));
    }
    let valid = if debit && !single { price < Decimal::ZERO } else { price > Decimal::ZERO };
    valid.then_some(price)
}

/// How many contracts each profit-target level closes: level 1 takes its
/// share of the quantity (rounded up), the next levels theirs, and what is left over goes to the
/// last level that has a target - a level without a target or without a share passes its part
/// on to the level before it.
#[must_use]
pub fn split_profit_qty(qty: u32, template: &TradeTemplate) -> [u32; 4] {
    // `ceil(qty * pct / 100)`; an unreadable share counts as 0.
    let share = |pct: Option<f64>| -> u32 {
        let part = dec_opt(pct).and_then(|pct| Decimal::from(qty).checked_mul(pct)?.checked_div(Decimal::ONE_HUNDRED));
        part.map_or(0, |part| part.ceil().to_u32().unwrap_or(0))
    };
    let wanted = |target: Option<f64>, pct: Option<f64>| target.is_some() && pct.is_some_and(|pct| pct > 0.0);
    let mut levels = [0_u32; 4];
    levels[0] = share(Some(template.profit_target_trade_percent.unwrap_or(100.0))).min(qty);
    let mut left = qty.saturating_sub(levels[0]);
    if left > 0 {
        if wanted(template.profit_target2, template.profit_target2_trade_percent) {
            levels[1] = share(template.profit_target2_trade_percent).min(left);
            left -= levels[1];
        } else {
            levels[0] += left;
            left = 0;
        }
    }
    if left > 0 {
        if wanted(template.profit_target3, template.profit_target3_trade_percent) {
            levels[2] = share(template.profit_target3_trade_percent).min(left);
            left -= levels[2];
        } else {
            levels[1] += left;
            left = 0;
        }
    }
    if left > 0 {
        if wanted(template.profit_target4, template.profit_target4_trade_percent) {
            levels[3] = left;
        } else {
            levels[2] += left;
        }
    }
    levels
}

/// The price per contract (negative = paid to close) a trade averaged over its closes: the
/// profit-target levels `closed` so far (contracts and price), plus `remaining` contracts at
/// `final_price`, divided by `total` contracts, rounded half away from zero to 4 decimals.
#[must_use]
pub fn average_close(closed: &[(u32, String)], remaining: u32, final_price: Decimal, total: u32) -> Option<Decimal> {
    let mut sum = final_price.checked_mul(Decimal::from(remaining))?;
    for (qty, price) in closed {
        sum = sum.checked_add(price.parse::<Decimal>().ok()?.checked_mul(Decimal::from(*qty))?)?;
    }
    sum.checked_div(Decimal::from(total))?.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero).into()
}

/// The prices of the stop orders of `stop` for the stop order type `kind` (the template's offsets):
/// a market stop, a stop-limit, or both (`Limit stop with OCO`: the limit triggers at the stop, the market
/// order a Stop Market Offset beyond it as the backstop), or a trailing stop. `sell` is a single
/// option that is sold to close: its limit lies below the trigger, the backstop too.
fn stop_orders(template: &TradeTemplate, kind: StopOrderType, symbol: &str, legs: usize, sell: bool, stop: Decimal, out: &mut ExitPrices) {
    let beyond = |offset: Decimal| if sell { stop.checked_sub(offset) } else { stop.checked_add(offset) };
    // A `Limit stop with OCO` keeps its limit offset within 15 % of the stop and 0.50, whatever the template says: a limit too far from the
    // stop is refused by TWS and would leave only the market backstop.
    let limit = |capped: bool| -> Option<Decimal> {
        // `stop_limit_offset` defaults to 0.10.
        let mut offset = dec_opt(template.stop_limit_offset).unwrap_or(Decimal::new(10, 2));
        if capped {
            offset = offset.min(stop.checked_mul(Decimal::new(15, 2))?.min(Decimal::new(50, 2)));
        }
        fix_price(beyond(offset)?, symbol, legs)
    };
    match kind {
        StopOrderType::Trailing => {
            // The stop follows the price at the template's trailing amount (`stop_offset`).
            let trail = dec_opt(Some(template.stop_offset))
                .map(|offset| offset.abs())
                .filter(|offset| *offset > Decimal::ZERO)
                .and_then(|offset| fix_price(offset, symbol, legs));
            if let Some(trail) = trail {
                out.trail = Some(trail);
            } else {
                out.stop = None;
                out.notes.push("a Trailing stop needs a trailing amount (Stop Offset) above 0 - no stop placed".to_string());
            }
        }
        StopOrderType::StopLimit => {
            if let Some(limit) = limit(false) {
                out.limit = Some(limit);
            } else {
                out.stop = None;
                out.notes.push("the stop-limit price cannot be computed - no stop placed".to_string());
            }
        }
        StopOrderType::StopLimitOco => {
            let market = dec_opt(template.stop_limit_market_offset).and_then(beyond).and_then(|p| fix_price(p, symbol, legs));
            if let (Some(limit), Some(market)) = (limit(true), market) {
                out.limit = Some(limit);
                out.market = Some(market);
            } else {
                out.stop = None;
                out.notes.push("StopLimitOCO needs a Stop Limit Offset and a Stop Market Offset - no stop placed".to_string());
            }
        }
        StopOrderType::StopMarket => out.market = Some(stop),
        StopOrderType::None => out.stop = None,
    }
}

/// What the legs of a trade's opening order filled at, per contract of the trade: the sold legs
/// added up (`Trade.PriceShort`) and the bought ones (`Trade.PriceLong`), each leg weighted by its
/// side's ratio. `None` when the trade has no leg of that kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LegPrices {
    /// What the sold legs brought.
    pub short: Option<Decimal>,
    /// What the bought legs cost.
    pub long: Option<Decimal>,
}

/// The fill prices of the legs of `trade`, from the executions of its opening order (matched by
/// the order reference "`TradePilot` `OrderID`: N (Opening)"). A leg filled in several parts counts
/// at its average price. Empty (`LegPrices::default()`) when any leg of the trade has no
/// execution yet: the prices are then not known, and `stop_basis` falls back to the entry price.
#[must_use]
pub fn leg_prices(trade: &StoredTrade, executions: &[StoredExecution]) -> LegPrices {
    let average = |con_id: i32| -> Option<Decimal> {
        let (value, qty) = executions
            .iter()
            .filter(|e| e.con_id == con_id && crate::trade_rebuild::opening_order_id(&e.order_ref) == Some(trade.order_id))
            .try_fold((Decimal::ZERO, Decimal::ZERO), |(value, qty), e| {
                let (price, size) = (dec(e.price)?, dec(e.quantity)?);
                Some((value.checked_add(price.checked_mul(size)?)?, qty.checked_add(size)?))
            })?;
        (qty > Decimal::ZERO).then(|| value.checked_div(qty)).flatten()
    };
    let put_ratio = Decimal::from(trade.put_ratio.max(1));
    let call_ratio = Decimal::from(trade.call_ratio.max(1));
    let legs = [
        (&trade.short_put, put_ratio, true),
        (&trade.short_call, call_ratio, true),
        (&trade.long_put, put_ratio, false),
        (&trade.long_call, call_ratio, false),
    ];
    let (mut short, mut long) = (None::<Decimal>, None::<Decimal>);
    for (leg, ratio, sold) in legs {
        let Some(leg) = leg else { continue };
        let Some(price) = average(leg.con_id).and_then(|price| price.checked_mul(ratio)) else { return LegPrices::default() };
        let slot = if sold { &mut short } else { &mut long };
        let Some(sum) = slot.unwrap_or(Decimal::ZERO).checked_add(price) else { return LegPrices::default() };
        *slot = Some(sum);
    }
    LegPrices { short, long }
}

/// The average entry price of the trades one opening order created (an iron condor's two sides):
/// only when there are exactly two (`StopBasisPrice` of "Mean price"). Rounded half away from
/// zero to 4 decimals.
#[must_use]
pub fn average_entry(siblings: &[&StoredTrade]) -> Option<Decimal> {
    let [a, b] = siblings else { return None };
    let sum = a.price_open.checked_add(b.price_open)?;
    Some(sum.checked_div(Decimal::TWO)?.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero))
}

/// The price the stop is worked out from and what to log about it: the
/// entry price ("Given price"), the two sides' average ("Mean price"), what the sold legs
/// ("Short leg price") or the bought legs ("Long leg price") filled at. A basis that is not known (no
/// executions yet, no such leg, a single trade for an average) falls back to the entry price
/// and says so; so does a debit trade: these bases are for credit trades.
#[must_use]
pub fn stop_basis(basis: &str, price_open: Decimal, legs: LegPrices, average: Option<Decimal>) -> (Decimal, Option<String>) {
    if matches!(basis, "" | "Given price") {
        return (price_open, None);
    }
    if price_open <= Decimal::ZERO {
        return (price_open, Some(format!("stop basis '{basis}' is for credit trades - the exact entry price is used")));
    }
    let chosen = match basis {
        "Mean price" => average,
        "Short leg price" => legs.short,
        "Long leg price" => legs.long,
        _ => None,
    };
    match chosen.map(|price| price.abs()).filter(|price| *price > Decimal::ZERO) {
        Some(price) => (price, None),
        None => {
            (price_open, Some(format!("stop basis '{basis}' is not known (no fill prices of the legs yet, or no such leg) - the exact entry price is used")))
        }
    }
}

/// The limit and offset of a REL order: the template's on the tick grid,
/// 8 and 0.5 without; a limit below the stop price is lifted to the stop price plus 2.
fn rel_prices(limit: Option<f64>, offset: Option<f64>, symbol: &str, legs: usize, stop: Decimal) -> (Decimal, Decimal) {
    let limit = dec_opt(limit).and_then(|limit| fix_price(limit, symbol, legs)).unwrap_or(Decimal::from(8));
    let offset = dec_opt(offset).and_then(|offset| fix_price(offset, symbol, legs)).unwrap_or(Decimal::new(5, 1));
    let limit = if limit < stop { stop.checked_add(Decimal::TWO).unwrap_or(limit) } else { limit };
    (limit, offset)
}

/// TWS's contract id of an underlying, for the price conditions that watch it.
#[must_use]
pub fn underlying_con_id(symbol: &str) -> Option<i32> {
    Some(match symbol {
        "SPX" => 416_904,
        "SPY" => 756_733,
        "QQQ" => 320_227_571,
        "AAPL" => 265_598,
        "NDX" => 416_843,
        "MSFT" => 272_093,
        "XSP" => 137_851_301,
        "IWM" => 9_579_970,
        "GLD" => 51_529_211,
        "TSLA" => 76_792_991,
        "NVDA" => 4_815_747,
        "AMZN" => 3_691_937,
        "GOOGL" => 208_813_719,
        "META" => 107_113_386,
        "IBIT" => 677_037_673,
        "RUT" => 416_888,
        "SPCX" => 890_493_863,
        _ => return None,
    })
}

/// The exchange a price condition on an underlying names.
fn underlying_exchange(symbol: &str) -> &'static str {
    match symbol {
        "SPX" | "XSP" => "CBOE",
        "RUT" => "RUSSELL",
        "NDX" => "NASDAQ",
        _ => "SMART",
    }
}

/// A time `minutes` before the close of the day `expiry` (yyyyMMdd) as TWS's time condition
/// reads it: "yyyyMMdd HH:mm:00 Zone". The close is 16:00 US/Eastern (SPX: 15:00 US/Central, the
/// same moment), 13:00 on an early-close day. `None` for an unreadable date.
fn close_time(expiry: &str, symbol: &str, minutes: i32, early_close: bool) -> Option<String> {
    let date = NaiveDate::parse_from_str(expiry, "%Y%m%d").ok()?;
    let (hour, zone) = if symbol == "SPX" { (15, "US/Central") } else { (16, "US/Eastern") };
    let close = date.and_hms_opt(hour, 0, 0)?;
    let close = if early_close { close - Duration::hours(3) } else { close };
    let at = close - Duration::minutes(i64::from(minutes));
    Some(format!("{} {zone}", at.format("%Y%m%d %H:%M:00")))
}

/// One step of the volatility spike protection: from `minutes_from_close` before the close on, the
/// underlying has to be at or below (a put side) / at or above (a call side) `price`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rail {
    /// Minutes before the close the step ends at.
    pub minutes_from_close: i32,
    /// The price the underlying is compared with, rounded to cents (half to even).
    pub price: Decimal,
    /// The underlying has to be at or above `price` (a call side), else at or below it.
    pub greater_than: bool,
}

/// The twelve steps of the guardrail (`Trade.Rails`): the price path runs from the underlying's
/// price at the open, kept `offset_open` away from the strike, to `offset_exp` away from the strike
/// at the close, in equal steps, one every twelfth of the time left. Only for a `ShortPut` or
/// `PutSpread` (put side) and a `ShortCall` or `CallSpread` (call side) that expires today; empty
/// otherwise and on an unreadable price.
#[must_use]
pub fn guardrails(trade: &StoredTrade, rel: &RelSettings) -> Vec<Rail> {
    let rails = || -> Option<Vec<Rail>> {
        if !rel.guardrail || rel.minutes_to_close <= 0 {
            return None;
        }
        let put = matches!(trade.trade_type.as_str(), "ShortPut" | "PutSpread");
        let call = matches!(trade.trade_type.as_str(), "ShortCall" | "CallSpread");
        let strike: Decimal = if put {
            trade.short_put.as_ref()?
        } else if call {
            trade.short_call.as_ref()?
        } else {
            return None;
        }
        .strike
        .parse()
        .ok()?;
        let underlying: Decimal = rel.underlying_open.as_deref()?.parse().ok()?;
        let (open, exp) = (Decimal::from(rel.offset_open), Decimal::from(rel.offset_exp));
        // The path starts at `from` and ends at `base`.
        let (from, base) =
            if put { (underlying.checked_add(open)?, strike.checked_add(exp)?) } else { (underlying.checked_sub(open)?, strike.checked_sub(exp)?) };
        let step = from.checked_sub(base)?.checked_div(Decimal::from(11))?;
        // A twelfth of the minutes left, rounded up.
        let per = rel.minutes_to_close.checked_add(11)?.checked_div(12)?;
        let extra = if rel.itm_only { rel.itm_minutes.unwrap_or(30) } else { 0 };
        (1..=12_i32)
            .rev()
            .map(|k| {
                let price = base.checked_add(step.checked_mul(Decimal::from(k - 1))?)?;
                Some(Rail {
                    minutes_from_close: per.checked_mul(k - 1)?.checked_add(extra)?,
                    price: price.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointNearestEven),
                    greater_than: call,
                })
            })
            .collect()
    };
    rails().unwrap_or_default()
}

/// The conditions of a REL stop: the option's price condition at the stop
/// price (at or above it for a credit trade), and - from the template - the ITM protection (the
/// underlying in the money by `itm` points AND in the last minutes before the close), the "ITM
/// only" window, and the guardrail path. A guardrail that has no steps (the trade expires later)
/// is left out, not turned into a stop without its price condition. `None` when a condition
/// needs what is not known: the underlying's contract id, a strike, a date.
#[must_use]
pub fn rel_conditions(trade: &StoredTrade, option_con_id: i32, stop_price: Decimal, trigger_method: i32, rel: &RelSettings) -> Option<Vec<IbCondition>> {
    let debit = trade.price_open < Decimal::ZERO;
    // Only the triggers default, bid/ask and mid-point are mapped; any other is left at default.
    let method = if matches!(trigger_method, 4 | 8) { trigger_method } else { 0 };
    let symbol = trade.symbol.as_str();
    let option = |and_next: bool| -> Option<IbCondition> {
        Some(IbCondition::Price {
            and_next,
            is_more: !debit,
            price: stop_price.to_f64()?,
            con_id: option_con_id,
            exchange: "SMART".to_string(),
            trigger_method: method,
        })
    };
    let rails = guardrails(trade, rel);
    let underlying = underlying_con_id(symbol);
    if (rel.itm.is_some() || !rails.is_empty()) && underlying.is_none() {
        return None;
    }
    let on_underlying = |and_next: bool, is_more: bool, price: f64| -> Option<IbCondition> {
        Some(IbCondition::Price { and_next, is_more, price, con_id: underlying?, exchange: underlying_exchange(symbol).to_string(), trigger_method: 0 })
    };
    let itm_minutes = rel.itm_minutes.unwrap_or(30);
    let time = |minutes: i32, and_next: bool, is_more: bool| -> Option<IbCondition> {
        Some(IbCondition::Time { and_next, is_more, time: close_time(&trade.expiry, symbol, minutes, rel.early_close)? })
    };
    // The ITM protection: the underlying in the money by `itm` points, and the closing minutes.
    let itm = match rel.itm {
        Some(points) => {
            // The option the stop watches: the one of a custom trade that the order closes, else the short (or only) option of the side.
            let (put_side, strike): (bool, Decimal) = if let Some(custom) = &trade.custom_legs {
                let leg = custom.iter().find(|l| l.con_id == option_con_id)?;
                (leg.right == "P", leg.strike.parse().ok()?)
            } else {
                let put_side = matches!(trade.trade_type.as_str(), "PutSpread" | "ShortPut" | "LongPut" | "PutCalendar");
                let leg = if put_side { trade.short_put.as_ref().or(trade.long_put.as_ref()) } else { trade.short_call.as_ref().or(trade.long_call.as_ref()) }?;
                (put_side, leg.strike.parse().ok()?)
            };
            let offset = Decimal::from(points);
            let price = if put_side { strike.checked_add(offset)? } else { strike.checked_sub(offset)? };
            Some((on_underlying(true, !put_side, price.to_f64()?)?, time(itm_minutes, false, true)?))
        }
        None => None,
    };
    let mut conditions = Vec::new();
    if rel.guardrail && !rails.is_empty() {
        for rail in &rails {
            conditions.push(time(rail.minutes_from_close, true, false)?);
            conditions.push(option(true)?);
            conditions.push(on_underlying(false, rail.greater_than, rail.price.to_f64()?)?);
        }
    } else {
        if rel.itm_only {
            conditions.push(time(itm_minutes, true, false)?);
        }
        conditions.push(option(false)?);
    }
    if let Some((price, window)) = itm {
        conditions.push(price);
        conditions.push(window);
    }
    Some(conditions)
}

/// Gives the REL stop `order` its conditions (`rel_conditions`) and lets them also hold outside
/// regular hours when the order may. `false` when they cannot be built: the order must then not be
/// placed, because a REL order without conditions would be active at once.
pub fn attach_rel_conditions(
    order: &mut IbOrder,
    trade: &StoredTrade,
    option_con_id: i32,
    stop_price: Decimal,
    rel: &RelSettings,
    trigger_method: i32,
) -> bool {
    match rel_conditions(trade, option_con_id, stop_price, trigger_method, rel) {
        Some(conditions) if !conditions.is_empty() => {
            order.conditions = conditions;
            order.condition_flags.conditions_ignore_rth = order.handling.outside_rth;
            true
        }
        _ => false,
    }
}

/// Whether an adjustment kind is one of the `profit_target` triggers, and which level it needs.
fn profit_target_level_of(kind: &str) -> Option<u8> {
    kind.strip_prefix("Target ").and_then(|n| n.parse().ok())
}

/// The price a trade's `current_bid` has to reach for the adjustment (`Adjustment1PriceTrigger`):
/// for a credit trade a profit level (percent of the credit gone), an amount, or a profit; for a
/// debit trade the value its position has to reach. `None` for the `profit_target` kinds, for no
/// adjustment, and when the trigger value is missing or the price cannot be computed.
#[must_use]
pub fn adjustment_trigger(adjustment: &StopAdjustment, price_open: Decimal) -> Option<Decimal> {
    let value = dec_opt(adjustment.value)?;
    let debit = price_open < Decimal::ZERO;
    match (adjustment.kind.as_str(), debit) {
        ("Profit percent", false) => price_open.checked_mul(Decimal::ONE_HUNDRED.checked_sub(value)?)?.checked_div(Decimal::ONE_HUNDRED),
        ("Profit percent", true) => Some(price_open.checked_mul(Decimal::ONE_HUNDRED.checked_add(value)?)?.checked_div(Decimal::ONE_HUNDRED)?.abs()),
        ("Amount", _) => Some(value),
        ("Profit", false) => price_open.checked_sub(value),
        ("Profit", true) => Some(price_open.abs().checked_add(value)?),
        _ => None,
    }
}

/// Whether the adjustment is hit (`Adjustment1Hit`): a `ProfitTargetN` kind when profit-target level
/// N was hit (`profit_level` = levels hit so far), any other kind when `current_bid` - what the
/// position would bring now - reached the trigger: at or below it for a credit trade, at or above
/// it (as an amount) for a debit trade.
#[must_use]
pub fn adjustment_hit(adjustment: &StopAdjustment, price_open: Decimal, current_bid: Decimal, profit_level: u8) -> bool {
    if let Some(level) = profit_target_level_of(&adjustment.kind) {
        return level >= 1 && profit_level >= level;
    }
    let Some(trigger) = adjustment_trigger(adjustment, price_open) else { return false };
    if price_open < Decimal::ZERO {
        current_bid.abs() >= trigger
    } else {
        current_bid <= trigger
    }
}

/// The stop price an adjustment sets (`Adjustment1PriceNew`), on the tick grid of `legs` legs:
/// a fixed amount, or the entry price with the multiple added (credit) or taken off (debit), plus
/// the change offset - unless the new or the current stop is a `Limit stop`, where that offset is the
/// limit's. `None` when there is no change value or a credit stop would be below 0.
#[must_use]
pub fn adjustment_new_stop(adjustment: &StopAdjustment, current_order_type: &str, price_open: Decimal, symbol: &str, legs: usize) -> Option<Decimal> {
    if matches!(adjustment.kind.as_str(), "" | "None") {
        return None;
    }
    let change = dec_opt(adjustment.change)?;
    let debit = price_open < Decimal::ZERO;
    let mut price = match (adjustment.change_type.as_str(), debit) {
        ("Fixed price", _) => change,
        ("Stop factor", false) => price_open.checked_add(price_open.checked_mul(change)?)?,
        ("Stop factor", true) => price_open.checked_sub(price_open.checked_mul(change)?)?,
        // No price from an unknown change type: with an offset it would otherwise become a stop
        // at the offset alone.
        _ => return None,
    };
    if let Some(offset) = dec_opt(adjustment.change_offset) {
        let stop_limit = |name: &str| StopOrderType::parse(name) == Some(StopOrderType::StopLimit);
        if !stop_limit(&adjustment.order_type) && !stop_limit(current_order_type) {
            price = price.checked_add(offset)?;
        }
    }
    if price < Decimal::ZERO && !debit {
        return None;
    }
    fix_price(price, symbol, legs).filter(|fixed| *fixed != Decimal::ZERO)
}

/// The orders of the stop after an adjustment: the stop as `adjustments` describes it now,
/// closing the same legs, joined to the OCA `group` of the trade's profit target. `None` when
/// there is no stop any more ("None"), its stop order type is not supported, or its price or
/// contract cannot be put on the wire. `stop_id` is the id of the market or trailing stop,
/// `limit_id` that of the stop-limit.
pub fn replacement_stop_orders(
    trade: &StoredTrade,
    adjustments: &StopAdjustments,
    today: &str,
    group: &str,
    stop_id: i32,
    limit_id: i32,
) -> Option<ExitOrders> {
    let kind = StopOrderType::parse(&adjustments.order_type)?;
    if kind == StopOrderType::None {
        return None;
    }
    let legs: Vec<ComboLegSpec> = adjustments.legs.iter().map(ComboLegSpec::from_stored).collect();
    let template = TradeTemplate {
        stop_order_type: adjustments.order_type.clone(),
        stop_limit_offset: adjustments.limit_offset,
        stop_limit_market_offset: adjustments.market_offset,
        stop_offset: adjustments.trailing_offset.as_deref().and_then(|text| text.parse::<f64>().ok()).unwrap_or(0.0),
        stop_trigger: adjustments.trigger_method,
        order_flags: OrderSwitches { extended_hour_stop: adjustments.outside_rth, ..OrderSwitches::default() },
        ..TradeTemplate::default()
    };
    let stop: Decimal = adjustments.stop.as_deref()?.parse().ok()?;
    // A single option that is sold to close: its limit and backstop lie below the trigger.
    let sell = matches!(legs.as_slice(), [only] if only.action == Action::Sell);
    let mut prices = ExitPrices { stop: Some(stop), ..ExitPrices::default() };
    stop_orders(&template, kind, &trade.symbol, legs.len(), sell, stop, &mut prices);
    let rel = adjustments.rel.as_ref().filter(|_| prices.market.is_some() && legs.len() == 1);
    if let Some(rel) = rel {
        prices.rel = Some(rel_prices(rel.limit, rel.offset, &trade.symbol, legs.len(), stop));
    }
    let mut orders = exit_orders(trade, &template, &prices, &legs, today, stop_id, 0, limit_id, 0)?;
    if let (Some(rel), Some(order), Some(leg), Some(price)) = (rel, orders.stop.as_mut(), legs.first(), prices.market) {
        // Without its conditions a REL stop is not placed at all.
        if !attach_rel_conditions(order, trade, leg.con_id, price, rel, adjustments.trigger_method) {
            orders.stop = None;
        }
    }
    for order in [orders.stop.as_mut(), orders.stop_limit.as_mut()].into_iter().flatten() {
        order.oca_group = group.to_string();
    }
    (orders.stop.is_some() || orders.stop_limit.is_some()).then_some(orders)
}

/// The legs the exit orders close, in the order short call, short put, long put,
/// long call): the shorts are bought back; a long is sold too for a Vertical stop (and when the
/// trade has no stop at all), unless it has no bid (`has_bid` false), which is converted
/// to a Short stop. The debit types sell their longs whatever the stop type says (a `LongPut` or
/// `LongCall` always, the others unless the long has no bid); the sold types close only their shorts.
pub fn exit_legs(trade: &StoredTrade, template: &TradeTemplate, has_bid: &dyn Fn(i32) -> bool) -> Vec<ComboLegSpec> {
    if trade.custom_legs.is_some() {
        // A custom trade is stopped and targeted as a whole: every leg is closed.
        return close_legs(trade);
    }
    let trade_type = trade.trade_type.as_str();
    let no_stop = StopOrderType::parse(&template.stop_order_type) == Some(StopOrderType::None);
    let kind = |property: fn(TradeType) -> bool| crate::trade_type::is(trade_type, property);
    let debit_longs = kind(TradeType::closes_longs_always);
    let shorts_only = kind(TradeType::sells_only);
    let by_stop_type = |kinds: &[&str]| no_stop || kinds.contains(&template.stop_type.as_str());
    let put_longs = !shorts_only && (debit_longs || by_stop_type(&["Vertical", "VerticalP"]));
    let call_longs = !shorts_only && (debit_longs || by_stop_type(&["Vertical", "VerticalC"]));
    let single_long = kind(TradeType::is_single_long);
    let put_ratio = i32::try_from(trade.put_ratio.max(1)).unwrap_or(1);
    let call_ratio = i32::try_from(trade.call_ratio.max(1)).unwrap_or(1);
    let mut legs = Vec::new();
    if let Some(leg) = &trade.short_call {
        legs.push(ComboLegSpec { con_id: leg.con_id, ratio: call_ratio, action: Action::Buy });
    }
    if let Some(leg) = &trade.short_put {
        legs.push(ComboLegSpec { con_id: leg.con_id, ratio: put_ratio, action: Action::Buy });
    }
    if let (true, Some(leg)) = (put_longs, &trade.long_put) {
        if single_long || has_bid(leg.con_id) {
            legs.push(ComboLegSpec { con_id: leg.con_id, ratio: put_ratio, action: Action::Sell });
        }
    }
    if let (true, Some(leg)) = (call_longs, &trade.long_call) {
        if single_long || has_bid(leg.con_id) {
            legs.push(ComboLegSpec { con_id: leg.con_id, ratio: call_ratio, action: Action::Sell });
        }
    }
    legs
}

/// Why the exit orders of a debit trade cannot be priced for the legs they close, `None` when they
/// can. A debit trade's stop and target are worked out from what the whole position cost, so they
/// only fit orders that close every one of its legs. A calendar with a Short stop (only the near
/// leg bought back), or a debit spread whose long has no bid, would put a price meant for the
/// whole position on part of it - and for a single bought-back leg on the wrong side of the
/// market, as the price rules treat a single leg of a debit trade as a long that is sold. A credit
/// trade's exit closing only its shorts at a multiple of its credit is the usual stop and stays allowed.
#[must_use]
pub fn partial_debit_exit(trade: &StoredTrade, legs: &[ComboLegSpec]) -> Option<String> {
    let debit = trade.price_open < Decimal::ZERO;
    let held = [&trade.short_put, &trade.long_put, &trade.short_call, &trade.long_call].iter().filter(|leg| leg.is_some()).count();
    (debit && legs.len() < held).then(|| {
        format!(
            "the exit would close {} of the {held} legs of this debit trade, but its prices are for the whole position (a calendar needs the Stop Type Vertical; a long without a bid cannot be sold)",
            legs.len()
        )
    })
}

/// The legs that close the whole trade (every leg reversed): what "Close Trade" sends.
#[must_use]
pub fn close_legs(trade: &StoredTrade) -> Vec<ComboLegSpec> {
    if let Some(custom) = &trade.custom_legs {
        // A custom trade: every leg the other way round.
        return custom.iter().map(|leg| ComboLegSpec { con_id: leg.con_id, ratio: leg.ratio.abs().max(1), action: Action::of(leg.ratio > 0) }).collect();
    }
    let put_ratio = i32::try_from(trade.put_ratio.max(1)).unwrap_or(1);
    let call_ratio = i32::try_from(trade.call_ratio.max(1)).unwrap_or(1);
    let mut legs = Vec::new();
    for (leg, ratio, action) in [
        (&trade.long_put, put_ratio, Action::Sell),
        (&trade.short_put, put_ratio, Action::Buy),
        (&trade.long_call, call_ratio, Action::Sell),
        (&trade.short_call, call_ratio, Action::Buy),
    ] {
        if let Some(leg) = leg {
            legs.push(ComboLegSpec { con_id: leg.con_id, ratio, action });
        }
    }
    legs
}

/// The TWS contract of a closing order: the combo of `legs`, or the one option when a single leg
/// is left. `None` for no legs or a strike that cannot be sent.
#[must_use]
pub fn exit_contract(trade: &StoredTrade, legs: &[ComboLegSpec]) -> Option<IbContract> {
    let exchange = "SMART".to_string();
    let mut contract = IbContract { symbol: trade.symbol.clone(), exchange: exchange.clone(), currency: "USD".to_string(), ..IbContract::default() };
    match legs {
        [] => None,
        [only] => {
            if let Some(leg) = trade.custom_legs.as_ref().and_then(|legs| legs.iter().find(|l| l.con_id == only.con_id)) {
                contract.sec_type = "OPT".to_string();
                contract.trading_class = crate::trade_entry::trading_class(&trade.symbol);
                contract.last_trade_date_or_contract_month.clone_from(&leg.expiry);
                contract.right.clone_from(&leg.right);
                contract.strike = leg.strike.parse::<Decimal>().ok()?.to_f64()?;
                return Some(contract);
            }
            let (leg, right) = [(&trade.short_put, "P"), (&trade.long_put, "P"), (&trade.short_call, "C"), (&trade.long_call, "C")]
                .into_iter()
                .find_map(|(leg, right)| leg.as_ref().filter(|l| l.con_id == only.con_id).map(|l| (l, right)))?;
            contract.sec_type = "OPT".to_string();
            contract.trading_class = crate::trade_entry::trading_class(&trade.symbol);
            contract.last_trade_date_or_contract_month.clone_from(&trade.expiry);
            contract.right = right.to_string();
            contract.strike = leg.strike.parse::<Decimal>().ok()?.to_f64()?;
            Some(contract)
        }
        many => {
            contract.sec_type = "BAG".to_string();
            contract.combo_legs = many
                .iter()
                .map(|l| IbComboLeg { con_id: l.con_id, ratio: l.ratio, action: l.action.as_str().to_string(), exchange: exchange.clone() })
                .collect();
            Some(contract)
        }
    }
}

/// The action of a closing order over `legs`: a single long option is sold; a single sold option is
/// bought back, and so is a combo (its legs carry their own actions).
#[must_use]
pub fn closing_action(legs: &[ComboLegSpec]) -> Action {
    match legs {
        [only] => only.action,
        _ => Action::Buy,
    }
}

/// The OCA group a trade's exit orders share, named after the first of the three order ids they
/// are placed with: the stop's is that id, the profit target's the next one and the stop-limit's
/// the one after. A later close order joins the group (`OcaType` 3), so a fill of any of them
/// reduces the others and a stop cannot fire while the trade is being closed.
#[must_use]
pub fn oca_group(first_id: i32) -> String {
    format!("TradePilot-{first_id}")
}

/// The OCA group of a stored trade's exit orders, `None` for a trade without any: the group stored
/// when they were placed. Only a trade of an earlier version, which has none stored, falls back to
/// deriving it from the order ids - that holds only as long as no stop has been replaced.
pub fn trade_oca_group(trade: &StoredTrade) -> Option<String> {
    if let Some(group) = trade.oca_group.as_ref().filter(|group| !group.is_empty()) {
        return Some(group.clone());
    }
    trade
        .stop_order_id
        .or_else(|| trade.profit_order_id.map(|id| id.saturating_sub(1)))
        .or_else(|| trade.stop_limit_order_id.map(|id| id.saturating_sub(2)))
        .map(oca_group)
}

/// The timed exit of `template` for a trade filled at `now` (New York time) that expires on `expiry` (yyyyMMdd): `exit_minutes_in_trade`
/// minutes after the fill, else the exit time of the day (`exit_hour`:`exit_minute`) on the day that is `exit_dte` days before the
/// expiry (0: the expiry day itself; a day without trading, a weekend or a holiday, gives way to the trading day before it). `None` when
/// the template has neither. An exit time that has passed already counts as now, so a trade entered after it is closed at once. An
/// expiry that cannot be read counts as the day of the fill.
#[must_use]
pub fn timed_exit_of(template: &TradeTemplate, expiry: &str, now: chrono::NaiveDateTime) -> Option<crate::records::TimedExit> {
    let at = if template.exit_minutes_in_trade > 0 {
        now.checked_add_signed(Duration::minutes(i64::from(template.exit_minutes_in_trade)))?
    } else if template.exit_hour > 0 {
        let hour = u32::try_from(template.exit_hour).ok()?;
        let minute = u32::try_from(template.exit_minute.max(0)).ok()?;
        let days_before = chrono::Days::new(u64::try_from(template.exit_dte.max(0)).ok()?);
        let day =
            chrono::NaiveDate::parse_from_str(expiry, "%Y%m%d").ok().and_then(|expiry| expiry.checked_sub_days(days_before)).unwrap_or_else(|| now.date());
        trading_day_on_or_before(day).and_hms_opt(hour, minute, 0)?.max(now)
    } else {
        return None;
    };
    Some(crate::records::TimedExit { at: at.format("%Y-%m-%d %H:%M").to_string(), limit: template.exit_flags.exit_order_limit })
}

/// `day` when the market trades on it, else the last trading day before it.
fn trading_day_on_or_before(day: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    let mut day = day;
    // A week and a half holds no more than a weekend and a holiday; the bound only keeps a loop from running for ever.
    for _ in 0..10 {
        if matches!(day.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) || crate::market_calendar::holiday(day).is_some() {
            match day.pred_opt() {
                Some(before) => day = before,
                None => break,
            }
        } else {
            break;
        }
    }
    day
}

/// When the profit target of a trade of `template` is cancelled: `profit_target_expiration_hour`:`profit_target_expiration_minute` (New York
/// time) on the expiry day (`expiry` as yyyyMMdd; the day of `now` when it cannot be read). `None` when the template sets no
/// such time or has no profit target.
#[must_use]
pub fn profit_cancel_of(template: &TradeTemplate, expiry: &str, now: chrono::NaiveDateTime) -> Option<String> {
    if template.profit_target_expiration_hour <= 0 || matches!(template.profit_target_type.as_str(), "" | "None") {
        return None;
    }
    let hour = u32::try_from(template.profit_target_expiration_hour).ok()?;
    let minute = u32::try_from(template.profit_target_expiration_minute.max(0)).ok()?;
    let day = chrono::NaiveDate::parse_from_str(expiry, "%Y%m%d").unwrap_or_else(|_| now.date());
    Some(day.and_hms_opt(hour, minute, 0)?.format("%Y-%m-%d %H:%M").to_string())
}

/// The stop and profit-target orders of one trade.
pub struct ExitOrders {
    /// The contract the orders are sent for.
    pub contract: IbContract,
    /// The market stop (`STP`, `AuxPrice` = stop price), if one is placed.
    pub stop: Option<IbOrder>,
    /// The stop-limit (`STP LMT`, `AuxPrice` = stop price, `LmtPrice` = limit), if one is placed.
    pub stop_limit: Option<IbOrder>,
    /// The profit target (`LMT`), if one is placed.
    pub profit: Option<IbOrder>,
}

/// The TWS orders for `prices` over `legs`. `today` is New York's date (yyyyMMdd): a trade that
/// expires today gets DAY orders, any other GTC. They always share one OCA group
/// (`OcaType` 3: a fill cancels the others); the group is named after `stop_id`, `profit_id` and
/// `limit_id` are the ids of the profit target and the stop-limit. A closing combo is bought; a
/// single long option is sold, a single short one bought. The stops cover the contracts still
/// open (`remaining_qty`: all of them at the fill, fewer once profit-target levels have closed
/// some), never more - a stop for more than is open would turn into a new, opposite position.
/// `None` when the contract or a price cannot be put on the wire.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn exit_orders(
    trade: &StoredTrade,
    template: &TradeTemplate,
    prices: &ExitPrices,
    legs: &[ComboLegSpec],
    today: &str,
    stop_id: i32,
    profit_id: i32,
    limit_id: i32,
    profit_qty: u32,
) -> Option<ExitOrders> {
    let contract = exit_contract(trade, legs)?;
    let tif = if trade.expiry == today { "DAY" } else { "GTC" };
    let group = oca_group(stop_id);
    let stop_qty = trade.remaining_qty();
    let action = closing_action(legs).as_str();
    let base = |reference: String, qty: u32| IbOrder {
        action: action.to_string(),
        total_quantity: f64::from(qty),
        tif: tif.to_string(),
        account: trade.account.clone(),
        order_ref: reference,
        oca_group: group.clone(),
        oca_type: 3,
        handling: OrderHandling { transmit: true, ..OrderHandling::default() },
        ..IbOrder::default()
    };
    let stop = match (prices.rel, prices.market, prices.trail, prices.stop) {
        // The conditions of a REL order are added by the caller (`attach_rel_conditions`): without
        // them the order would be active at once.
        (Some((limit, offset)), Some(_), _, _) => Some(IbOrder {
            order_type: "REL".to_string(),
            lmt_price: limit.to_f64()?,
            aux_price: offset.to_f64()?,
            handling: OrderHandling { outside_rth: template.order_flags.extended_hour_stop, ..OrderHandling::default() },
            condition_flags: OrderConditionFlags { conditions_ignore_rth: template.order_flags.extended_hour_stop, ..OrderConditionFlags::default() },
            ..base(crate::trade_rebuild::order_ref(stop_id, "SL"), stop_qty)
        }),
        (None, Some(price), _, _) => Some(IbOrder {
            order_type: "STP".to_string(),
            aux_price: price.to_f64()?,
            trigger_method: template.stop_trigger,
            handling: OrderHandling { outside_rth: template.order_flags.extended_hour_stop, ..OrderHandling::default() },
            ..base(crate::trade_rebuild::order_ref(stop_id, "SL"), stop_qty)
        }),
        // `AuxPrice` is the trailing amount, `TrailStopPrice` the stop price it starts at.
        (None, None, Some(trail), Some(start)) => Some(IbOrder {
            order_type: "TRAIL".to_string(),
            aux_price: trail.to_f64()?,
            trail_stop_price: start.to_f64()?,
            trigger_method: template.stop_trigger,
            handling: OrderHandling { outside_rth: template.order_flags.extended_hour_stop, ..OrderHandling::default() },
            ..base(crate::trade_rebuild::order_ref(stop_id, "SL"), stop_qty)
        }),
        _ => None,
    };
    let stop_limit = match (prices.stop, prices.limit) {
        (Some(trigger), Some(limit)) => Some(IbOrder {
            order_type: "STP LMT".to_string(),
            aux_price: trigger.to_f64()?,
            lmt_price: limit.to_f64()?,
            trigger_method: template.stop_trigger,
            handling: OrderHandling { outside_rth: template.order_flags.extended_hour_stop, ..OrderHandling::default() },
            ..base(crate::trade_rebuild::order_ref(limit_id, "SL"), stop_qty)
        }),
        _ => None,
    };
    let profit = match prices.profit {
        Some(price) => Some(IbOrder {
            order_type: "LMT".to_string(),
            lmt_price: price.to_f64()?,
            handling: OrderHandling { outside_rth: template.order_flags.extended_hour_pt, ..OrderHandling::default() },
            ..base(crate::trade_rebuild::order_ref(profit_id, "TP"), profit_qty)
        }),
        None => None,
    };
    Some(ExitOrders { contract, stop, stop_limit, profit })
}

/// Whether the orders that close `trade` sell - a single long option - rather than buy: a combo is
/// bought (a credit combo at a positive price, a debit combo at a negative one) and so is a single
/// short option. Decides the sign of the price a filled closing order paid.
#[must_use]
pub fn closes_by_selling(trade: &StoredTrade) -> bool {
    if let Some(custom) = &trade.custom_legs {
        return matches!(custom.as_slice(), [only] if only.ratio > 0);
    }
    let legs = [&trade.short_put, &trade.long_put, &trade.short_call, &trade.long_call];
    legs.iter().filter(|leg| leg.is_some()).count() == 1 && (trade.long_put.is_some() || trade.long_call.is_some())
}

/// Whether closing `trade` brings money instead of costing it: a single bought option is sold, and a combo bought as a debit is closed by
/// buying its legs the other way round for a credit (a negative limit). Its close limit is then the price it brings, and a limit that is
/// moved towards the market goes down (towards the bid), not up.
#[must_use]
pub fn close_receives(trade: &StoredTrade) -> bool {
    closes_by_selling(trade) || (trade.price_open < Decimal::ZERO && close_legs(trade).len() > 1)
}

/// The profit-target order of a later level (2-4): the same closing legs and contract as the
/// first one, joined to the OCA `group` of the trade's other exit orders.
/// `None` when the contract or the price cannot be put on the wire.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn level_order(
    trade: &StoredTrade,
    legs: &[ComboLegSpec],
    today: &str,
    price: Decimal,
    qty: u32,
    order_id: i32,
    group: &str,
    outside_rth: bool,
) -> Option<(IbContract, IbOrder)> {
    let contract = exit_contract(trade, legs)?;
    let action = closing_action(legs).as_str();
    let order = IbOrder {
        action: action.to_string(),
        total_quantity: f64::from(qty),
        order_type: "LMT".to_string(),
        lmt_price: price.to_f64()?,
        tif: if trade.expiry == today { "DAY" } else { "GTC" }.to_string(),
        account: trade.account.clone(),
        order_ref: crate::trade_rebuild::order_ref(order_id, "TP"),
        oca_group: group.to_string(),
        oca_type: 3,
        handling: OrderHandling { outside_rth, transmit: true, ..OrderHandling::default() },
        ..IbOrder::default()
    };
    Some((contract, order))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::StoredLeg;
    use crate::templates::StopSwitches;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn a_template_exit_is_a_time_of_the_day_or_minutes_after_the_fill() {
        let filled = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(10, 15, 30).unwrap();
        let mut template = TradeTemplate::default();
        assert_eq!(timed_exit_of(&template, "20261116", filled), None, "no exit set");
        template.exit_hour = 15;
        template.exit_minute = 45;
        assert_eq!(timed_exit_of(&template, "20261116", filled).map(|e| (e.at, e.limit)), Some(("2026-11-16 15:45".to_string(), false)));
        template.exit_flags.exit_order_limit = true;
        assert_eq!(timed_exit_of(&template, "20261116", filled).map(|e| e.limit), Some(true));
        // Minutes in the trade win over the time of the day.
        template.exit_minutes_in_trade = 90;
        assert_eq!(timed_exit_of(&template, "20261116", filled).map(|e| e.at), Some("2026-11-16 11:45".to_string()));
        // An exit time that has passed counts as now.
        template.exit_minutes_in_trade = 0;
        template.exit_hour = 9;
        template.exit_minute = 30;
        assert_eq!(timed_exit_of(&template, "20261116", filled).map(|e| e.at), Some("2026-11-16 10:15".to_string()));
    }

    #[test]
    fn the_exit_time_of_the_day_is_on_the_day_exit_dte_days_before_the_expiry() {
        // Filled on Monday 16 Nov 2026 for a trade that expires on Wednesday the 18th (2 DTE).
        let filled = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(10, 15, 30).unwrap();
        let mut template = TradeTemplate { exit_hour: 15, exit_minute: 0, ..TradeTemplate::default() };
        let at = |template: &TradeTemplate| timed_exit_of(template, "20261118", filled).map(|e| e.at);
        assert_eq!(at(&template), Some("2026-11-18 15:00".to_string()), "DTE 0: on the expiry day, not on the day of the fill");
        template.exit_dte = 1;
        assert_eq!(at(&template), Some("2026-11-17 15:00".to_string()), "DTE 1: the day before the expiry");
        template.exit_dte = 2;
        assert_eq!(at(&template), Some("2026-11-16 15:00".to_string()), "DTE 2: the day of the fill");
        template.exit_dte = 5;
        assert_eq!(at(&template), Some("2026-11-16 10:15".to_string()), "that day is before the fill: the exit has passed and counts as now");
        // A 0-DTE trade is closed on its own day as before.
        template.exit_dte = 0;
        assert_eq!(timed_exit_of(&template, "20261116", filled).map(|e| e.at), Some("2026-11-16 15:00".to_string()));
        // Minutes in the trade do not look at the expiry.
        template.exit_minutes_in_trade = 30;
        assert_eq!(at(&template), Some("2026-11-16 10:45".to_string()));
        // An expiry that cannot be read is the day of the fill.
        template.exit_minutes_in_trade = 0;
        assert_eq!(timed_exit_of(&template, "", filled).map(|e| e.at), Some("2026-11-16 15:00".to_string()));
    }

    #[test]
    fn an_exit_day_without_trading_gives_way_to_the_trading_day_before_it() {
        // Friday 20 Nov 2026, filled; the trade expires on Monday the 23rd: 1 DTE would be Sunday, the exit is Friday.
        let filled = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap().and_hms_opt(10, 0, 0).unwrap();
        let template = TradeTemplate { exit_hour: 15, exit_dte: 1, ..TradeTemplate::default() };
        assert_eq!(timed_exit_of(&template, "20261123", filled).map(|e| e.at), Some("2026-11-20 15:00".to_string()));
        // Thanksgiving, Thursday 26 Nov 2026: the exit before an expiry on Friday the 27th with 1 DTE is Wednesday.
        let before = chrono::NaiveDate::from_ymd_opt(2026, 11, 24).unwrap().and_hms_opt(10, 0, 0).unwrap();
        assert_eq!(timed_exit_of(&template, "20261127", before).map(|e| e.at), Some("2026-11-25 15:00".to_string()));
    }

    #[test]
    fn the_limit_offset_of_a_limit_stop_with_oco_is_kept_within_15_percent_of_the_stop_and_half_a_point() {
        let oco = |offset: f64| TradeTemplate {
            stop_order_type: "Limit stop with OCO".into(),
            stop_limit_offset: Some(offset),
            stop_limit_market_offset: Some(0.5),
            ..template()
        };
        // Stop 2.00: 15 % are 0.30, the offset of 0.80 is cut to it.
        let small = exit_prices(&oco(0.8), "SPX", 2, d("0.5"));
        let stop = small.stop.unwrap();
        let limit = small.limit.unwrap();
        assert!(limit - stop <= d("0.5"), "never more than 0.50 beyond the stop: {limit} after {stop}");
        assert!(limit - stop <= stop * d("0.15"), "never more than 15 % of the stop: {limit} after {stop}");
        // A small offset stays as it is.
        let kept = exit_prices(&oco(0.05), "SPX", 2, d("0.5"));
        assert_eq!(kept.limit.unwrap() - kept.stop.unwrap(), d("0.05"));
        // A plain Limit stop is not capped.
        let plain = exit_prices(&TradeTemplate { stop_order_type: "Limit stop".into(), stop_limit_offset: Some(0.8), ..template() }, "SPX", 2, d("0.5"));
        assert_eq!(plain.limit.unwrap() - plain.stop.unwrap(), d("0.8"));
    }
    #[test]
    fn exit_prices_are_described_in_one_line_with_a_dash_for_what_is_not_set() {
        let prices =
            ExitPrices { stop: Some(d("0.90")), market: Some(d("0.90")), profit: Some(d("0.15")), rel: Some((d("8.0"), d("0.5"))), ..ExitPrices::default() };
        assert_eq!(prices.describe(), "stop 0.9, market stop 0.9, stop limit -, trailing -, REL limit 8 offset 0.5, profit target 0.15");
        assert_eq!(ExitPrices::default().describe(), "stop -, market stop -, stop limit -, trailing -, REL limit -, profit target -");
    }

    fn template() -> TradeTemplate {
        TradeTemplate {
            stop_type: "Short".into(),
            stop_multiple: 2.0,
            stop_order_type: "Market stop".into(),
            stop_target_type: "Multiple".into(),
            profit_target_type: "Percentage".into(),
            profit_target: Some(50.0),
            ..TradeTemplate::default()
        }
    }

    #[allow(clippy::unnecessary_wraps)] // the field it fills is an `Option`
    fn leg(con_id: i32, strike: &str) -> Option<StoredLeg> {
        Some(StoredLeg { con_id, strike: strike.into() })
    }

    fn iron_condor() -> StoredTrade {
        StoredTrade {
            day: "2026-11-16".into(),
            opened: "16-11-2026 10:00:00".into(),
            account: "DU1".into(),
            symbol: "SPX".into(),
            trade_type: "IronCondor-Full".into(),
            strikes: String::new(),
            order_id: 5,
            qty: 2,
            multiplier: 100,
            price_open: rust_decimal::Decimal::from_str_exact("3").unwrap(),
            delta: 0.0,
            short_put: leg(2, "6600"),
            long_put: leg(1, "6590"),
            short_call: leg(3, "6800"),
            long_call: leg(4, "6810"),
            put_ratio: 1,
            call_ratio: 1,
            expiry: "20261116".into(),
            status: "Open".into(),
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
            custom_legs: None,
        }
    }

    #[test]
    fn a_credit_trade_gets_a_multiple_stop_and_a_percentage_target() {
        // Stop: 3 * (2 + 1) = 9; target: 3 * 50 % = 1.50.
        let prices = exit_prices(&template(), "SPX", 2, d("3"));
        assert_eq!((prices.stop, prices.profit), (Some(d("9")), Some(d("1.5"))));
        assert!(prices.notes.is_empty(), "{:?}", prices.notes);
    }

    #[test]
    fn an_amount_stop_and_an_amount_target_work_from_the_entry_price() {
        let t = TradeTemplate {
            stop_target_type: "Amount".into(),
            stop_multiple: 1.5,
            stop_offset: 0.25,
            profit_target_type: "Amount".into(),
            profit_target: Some(1.0),
            ..template()
        };
        let prices = exit_prices(&t, "SPX", 2, d("2"));
        assert_eq!((prices.stop, prices.profit), (Some(d("3.75")), Some(d("1"))));
    }

    #[test]
    fn an_exit_price_target_is_used_as_it_is_and_zero_becomes_a_nickel() {
        let t = TradeTemplate { profit_target_type: "Target price".into(), profit_target: Some(0.0), ..template() };
        assert_eq!(exit_prices(&t, "SPX", 2, d("3")).profit, Some(d("0.05")));
        let t = TradeTemplate { profit_target_type: "Target price".into(), profit_target: Some(0.65), ..template() };
        assert_eq!(exit_prices(&t, "SPX", 2, d("3")).profit, Some(d("0.65")));
    }

    #[test]
    fn prices_land_on_the_tick_grid() {
        // 1.15 * 3 = 3.45 -> a combo ticks in 0.05 steps.
        assert_eq!(exit_prices(&template(), "SPX", 2, d("1.15")).stop, Some(d("3.45")));
        // 1.17 * 3 = 3.51 -> rounded down to 3.50.
        assert_eq!(exit_prices(&template(), "SPX", 2, d("1.17")).stop, Some(d("3.5")));
    }

    #[test]
    fn no_stop_and_no_target_when_the_template_has_none() {
        let t = TradeTemplate { stop_order_type: "None".into(), profit_target_type: "None".into(), ..template() };
        assert_eq!(exit_prices(&t, "SPX", 2, d("3")), ExitPrices::default());
    }

    #[test]
    fn what_is_not_supported_is_reported_and_never_guessed() {
        // A debit stop of 2x loses more than the whole price: it is on the wrong side of 0.
        let debit = exit_prices(&template(), "SPX", 2, d("-1.5"));
        assert_eq!((debit.stop, debit.profit), (None, Some(d("-2.25"))), "the target is still placed");
        assert_ne!(debit.notes, [] as [std::string::String; 0]);
        let trailing = exit_prices(&TradeTemplate { stop_order_type: "Trailing".into(), ..template() }, "SPX", 2, d("3"));
        assert_eq!((trailing.stop, trailing.profit), (None, Some(d("1.5"))), "the target is still placed");
        assert!(trailing.notes.iter().any(|n| n.contains("Trailing")));
        let es = exit_prices(&template(), "ES", 2, d("3"));
        assert_eq!((es.stop, es.profit), (None, None));
    }

    fn levels_template(qty_pcts: [Option<f64>; 4], targets: [Option<f64>; 4]) -> TradeTemplate {
        TradeTemplate {
            profit_target_type: "Percentage".into(),
            profit_target: targets[0],
            profit_target_trade_percent: qty_pcts[0],
            profit_target2: targets[1],
            profit_target2_trade_percent: qty_pcts[1],
            profit_target3: targets[2],
            profit_target3_trade_percent: qty_pcts[2],
            profit_target4: targets[3],
            profit_target4_trade_percent: qty_pcts[3],
            ..template()
        }
    }

    #[test]
    fn the_contracts_are_split_over_the_profit_target_levels_like_set_pt_qty() {
        let some = |v: f64| Some(v);
        // 10 contracts: 50 % rounded up, 30 %, 20 %.
        let t = levels_template([some(50.0), some(30.0), some(20.0), None], [some(25.0), some(50.0), some(75.0), None]);
        assert_eq!(split_profit_qty(10, &t), [5, 3, 2, 0]);
        // 3 contracts: ceil(1.5) = 2 for level 1, one is left for level 2 (50 % would be 2, capped).
        let t = levels_template([some(50.0), some(50.0), None, None], [some(25.0), some(50.0), None, None]);
        assert_eq!(split_profit_qty(3, &t), [2, 1, 0, 0]);
        // Without a second level everything goes to the first.
        let t = levels_template([some(40.0), None, None, None], [some(25.0), None, None, None]);
        assert_eq!(split_profit_qty(5, &t), [5, 0, 0, 0]);
        // A missing share of level 1 counts as 100 %.
        let t = levels_template([None, some(30.0), None, None], [some(25.0), some(50.0), None, None]);
        assert_eq!(split_profit_qty(4, &t), [4, 0, 0, 0]);
        // What is left after level 2 goes to level 2 when there is no level 3 ...
        let t = levels_template([some(50.0), some(30.0), None, None], [some(25.0), some(50.0), None, None]);
        assert_eq!(split_profit_qty(10, &t), [5, 5, 0, 0]);
        // ... and the rest of four levels goes to the fourth.
        let t = levels_template([some(40.0), some(30.0), some(20.0), some(10.0)], [some(25.0), some(50.0), some(75.0), some(90.0)]);
        assert_eq!(split_profit_qty(10, &t), [4, 3, 2, 1]);
        // A level whose share is 0 passes its part on, and one contract cannot be split.
        let t = levels_template([some(50.0), some(0.0), None, None], [some(25.0), some(50.0), None, None]);
        assert_eq!(split_profit_qty(4, &t), [4, 0, 0, 0]);
        let t = levels_template([some(50.0), some(50.0), None, None], [some(25.0), some(50.0), None, None]);
        assert_eq!(split_profit_qty(1, &t), [1, 0, 0, 0]);
        for qty in 1..40 {
            let t = levels_template([some(30.0), some(30.0), some(20.0), some(20.0)], [some(25.0), some(50.0), some(75.0), some(90.0)]);
            assert_eq!(split_profit_qty(qty, &t).iter().sum::<u32>(), qty, "every contract belongs to a level ({qty})");
        }
    }

    #[test]
    fn every_level_has_its_own_price() {
        let t = levels_template([None; 4], [Some(50.0), Some(75.0), None, None]);
        assert_eq!(profit_level_price(&t, "SPX", 2, d("3"), 0), Some(d("1.5")));
        assert_eq!(profit_level_price(&t, "SPX", 2, d("3"), 1), Some(d("0.75")));
        assert_eq!(profit_level_price(&t, "SPX", 2, d("3"), 2), None, "no third target");
        // An amount target for a debit trade: the closing combo brings the entry price plus the amount.
        let t = TradeTemplate { profit_target_type: "Amount".into(), profit_target: Some(1.0), profit_target2: Some(2.0), ..template() };
        assert_eq!(profit_level_price(&t, "SPX", 2, d("-1.5"), 0), Some(d("-2.5")));
        assert_eq!(profit_level_price(&t, "SPX", 2, d("-1.5"), 1), Some(d("-3.5")));
        // A target of 0 on a credit trade becomes the smallest price.
        let t = TradeTemplate { profit_target_type: "Target price".into(), profit_target: Some(0.0), ..template() };
        assert_eq!(profit_level_price(&t, "SPX", 2, d("3"), 0), Some(d("0.05")));
    }

    #[test]
    fn the_average_close_weighs_every_level_by_its_contracts() {
        let closed = vec![(2, "-0.15".to_string())];
        assert_eq!(average_close(&closed, 2, d("-0.9"), 4), Some(d("-0.525")));
        assert_eq!(average_close(&[], 3, d("-0.4"), 3), Some(d("-0.4")));
        // Every level hit: nothing is left at a final price.
        let all = vec![(2, "-0.15".to_string()), (2, "-0.05".to_string())];
        assert_eq!(average_close(&all, 0, Decimal::ZERO, 4), Some(d("-0.1")));
        // Rounded half away from zero to 4 decimals: (1 * 0.33333 + 2 * 0.1) / 3 = 0.17778.
        assert_eq!(average_close(&[(1, "-0.33333".to_string())], 2, d("-0.1"), 3), Some(d("-0.1778")));
        assert_eq!(average_close(&[(1, "abc".to_string())], 2, d("-0.1"), 3), None);
        assert_eq!(average_close(&[], 1, d("-0.1"), 0), None, "no division by 0");
    }

    #[test]
    fn a_later_level_joins_the_group_with_its_own_quantity_and_reference() {
        let trade = iron_condor();
        let legs = [ComboLegSpec { con_id: 3, ratio: 1, action: Action::Buy }, ComboLegSpec { con_id: 2, ratio: 1, action: Action::Buy }];
        let (contract, order) = level_order(&trade, &legs, "20261110", d("0.75"), 1, 14, "TradePilot-11", true).unwrap();
        assert_eq!((contract.sec_type.as_str(), contract.combo_legs.len()), ("BAG", 2));
        assert_eq!((order.action.as_str(), order.order_type.as_str(), order.lmt_price, order.total_quantity), ("BUY", "LMT", 0.75, 1.0));
        assert_eq!((order.oca_group.as_str(), order.oca_type, order.tif.as_str(), order.handling.outside_rth), ("TradePilot-11", 3, "GTC", true));
        assert_eq!(order.order_ref, "TraPi-14 (TP)");
    }

    #[test]
    fn what_closes_a_trade_decides_the_sign_of_its_price() {
        // A combo or a short option is bought; only a single long option is sold.
        assert!(!closes_by_selling(&iron_condor()));
        let mut long = iron_condor();
        long.short_put = None;
        long.long_put = None;
        long.short_call = None;
        assert!(closes_by_selling(&long));
        let mut short = iron_condor();
        short.long_put = None;
        short.short_call = None;
        short.long_call = None;
        assert!(!closes_by_selling(&short));
    }

    #[test]
    fn a_debit_trade_is_stopped_at_a_fraction_of_its_price_and_targeted_above_it() {
        // Entry -1.50 (a debit): a 0.5 multiple stop is -1.50 - (-1.50 * 0.5) = -0.75; the 50 %
        // target is -1.50 + (-1.50 * 50 %) = -2.25 (what the closing combo brings).
        let t = TradeTemplate { stop_multiple: 0.5, ..template() };
        let prices = exit_prices(&t, "SPX", 2, d("-1.5"));
        assert_eq!((prices.stop, prices.market, prices.profit), (Some(d("-0.75")), Some(d("-0.75")), Some(d("-2.25"))));
        // An Amount stop moves towards 0 by that amount, an Exit Price target is always negative.
        let t = TradeTemplate {
            stop_target_type: "Amount".into(),
            stop_multiple: 0.5,
            profit_target_type: "Target price".into(),
            profit_target: Some(2.5),
            ..template()
        };
        let prices = exit_prices(&t, "SPX", 2, d("-1.5"));
        assert_eq!((prices.stop, prices.profit), (Some(d("-1")), Some(d("-2.5"))));
        // A single option is priced positive: entry -2.00, stop 1.00, target 3.00.
        let t = TradeTemplate { stop_multiple: 0.5, ..template() };
        let single = exit_prices(&t, "SPY", 1, d("-2"));
        assert_eq!((single.stop, single.profit), (Some(d("1")), Some(d("3"))));
    }

    #[test]
    fn a_stop_limit_places_only_the_limit_order_and_oco_adds_a_market_backstop() {
        let limit = exit_prices(&TradeTemplate { stop_order_type: "Limit stop".into(), stop_limit_offset: Some(0.2), ..template() }, "SPX", 2, d("3"));
        assert_eq!((limit.stop, limit.market, limit.limit), (Some(d("9")), None, Some(d("9.2"))));
        // The offset defaults to 0.10.
        let default = exit_prices(&TradeTemplate { stop_order_type: "Limit stop".into(), ..template() }, "SPX", 2, d("3"));
        assert_eq!(default.limit, Some(d("9.1")));
        let oco = exit_prices(
            &TradeTemplate { stop_order_type: "Limit stop with OCO".into(), stop_limit_offset: Some(0.2), stop_limit_market_offset: Some(0.5), ..template() },
            "SPX",
            2,
            d("3"),
        );
        assert_eq!((oco.stop, oco.market, oco.limit), (Some(d("9")), Some(d("9.5")), Some(d("9.2"))));
        // OCO without its market offset is not guessed.
        let missing = exit_prices(&TradeTemplate { stop_order_type: "Limit stop with OCO".into(), ..template() }, "SPX", 2, d("3"));
        assert_eq!((missing.stop, missing.market, missing.limit), (None, None, None));
        assert!(missing.notes.iter().any(|n| n.contains("Stop Market Offset")));
        // A sold single option: the limit and the backstop lie below the trigger.
        let t = TradeTemplate {
            stop_order_type: "Limit stop with OCO".into(),
            stop_multiple: 0.5,
            stop_limit_offset: Some(0.2),
            stop_limit_market_offset: Some(0.5),
            ..template()
        };
        let single = exit_prices(&t, "SPY", 1, d("-2"));
        assert_eq!((single.stop, single.limit, single.market), (Some(d("1")), Some(d("0.85")), Some(d("0.5"))), "the offset is cut to 15 % of the stop");
    }

    #[test]
    fn the_orders_follow_the_stop_type_and_a_long_option_is_sold() {
        let trade = iron_condor();
        let legs = [ComboLegSpec { con_id: 3, ratio: 1, action: Action::Buy }, ComboLegSpec { con_id: 2, ratio: 1, action: Action::Buy }];
        let t =
            TradeTemplate { stop_order_type: "Limit stop with OCO".into(), stop_limit_offset: Some(0.2), stop_limit_market_offset: Some(0.5), ..template() };
        let prices = exit_prices(&t, "SPX", 2, d("3"));
        let orders = exit_orders(&trade, &t, &prices, &legs, "20261110", 11, 12, 13, trade.qty).unwrap();
        let (stop, limit, profit) = (orders.stop.unwrap(), orders.stop_limit.unwrap(), orders.profit.unwrap());
        assert_eq!((stop.order_type.as_str(), stop.aux_price), ("STP", 9.5));
        assert_eq!((limit.order_type.as_str(), limit.aux_price, limit.lmt_price), ("STP LMT", 9.0, 9.2));
        assert_eq!(profit.order_type, "LMT");
        // One OCA group for all three.
        assert!([&stop, &limit, &profit].iter().all(|o| o.oca_group == "TradePilot-11" && o.oca_type == 3));
        assert_eq!(limit.order_ref, "TraPi-13 (SL)");
        // A StopLimit template has no market stop.
        let only = TradeTemplate { stop_order_type: "Limit stop".into(), ..template() };
        let orders = exit_orders(&trade, &only, &exit_prices(&only, "SPX", 2, d("3")), &legs, "20261110", 11, 12, 13, trade.qty).unwrap();
        assert!(orders.stop.is_none() && orders.stop_limit.is_some());
        // A long call that is closed alone is sold.
        let mut long = iron_condor();
        long.trade_type = "LongCall".into();
        long.short_put = None;
        long.long_put = None;
        long.short_call = None;
        let sell = [ComboLegSpec { con_id: 4, ratio: 1, action: Action::Sell }];
        let t = TradeTemplate { stop_multiple: 0.5, ..template() };
        let orders = exit_orders(&long, &t, &exit_prices(&t, "SPY", 1, d("-2")), &sell, "20261110", 11, 12, 13, long.qty).unwrap();
        assert_eq!(orders.stop.map(|o| (o.action, o.aux_price)), Some(("SELL".to_string(), 1.0)));
        assert_eq!(orders.profit.map(|o| (o.action, o.lmt_price)), Some(("SELL".to_string(), 3.0)));
    }

    #[test]
    fn the_oca_group_is_found_from_whichever_order_the_trade_has() {
        let mut trade = iron_condor();
        trade.stop_limit_order_id = Some(13);
        assert_eq!(trade_oca_group(&trade).as_deref(), Some("TradePilot-11"));
        trade.profit_order_id = Some(12);
        assert_eq!(trade_oca_group(&trade).as_deref(), Some("TradePilot-11"));
        trade.stop_order_id = Some(11);
        assert_eq!(trade_oca_group(&trade).as_deref(), Some("TradePilot-11"));
        assert_eq!(trade_oca_group(&iron_condor()), None);
    }

    #[test]
    fn debit_types_sell_their_longs_and_sold_types_close_only_their_shorts() {
        let all_bid = |_: i32| true;
        let t = TradeTemplate { stop_type: "Short".into(), ..template() };
        // A Short stop would keep an iron condor's longs; a debit spread always sells its long.
        let mut spread = iron_condor();
        spread.trade_type = "PutDebitSpread".into();
        spread.short_call = None;
        spread.long_call = None;
        let legs = exit_legs(&spread, &t, &all_bid);
        assert_eq!(legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), vec![(2, "BUY"), (1, "SELL")]);
        let mut short_put = iron_condor();
        short_put.trade_type = "ShortPut".into();
        short_put.long_put = None;
        let v = TradeTemplate { stop_type: "Vertical".into(), ..template() };
        assert_eq!(exit_legs(&short_put, &v, &all_bid).iter().map(|l| l.con_id).collect::<Vec<_>>(), vec![3, 2]);
        // A long call without a bid is still sold; a debit spread's long without a bid is not.
        let mut long = iron_condor();
        long.trade_type = "LongCall".into();
        long.short_put = None;
        long.long_put = None;
        long.short_call = None;
        assert_eq!(exit_legs(&long, &t, &|_| false).len(), 1);
        assert_eq!(exit_legs(&spread, &t, &|_| false).len(), 1);
    }

    fn adjustment(kind: &str, value: Option<f64>) -> StopAdjustment {
        StopAdjustment {
            kind: kind.into(),
            value,
            order_type: "Same".into(),
            change_type: "Stop factor".into(),
            change: Some(0.0),
            change_offset: None,
            hour: 0,
            minute: 0,
        }
    }

    #[test]
    fn an_adjustment_trigger_is_worked_out_from_the_entry_price() {
        // Credit 3.00: a 50 % profit level is 1.50, an amount is itself, a profit of 1 leaves 2.00.
        assert_eq!(adjustment_trigger(&adjustment("Profit percent", Some(50.0)), d("3")), Some(d("1.5")));
        assert_eq!(adjustment_trigger(&adjustment("Amount", Some(0.4)), d("3")), Some(d("0.4")));
        assert_eq!(adjustment_trigger(&adjustment("Profit", Some(1.0)), d("3")), Some(d("2")));
        // Debit 2.00 (entry -2.00): 50 % profit means the position has to be worth 3.00; a profit of 1 too.
        assert_eq!(adjustment_trigger(&adjustment("Profit percent", Some(50.0)), d("-2")), Some(d("3")));
        assert_eq!(adjustment_trigger(&adjustment("Profit", Some(1.0)), d("-2")), Some(d("3")));
        // No value, no trigger; the ProfitTarget kinds have none either.
        assert_eq!(adjustment_trigger(&adjustment("Profit", None), d("3")), None);
        assert_eq!(adjustment_trigger(&adjustment("Target 1", None), d("3")), None);
        assert_eq!(adjustment_trigger(&adjustment("None", Some(1.0)), d("3")), None);
    }

    #[test]
    fn an_adjustment_is_hit_when_the_position_reaches_its_trigger_or_its_profit_target() {
        let level = adjustment("Profit percent", Some(50.0));
        // Credit 3.00, trigger 1.50: the position's value now is at or below it.
        assert!(adjustment_hit(&level, d("3"), d("1.5"), 0));
        assert!(!adjustment_hit(&level, d("3"), d("1.55"), 0));
        // Debit: its value (what it brings, as an amount) at or above 3.00.
        assert!(adjustment_hit(&level, d("-2"), d("-3.2"), 0));
        assert!(!adjustment_hit(&level, d("-2"), d("-2.9"), 0));
        // A ProfitTargetN is hit with level N of the profit target, whatever the price.
        let target = adjustment("Target 2", None);
        assert!(!adjustment_hit(&target, d("3"), d("0.1"), 1));
        assert!(adjustment_hit(&target, d("3"), d("9"), 2));
        assert!(!adjustment_hit(&adjustment("Target 1", None), d("3"), d("0.1"), 0));
        assert!(!adjustment_hit(&adjustment("Profit", None), d("3"), d("0"), 0), "no value: never hit");
    }

    #[test]
    fn the_new_stop_follows_the_change_type_and_the_offset() {
        let mut a = adjustment("Profit percent", Some(50.0));
        // Stop Multiple 0 puts the stop at the entry price (break even), a credit trade adds the multiple.
        assert_eq!(adjustment_new_stop(&a, "Market stop", d("0.3"), "SPX", 2), Some(d("0.3")));
        a.change = Some(1.0);
        assert_eq!(adjustment_new_stop(&a, "Market stop", d("0.3"), "SPX", 2), Some(d("0.6")));
        // The offset is added - but not to a StopLimit, where it is the limit's.
        a.change = Some(0.0);
        a.change_offset = Some(0.1);
        assert_eq!(adjustment_new_stop(&a, "Market stop", d("0.3"), "SPX", 2), Some(d("0.4")));
        assert_eq!(adjustment_new_stop(&a, "Limit stop", d("0.3"), "SPX", 2), Some(d("0.3")));
        let to_limit = StopAdjustment { order_type: "Limit stop".into(), ..a.clone() };
        assert_eq!(adjustment_new_stop(&to_limit, "Market stop", d("0.3"), "SPX", 2), Some(d("0.3")));
        // A fixed amount; a negative or zero credit stop is no stop.
        let fixed = StopAdjustment { change_type: "Fixed price".into(), change: Some(0.5), change_offset: None, ..a.clone() };
        assert_eq!(adjustment_new_stop(&fixed, "Market stop", d("0.3"), "SPX", 2), Some(d("0.5")));
        let negative = StopAdjustment { change: Some(-1.0), ..fixed.clone() };
        assert_eq!(adjustment_new_stop(&negative, "Market stop", d("0.3"), "SPX", 2), None);
        let zero = StopAdjustment { change: Some(0.0), ..fixed.clone() };
        assert_eq!(adjustment_new_stop(&zero, "Market stop", d("0.3"), "SPX", 2), None);
        let missing = StopAdjustment { change: None, ..fixed };
        assert_eq!(adjustment_new_stop(&missing, "Market stop", d("0.3"), "SPX", 2), None);
        // An unknown change type gives no price - not a stop at the offset alone.
        let unknown =
            StopAdjustment { change_type: "Something".into(), change: Some(1.0), change_offset: Some(0.25), ..adjustment("Profit percent", Some(50.0)) };
        assert_eq!(adjustment_new_stop(&unknown, "Market stop", d("0.3"), "SPX", 2), None);
        // A debit trade loses the multiple of its price: entry -2.00, multiple 0.5 -> stop -1.00.
        let debit = StopAdjustment { change: Some(0.5), change_offset: None, ..adjustment("Profit", Some(1.0)) };
        assert_eq!(adjustment_new_stop(&debit, "Market stop", d("-2"), "SPX", 2), Some(d("-1")));
    }

    fn adjusted(order_type: &str) -> StopAdjustments {
        StopAdjustments {
            items: vec![adjustment("Profit percent", Some(50.0))],
            level: 1,
            order_type: order_type.into(),
            stop: Some("0.6".into()),
            trailing_offset: Some("0.5".into()),
            limit_offset: Some(0.2),
            market_offset: Some(0.5),
            legs: vec![(2, 1, false)],
            trigger_method: 4,
            outside_rth: false,
            replace_pending: true,
            rel: None,
            basis: None,
        }
    }

    #[test]
    fn stop_order_types_are_read_by_their_names_and_an_unknown_one_is_no_market_stop() {
        for kind in [StopOrderType::None, StopOrderType::StopMarket, StopOrderType::StopLimit, StopOrderType::StopLimitOco, StopOrderType::Trailing] {
            assert_eq!(StopOrderType::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(StopOrderType::parse(""), Some(StopOrderType::None), "nothing set is no stop");
        for unknown in ["TrailingPct", "stopmarket", "Same", "STP"] {
            assert_eq!(StopOrderType::parse(unknown), None, "{unknown}");
        }
        // A template asking for an unknown type gets no stop - not a market stop - and says why.
        let prices = exit_prices(&TradeTemplate { stop_order_type: "TrailingPct".into(), ..template() }, "SPX", 2, d("3"));
        assert_eq!((prices.stop, prices.market), (None, None));
        assert!(prices.notes.iter().any(|n| n.contains("'TrailingPct' is not supported yet")), "{:?}", prices.notes);
        // An adjusted stop of an unknown type is not built either.
        let trade = iron_condor();
        assert!(replacement_stop_orders(&trade, &adjusted("TrailingPct"), "20261110", "TradePilot-11", 20, 21).is_none());
    }

    #[test]
    fn a_closing_order_sells_a_single_long_and_buys_everything_else() {
        assert_eq!((Action::Buy.as_str(), Action::Sell.to_string()), ("BUY", "SELL".to_string()), "as TWS writes them");
        // A leg as a trade stores it and back.
        let leg = ComboLegSpec { con_id: 7, ratio: 2, action: Action::Sell };
        assert_eq!((leg.stored(), ComboLegSpec::from_stored(&leg.stored())), ((7, 2, true), leg.clone()));
        assert_eq!(closing_action(&[leg]), Action::Sell, "a single long is sold");
        assert_eq!(closing_action(&[spec(2, "BUY")]), Action::Buy, "a single short is bought back");
        assert_eq!(closing_action(&[spec(2, "BUY"), spec(1, "SELL")]), Action::Buy, "a combo is bought");
        assert_eq!(closing_action(&[]), Action::Buy);
    }

    fn spec(con_id: i32, action: &'static str) -> ComboLegSpec {
        ComboLegSpec { con_id, ratio: 1, action: Action::of(action == "SELL") }
    }

    #[test]
    fn a_debit_trade_is_only_priced_for_exits_that_close_all_of_its_legs() {
        // A put calendar bought for 2.00: near short put #2, far long put #1.
        let calendar = StoredTrade {
            trade_type: "PutCalendar".into(),
            price_open: rust_decimal::Decimal::from_str_exact("-2").unwrap(),
            short_call: None,
            long_call: None,
            ..iron_condor()
        };
        // A Short stop would buy back the near leg alone at a price meant for the calendar.
        let note = partial_debit_exit(&calendar, &[spec(2, "BUY")]).unwrap();
        assert!(note.contains("1 of the 2 legs") && note.contains("Vertical"), "{note}");
        assert_eq!(partial_debit_exit(&calendar, &[spec(2, "BUY"), spec(1, "SELL")]), None, "the whole calendar");
        // A debit spread whose long has no bid would be left with its short alone.
        let debit_spread = StoredTrade { trade_type: "PutDebitSpread".into(), ..calendar.clone() };
        assert!(partial_debit_exit(&debit_spread, &[spec(2, "BUY")]).is_some());
        // A single long option is the whole trade.
        let long_put = StoredTrade { trade_type: "LongPut".into(), short_put: None, ..calendar };
        assert_eq!(partial_debit_exit(&long_put, &[spec(1, "SELL")]), None);
        // A credit trade's Short stop closing only the shorts is the usual stop.
        assert_eq!(partial_debit_exit(&iron_condor(), &[spec(3, "BUY"), spec(2, "BUY")]), None);
    }

    #[test]
    fn a_replacement_stop_takes_its_side_from_the_leg_it_closes() {
        // A single bought-back leg is bought: the stop-limit's limit lies above the trigger, the
        // OCO backstop too - also when the trade's entry price is negative.
        let debit = StoredTrade { price_open: rust_decimal::Decimal::from_str_exact("-2").unwrap(), ..iron_condor() };
        let bought = replacement_stop_orders(&debit, &adjusted("Limit stop with OCO"), "20261110", "TradePilot-11", 20, 21).unwrap();
        let (backstop, limit) = (bought.stop.unwrap(), bought.stop_limit.unwrap());
        assert_eq!((backstop.action.as_str(), backstop.aux_price, limit.aux_price, limit.lmt_price), ("BUY", 1.1, 0.6, 0.65));
        // A single long that is sold: both lie below the trigger.
        let sold = StopAdjustments { legs: vec![(1, 1, true)], ..adjusted("Limit stop with OCO") };
        let orders = replacement_stop_orders(&debit, &sold, "20261110", "TradePilot-11", 20, 21).unwrap();
        let (backstop, limit) = (orders.stop.unwrap(), orders.stop_limit.unwrap());
        assert_eq!((backstop.action.as_str(), backstop.aux_price, limit.aux_price, limit.lmt_price), ("SELL", 0.1, 0.6, 0.5));
    }

    #[test]
    fn the_replacement_stop_follows_the_adjusted_type_in_the_trades_oca_group() {
        let trade = iron_condor();
        let market = replacement_stop_orders(&trade, &adjusted("Market stop"), "20261110", "TradePilot-11", 20, 21).unwrap();
        let stop = market.stop.unwrap();
        assert_eq!((stop.order_type.as_str(), stop.aux_price, stop.oca_group.as_str(), stop.trigger_method), ("STP", 0.6, "TradePilot-11", 4));
        assert!(market.stop_limit.is_none());
        let limit = replacement_stop_orders(&trade, &adjusted("Limit stop"), "20261110", "TradePilot-11", 20, 21).unwrap();
        assert!(limit.stop.is_none());
        let lmt = limit.stop_limit.unwrap();
        assert_eq!((lmt.order_type.as_str(), lmt.aux_price, lmt.lmt_price), ("STP LMT", 0.6, 0.8));
        assert_eq!(lmt.order_ref, "TraPi-21 (SL)");
        let oco = replacement_stop_orders(&trade, &adjusted("Limit stop with OCO"), "20261110", "TradePilot-11", 20, 21).unwrap();
        assert_eq!(
            (oco.stop.map(|o| o.aux_price), oco.stop_limit.map(|o| o.lmt_price)),
            (Some(1.1), Some(0.65)),
            "the offset of the OCO limit is cut to 15 % of the stop"
        );
        let trail = replacement_stop_orders(&trade, &adjusted("Trailing"), "20261110", "TradePilot-11", 20, 21).unwrap();
        let t = trail.stop.unwrap();
        assert_eq!((t.order_type.as_str(), t.aux_price, t.trail_stop_price), ("TRAIL", 0.5, 0.6));
        // No stop any more: nothing to place.
        assert!(replacement_stop_orders(&trade, &adjusted("None"), "20261110", "TradePilot-11", 20, 21).is_none());
    }

    #[test]
    fn a_trailing_stop_starts_at_the_entry_price_plus_its_amount_and_follows_at_that_distance() {
        let t = TradeTemplate { stop_order_type: "Trailing".into(), stop_multiple: 0.0, stop_offset: 0.5, ..template() };
        let prices = exit_prices(&t, "SPX", 2, d("3"));
        assert_eq!((prices.stop, prices.trail, prices.market), (Some(d("3.5")), Some(d("0.5")), None));
        let trade = iron_condor();
        let legs = [ComboLegSpec { con_id: 3, ratio: 1, action: Action::Buy }, ComboLegSpec { con_id: 2, ratio: 1, action: Action::Buy }];
        let orders = exit_orders(&trade, &t, &prices, &legs, "20261110", 11, 12, 13, trade.qty).unwrap();
        let stop = orders.stop.unwrap();
        assert_eq!((stop.order_type.as_str(), stop.aux_price, stop.trail_stop_price), ("TRAIL", 0.5, 3.5));
        assert_eq!(stop.order_ref, "TraPi-11 (SL)");
        // Without a trailing amount there is no stop.
        let none = exit_prices(&TradeTemplate { stop_offset: 0.0, ..t }, "SPX", 2, d("3"));
        assert_eq!((none.stop, none.trail), (None, None));
        assert!(none.notes.iter().any(|n| n.contains("trailing amount")));
    }

    // ---- stop bases ----

    fn execution(order: i32, con_id: i32, side: &str, qty: f64, price: f64) -> StoredExecution {
        StoredExecution {
            exec_id: format!("e{con_id}-{qty}-{price}"),
            time: "20261116 10:00:00 America/New_York".into(),
            account: "DU1".into(),
            symbol: "SPX".into(),
            con_id,
            order_id: order,
            order_ref: format!("TraPi-{order}"),
            sec_type: "OPT".into(),
            expiry: "20261116".into(),
            right: "P".into(),
            strike: 6600.0,
            side: side.into(),
            quantity: qty,
            price,
        }
    }

    fn spread_trade() -> StoredTrade {
        StoredTrade { order_id: 5, price_open: rust_decimal::Decimal::from_str_exact("0.3").unwrap(), short_call: None, long_call: None, ..iron_condor() }
    }

    #[test]
    fn the_leg_prices_are_what_the_opening_orders_executions_filled_at() {
        // Short put 2 sold at 1.95, long put 1 bought at 1.65: a credit of 0.30.
        let executions = [execution(5, 2, "SLD", 2.0, 1.95), execution(5, 1, "BOT", 2.0, 1.65)];
        let prices = leg_prices(&spread_trade(), &executions);
        assert_eq!((prices.short, prices.long), (Some(d("1.95")), Some(d("1.65"))));
        // A leg filled in parts counts at its average, weighted by the contracts.
        let parts = [execution(5, 2, "SLD", 1.0, 1.9), execution(5, 2, "SLD", 3.0, 2.0), execution(5, 1, "BOT", 4.0, 1.7)];
        let prices = leg_prices(&spread_trade(), &parts);
        assert_eq!((prices.short, prices.long), (Some(d("1.975")), Some(d("1.7"))));
    }

    #[test]
    fn the_leg_prices_of_an_iron_condor_add_up_and_follow_the_ratios() {
        let executions =
            [execution(5, 2, "SLD", 1.0, 1.0), execution(5, 3, "SLD", 1.0, 0.8), execution(5, 1, "BOT", 1.0, 0.4), execution(5, 4, "BOT", 1.0, 0.3)];
        let prices = leg_prices(&iron_condor(), &executions);
        assert_eq!((prices.short, prices.long), (Some(d("1.8")), Some(d("0.7"))));
        // A put ratio of 2 doubles the put legs.
        let ratio = StoredTrade { put_ratio: 2, ..iron_condor() };
        let prices = leg_prices(&ratio, &executions);
        assert_eq!((prices.short, prices.long), (Some(d("2.8")), Some(d("1.1"))));
    }

    #[test]
    fn the_leg_prices_are_unknown_until_every_leg_has_an_execution_of_this_order() {
        let both = [execution(5, 2, "SLD", 1.0, 1.95), execution(5, 1, "BOT", 1.0, 1.65)];
        assert_eq!(leg_prices(&spread_trade(), &both[..1]), LegPrices::default(), "the long leg is missing");
        // Executions of another order, or of none of ours, do not count.
        let other = [execution(6, 2, "SLD", 1.0, 1.95), execution(6, 1, "BOT", 1.0, 1.65)];
        assert_eq!(leg_prices(&spread_trade(), &other), LegPrices::default());
        let manual = StoredExecution { order_ref: String::new(), ..execution(5, 2, "SLD", 1.0, 1.95) };
        assert_eq!(leg_prices(&spread_trade(), &[manual, both[1].clone()]), LegPrices::default());
        assert_eq!(leg_prices(&spread_trade(), &[]), LegPrices::default());
        // A single short option has no long price.
        let short = StoredTrade { long_put: None, ..spread_trade() };
        let prices = leg_prices(&short, &both);
        assert_eq!((prices.short, prices.long), (Some(d("1.95")), None));
        // An execution without a size cannot be averaged.
        assert_eq!(leg_prices(&short, &[execution(5, 2, "SLD", 0.0, 1.95)]), LegPrices::default());
    }

    #[test]
    fn the_average_entry_needs_exactly_two_trades_of_one_order() {
        let side = |price: &str| StoredTrade { price_open: price.parse().unwrap(), ..spread_trade() };
        let (a, b, c) = (side("0.3"), side("0.5"), side("0.1"));
        assert_eq!(average_entry(&[&a, &b]), Some(d("0.4")));
        assert_eq!(average_entry(&[&a]), None);
        assert_eq!(average_entry(&[&a, &b, &c]), None);
        assert_eq!(average_entry(&[]), None);
        // Rounded half away from zero to 4 decimals: (0.3333 + 0.3334) / 2 = 0.33335.
        assert_eq!(average_entry(&[&side("0.3333"), &side("0.3334")]), Some(d("0.3334")));
    }

    #[test]
    fn the_stop_basis_is_picked_from_the_template_and_falls_back_to_the_entry_price() {
        let legs = LegPrices { short: Some(d("1.95")), long: Some(d("-1.65")) };
        let avg = Some(d("0.4"));
        assert_eq!(stop_basis("Given price", d("0.3"), legs, avg), (d("0.3"), None));
        assert_eq!(stop_basis("", d("0.3"), legs, avg), (d("0.3"), None));
        assert_eq!(stop_basis("Mean price", d("0.3"), legs, avg), (d("0.4"), None));
        assert_eq!(stop_basis("Short leg price", d("0.3"), legs, avg), (d("1.95"), None));
        assert_eq!(stop_basis("Long leg price", d("0.3"), legs, avg), (d("1.65"), None), "the absolute value");
        // Not known: the entry price, and the log is told.
        for basis in ["Mean price", "Short leg price", "Long leg price"] {
            let (price, note) = stop_basis(basis, d("0.3"), LegPrices::default(), None);
            assert_eq!(price, d("0.3"), "{basis}");
            assert!(note.is_some_and(|n| n.contains(basis) && n.contains("entry price")), "{basis}");
        }
        // A price of 0 is not a basis either.
        let (price, note) = stop_basis("Short leg price", d("0.3"), LegPrices { short: Some(Decimal::ZERO), long: None }, None);
        assert_eq!((price, note.is_some()), (d("0.3"), true));
        // Debit trades and unknown names keep the entry price and say so.
        let (price, note) = stop_basis("Short leg price", d("-1.5"), legs, avg);
        assert_eq!((price, note.is_some_and(|n| n.contains("credit"))), (d("-1.5"), true));
        let (price, note) = stop_basis("Nonsense", d("0.3"), legs, avg);
        assert_eq!((price, note.is_some()), (d("0.3"), true));
    }

    #[test]
    fn the_stop_follows_the_basis_but_the_profit_target_the_entry_price() {
        // Stop: 1.50 * (2 + 1) = 4.50; target: 50 % of the entry 3.00 = 1.50.
        let prices = exit_prices_with_basis(&template(), "SPX", 2, d("3"), d("1.5"));
        assert_eq!((prices.stop, prices.market, prices.profit), (Some(d("4.5")), Some(d("4.5")), Some(d("1.5"))));
        // An amount stop adds the amount to the basis.
        let t = TradeTemplate { stop_target_type: "Amount".into(), stop_multiple: 1.0, stop_offset: 0.25, ..template() };
        assert_eq!(exit_prices_with_basis(&t, "SPX", 2, d("3"), d("1.5")).stop, Some(d("2.75")));
        // With the entry price as basis it is the old calculation.
        assert_eq!(exit_prices_with_basis(&template(), "SPX", 2, d("3"), d("3")), exit_prices(&template(), "SPX", 2, d("3")));
    }

    // ---- REL stops, ITM protection and guardrails ----

    fn rel_trade(trade_type: &str, put: bool) -> StoredTrade {
        let mut trade = iron_condor();
        trade.trade_type = trade_type.into();
        trade.expiry = "20261116".into();
        trade.price_open = rust_decimal::Decimal::from_str_exact("0.3").unwrap();
        trade.long_put = None;
        trade.long_call = None;
        if put {
            trade.short_call = None;
            trade.short_put = leg(2, "6600");
        } else {
            trade.short_put = None;
            trade.short_call = leg(3, "6800");
        }
        trade
    }

    fn rel_settings() -> RelSettings {
        RelSettings { offset_open: 10, offset_exp: 10, underlying_open: Some("6700".into()), minutes_to_close: 132, ..RelSettings::default() }
    }

    fn price(condition: &IbCondition) -> Option<f64> {
        if let IbCondition::Price { price, .. } = condition {
            Some(*price)
        } else {
            None
        }
    }

    #[test]
    fn the_rel_limit_and_offset_default_and_never_lie_below_the_stop() {
        assert_eq!(rel_prices(None, None, "SPX", 1, d("0.9")), (d("8"), d("0.5")));
        assert_eq!(rel_prices(Some(3.0), Some(0.25), "SPX", 1, d("0.9")), (d("3"), d("0.25")));
        // A limit below the stop price is lifted to the stop price plus 2.
        assert_eq!(rel_prices(Some(1.0), None, "SPX", 1, d("3")), (d("5"), d("0.5")));
        // On the tick grid of the option: SPX options above 3.00 tick in 0.10.
        assert_eq!(rel_prices(Some(4.37), Some(0.42), "SPX", 1, d("1")), (d("4.3"), d("0.4")));
    }

    #[test]
    fn a_rel_stop_replaces_the_market_stop_of_a_single_option_only() {
        let t = TradeTemplate {
            stop_flags: StopSwitches { stop_rel: true, ..template().stop_flags },
            stop_rel_limit: Some(3.0),
            stop_rel_offset: Some(0.25),
            ..template()
        };
        let single = exit_prices(&t, "SPX", 1, d("0.3"));
        assert_eq!((single.stop, single.market, single.rel), (Some(d("0.9")), Some(d("0.9")), Some((d("3"), d("0.25")))));
        assert_eq!(single.notes, [] as [std::string::String; 0]);
        // A combo is stopped by a plain market stop, and the log says why there is no REL.
        let combo = exit_prices(&t, "SPX", 2, d("0.3"));
        assert_eq!((combo.market, combo.rel), (Some(d("0.9")), None));
        assert!(combo.notes.iter().any(|n| n.contains("REL works for a single option only")));
        // Without the switch there is none.
        assert_eq!(exit_prices(&template(), "SPX", 1, d("0.3")).rel, None);
        // StopLimit has no market stop to replace.
        let limit = exit_prices(&TradeTemplate { stop_order_type: "Limit stop".into(), ..t.clone() }, "SPX", 1, d("0.3"));
        assert_eq!(limit.rel, None);
        // ES is not supported at all.
        assert_eq!(exit_prices(&t, "ES", 1, d("0.3")).rel, None);
    }

    #[test]
    fn a_rel_stop_is_a_rel_order_with_its_limit_and_offset() {
        let trade = rel_trade("ShortPut", true);
        let t = TradeTemplate {
            stop_flags: StopSwitches { stop_rel: true, ..template().stop_flags },
            order_flags: OrderSwitches { extended_hour_stop: true, ..template().order_flags },
            stop_rel_limit: Some(3.0),
            stop_rel_offset: Some(0.25),
            stop_trigger: 4,
            ..template()
        };
        let prices = exit_prices(&t, "SPX", 1, d("0.3"));
        let legs = [ComboLegSpec { con_id: 2, ratio: 1, action: Action::Buy }];
        let orders = exit_orders(&trade, &t, &prices, &legs, "20261110", 11, 12, 13, trade.qty).unwrap();
        let stop = orders.stop.unwrap();
        assert_eq!((stop.order_type.as_str(), stop.lmt_price, stop.aux_price, stop.action.as_str()), ("REL", 3.0, 0.25, "BUY"));
        assert_eq!((stop.handling.outside_rth, stop.condition_flags.conditions_ignore_rth), (true, true));
        assert_eq!(stop.trigger_method, IbOrder::default().trigger_method, "a REL order carries no trigger method");
        assert!(stop.conditions.is_empty(), "the conditions are added by the caller, which must not forget them");
        assert_eq!(stop.order_ref, "TraPi-11 (SL)");
    }

    #[test]
    fn a_time_condition_names_the_minutes_before_the_close_in_the_zone_of_the_symbol() {
        // SPX closes at 15:00 Central, everything else at 16:00 Eastern.
        assert_eq!(close_time("20261116", "SPX", 30, false).as_deref(), Some("20261116 14:30:00 US/Central"));
        assert_eq!(close_time("20261116", "SPY", 30, false).as_deref(), Some("20261116 15:30:00 US/Eastern"));
        assert_eq!(close_time("20261116", "SPX", 0, false).as_deref(), Some("20261116 15:00:00 US/Central"));
        // An early-close day ends three hours earlier.
        assert_eq!(close_time("20261127", "SPX", 30, true).as_deref(), Some("20261127 11:30:00 US/Central"));
        assert_eq!(close_time("20261127", "QQQ", 0, true).as_deref(), Some("20261127 13:00:00 US/Eastern"));
        // More minutes than the day has go over midnight; a bad date gives nothing.
        assert_eq!(close_time("20261116", "SPY", 17 * 60, false).as_deref(), Some("20261115 23:00:00 US/Eastern"));
        assert_eq!(close_time("2026-11-16", "SPX", 30, false), None);
    }

    #[test]
    fn the_guardrail_of_a_put_side_closes_in_on_the_strike_in_twelve_steps() {
        let trade = rel_trade("PutSpread", true);
        let rails = guardrails(&trade, &RelSettings { guardrail: true, ..rel_settings() });
        assert_eq!(rails.len(), 12);
        // Strike 6600 + 10 at the close, underlying 6700 + 10 at the open; 132 minutes / 12 = 11 each.
        assert_eq!((rails[0].minutes_from_close, rails[0].price, rails[0].greater_than), (121, d("6710"), false));
        assert_eq!((rails[11].minutes_from_close, rails[11].price), (0, d("6610")));
        // Step 7: 6610 + 6 * 100 / 11 = 6664.5454... -> 6664.55, 66 minutes.
        assert_eq!((rails[5].minutes_from_close, rails[5].price), (66, d("6664.55")));
        assert!(rails.windows(2).all(|pair| pair[0].price >= pair[1].price && pair[0].minutes_from_close > pair[1].minutes_from_close));
    }

    #[test]
    fn the_guardrail_of_a_call_side_mirrors_it_and_the_itm_window_moves_every_step() {
        let trade = rel_trade("ShortCall", false);
        let rails = guardrails(&trade, &RelSettings { guardrail: true, ..rel_settings() });
        // Strike 6800 - 10 at the close, underlying 6700 - 10 at the open: the path rises to 6790.
        assert_eq!((rails[0].price, rails[0].greater_than), (d("6690"), true));
        assert_eq!(rails[11].price, d("6790"));
        assert!(rails.windows(2).all(|pair| pair[0].price <= pair[1].price));
        // "ITM only" shifts every step by the ITM minutes (30 without a value).
        let only = guardrails(&trade, &RelSettings { guardrail: true, itm_only: true, ..rel_settings() });
        assert_eq!(only[11].minutes_from_close, 30);
        assert_eq!(only[0].minutes_from_close, 121 + 30);
        let only = guardrails(&trade, &RelSettings { guardrail: true, itm_only: true, itm_minutes: Some(45), ..rel_settings() });
        assert_eq!(only[11].minutes_from_close, 45);
    }

    #[test]
    fn there_is_no_guardrail_without_its_prerequisites() {
        let on = RelSettings { guardrail: true, ..rel_settings() };
        assert!(guardrails(&rel_trade("PutSpread", true), &rel_settings()).is_empty(), "switched off");
        assert!(guardrails(&rel_trade("PutSpread", true), &RelSettings { minutes_to_close: 0, ..on.clone() }).is_empty(), "expires another day");
        assert_eq!(guardrails(&rel_trade("PutSpread", true), &RelSettings { underlying_open: None, ..on.clone() }), [] as [crate::trade_exit::Rail; 0]);
        assert_eq!(
            guardrails(&rel_trade("PutSpread", true), &RelSettings { underlying_open: Some("x".into()), ..on.clone() }),
            [] as [crate::trade_exit::Rail; 0]
        );
        for other in ["IronCondor-Full", "ShortStrangle", "LongPut", "PutDebitSpread"] {
            assert!(guardrails(&rel_trade(other, true), &on).is_empty(), "{other}");
        }
        // The strike of the side must be there.
        let mut no_strike = rel_trade("ShortPut", true);
        no_strike.short_put = None;
        assert_eq!(guardrails(&no_strike, &on), [] as [crate::trade_exit::Rail; 0]);
        // 5 minutes left: every step is 1 minute (rounded up), the last one at the close.
        let few = guardrails(&rel_trade("ShortPut", true), &RelSettings { minutes_to_close: 5, ..on });
        assert_eq!((few[0].minutes_from_close, few[11].minutes_from_close), (11, 0));
    }

    #[test]
    fn a_plain_rel_stop_has_one_condition_the_price_of_the_option() {
        let trade = rel_trade("ShortPut", true);
        let conditions = rel_conditions(&trade, 2, d("0.9"), 4, &RelSettings::default()).unwrap();
        assert_eq!(conditions, [IbCondition::Price { and_next: false, is_more: true, price: 0.9, con_id: 2, exchange: "SMART".into(), trigger_method: 4 }]);
        // A trigger other than default, bid/ask and mid-point is left at default.
        for (given, sent) in [(0, 0), (4, 4), (8, 8), (2, 0), (7, 0), (1, 0)] {
            let c = rel_conditions(&trade, 2, d("0.9"), given, &RelSettings::default()).unwrap();
            assert!(matches!(c.first(), Some(IbCondition::Price { trigger_method, .. }) if *trigger_method == sent), "{given}");
        }
        // A debit trade is stopped when the option falls to the price: at or below.
        let mut debit = trade.clone();
        debit.price_open = rust_decimal::Decimal::from_str_exact("-0.3").unwrap();
        let c = rel_conditions(&debit, 2, d("0.9"), 0, &RelSettings::default()).unwrap();
        assert!(matches!(c.first(), Some(IbCondition::Price { is_more: false, .. })));
    }

    #[test]
    fn the_itm_protection_adds_the_underlying_and_the_last_minutes() {
        let put = rel_trade("PutSpread", true);
        let rel = RelSettings { itm: Some(5), ..RelSettings::default() };
        let c = rel_conditions(&put, 2, d("0.9"), 0, &rel).unwrap();
        assert_eq!(c.len(), 3);
        // The option's price OR (the underlying at or below strike + 5 AND the last 30 minutes).
        assert!(matches!(&c[0], IbCondition::Price { and_next: false, con_id: 2, .. }));
        assert_eq!(c[1], IbCondition::Price { and_next: true, is_more: false, price: 6605.0, con_id: 416_904, exchange: "CBOE".into(), trigger_method: 0 });
        assert_eq!(c[2], IbCondition::Time { and_next: false, is_more: true, time: "20261116 14:30:00 US/Central".into() });
        // A call side: the underlying at or above strike - 5, and a custom number of minutes.
        let call = rel_trade("CallSpread", false);
        let rel = RelSettings { itm: Some(5), itm_minutes: Some(10), ..RelSettings::default() };
        let c = rel_conditions(&call, 3, d("0.9"), 0, &rel).unwrap();
        assert_eq!(c[1], IbCondition::Price { and_next: true, is_more: true, price: 6795.0, con_id: 416_904, exchange: "CBOE".into(), trigger_method: 0 });
        assert_eq!(c[2], IbCondition::Time { and_next: false, is_more: true, time: "20261116 14:50:00 US/Central".into() });
        // The underlying of another symbol is named on its own exchange.
        let mut rut = put.clone();
        rut.symbol = "RUT".into();
        let c = rel_conditions(&rut, 2, d("0.9"), 0, &RelSettings { itm: Some(5), ..RelSettings::default() }).unwrap();
        assert!(matches!(&c[1], IbCondition::Price { con_id: 416_888, exchange, .. } if exchange == "RUSSELL"));
        // "ITM only" starts with a window: only in the last minutes.
        let only = rel_conditions(&put, 2, d("0.9"), 0, &RelSettings { itm_only: true, ..RelSettings::default() }).unwrap();
        assert_eq!(only.len(), 2);
        assert_eq!(only[0], IbCondition::Time { and_next: true, is_more: false, time: "20261116 14:30:00 US/Central".into() });
    }

    #[test]
    fn the_itm_protection_of_a_custom_trade_uses_the_option_its_order_closes() {
        let mut trade = rel_trade("Custom", true);
        trade.short_put = None;
        trade.short_call = None;
        trade.custom_legs =
            Some(vec![crate::records::CustomLegStored { con_id: 7, right: "C".into(), strike: "6800".into(), expiry: "20261116".into(), ratio: -1 }]);
        let rel = RelSettings { itm: Some(5), ..rel_settings() };
        let c = rel_conditions(&trade, 7, d("0.9"), 0, &rel).unwrap();
        // A call: the underlying at or above 6795, as for the short call of a standard trade.
        assert!(matches!(&c[1], IbCondition::Price { and_next: true, is_more: true, price, .. } if price.total_cmp(&6795.0).is_eq()), "{c:?}");
        assert_eq!(rel_conditions(&trade, 99, d("0.9"), 0, &rel), None, "an option the trade does not hold");
    }

    #[test]
    fn the_guardrail_conditions_are_three_per_step_followed_by_the_itm_protection() {
        let trade = rel_trade("PutSpread", true);
        let rel = RelSettings { guardrail: true, itm: Some(5), ..rel_settings() };
        let c = rel_conditions(&trade, 2, d("0.9"), 0, &rel).unwrap();
        assert_eq!(c.len(), 12 * 3 + 2);
        // Step one: until 121 minutes before the close (time before), AND the option's price, AND the underlying at or below the rail.
        assert_eq!(c[0], IbCondition::Time { and_next: true, is_more: false, time: "20261116 12:59:00 US/Central".into() });
        assert!(matches!(&c[1], IbCondition::Price { and_next: true, con_id: 2, .. }));
        assert_eq!(c[2], IbCondition::Price { and_next: false, is_more: false, price: 6710.0, con_id: 416_904, exchange: "CBOE".into(), trigger_method: 0 });
        // The last step ends at the close; the ITM protection follows the steps.
        assert_eq!(price(&c[3 * 11 + 2]), Some(6610.0));
        assert!(matches!(&c[36], IbCondition::Price { and_next: true, price, .. } if price.total_cmp(&6605.0).is_eq()));
        assert!(matches!(&c[37], IbCondition::Time { and_next: false, is_more: true, .. }));
    }

    #[test]
    fn a_guardrail_without_steps_never_turns_the_stop_into_one_without_its_price_condition() {
        // Expires another day: sending only the ITM conditions here would drop the price condition; the option's price stays.
        let trade = rel_trade("PutSpread", true);
        let rel = RelSettings { guardrail: true, minutes_to_close: 0, ..rel_settings() };
        let c = rel_conditions(&trade, 2, d("0.9"), 0, &rel).unwrap();
        assert!(matches!(c.as_slice(), [IbCondition::Price { con_id: 2, .. }]));
    }

    #[test]
    fn conditions_that_need_what_is_unknown_are_not_built() {
        let mut trade = rel_trade("PutSpread", true);
        let itm = RelSettings { itm: Some(5), ..RelSettings::default() };
        trade.symbol = "XYZ".into();
        assert_eq!(rel_conditions(&trade, 2, d("0.9"), 0, &itm), None, "no contract id for the underlying");
        assert!(rel_conditions(&trade, 2, d("0.9"), 0, &RelSettings::default()).is_some(), "the plain stop needs none");
        let mut no_strike = rel_trade("PutSpread", true);
        no_strike.short_put = None;
        assert_eq!(rel_conditions(&no_strike, 2, d("0.9"), 0, &itm), None);
        let mut bad_date = rel_trade("PutSpread", true);
        bad_date.expiry = "soon".into();
        assert_eq!(rel_conditions(&bad_date, 2, d("0.9"), 0, &itm), None);
    }

    #[test]
    fn the_underlyings_have_their_contract_ids() {
        for (symbol, id) in [("SPX", 416_904), ("XSP", 137_851_301), ("RUT", 416_888), ("NDX", 416_843), ("SPY", 756_733), ("IWM", 9_579_970)] {
            assert_eq!(underlying_con_id(symbol), Some(id), "{symbol}");
        }
        assert_eq!(underlying_con_id("ES"), None);
        assert_eq!(underlying_con_id(""), None);
    }

    #[test]
    fn a_rel_order_only_goes_out_with_its_conditions() {
        let trade = rel_trade("ShortPut", true);
        let legs = [ComboLegSpec { con_id: 2, ratio: 1, action: Action::Buy }];
        let t = TradeTemplate {
            stop_flags: StopSwitches { stop_rel: true, ..template().stop_flags },
            order_flags: OrderSwitches { extended_hour_stop: true, ..template().order_flags },
            ..template()
        };
        let prices = exit_prices(&t, "SPX", 1, d("0.3"));
        let mut order = exit_orders(&trade, &t, &prices, &legs, "20261110", 11, 12, 13, trade.qty).unwrap().stop.unwrap();
        assert!(attach_rel_conditions(&mut order, &trade, 2, d("0.9"), &RelSettings::default(), 0));
        assert_eq!((order.conditions.len(), order.condition_flags.conditions_ignore_rth), (1, true));
        // Not buildable: nothing is changed and the caller has to drop the order.
        let mut other = exit_orders(&trade, &t, &prices, &legs, "20261110", 11, 12, 13, trade.qty).unwrap().stop.unwrap();
        let unknown = StoredTrade { symbol: "XYZ".into(), ..trade };
        assert!(!attach_rel_conditions(&mut other, &unknown, 2, d("0.9"), &RelSettings { itm: Some(5), ..RelSettings::default() }, 0));
        assert_eq!(other.conditions, [] as [broker::IbCondition; 0]);
    }

    #[test]
    fn an_adjusted_rel_stop_is_rebuilt_with_its_conditions_or_not_at_all() {
        let trade = rel_trade("ShortPut", true);
        let mut adjustments = StopAdjustments { rel: Some(RelSettings { itm: Some(5), ..RelSettings::default() }), ..adjusted("Market stop") };
        adjustments.legs = vec![(2, 1, false)];
        let orders = replacement_stop_orders(&trade, &adjustments, "20261110", "TradePilot-11", 20, 21).unwrap();
        let stop = orders.stop.unwrap();
        assert_eq!((stop.order_type.as_str(), stop.lmt_price, stop.aux_price, stop.oca_group.as_str()), ("REL", 8.0, 0.5, "TradePilot-11"));
        assert_eq!(stop.conditions.len(), 3);
        // The conditions cannot be built for an unknown underlying: no stop is better than an unconditional REL.
        let unknown = StoredTrade { symbol: "XYZ".into(), ..trade };
        assert!(replacement_stop_orders(&unknown, &adjustments, "20261110", "TradePilot-11", 20, 21).is_none());
        // A combo is stopped by the plain market stop.
        let combo = StopAdjustments { legs: vec![(2, 1, false), (3, 1, false)], ..adjustments };
        let plain = replacement_stop_orders(&iron_condor(), &combo, "20261110", "TradePilot-11", 20, 21).unwrap().stop.unwrap();
        assert_eq!((plain.order_type.as_str(), plain.conditions.len()), ("STP", 0));
    }

    #[test]
    fn a_short_stop_buys_back_only_the_shorts_and_a_vertical_stop_sells_the_longs_too() {
        let trade = iron_condor();
        let all_bid = |_: i32| true;
        let short = exit_legs(&trade, &template(), &all_bid);
        assert_eq!(short.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), vec![(3, "BUY"), (2, "BUY")]);
        let vertical = exit_legs(&trade, &TradeTemplate { stop_type: "Vertical".into(), ..template() }, &all_bid);
        assert_eq!(vertical.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), vec![(3, "BUY"), (2, "BUY"), (1, "SELL"), (4, "SELL")]);
        let put_side = exit_legs(&trade, &TradeTemplate { stop_type: "VerticalP".into(), ..template() }, &all_bid);
        assert_eq!(put_side.iter().map(|l| l.con_id).collect::<Vec<_>>(), vec![3, 2, 1]);
        // A long without a bid is dropped (that becomes a Short stop).
        let no_put_bid = exit_legs(&trade, &TradeTemplate { stop_type: "Vertical".into(), ..template() }, &|id| id != 1);
        assert_eq!(no_put_bid.iter().map(|l| l.con_id).collect::<Vec<_>>(), vec![3, 2, 4]);
        // Without a stop the target closes the whole vertical.
        let target_only = exit_legs(&trade, &TradeTemplate { stop_order_type: "None".into(), ..template() }, &all_bid);
        assert_eq!(target_only.len(), 4);
    }

    #[test]
    fn closing_a_trade_reverses_every_leg() {
        let legs = close_legs(&iron_condor());
        assert_eq!(legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), vec![(1, "SELL"), (2, "BUY"), (4, "SELL"), (3, "BUY")]);
    }

    #[test]
    fn stop_and_target_share_an_oca_group_and_the_combo_legs() {
        let trade = iron_condor();
        let t = template();
        let prices = exit_prices(&t, "SPX", 2, d("3"));
        let legs = exit_legs(&trade, &t, &|_| true);
        let orders = exit_orders(&trade, &t, &prices, &legs, "20261116", 11, 12, 13, trade.qty).unwrap();
        assert_eq!((orders.contract.sec_type.as_str(), orders.contract.combo_legs.len()), ("BAG", 2));
        let (stop, profit) = (orders.stop.unwrap(), orders.profit.unwrap());
        assert_eq!((stop.order_type.as_str(), stop.aux_price, stop.action.as_str(), stop.total_quantity), ("STP", 9.0, "BUY", 2.0));
        assert_eq!((profit.order_type.as_str(), profit.lmt_price), ("LMT", 1.5));
        assert_eq!((stop.oca_group.as_str(), stop.oca_type), ("TradePilot-11", 3));
        assert_eq!((profit.oca_group.as_str(), profit.oca_type), ("TradePilot-11", 3));
        assert_eq!(stop.tif, "DAY");
        let later = exit_orders(&trade, &t, &prices, &legs, "20261110", 11, 12, 13, trade.qty).unwrap();
        assert_eq!(later.stop.unwrap().tif, "GTC", "a trade that does not expire today needs GTC orders");
    }

    #[test]
    fn a_lone_target_still_has_its_oca_group_for_a_later_close_order_and_a_single_leg_goes_out_as_an_option() {
        let trade = StoredTrade { trade_type: "ShortPut".into(), long_put: None, short_call: None, long_call: None, ..iron_condor() };
        let t = TradeTemplate { stop_order_type: "None".into(), ..template() };
        let prices = exit_prices(&t, "SPX", 1, d("3"));
        let legs = exit_legs(&trade, &t, &|_| true);
        assert_eq!(legs.len(), 1);
        let orders = exit_orders(&trade, &t, &prices, &legs, "20261116", 11, 12, 13, trade.qty).unwrap();
        assert!(orders.stop.is_none());
        let profit = orders.profit.unwrap();
        assert_eq!((profit.oca_group.as_str(), profit.oca_type), ("TradePilot-11", 3));
        let c = orders.contract;
        assert_eq!((c.sec_type.as_str(), c.trading_class.as_str(), c.right.as_str(), c.strike), ("OPT", "SPXW", "P", 6600.0));
        assert_eq!(c.last_trade_date_or_contract_month, "20261116");
    }
}
