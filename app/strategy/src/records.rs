//! The records of the app that its trade logic works with: a trade as it is kept, its exit orders and adjustments, an execution TWS
//! reported, and a symbol of the market data settings. Plain data with serde; reading and writing them is the job of the app.

use serde::{Deserialize, Serialize};

/// One row of Settings > Market Data > "Configure Symbols" - a user-maintained list (this
/// app never invents a row; it starts empty until the user adds one via "Save Symbol").
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolConfig {
    /// The underlying, e.g. "SPX".
    pub symbol: String,
    /// The expirations to keep quotes for, as days to expiry separated by commas ("0,1,2"); empty = 0.
    pub required_dtes: String,
    /// How many points below the price puts are quoted; empty = the default of the symbol.
    pub put_otm: String,
    /// How many points above the price calls are quoted; empty = the default of the symbol.
    pub call_otm: String,
    /// How many points above the price puts are quoted; empty = the default of the symbol.
    pub put_itm: String,
    /// How many points below the price calls are quoted; empty = the default of the symbol.
    pub call_itm: String,
    /// Quote more strikes than the window asks for.
    pub extended_quote_strikes: bool,
}

/// One trade this app opened (the fields stored so far). Prices are `Decimal` (`coding_guideline.md`
/// section 3) and stored as decimal text, so nothing is rounded on the way to disk and the files of
/// earlier versions read unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredTrade {
    /// Day the trade was opened, YYYY-MM-DD (local), used to pick "today's trades".
    pub day: String,
    /// Opening time, DD-MM-YYYY HH:MM:SS (local), for display.
    pub opened: String,
    /// The TWS account the trade was entered on.
    pub account: String,
    /// The underlying, e.g. "SPX".
    pub symbol: String,
    /// The trade type as the templates name it, e.g. "`PutSpread`".
    pub trade_type: String,
    /// The strikes as text, e.g. "7235/7225 P".
    pub strikes: String,
    /// TWS order id of the opening order.
    pub order_id: i32,
    /// Contracts the trade was entered with (more than 0).
    pub qty: u32,
    /// Contract multiplier (100; ES 50).
    pub multiplier: u32,
    /// `PriceOpen` per contract: positive = credit, negative = debit.
    pub price_open: rust_decimal::Decimal,
    /// `Trade.Delta` at entry (a greek, not money).
    pub delta: f64,
    /// The options the trade holds (missing in files written before V0.97).
    #[serde(default)]
    pub short_put: Option<StoredLeg>,
    /// The bought put, if the trade has one.
    #[serde(default)]
    pub long_put: Option<StoredLeg>,
    /// The sold call, if the trade has one.
    #[serde(default)]
    pub short_call: Option<StoredLeg>,
    /// The bought call, if the trade has one.
    #[serde(default)]
    pub long_call: Option<StoredLeg>,
    /// How many puts of each side make one contract of the trade (1 unless the trade is a ratio).
    #[serde(default = "one")]
    pub put_ratio: u32,
    /// How many calls of each side make one contract of the trade (1 unless the trade is a ratio).
    #[serde(default = "one")]
    pub call_ratio: u32,
    /// Expiration yyyyMMdd.
    #[serde(default)]
    pub expiry: String,
    /// "Open" or "Expired" (`Trade.Status` 1 / 4).
    #[serde(default = "open_status")]
    pub status: String,
    /// `PriceClose` per contract once the trade is finished.
    #[serde(default)]
    pub price_close: Option<rust_decimal::Decimal>,
    /// TWS order id of the stop order this app placed after the fill (`OrderIDStop`).
    #[serde(default)]
    pub stop_order_id: Option<i32>,
    /// TWS order id of the profit-target order (`order_id_profit_target`).
    #[serde(default)]
    pub profit_order_id: Option<i32>,
    /// TWS order id of the stop-limit order (`OrderIDStopLimit`): the only stop of a `Limit stop`
    /// template, the second one of `Limit stop with OCO`.
    #[serde(default)]
    pub stop_limit_order_id: Option<i32>,
    /// The profit-target levels of a trade whose target is split into several (`ProfitTargetQty`
    /// .. `ProfitTarget4Qty`); `None` for a trade with one target for everything.
    #[serde(default)]
    pub exit_levels: Option<ExitLevels>,
    /// The stop adjustments of the template and the stop as it is now (`StopAdjustmentLevel`,
    /// `PriceStopTarget`, ...); `None` for a trade whose stop is never adjusted.
    #[serde(default)]
    pub adjustments: Option<StopAdjustments>,
    /// The OCA group the trade's exit orders share, as it was named when the first of them was
    /// placed. Kept because it cannot be derived from the order ids once a stop has been replaced
    /// or a profit-target level has started a group of its own. `None` for a trade stored by an
    /// earlier version (then `trade_exit::trade_oca_group` falls back to the order ids).
    #[serde(default)]
    pub oca_group: Option<String>,
    /// TWS order id of a close order sent by hand (`OrderIDClose`).
    #[serde(default)]
    pub close_order_id: Option<i32>,
    /// The stop price per contract, as placed.
    #[serde(default)]
    pub stop_price: Option<rust_decimal::Decimal>,
    /// The profit-target price per contract, as placed.
    #[serde(default)]
    pub profit_price: Option<rust_decimal::Decimal>,
    /// A trade rebuilt from the expiry bookings alone: its entry price is not known (`price_open`
    /// is then "0" and only a placeholder), so it is listed but never counted in any figure.
    #[serde(default)]
    pub entry_unknown: bool,
    /// The strategy of the template the trade was entered with (empty: none, or a trade from before it was stored).
    #[serde(default)]
    pub strategy: String,
    /// The timed exit of the template (`exit_hour`/`exit_minute` or `exit_minutes_in_trade`), worked out when the trade was filled;
    /// `None` for a trade without one, or once it has been carried out or has run out.
    #[serde(default)]
    pub timed_exit: Option<TimedExit>,
    /// When the profit target is cancelled (New York date and time, yyyy-MM-dd HH:mm): `profit_target_expiration_hour`/`Minute` of the
    /// template on the expiry day. `None` without one, or once it has been carried out.
    #[serde(default)]
    pub profit_cancel: Option<String>,
    /// Where the trade comes from, for re-entries: the template it was entered with and its place in a chain of re-entries.
    #[serde(default)]
    pub origin: Option<TradeOrigin>,
    /// The legs of a custom trade (`trade_type` "Custom"): any mix of bought and sold options; `None` for every other type.
    #[serde(default)]
    pub custom_legs: Option<Vec<CustomLegStored>>,
}

