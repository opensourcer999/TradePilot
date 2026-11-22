//! Trade conditions: a named set of rules that a schedule
//! checks before it enters its trade. A rule compares a value of the market (VIX, SPX, the time to the
//! close) with a number or with another value. Rules of one *group* must all be true, a condition is true
//! if any of its groups is. The checks are plain functions of a `Snapshot` of the market, so they need no
//! connection and are tested without one. Conditions live in `conditions.json` per mode, written as a whole
//! like the other lists.
//!
//! Only the values the app has live are offered: the quotes of SPX, VIX and `VIX9D`, the time of the
//! session and, for the early exit of a trade, the values of that open trade (`Trade_*`). Averages, RSI, VIX rank and percentile
//! are worked out of bars (`bars`); every input of the list is worked out (`ES` stays unknown: the future is not subscribed).

use crate::bars::{self, Bar};
use chrono::{NaiveDateTime, NaiveTime, Timelike};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A value of the market a rule can look at. The file keeps its own keys (`snake_case`); the screens show `name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    /// The VIX now.
    #[serde(rename = "vix")]
    Vix,
    /// The VIX at today's open.
    #[serde(rename = "vix_open")]
    VixOpen,
    /// The VIX at yesterday's close.
    #[serde(rename = "vix_prior_close")]
    VixPriorClose,
    /// The VIX now minus its open, in points.
    #[serde(rename = "vix_intraday_points")]
    VixIntradayPts,
    /// The VIX now against its open, in percent.
    #[serde(rename = "vix_intraday_percent")]
    VixIntradayPct,
    /// The VIX now minus yesterday's close, in points.
    #[serde(rename = "vix_change_points")]
    VixChangePts,
    /// The VIX now against yesterday's close, in percent.
    #[serde(rename = "vix_change_percent")]
    VixChangePct,
    /// The VIX at the open minus yesterday's close, in points.
    #[serde(rename = "vix_gap_points")]
    VixGapPts,
    /// The VIX at the open against yesterday's close, in percent.
    #[serde(rename = "vix_gap_percent")]
    VixGapPct,
    /// The VIX9D divided by the VIX.
    #[serde(rename = "vix9d_over_vix")]
    Vix9dOverVix,
    /// The SPX now.
    #[serde(rename = "spx")]
    Spx,
    /// The SPX at today's open.
    #[serde(rename = "spx_open")]
    SpxOpen,
    /// The SPX at yesterday's close.
    #[serde(rename = "spx_prior_close")]
    SpxPriorClose,
    /// The SPX now minus its open, in points.
    #[serde(rename = "spx_intraday_points")]
    SpxIntradayPts,
    /// The SPX now against its open, in percent.
    #[serde(rename = "spx_intraday_percent")]
    SpxIntradayPct,
    /// The SPX now minus yesterday's close, in points.
    #[serde(rename = "spx_change_points")]
    SpxChangePts,
    /// The SPX now against yesterday's close, in percent.
    #[serde(rename = "spx_change_percent")]
    SpxChangePct,
    /// The SPX at the open minus yesterday's close, in points.
    #[serde(rename = "spx_gap_points")]
    SpxGapPts,
    /// The SPX at the open against yesterday's close, in percent.
    #[serde(rename = "spx_gap_percent")]
    SpxGapPct,
    /// The whole minutes since the session opened (09:30 New York); negative before it.
    #[serde(rename = "minutes_from_open")]
    MinutesFromOpen,
    /// The minutes to the close (16:00, 13:00 on an early close), rounded up; negative after it.
    #[serde(rename = "minutes_to_close")]
    MinutesToClose,
    /// The profit of the trade in dollars now (trade rules only).
    #[serde(rename = "trade_profit_amount")]
    TradeProfitAmt,
    /// The profit of the trade in percent of what it took in or cost.
    #[serde(rename = "trade_profit_percent")]
    TradeProfitPct,
    /// What closing the trade costs now (credit trade).
    #[serde(rename = "trade_ask_price")]
    TradeAskPrice,
    /// What closing the trade brings now (debit trade).
    #[serde(rename = "trade_bid_price")]
    TradeBidPrice,
    /// The delta of the long call, times 100, without sign.
    #[serde(rename = "trade_long_call_delta")]
    TradeLongCallDelta,
    /// The delta of the long put, times 100, without sign.
    #[serde(rename = "trade_long_put_delta")]
    TradeLongPutDelta,
    /// The delta of the short call, times 100, without sign.
    #[serde(rename = "trade_short_call_delta")]
    TradeShortCallDelta,
    /// The delta of the short put, times 100, without sign.
    #[serde(rename = "trade_short_put_delta")]
    TradeShortPutDelta,
    /// The long call strike minus the underlying, in points.
    #[serde(rename = "trade_long_call_otm_points")]
    TradeLongCallOtmPts,
    /// The underlying minus the long put strike, in points.
    #[serde(rename = "trade_long_put_otm_points")]
    TradeLongPutOtmPts,
    /// The short call strike minus the underlying, in points.
    #[serde(rename = "trade_short_call_otm_points")]
    TradeShortCallOtmPts,
    /// The underlying minus the short put strike, in points.
    #[serde(rename = "trade_short_put_otm_points")]
    TradeShortPutOtmPts,
    /// The long call distance in percent of the underlying.
    #[serde(rename = "trade_long_call_otm_percent")]
    TradeLongCallOtmPct,
    /// The long put distance in percent of the underlying.
    #[serde(rename = "trade_long_put_otm_percent")]
    TradeLongPutOtmPct,
    /// The short call distance in percent of the underlying.
    #[serde(rename = "trade_short_call_otm_percent")]
    TradeShortCallOtmPct,
    /// The short put distance in percent of the underlying.
    #[serde(rename = "trade_short_put_otm_percent")]
    TradeShortPutOtmPct,
    /// The asks of the short legs divided by the asks of the long legs.
    #[serde(rename = "trade_short_long_ratio")]
    TradeShortLongRatio,
    /// The exponential average of 5 one-minute bars of the underlying.
    #[serde(rename = "ema5")]
    Ema5,
    /// The exponential average of 20 one-minute bars of the underlying.
    #[serde(rename = "ema20")]
    Ema20,
    /// The exponential average of 40 one-minute bars of the underlying.
    #[serde(rename = "ema40")]
    Ema40,
    /// The exponential average of 750 one-minute bars of the underlying.
    #[serde(rename = "ema750")]
    Ema750,
    /// The exponential average of a number of one-minute bars of the underlying (the length of the rule).
    #[serde(rename = "ema")]
    Ema,
    /// The simple average of a number of daily SPX bars (the length of the rule).
    #[serde(rename = "sma_daily")]
    SmaDaily,
    /// The RSI (14) of the daily SPX bars.
    #[serde(rename = "rsi")]
    Rsi,
    /// Where the VIX stands among the highs and lows of a number of one-minute bars (the length of the rule), 0 to 100.
    #[serde(rename = "vix_rank")]
    VixRank,
    /// The share of a number of one-minute VIX closes (the length of the rule) below the latest one, in percent.
    #[serde(rename = "vix_percentile")]
    VixPercentile,
    /// The price of the underlying of the template or the trade.
    #[serde(rename = "underlying_price")]
    UnderlyingPrice,
    /// The SPY price (when SPY is among the market data symbols).
    #[serde(rename = "spy")]
    Spy,
    /// The QQQ price (when QQQ is among the market data symbols).
    #[serde(rename = "qqq")]
    Qqq,
    /// The ES future (not subscribed: never known).
    #[serde(rename = "es")]
    Es,
    /// The total delta of every open trade.
    #[serde(rename = "delta_total")]
    DeltaTotal,
    /// The total delta of the open trades on the underlying.
    #[serde(rename = "delta_total_symbol")]
    DeltaTotalSymbol,
    /// The total delta of the open trades of the account.
    #[serde(rename = "delta_account")]
    DeltaAccount,
    /// The total delta of the open trades of the account on the underlying.
    #[serde(rename = "delta_account_symbol")]
    DeltaAccountSymbol,
    /// The delta of the trade per contract (trade rules only).
    #[serde(rename = "trade_position_delta")]
    TradePositionDelta,
    /// The delta of the whole trade.
    #[serde(rename = "trade_position_total_delta")]
    TradePositionTotalDelta,
    /// How the short/long ratio changed since the entry, in percent.
    #[serde(rename = "trade_short_long_ratio_change_percent")]
    TradeShortLongRatioChangePct,
    /// The delta of custom leg 1, times 100, without sign.
    #[serde(rename = "trade_custom_leg1_delta")]
    TradeCustomLeg1Delta,
    /// The delta of custom leg 2, times 100, without sign.
    #[serde(rename = "trade_custom_leg2_delta")]
    TradeCustomLeg2Delta,
    /// The delta of custom leg 3, times 100, without sign.
    #[serde(rename = "trade_custom_leg3_delta")]
    TradeCustomLeg3Delta,
    /// The delta of custom leg 4, times 100, without sign.
    #[serde(rename = "trade_custom_leg4_delta")]
    TradeCustomLeg4Delta,
}

impl Input {
    /// Every value, in the order of the drop-downs.
    pub const ALL: [Self; 62] = [
        Self::Vix,
        Self::VixOpen,
        Self::VixPriorClose,
        Self::VixIntradayPts,
        Self::VixIntradayPct,
        Self::VixChangePts,
        Self::VixChangePct,
        Self::VixGapPts,
        Self::VixGapPct,
        Self::Vix9dOverVix,
        Self::Spx,
        Self::SpxOpen,
        Self::SpxPriorClose,
        Self::SpxIntradayPts,
        Self::SpxIntradayPct,
        Self::SpxChangePts,
        Self::SpxChangePct,
        Self::SpxGapPts,
        Self::SpxGapPct,
        Self::MinutesFromOpen,
        Self::MinutesToClose,
        Self::TradeProfitAmt,
        Self::TradeProfitPct,
        Self::TradeAskPrice,
        Self::TradeBidPrice,
        Self::TradeLongCallDelta,
        Self::TradeLongPutDelta,
        Self::TradeShortCallDelta,
        Self::TradeShortPutDelta,
        Self::TradeLongCallOtmPts,
        Self::TradeLongPutOtmPts,
        Self::TradeShortCallOtmPts,
        Self::TradeShortPutOtmPts,
        Self::TradeLongCallOtmPct,
        Self::TradeLongPutOtmPct,
        Self::TradeShortCallOtmPct,
        Self::TradeShortPutOtmPct,
        Self::TradeShortLongRatio,
        Self::Ema5,
        Self::Ema20,
        Self::Ema40,
        Self::Ema750,
        Self::Ema,
        Self::SmaDaily,
        Self::Rsi,
        Self::VixRank,
        Self::VixPercentile,
        Self::UnderlyingPrice,
        Self::Spy,
        Self::Qqq,
        Self::Es,
        Self::DeltaTotal,
        Self::DeltaTotalSymbol,
        Self::DeltaAccount,
        Self::DeltaAccountSymbol,
        Self::TradePositionDelta,
        Self::TradePositionTotalDelta,
        Self::TradeShortLongRatioChangePct,
        Self::TradeCustomLeg1Delta,
        Self::TradeCustomLeg2Delta,
        Self::TradeCustomLeg3Delta,
        Self::TradeCustomLeg4Delta,
    ];

    /// Whether a rule needs a length (a number of bars) with this value.
    #[must_use]
    pub fn takes_length(self) -> bool {
        matches!(self, Self::Ema | Self::SmaDaily | Self::VixRank | Self::VixPercentile)
    }

    /// The name with its length, e.g. `EMA(20)`; the plain name for a value without one.
    #[must_use]
    pub fn text(self, length: u32) -> String {
        if self.takes_length() {
            format!("{}({length})", self.name())
        } else {
            self.name().to_string()
        }
    }

    /// The name on the screens.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Vix => "VIX",
            Self::VixOpen => "VIXOpen",
            Self::VixPriorClose => "VIXPriorClose",
            Self::VixIntradayPts => "VIXIntradayPts",
            Self::VixIntradayPct => "VIXIntradayPct",
            Self::VixChangePts => "VIXChangePts",
            Self::VixChangePct => "VIXChangePct",
            Self::VixGapPts => "VIXGapPts",
            Self::VixGapPct => "VIXGapPct",
            Self::Vix9dOverVix => "VIX9D/VIX",
            Self::Spx => "SPX",
            Self::SpxOpen => "SPXOpen",
            Self::SpxPriorClose => "SPXPriorClose",
            Self::SpxIntradayPts => "SPXIntradayPts",
            Self::SpxIntradayPct => "SPXIntradayPct",
            Self::SpxChangePts => "SPXChangePts",
            Self::SpxChangePct => "SPXChangePct",
            Self::SpxGapPts => "SPXGapPts",
            Self::SpxGapPct => "SPXGapPct",
            Self::MinutesFromOpen => "Time_MinutesFromMarketOpen",
            Self::MinutesToClose => "Time_MinutesToMarketClose",
            Self::TradeAskPrice => "Trade_AskPrice",
            Self::TradeBidPrice => "Trade_BidPrice",
            Self::TradeLongCallDelta => "Trade_LongCallDelta",
            Self::TradeLongCallOtmPct => "Trade_LongCallOTMPct",
            Self::TradeLongCallOtmPts => "Trade_LongCallOTMPts",
            Self::TradeLongPutDelta => "Trade_LongPutDelta",
            Self::TradeLongPutOtmPct => "Trade_LongPutOTMPct",
            Self::TradeLongPutOtmPts => "Trade_LongPutOTMPts",
            Self::TradeProfitAmt => "Trade_ProfitAmt",
            Self::TradeProfitPct => "Trade_ProfitPct",
            Self::TradeShortCallDelta => "Trade_ShortCallDelta",
            Self::TradeShortCallOtmPct => "Trade_ShortCallOTMPct",
            Self::TradeShortCallOtmPts => "Trade_ShortCallOTMPts",
            Self::TradeShortLongRatio => "Trade_ShortLongRatio",
            Self::Ema5 => "EMA5",
            Self::Ema20 => "EMA20",
            Self::Ema40 => "EMA40",
            Self::Ema750 => "EMA750",
            Self::Ema => "EMA",
            Self::SmaDaily => "SMADaily",
            Self::Rsi => "RSI",
            Self::VixRank => "VIXRank",
            Self::VixPercentile => "VIXPercentile",
            Self::UnderlyingPrice => "UnderlyingPrice",
            Self::Spy => "SPY",
            Self::Qqq => "QQQ",
            Self::Es => "ES",
            Self::DeltaTotal => "DeltaTotal",
            Self::DeltaTotalSymbol => "DeltaTotalSymbol",
            Self::DeltaAccount => "DeltaAccount",
            Self::DeltaAccountSymbol => "DeltaAccountSymbol",
            Self::TradePositionDelta => "Trade_PositionDelta",
            Self::TradePositionTotalDelta => "Trade_PositionTotalDelta",
            Self::TradeShortLongRatioChangePct => "Trade_ShortLongRatioChangePct",
            Self::TradeCustomLeg1Delta => "Trade_CustomLeg1Delta",
            Self::TradeCustomLeg2Delta => "Trade_CustomLeg2Delta",
            Self::TradeCustomLeg3Delta => "Trade_CustomLeg3Delta",
            Self::TradeCustomLeg4Delta => "Trade_CustomLeg4Delta",
            Self::TradeShortPutDelta => "Trade_ShortPutDelta",
            Self::TradeShortPutOtmPct => "Trade_ShortPutOTMPct",
            Self::TradeShortPutOtmPts => "Trade_ShortPutOTMPts",
        }
    }
}

/// How a rule compares its value with the other side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operator {
    /// Above.
    #[serde(rename = ">")]
    Greater,
    /// Above or equal.
    #[serde(rename = ">=")]
    GreaterOrEqual,
    /// Below.
    #[serde(rename = "<")]
    Less,
    /// Below or equal.
    #[serde(rename = "<=")]
    LessOrEqual,
    /// Equal.
    #[serde(rename = "=")]
    Equal,
}

impl Operator {
    /// Every operator, in the order of the drop-down.
    pub const ALL: [Self; 5] = [Self::Greater, Self::GreaterOrEqual, Self::Less, Self::LessOrEqual, Self::Equal];

    /// The sign on the screens.
    #[must_use]
    pub fn sign(self) -> &'static str {
        match self {
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Equal => "=",
        }
    }

    /// Whether `left` stands to `right` as the operator says.
    #[must_use]
    pub fn holds(self, left: Decimal, right: Decimal) -> bool {
        match self {
            Self::Greater => left > right,
            Self::GreaterOrEqual => left >= right,
            Self::Less => left < right,
            Self::LessOrEqual => left <= right,
            Self::Equal => left == right,
        }
    }
}

/// What a rule compares its value with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparison {
    /// A fixed number.
    Value(Decimal),
    /// Another value of the market.
    Input(Input),
}

/// One rule of a condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The group (1, 2, ...): rules of one group must all be true.
    pub group: u32,
    /// The value looked at.
    pub input: Input,
    /// The comparison.
    pub operator: Operator,
    /// What it is compared with.
    pub comparison: Comparison,
    /// The number of bars of the value looked at, for a value that takes one (0 otherwise).
    #[serde(default)]
    pub input_length: u32,
    /// The number of bars of the value it is compared with, for a value that takes one (0 otherwise).
    #[serde(default)]
    pub comparison_length: u32,
}

/// A named set of rules.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Condition {
    /// The id (1, 2, ...), unique in the file; 0 for one that is not saved yet.
    pub id: u32,
    /// The name.
    pub name: String,
    /// If no group is true: keep checking until the window of the schedule ends, instead of giving up at once.
    pub retry_until_expiration: bool,
    /// The rules.
    pub rules: Vec<Rule>,
}

/// The values of the market at one moment; `None` where a value is not known (no quote yet, or zero).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The VIX.
    pub vix: Option<Decimal>,
    /// The VIX at the open.
    pub vix_open: Option<Decimal>,
    /// The VIX at yesterday's close.
    pub vix_prior_close: Option<Decimal>,
    /// The VIX9D.
    pub vix9d: Option<Decimal>,
    /// The SPX.
    pub spx: Option<Decimal>,
    /// The SPX at the open.
    pub spx_open: Option<Decimal>,
    /// The SPX at yesterday's close.
    pub spx_prior_close: Option<Decimal>,
    /// The minutes since the session opened.
    pub minutes_from_open: Option<Decimal>,
    /// The minutes to the close.
    pub minutes_to_close: Option<Decimal>,
    /// The price of the underlying of the trade the rules are checked for (trade rules only).
    pub underlying: Option<Decimal>,
    /// The trade the rules are checked for, if any.
    pub trade: Option<TradeValues>,
    /// The symbol of the underlying the averages are worked out for (its one-minute bars).
    pub symbol: String,
    /// The bars the rules need, by key: `min:SYMBOL` (one-minute bars) and `day:SPX` (daily bars), oldest first.
    pub bars: std::collections::BTreeMap<String, Vec<Bar>>,
    /// The SPY price, if it is among the market data symbols.
    pub spy: Option<Decimal>,
    /// The QQQ price, if it is among the market data symbols.
    pub qqq: Option<Decimal>,
    /// The ES price: the app does not subscribe the future, so it is never known.
    pub es: Option<Decimal>,
    /// The account the rules are checked for (the account of the schedule or the trade).
    pub account: String,
    /// The open trades with their total delta, for the delta totals.
    pub open_deltas: Vec<OpenDelta>,
}

/// The total delta of one open trade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenDelta {
    /// The account of the trade.
    pub account: String,
    /// The underlying of the trade.
    pub symbol: String,
    /// The delta of the whole trade (delta per contract times the contracts).
    pub total: Decimal,
}

/// One held leg of a trade: its strike, its ask and its delta (times 100, without sign).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegValues {
    /// The strike.
    pub strike: Decimal,
    /// The ask; `None` without a quote.
    pub ask: Option<Decimal>,
    /// The delta times 100, without sign; `None` while TWS has none.
    pub delta: Option<Decimal>,
}

/// What the trade rules look at: the open trade valued at the quotes of now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TradeValues {
    /// The profit in dollars; `None` without quotes.
    pub profit: Option<Decimal>,
    /// The profit in percent of the premium; `None` without quotes.
    pub profit_pct: Option<Decimal>,
    /// What closing costs now (a credit trade).
    pub ask: Option<Decimal>,
    /// What closing brings now (a debit trade).
    pub bid: Option<Decimal>,
    /// The sold put.
    pub short_put: Option<LegValues>,
    /// The bought put.
    pub long_put: Option<LegValues>,
    /// The sold call.
    pub short_call: Option<LegValues>,
    /// The bought call.
    pub long_call: Option<LegValues>,
    /// The delta of the trade per contract; `None` while it is 0 (not known).
    pub position_delta: Option<Decimal>,
    /// The delta of the whole trade; `None` while it is 0.
    pub total_delta: Option<Decimal>,
    /// The short/long ratio at the entry, for its change.
    pub ratio_open: Option<Decimal>,
    /// The delta of each custom leg times 100, without sign.
    pub custom_deltas: [Option<Decimal>; 4],
}

impl TradeValues {
    /// The distance of a leg from the underlying in points: a call is out of the money above the price, a put below.
    fn otm_points(leg: Option<&LegValues>, underlying: Option<Decimal>, call: bool) -> Option<Decimal> {
        let (strike, price) = (leg?.strike, underlying.filter(|price| !price.is_zero())?);
        if call {
            strike.checked_sub(price)
        } else {
            price.checked_sub(strike)
        }
    }

    /// The asks of the short legs divided by the asks of the long legs; `None` if a held leg has no ask or there is no long leg.
    fn short_long_ratio(&self) -> Option<Decimal> {
        let asks = |legs: [Option<&LegValues>; 2]| -> Option<Decimal> {
            let mut sum = Decimal::ZERO;
            for leg in legs.into_iter().flatten() {
                sum = sum.checked_add(leg.ask.filter(|ask| *ask > Decimal::ZERO)?)?;
            }
            Some(sum)
        };
        let shorts = asks([self.short_call.as_ref(), self.short_put.as_ref()])?;
        let longs = asks([self.long_call.as_ref(), self.long_put.as_ref()])?;
        (!longs.is_zero()).then_some(longs).and_then(|longs| shorts.checked_div(longs))
    }

    /// The value of a trade input; `None` if what it needs is not known.
    fn value(&self, input: Input, underlying: Option<Decimal>) -> Option<Decimal> {
        let delta = |leg: &Option<LegValues>| leg.as_ref().and_then(|leg| leg.delta);
        let pts = |leg: &Option<LegValues>, call: bool| Self::otm_points(leg.as_ref(), underlying, call);
        let pct = |points: Option<Decimal>| {
            let price = underlying.filter(|price| !price.is_zero())?;
            points?.checked_div(price)?.checked_mul(Decimal::ONE_HUNDRED)
        };
        match input {
            Input::TradeProfitAmt => self.profit,
            Input::TradeProfitPct => self.profit_pct,
            Input::TradeAskPrice => self.ask,
            Input::TradeBidPrice => self.bid,
            Input::TradeLongCallDelta => delta(&self.long_call),
            Input::TradeLongPutDelta => delta(&self.long_put),
            Input::TradeShortCallDelta => delta(&self.short_call),
            Input::TradeShortPutDelta => delta(&self.short_put),
            Input::TradeLongCallOtmPts => pts(&self.long_call, true),
            Input::TradeLongPutOtmPts => pts(&self.long_put, false),
            Input::TradeShortCallOtmPts => pts(&self.short_call, true),
            Input::TradeShortPutOtmPts => pts(&self.short_put, false),
            Input::TradeLongCallOtmPct => pct(pts(&self.long_call, true)),
            Input::TradeLongPutOtmPct => pct(pts(&self.long_put, false)),
            Input::TradeShortCallOtmPct => pct(pts(&self.short_call, true)),
            Input::TradeShortPutOtmPct => pct(pts(&self.short_put, false)),
            Input::TradeShortLongRatio => self.short_long_ratio(),
            Input::TradePositionDelta => self.position_delta,
            Input::TradePositionTotalDelta => self.total_delta,
            Input::TradeShortLongRatioChangePct => {
                let open = self.ratio_open.filter(|open| !open.is_zero())?;
                self.short_long_ratio()?.checked_sub(open)?.checked_div(open)?.checked_mul(Decimal::ONE_HUNDRED)
            }
            Input::TradeCustomLeg1Delta => self.custom_deltas[0],
            Input::TradeCustomLeg2Delta => self.custom_deltas[1],
            Input::TradeCustomLeg3Delta => self.custom_deltas[2],
            Input::TradeCustomLeg4Delta => self.custom_deltas[3],
            _ => None,
        }
    }
}

/// `a` minus `b`, if both are known.
fn difference(a: Option<Decimal>, b: Option<Decimal>) -> Option<Decimal> {
    a?.checked_sub(b?)
}

/// `a` against `b` in percent, if both are known and `b` is not 0.
fn percent(a: Option<Decimal>, b: Option<Decimal>) -> Option<Decimal> {
    let base = b.filter(|base| !base.is_zero())?;
    difference(a, b)?.checked_div(base)?.checked_mul(Decimal::ONE_HUNDRED)
}

impl Snapshot {
    /// The value of `input` without a length, `None` if what it needs is not known.
    #[cfg(test)]
    #[must_use]
    pub fn value(&self, input: Input) -> Option<Decimal> {
        self.value_with(input, 0)
    }

    /// The total delta of the open trades `chosen` picks: 0 when there are none, else their sum - which is not known when it is 0.
    fn delta_total(&self, chosen: impl Fn(&OpenDelta) -> bool) -> Option<Decimal> {
        let picked: Vec<&OpenDelta> = self.open_deltas.iter().filter(|open| chosen(open)).collect();
        if picked.is_empty() {
            return Some(Decimal::ZERO);
        }
        let sum = picked.iter().try_fold(Decimal::ZERO, |sum, open| sum.checked_add(open.total))?;
        (!sum.is_zero()).then_some(sum)
    }

    /// The bars under `key`, if there are any.
    fn series(&self, key: &str) -> Option<&[Bar]> {
        self.bars.get(key).map(Vec::as_slice).filter(|bars| !bars.is_empty())
    }

    /// The value of `input` for a rule with `length` bars, `None` if what it needs is not known.
    #[must_use]
    pub fn value_with(&self, input: Input, length: u32) -> Option<Decimal> {
        let length = usize::try_from(length).ok()?;
        let minute = || self.series(&format!("min:{}", self.symbol));
        match input {
            Input::Ema5 => bars::ema(minute()?, 5),
            Input::Ema20 => bars::ema(minute()?, 20),
            Input::Ema40 => bars::ema(minute()?, 40),
            Input::Ema750 => bars::ema(minute()?, 750),
            Input::Ema => bars::ema(minute()?, length),
            Input::SmaDaily => bars::sma(self.series("day:SPX")?, length),
            Input::Rsi => bars::rsi(self.series("day:SPX")?, 14),
            Input::VixRank => bars::rank(self.series("min:VIX")?, length),
            Input::VixPercentile => bars::percentile(self.series("min:VIX")?, length),
            Input::Vix => self.vix,
            Input::VixOpen => self.vix_open,
            Input::VixPriorClose => self.vix_prior_close,
            Input::VixIntradayPts => difference(self.vix, self.vix_open),
            Input::VixIntradayPct => percent(self.vix, self.vix_open),
            Input::VixChangePts => difference(self.vix, self.vix_prior_close),
            Input::VixChangePct => percent(self.vix, self.vix_prior_close),
            Input::VixGapPts => difference(self.vix_open, self.vix_prior_close),
            Input::VixGapPct => percent(self.vix_open, self.vix_prior_close),
            Input::Vix9dOverVix => self.vix9d?.checked_div(self.vix.filter(|vix| !vix.is_zero())?),
            Input::Spx => self.spx,
            Input::SpxOpen => self.spx_open,
            Input::SpxPriorClose => self.spx_prior_close,
            Input::SpxIntradayPts => difference(self.spx, self.spx_open),
            Input::SpxIntradayPct => percent(self.spx, self.spx_open),
            Input::SpxChangePts => difference(self.spx, self.spx_prior_close),
            Input::SpxChangePct => percent(self.spx, self.spx_prior_close),
            Input::SpxGapPts => difference(self.spx_open, self.spx_prior_close),
            Input::SpxGapPct => percent(self.spx_open, self.spx_prior_close),
            Input::MinutesFromOpen => self.minutes_from_open,
            Input::MinutesToClose => self.minutes_to_close,
            Input::UnderlyingPrice => self.underlying,
            Input::Spy => self.spy,
            Input::Qqq => self.qqq,
            Input::Es => self.es,
            Input::DeltaTotal => self.delta_total(|_| true),
            Input::DeltaTotalSymbol => self.delta_total(|open| open.symbol == self.symbol),
            Input::DeltaAccount => self.delta_total(|open| open.account == self.account),
            Input::DeltaAccountSymbol => self.delta_total(|open| open.account == self.account && open.symbol == self.symbol),
            trade_input => self.trade.as_ref()?.value(trade_input, self.underlying),
        }
    }
}

/// The minutes since the open (rounded down) and to the close (rounded up) at `now` (New York time); the
/// session is 09:30 to 16:00, or to 13:00 on an early close.
#[must_use]
pub fn session_minutes(now: NaiveDateTime, early_close: bool) -> (i64, i64) {
    let seconds_of = |time: NaiveTime| i64::from(time.num_seconds_from_midnight());
    let (open, close) =
        (NaiveTime::from_hms_opt(9, 30, 0).map_or(0, seconds_of), NaiveTime::from_hms_opt(if early_close { 13 } else { 16 }, 0, 0).map_or(0, seconds_of));
    let now = seconds_of(now.time());
    ((now - open).div_euclid(60), (close - now + 59).div_euclid(60))
}

/// What the check of a condition says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The condition is true: go on.
    Pass,
    /// The condition is false.
    Fail,
    /// It cannot be said now (a value is not known): ask again later.
    Retry,
}

/// The verdict and what was looked at, as English log lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    /// The verdict.
    pub verdict: Verdict,
    /// One line per rule that was looked at, and one for the verdict.
    pub lines: Vec<String>,
}

/// The text of a value for the log.
fn show(value: Decimal) -> String {
    value.round_dp(4).normalize().to_string()
}

/// Checks `condition` against `snapshot`. The groups are looked at by number; the first group whose rules
/// are all true makes the condition true. A rule that is false ends its group; a rule whose value is not
/// known ends the whole check with `Retry`. If no group is true the check says `Retry` for a condition that
/// retries until expiration, else `Fail`. A condition without rules is false.
#[must_use]
pub fn evaluate(condition: &Condition, snapshot: &Snapshot) -> Evaluation {
    let mut lines = Vec::new();
    let mut groups: Vec<u32> = condition.rules.iter().map(|rule| rule.group).collect();
    groups.sort_unstable();
    groups.dedup();
    if groups.is_empty() {
        lines.push(format!("Condition {}: it has no rules", condition.name));
        return Evaluation { verdict: Verdict::Fail, lines };
    }
    for group in groups {
        let mut group_true = true;
        for rule in condition.rules.iter().filter(|rule| rule.group == group) {
            let Some(left) = snapshot.value_with(rule.input, rule.input_length) else {
                lines.push(format!("Condition {}: {} is not known yet", condition.name, rule.input.text(rule.input_length)));
                return Evaluation { verdict: Verdict::Retry, lines };
            };
            let (right, right_name) = match rule.comparison {
                Comparison::Value(value) => (Some(value), String::new()),
                Comparison::Input(input) => (snapshot.value_with(input, rule.comparison_length), format!("{} ", input.text(rule.comparison_length))),
            };
            let Some(right) = right else {
                lines.push(format!("Condition {}: {} is not known yet", condition.name, right_name.trim()));
                return Evaluation { verdict: Verdict::Retry, lines };
            };
            let holds = rule.operator.holds(left, right);
            lines.push(format!(
                "Condition {}: group {group}: {} {} {} {right_name}{} -> {holds}",
                condition.name,
                rule.input.text(rule.input_length),
                show(left),
                rule.operator.sign(),
                show(right)
            ));
            if !holds {
                group_true = false;
                break;
            }
        }
        if group_true {
            lines.push(format!("Condition {} is true (group {group})", condition.name));
            return Evaluation { verdict: Verdict::Pass, lines };
        }
    }
    let verdict = if condition.retry_until_expiration { Verdict::Retry } else { Verdict::Fail };
    lines.push(format!("Condition {} is false{}", condition.name, if verdict == Verdict::Retry { ", checking again until the window ends" } else { "" }));
    Evaluation { verdict, lines }
}

/// The rules of a condition in one line for the list, e.g. `VIX < 20 AND SPX > SPXOpen | OR | VIX9D/VIX <= 1.05`.
#[must_use]
pub fn condition_summary(condition: &Condition) -> String {
    let mut groups: Vec<u32> = condition.rules.iter().map(|rule| rule.group).collect();
    groups.sort_unstable();
    groups.dedup();
    let text = |rule: &Rule| {
        let right = match rule.comparison {
            Comparison::Value(value) => show(value),
            Comparison::Input(input) => input.text(rule.comparison_length),
        };
        format!("{} {} {right}", rule.input.text(rule.input_length), rule.operator.sign())
    };
    groups
        .iter()
        .map(|group| condition.rules.iter().filter(|rule| rule.group == *group).map(text).collect::<Vec<_>>().join(" AND "))
        .collect::<Vec<_>>()
        .join("  OR  ")
}

/// The id a new condition gets: one above the highest in `list`.
#[must_use]
pub fn next_id(list: &[Condition]) -> u32 {
    list.iter().map(|condition| condition.id).max().unwrap_or(0).saturating_add(1)
}

/// Stores `condition`: a known id replaces that condition, id 0 (or an unknown one) is added with a new
/// id. Returns the id it has now.
pub fn upsert(list: &mut Vec<Condition>, mut condition: Condition) -> u32 {
    if let Some(slot) = list.iter_mut().find(|c| c.id == condition.id && condition.id != 0) {
        *slot = condition;
        return slot.id;
    }
    condition.id = next_id(list);
    let id = condition.id;
    list.push(condition);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            vix: Some(d("18")),
            vix_open: Some(d("17")),
            vix_prior_close: Some(d("16")),
            vix9d: Some(d("19.8")),
            spx: Some(d("6600")),
            spx_open: Some(d("6650")),
            spx_prior_close: Some(d("6500")),
            minutes_from_open: Some(d("5")),
            minutes_to_close: Some(d("385")),
            ..Snapshot::default()
        }
    }

    fn rule(group: u32, input: Input, operator: Operator, comparison: Comparison) -> Rule {
        Rule { group, input, operator, comparison, input_length: 0, comparison_length: 0 }
    }

    #[test]
    fn rules_on_bars_use_their_length_and_are_unknown_without_bars() {
        let bar = |close: &str| Bar { date: String::new(), high: d(close), low: d(close), close: d(close) };
        // Closed closes 1..=6 and a forming bar.
        let minutes: Vec<Bar> = ["1", "2", "3", "4", "5", "6", "100"].iter().enumerate().map(|(i, c)| Bar { date: format!("{i}"), ..bar(c) }).collect();
        let mut with_bars = Snapshot { symbol: "SPX".into(), ..snapshot() };
        with_bars.bars.insert("min:SPX".into(), minutes);
        assert_eq!(with_bars.value_with(Input::Ema, 3), Some(d("5")), "EMA(3) of 1..6");
        assert_eq!(with_bars.value_with(Input::Ema, 7), None, "not enough closed bars");
        assert_eq!(with_bars.value_with(Input::Ema5, 0), with_bars.value_with(Input::Ema, 5), "EMA5 is EMA with 5 bars");
        assert_eq!(with_bars.value_with(Input::Ema750, 0), None);
        assert_eq!(snapshot().value_with(Input::Ema, 3), None, "no bars");
        assert_eq!(with_bars.value_with(Input::SmaDaily, 3), None, "no daily bars");
        // A rule with a length reads it, and the log line names it.
        let rule = Rule { input_length: 3, ..rule(1, Input::Ema, Operator::Greater, Comparison::Value(d("4"))) };
        let condition = Condition { id: 1, name: "t".into(), retry_until_expiration: false, rules: vec![rule] };
        let evaluation = evaluate(&condition, &with_bars);
        assert_eq!(evaluation.verdict, Verdict::Pass);
        assert!(evaluation.lines.iter().any(|l| l.contains("EMA(3) 5 > 4")), "{:?}", evaluation.lines);
        assert_eq!(condition_summary(&condition), "EMA(3) > 4");
        assert_eq!(evaluate(&condition, &snapshot()).verdict, Verdict::Retry);
        assert!(Input::ALL.iter().all(|input| input.takes_length() == matches!(input, Input::Ema | Input::SmaDaily | Input::VixRank | Input::VixPercentile)));
    }

    #[test]
    fn delta_totals_prices_and_the_new_trade_values_are_read_from_the_snapshot() {
        let open = |account: &str, symbol: &str, total: &str| OpenDelta { account: account.into(), symbol: symbol.into(), total: d(total) };
        let market = Snapshot {
            symbol: "SPX".into(),
            account: "DU1".into(),
            underlying: Some(d("6705")),
            spy: Some(d("670.5")),
            open_deltas: vec![open("DU1", "SPX", "30"), open("DU1", "XSP", "5"), open("DU2", "SPX", "-12"), open("DU2", "XSP", "-3")],
            ..snapshot()
        };
        assert_eq!(market.value(Input::DeltaTotal), Some(d("20")));
        assert_eq!(market.value(Input::DeltaTotalSymbol), Some(d("18")), "SPX: 30 - 12");
        assert_eq!(market.value(Input::DeltaAccount), Some(d("35")), "DU1: 30 + 5");
        assert_eq!(market.value(Input::DeltaAccountSymbol), Some(d("30")));
        assert_eq!(
            (market.value(Input::UnderlyingPrice), market.value(Input::Spy), market.value(Input::Qqq), market.value(Input::Es)),
            (Some(d("6705")), Some(d("670.5")), None, None)
        );
        // No open trade: 0; trades that add up to 0: not known (a sum of 0 cannot be told from a missing delta).
        assert_eq!(snapshot().value(Input::DeltaTotal), Some(Decimal::ZERO));
        let flat = Snapshot { open_deltas: vec![open("DU1", "SPX", "10"), open("DU1", "SPX", "-10")], ..snapshot() };
        assert_eq!(flat.value(Input::DeltaTotal), None);
        // The trade values.
        let leg = |strike: &str, ask: &str| Some(LegValues { strike: d(strike), ask: Some(d(ask)), delta: Some(d("20")) });
        let trade = TradeValues {
            short_put: leg("6690", "1.5"),
            long_put: leg("6680", "0.5"),
            position_delta: Some(d("12")),
            total_delta: Some(d("24")),
            ratio_open: Some(d("2")),
            custom_deltas: [Some(d("22")), None, None, Some(d("5"))],
            ..TradeValues::default()
        };
        let with_trade = Snapshot { trade: Some(trade), ..snapshot() };
        assert_eq!((with_trade.value(Input::TradePositionDelta), with_trade.value(Input::TradePositionTotalDelta)), (Some(d("12")), Some(d("24"))));
        assert_eq!(with_trade.value(Input::TradeShortLongRatioChangePct), Some(d("50")), "ratio 3 against 2 at the entry");
        assert_eq!(
            (with_trade.value(Input::TradeCustomLeg1Delta), with_trade.value(Input::TradeCustomLeg2Delta), with_trade.value(Input::TradeCustomLeg4Delta)),
            (Some(d("22")), None, Some(d("5")))
        );
        assert_eq!(snapshot().value(Input::TradeShortLongRatioChangePct), None);
    }

    #[test]
    fn trade_rules_read_the_open_trade_and_are_unknown_without_one() {
        let leg = |strike: &str, ask: &str, delta: &str| Some(LegValues { strike: d(strike), ask: Some(d(ask)), delta: Some(d(delta)) });
        let trade = TradeValues {
            profit: Some(d("40")),
            profit_pct: Some(d("25")),
            ask: Some(d("0.9")),
            bid: Some(d("0.4")),
            short_put: leg("6690", "1.5", "18"),
            long_put: leg("6680", "0.5", "12"),
            short_call: None,
            long_call: None,
            ..TradeValues::default()
        };
        let market = Snapshot { underlying: Some(d("6700")), trade: Some(trade), ..snapshot() };
        assert_eq!(market.value(Input::TradeProfitAmt), Some(d("40")));
        assert_eq!(market.value(Input::TradeShortPutDelta), Some(d("18")));
        assert_eq!(market.value(Input::TradeShortPutOtmPts), Some(d("10")), "6700 - 6690");
        assert_eq!(market.value(Input::TradeLongPutOtmPts), Some(d("20")));
        assert_eq!(market.value(Input::TradeShortPutOtmPct).map(|v| v.round_dp(4)), Some(d("0.1493")), "10 / 6700 * 100");
        assert_eq!(market.value(Input::TradeShortLongRatio), Some(d("3")), "1.5 / 0.5");
        assert_eq!(market.value(Input::TradeShortCallDelta), None, "the trade has no short call");
        assert_eq!(snapshot().value(Input::TradeAskPrice), None, "no trade, no value");
        // A rule on a trade value is "not known yet" without a trade: the check says Retry.
        let condition = Condition {
            id: 1,
            name: "t".into(),
            retry_until_expiration: false,
            rules: vec![rule(1, Input::TradeProfitPct, Operator::Greater, Comparison::Value(d("10")))],
        };
        assert_eq!(evaluate(&condition, &snapshot()).verdict, Verdict::Retry);
        assert_eq!(evaluate(&condition, &market).verdict, Verdict::Pass);
        // The file has its own keys; the screens show the names.
        let json = serde_json::to_string(&Input::TradeShortLongRatio).unwrap();
        assert_eq!(json, "\"trade_short_long_ratio\"");
        let keys: Vec<String> = Input::ALL.iter().map(|input| serde_json::to_string(input).unwrap()).collect();
        assert!(keys.iter().all(|key| key.chars().all(|c| c == '"' || c == '_' || c.is_ascii_lowercase() || c.is_ascii_digit())), "{keys:?}");
        assert_eq!(keys.len(), keys.iter().collect::<std::collections::HashSet<_>>().len());
    }

    fn condition(retry: bool, rules: Vec<Rule>) -> Condition {
        Condition { id: 1, name: "Calm".into(), retry_until_expiration: retry, rules }
    }

    #[test]
    fn the_derived_values_follow_the_formulas_and_are_unknown_without_their_parts() {
        let s = snapshot();
        assert_eq!(s.value(Input::VixIntradayPts), Some(d("1")));
        assert_eq!(s.value(Input::VixChangePts), Some(d("2")));
        assert_eq!(s.value(Input::VixChangePct), Some(d("12.5")));
        assert_eq!(s.value(Input::VixGapPts), Some(d("1")));
        assert_eq!(s.value(Input::Vix9dOverVix), Some(d("1.1")));
        assert_eq!(s.value(Input::SpxGapPts), Some(d("150")));
        assert_eq!(s.value(Input::SpxChangePts), Some(d("100")));
        assert_eq!(s.value(Input::SpxIntradayPts), Some(d("-50")));
        assert_eq!(s.value(Input::SpxChangePct).map(|v| v.round_dp(4)), Some(d("1.5385")));
        let none = Snapshot::default();
        for input in Input::ALL {
            // Without open trades the delta totals are 0, everything else is not known.
            let expected =
                matches!(input, Input::DeltaTotal | Input::DeltaTotalSymbol | Input::DeltaAccount | Input::DeltaAccountSymbol).then_some(Decimal::ZERO);
            assert_eq!(none.value(input), expected, "{}", input.name());
        }
        let zero_base = Snapshot { vix: Some(d("18")), vix_open: Some(Decimal::ZERO), vix9d: Some(d("1")), ..Snapshot::default() };
        assert_eq!(zero_base.value(Input::VixIntradayPct), None, "no division by zero");
        assert_eq!(Snapshot { vix9d: Some(d("1")), vix: Some(Decimal::ZERO), ..Snapshot::default() }.value(Input::Vix9dOverVix), None);
    }

    #[test]
    fn the_operators_compare_as_their_signs_say() {
        let (one, two) = (d("1"), d("2"));
        for (operator, expected) in [
            (Operator::Greater, [false, false, true]),
            (Operator::GreaterOrEqual, [false, true, true]),
            (Operator::Less, [true, false, false]),
            (Operator::LessOrEqual, [true, true, false]),
            (Operator::Equal, [false, true, false]),
        ] {
            assert_eq!([operator.holds(one, two), operator.holds(two, two), operator.holds(two, one)], expected, "{}", operator.sign());
        }
    }

    #[test]
    fn rules_of_a_group_are_anded_and_groups_are_ored() {
        let vix_low = rule(1, Input::Vix, Operator::Less, Comparison::Value(d("20")));
        let spx_up = rule(1, Input::SpxChangePts, Operator::Greater, Comparison::Value(d("0")));
        assert_eq!(evaluate(&condition(false, vec![vix_low, spx_up]), &snapshot()).verdict, Verdict::Pass);
        // One false rule makes the group false; a second group can still save the condition.
        let vix_high = rule(1, Input::Vix, Operator::Greater, Comparison::Value(d("20")));
        assert_eq!(evaluate(&condition(false, vec![vix_high, spx_up]), &snapshot()).verdict, Verdict::Fail);
        let second_group = rule(2, Input::SpxChangePts, Operator::Greater, Comparison::Value(d("0")));
        let result = evaluate(&condition(false, vec![vix_high, spx_up, second_group]), &snapshot());
        assert_eq!(result.verdict, Verdict::Pass);
        assert!(result.lines.last().is_some_and(|l| l.contains("true (group 2)")), "{:?}", result.lines);
        // A rule can compare with another value of the market.
        let against_input = rule(1, Input::Vix, Operator::Greater, Comparison::Input(Input::VixPriorClose));
        assert_eq!(evaluate(&condition(false, vec![against_input]), &snapshot()).verdict, Verdict::Pass);
    }

    #[test]
    fn an_unknown_value_says_retry_and_a_false_condition_retries_only_if_asked() {
        let needs_vix = rule(1, Input::Vix, Operator::Less, Comparison::Value(d("20")));
        let no_vix = Snapshot { vix: None, ..snapshot() };
        let result = evaluate(&condition(false, vec![needs_vix]), &no_vix);
        assert_eq!(result.verdict, Verdict::Retry);
        assert!(result.lines.iter().any(|l| l.contains("VIX is not known yet")));
        let against_unknown = rule(1, Input::Spx, Operator::Greater, Comparison::Input(Input::VixOpen));
        assert_eq!(evaluate(&condition(false, vec![against_unknown]), &Snapshot { vix_open: None, ..snapshot() }).verdict, Verdict::Retry);
        // A false rule stops its group before a later unknown one is looked at.
        let false_first = rule(1, Input::Vix, Operator::Greater, Comparison::Value(d("50")));
        assert_eq!(evaluate(&condition(false, vec![false_first, needs_vix]), &no_vix).verdict, Verdict::Retry, "the first rule needs the VIX itself");
        let false_then_unknown = rule(1, Input::Spx, Operator::Greater, Comparison::Value(d("9999")));
        assert_eq!(evaluate(&condition(false, vec![false_then_unknown, needs_vix]), &no_vix).verdict, Verdict::Fail);
        // Nothing is true: give up, or keep trying for a condition that asks for it.
        assert_eq!(evaluate(&condition(false, vec![false_first]), &snapshot()).verdict, Verdict::Fail);
        assert_eq!(evaluate(&condition(true, vec![false_first]), &snapshot()).verdict, Verdict::Retry);
        // No rules: false, also for the retrying one.
        assert_eq!(evaluate(&condition(true, vec![]), &snapshot()).verdict, Verdict::Fail);
    }

    #[test]
    fn the_session_minutes_round_the_way_the_reference_does() {
        let at = |h, m, s| NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(h, m, s).unwrap();
        assert_eq!(session_minutes(at(9, 30, 0), false), (0, 390));
        assert_eq!(session_minutes(at(9, 35, 20), false), (5, 385), "from the open rounds down, to the close rounds up");
        assert_eq!(session_minutes(at(9, 0, 0), false), (-30, 420));
        assert_eq!(session_minutes(at(9, 29, 30), false), (-1, 391));
        assert_eq!(session_minutes(at(16, 0, 0), false), (390, 0));
        assert_eq!(session_minutes(at(16, 10, 0), false), (400, -10));
        assert_eq!(session_minutes(at(12, 0, 0), true), (150, 60), "an early close ends at 13:00");
    }

    #[test]
    fn a_condition_is_summed_up_in_one_line() {
        let stored = condition(
            false,
            vec![
                rule(1, Input::Vix, Operator::Less, Comparison::Value(d("20"))),
                rule(1, Input::Spx, Operator::Greater, Comparison::Input(Input::SpxOpen)),
                rule(2, Input::Vix9dOverVix, Operator::LessOrEqual, Comparison::Value(d("1.05"))),
            ],
        );
        assert_eq!(condition_summary(&stored), "VIX < 20 AND SPX > SPXOpen  OR  VIX9D/VIX <= 1.05");
        assert_eq!(condition_summary(&condition(false, vec![])), "");
    }

    #[test]
    fn conditions_are_stored_with_their_own_keys_and_ids_do_not_repeat() {
        let stored = condition(
            true,
            vec![
                rule(1, Input::Vix9dOverVix, Operator::LessOrEqual, Comparison::Value(d("1.05"))),
                rule(2, Input::Spx, Operator::Greater, Comparison::Input(Input::SpxOpen)),
            ],
        );
        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("\"vix9d_over_vix\"") && json.contains("\"<=\"") && json.contains("\"spx_open\""), "{json}");
        assert_eq!(serde_json::from_str::<Condition>(&json).unwrap(), stored);
        assert!(
            serde_json::from_str::<Condition>(r#"{"rules":[{"group":1,"input":"NoSuchValue","operator":">","comparison":{"Value":"1"}}]}"#).is_err(),
            "an unsupported value is refused, not guessed"
        );
        let mut list = Vec::new();
        assert_eq!(upsert(&mut list, Condition { name: "A".into(), ..Condition::default() }), 1);
        assert_eq!(upsert(&mut list, Condition { name: "B".into(), ..Condition::default() }), 2);
        assert_eq!(upsert(&mut list, Condition { id: 1, name: "A2".into(), ..Condition::default() }), 1);
        assert_eq!(upsert(&mut list, Condition { id: 9, ..Condition::default() }), 3);
        assert_eq!(list.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["A2", "B", ""]);
    }
}