/// One leg of a custom trade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomLegStored {
    /// TWS contract id.
    pub con_id: i32,
    /// "P" or "C".
    pub right: String,
    /// The strike as text.
    pub strike: String,
    /// Expiration, yyyyMMdd.
    pub expiry: String,
    /// The ratio with its sign: a sold leg is negative.
    pub ratio: i32,
}

/// The template a trade was entered with and its place in a chain of re-entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeOrigin {
    /// `id` of the template.
    pub template_id: i32,
    /// The id of the order of the first entry of the chain (the order of this trade itself for a first entry).
    pub root: i32,
    /// How many re-entries came before this trade (0 for a first entry).
    pub entries: u32,
    /// How many minutes a re-entry made after this trade may take to get its entry.
    pub window_minutes: u32,
    /// The trigger that started the entry (`Trigger::base.id`); 0 for none.
    #[serde(default)]
    pub trigger: u32,
    /// The short/long ratio at the entry (the asks of the legs), as text; for `Trade_ShortLongRatioChangePct`.
    #[serde(default)]
    pub ratio_open: Option<String>,
}

/// When a trade is closed by its template: the New York date and time (yyyy-MM-dd HH:mm) and whether a limit order is
/// tried before the market order (`exit_order_limit`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedExit {
    /// When the trade is closed (New York date and time, yyyy-MM-dd HH:mm).
    pub at: String,
    /// Close with a limit order first (else a market order).
    pub limit: bool,
}

/// One stop adjustment of the template (`adjustment1_type`, `adjustment1`, ...): when the trade
/// reaches the trigger, the stop moves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StopAdjustment {
    /// The trigger kind: "Profit percent" (percent), "Amount", "Profit", "`Target 1`" ..
    /// "`profit_target3`".
    pub kind: String,
    /// The trigger value (not used by the `profit_target` kinds).
    pub value: Option<f64>,
    /// The order type the stop becomes: "Same", "`Market stop`", "`Limit stop`", "`Limit stop with OCO`",
    /// "Trailing" or "None" (no stop any more).
    pub order_type: String,
    /// How the new stop is worked out: "Stop factor" or "Fixed price".
    pub change_type: String,
    /// The multiple or the amount of the new stop.
    pub change: Option<f64>,
    /// Added to the new stop; the trailing amount of a Trailing stop; the limit offset of a
    /// `Limit stop`.
    pub change_offset: Option<f64>,
    /// The adjustment only applies from this time on the day of the expiration (24 h, New York
    /// time); hour 0 = no time condition.
    pub hour: i32,
    /// See `hour`.
    pub minute: i32,
}

/// The settings of a REL stop (a relative order that follows the market at an offset) with its
/// protections, as the template gave them, plus what the order conditions need from the time of
/// the fill.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RelSettings {
    /// The most the REL order may pay (`stop_rel_limit`); 8 without one.
    pub limit: Option<f64>,
    /// The distance the REL order follows the market at (`stop_rel_offset`); 0.5 without one.
    pub offset: Option<f64>,
    /// ITM protection: the stop only becomes active when the underlying is this far in the money
    /// (`stop_rel_itm`), and in the last `itm_minutes` before the close.
    pub itm: Option<i32>,
    /// The minutes before the close the ITM protection stands for (`stop_rel_itm_minutes`, 30 without).
    pub itm_minutes: Option<i32>,
    /// The stop only works in the last `itm_minutes` (`stop_rel_itm_only`).
    pub itm_only: bool,
    /// Volatility spike protection (`stop_guardrail`): the stop only works along a price path that
    /// closes in on the short strike towards the close.
    pub guardrail: bool,
    /// The guardrail's distance from the strike at the open (`stop_guardrail_offset_open`).
    pub offset_open: i32,
    /// The guardrail's distance from the strike at the close (`stop_guardrail_offset_exp`).
    pub offset_exp: i32,
    /// The underlying's price when the trade was filled, as text.
    pub underlying_open: Option<String>,
    /// Minutes from the fill to the close of the day of the expiration, 0 when it expires later.
    pub minutes_to_close: i32,
    /// The day of the expiration closes early (13:00 New York time).
    pub early_close: bool,
}

/// The stop adjustments of a trade and the stop as it stands (`StopAdjustmentLevel` and the stop
/// settings the adjustments change).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StopAdjustments {
    /// The adjustments of the template that are set (at most 3), in order.
    pub items: Vec<StopAdjustment>,
    /// How many adjustments were applied so far: the next one to check is `items[level]`.
    pub level: u8,
    /// The stop's order type now: "`Market stop`", "`Limit stop`", "`Limit stop with OCO`", "Trailing" or "None".
    pub order_type: String,
    /// The stop price now (`PriceStopTarget`), as text.
    pub stop: Option<String>,
    /// The trailing amount of a Trailing stop (`stop_offset`).
    pub trailing_offset: Option<String>,
    /// The offset of the limit of a `Limit stop` / `Limit stop with OCO` (`stop_limit_offset`).
    pub limit_offset: Option<f64>,
    /// The offset of the market backstop of a `Limit stop with OCO` (`stop_limit_market_offset`).
    pub market_offset: Option<f64>,
    /// The legs the stop closes: TWS contract id, ratio, and whether the leg is sold.
    pub legs: Vec<(i32, i32, bool)>,
    /// The stop's trigger method (`stop_trigger`).
    pub trigger_method: i32,
    /// The stop may trigger outside regular trading hours.
    pub outside_rth: bool,
    /// The old stop was cancelled for an adjustment and its replacement has not been placed yet.
    pub replace_pending: bool,
    /// The price the stop is worked out from, as text: an adjustment that
    /// sets the stop by a multiple starts from it.
    #[serde(default)]
    pub basis: Option<String>,
    /// The REL settings of the stop, so that an adjusted stop can be built the same way.
    #[serde(default)]
    pub rel: Option<RelSettings>,
}

/// The profit-target levels of a trade (`Trade.ProfitTargetLevel` and the quantities and orders of
/// the four levels). The levels work one after the other: the order of the current level is
/// live, and when it fills the next one is placed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitLevels {
    /// Contracts per level (`ProfitTargetQty`, `ProfitTarget2Qty`, ...): they add up to the
    /// trade's quantity.
    pub qty: [u32; 4],
    /// The level whose order is working: 0-3, or 4 once every level was hit.
    pub level: u8,
    /// The TWS order id of each level's order once it was placed (level 1's is also the trade's
    /// `profit_order_id`).
    pub order_ids: [Option<i32>; 4],
    /// The levels hit so far: contracts and the price per contract paid to close (as text,
    /// negative for a credit trade), in the order they were hit.
    pub closed: Vec<(u32, String)>,
    /// The limit price of each level as text, worked out when the trade was filled.
    pub prices: [Option<String>; 4],
    /// The legs the levels close: TWS contract id, ratio, and whether the leg is sold.
    pub legs: Vec<(i32, i32, bool)>,
    /// The orders may fill outside regular trading hours.
    pub outside_rth: bool,
}

impl ExitLevels {
    /// Contracts closed by the levels hit so far.
    pub fn closed_qty(&self) -> u32 {
        self.closed.iter().map(|(qty, _)| *qty).fold(0, u32::saturating_add)
    }
}

impl StoredTrade {
    /// Every TWS order the app placed for this trade after the fill: stop, stop-limit, profit
    /// targets (every level) and a close order, each id once.
    #[must_use]
    pub fn exit_order_ids(&self) -> Vec<i32> {
        let mut ids: Vec<i32> = [self.stop_order_id, self.stop_limit_order_id, self.profit_order_id, self.close_order_id].into_iter().flatten().collect();
        for id in self.exit_levels.iter().flat_map(|levels| levels.order_ids.iter().flatten()) {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        ids
    }

    /// The stop as it stands: the adjusted one while the stop adjustments have moved it, else the one placed; `None`
    /// without a stop.
    #[must_use]
    pub fn stop_now(&self) -> Option<rust_decimal::Decimal> {
        self.adjustments.as_ref().and_then(|a| a.stop.as_deref()).and_then(|stop| stop.parse().ok()).or(self.stop_price)
    }

    /// Where the trade stands; `None` for a status text that is no known status.
    #[must_use]
    pub fn state(&self) -> Option<crate::trade_summary::TradeStatus> {
        crate::trade_summary::TradeStatus::parse(&self.status)
    }

    /// The trade is still open (`TradeStatus::Open`).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.state() == Some(crate::trade_summary::TradeStatus::Open)
    }

    /// What tells this trade apart from every other one, also from another trade of the same opening order (the two sides of an
    /// `IronCondor`, the sides of a split trade): the opening order, the type and the strikes. For state kept per trade while the app
    /// runs.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.order_id, self.trade_type, self.strikes)
    }

    /// The trade was split into one trade per side (`TradeStatus::Split`): hidden, its sides count instead.
    #[must_use]
    pub fn is_split(&self) -> bool {
        self.state() == Some(crate::trade_summary::TradeStatus::Split)
    }

    /// Whether `other` is the same trade, in another state: the same day, opening order, type and strikes.
    #[must_use]
    pub fn is_same_trade(&self, other: &StoredTrade) -> bool {
        self.day == other.day && self.order_id == other.order_id && self.trade_type == other.trade_type && self.strikes == other.strikes
    }

    /// The trade as the log and the screens name it: `SPX PutSpread 7265/7255 P`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} {} {}", self.symbol, self.trade_type, self.strikes)
    }

    /// Contracts still open: the quantity minus what the profit-target levels hit so far closed.
    pub fn remaining_qty(&self) -> u32 {
        self.qty.saturating_sub(self.exit_levels.as_ref().map_or(0, ExitLevels::closed_qty))
    }

    /// The profit the levels hit so far made: `(entry price + closing price) * contracts *
    /// multiplier` per level. `Some(0)` without any; `None` on an unreadable price or overflow.
    #[must_use]
    pub fn realized_profit(&self, price_open: rust_decimal::Decimal) -> Option<rust_decimal::Decimal> {
        let Some(levels) = &self.exit_levels else { return Some(rust_decimal::Decimal::ZERO) };
        levels.closed.iter().try_fold(rust_decimal::Decimal::ZERO, |sum, (qty, price)| {
            let close: rust_decimal::Decimal = price.parse().ok()?;
            let part =
                price_open.checked_add(close)?.checked_mul(rust_decimal::Decimal::from(*qty))?.checked_mul(rust_decimal::Decimal::from(self.multiplier))?;
            sum.checked_add(part)
        })
    }
}

/// One option of a stored trade; the strike as decimal text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredLeg {
    /// TWS contract id of the option.
    pub con_id: i32,
    /// The strike as text.
    pub strike: String,
}

fn one() -> u32 {
    1
}

fn open_status() -> String {
    crate::trade_summary::TradeStatus::Open.as_str().to_string()
}

/// One execution TWS reported, kept on disk because TWS itself only holds the current trading
/// day. `order_ref` tells this app's own orders apart ("`TradePilot` `OrderID`: N (Opening)").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredExecution {
    /// The id TWS gives the execution; an execution is stored once.
    pub exec_id: String,
    /// As TWS sends it: "yyyyMMdd HH:mm:ss Zone".
    pub time: String,
    /// The TWS account of the fill.
    pub account: String,
    /// The underlying, e.g. "SPX".
    pub symbol: String,
    /// TWS contract id of the option (0 for a combo).
    pub con_id: i32,
    /// The id of the order that was filled.
    pub order_id: i32,
    /// The reference of the order (see `trade_rebuild::order_ref`); empty for an order of another origin.
    pub order_ref: String,
    /// The security type: "OPT" or "BAG".
    pub sec_type: String,
    /// yyyyMMdd.
    pub expiry: String,
    /// "P" or "C".
    pub right: String,
    /// The strike of the option.
    pub strike: f64,
    /// "BOT" or "SLD".
    pub side: String,
    /// Contracts filled.
    pub quantity: f64,
    /// The price per contract of the fill.
    pub price: f64,
}
