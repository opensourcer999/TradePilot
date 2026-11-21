//! The order, contract and condition types as the wire format has them.

use crate::param_bool;

/// `IBApi.ComboLeg` as a template entry fills it (`new ComboLeg()` + `ConId`,
/// Ratio, Action, Exchange); the remaining fields keep the parameterless constructor's
/// defaults (0 / null), which is what the API client sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IbComboLeg {
    /// `IBApi.ComboLeg.ConId`.
    pub con_id: i32,
    /// `IBApi.ComboLeg.Ratio`.
    pub ratio: i32,
    /// "BUY" or "SELL".
    pub action: String,
    /// `IBApi.ComboLeg.Exchange`.
    pub exchange: String,
}

/// Mirrors `IBApi.Contract`'s fields relevant to a single option/stock/index leg, plus the
/// combo legs of a BAG contract.
#[derive(Clone)]
pub struct IbContract {
    /// `IBApi.Contract.ConId`.
    pub con_id: i32,
    /// `IBApi.Contract.Symbol`.
    pub symbol: String,
    /// `IBApi.Contract.SecType`.
    pub sec_type: String,
    /// `IBApi.Contract.LastTradeDateOrContractMonth`.
    pub last_trade_date_or_contract_month: String,
    /// `IBApi.Contract.Strike`.
    pub strike: f64,
    /// `IBApi.Contract.Right`.
    pub right: String,
    /// `IBApi.Contract.Multiplier`.
    pub multiplier: String,
    /// `IBApi.Contract.Exchange`.
    pub exchange: String,
    /// `IBApi.Contract.PrimaryExch`.
    pub primary_exch: String,
    /// `IBApi.Contract.Currency`.
    pub currency: String,
    /// `IBApi.Contract.LocalSymbol`.
    pub local_symbol: String,
    /// `IBApi.Contract.TradingClass`.
    pub trading_class: String,
    /// `Contract.ComboLegs` - only sent when `sec_type` is "BAG".
    pub combo_legs: Vec<IbComboLeg>,
}

impl Default for IbContract {
    fn default() -> Self {
        Self {
            con_id: 0,
            symbol: String::new(),
            sec_type: String::new(),
            last_trade_date_or_contract_month: String::new(),
            strike: f64::MAX, // real Contract() default
            right: String::new(),
            multiplier: String::new(),
            exchange: String::new(),
            primary_exch: String::new(),
            currency: String::new(),
            local_symbol: String::new(),
            trading_class: String::new(),
            combo_legs: Vec::new(),
        }
    }
}

/// One condition of an order (`IBApi.OrderCondition`): the order only becomes active while the
/// conditions hold. `and_next` is the connection to the condition that follows (`IsConjunctionConnection`):
/// true = "and", false = "or".
#[derive(Clone, Debug, PartialEq)]
pub enum IbCondition {
    /// The price of a contract (`PriceCondition`): `is_more` = at or above `price`, else at or below.
    /// `trigger_method` is TWS's (0 default, 4 bid/ask, 8 mid-point ...).
    Price {
        /// The connection to the next condition: true = and, false = or.
        and_next: bool,
        /// At or above `price`, else at or below it.
        is_more: bool,
        /// The price the contract is compared with.
        price: f64,
        /// The contract watched.
        con_id: i32,
        /// The exchange of the contract.
        exchange: String,
        /// TWS's trigger method (0 default, 4 bid/ask, 8 mid-point).
        trigger_method: i32,
    },
    /// A time (`TimeCondition`): `is_more` = at or after `time` ("yyyyMMdd HH:mm:ss Zone"), else before.
    Time {
        /// The connection to the next condition: true = and, false = or.
        and_next: bool,
        /// At or after `time`, else before it.
        is_more: bool,
        /// The time, "yyyyMMdd HH:mm:ss Zone".
        time: String,
    },
}

impl IbCondition {
    /// The condition type TWS knows it by (`OrderConditionType`).
    pub(crate) fn type_code(&self) -> i32 {
        match self {
            Self::Price { .. } => 1,
            Self::Time { .. } => 3,
        }
    }

    /// The fields `OrderCondition.Serialize` writes, in its order: the connection ("a"/"o"), the
    /// operator, the value, then what the kind adds (a contract: id and exchange, a price: the
    /// trigger method).
    pub(crate) fn fields(&self) -> Vec<String> {
        let connection = |and_next: bool| if and_next { "a" } else { "o" }.to_string();
        match self {
            Self::Price { and_next, is_more, price, con_id, exchange, trigger_method } => {
                vec![connection(*and_next), param_bool(*is_more), price.to_string(), con_id.to_string(), exchange.clone(), trigger_method.to_string()]
            }
            Self::Time { and_next, is_more, time } => vec![connection(*and_next), param_bool(*is_more), time.clone()],
        }
    }
}

/// How the order is handled: sent at once or held, what-if, outside regular hours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderHandling {
    /// The `transmit` switch.
    pub transmit: bool,
    /// The `what_if` switch.
    pub what_if: bool,
    /// The `outside_rth` switch.
    pub outside_rth: bool,
}

/// How the order may fill: all or none, block, sweep to fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderFill {
    /// The `all_or_none` switch.
    pub all_or_none: bool,
    /// The `block_order` switch.
    pub block_order: bool,
    /// The `sweep_to_fill` switch.
    pub sweep_to_fill: bool,
}

/// How the order is routed: hidden, no smart routing, not held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderRouting {
    /// The `hidden` switch.
    pub hidden: bool,
    /// The `opt_out_smart_routing` switch.
    pub opt_out_smart_routing: bool,
    /// The `not_held` switch.
    pub not_held: bool,
}

/// What the conditions of the order do: ignore regular hours, cancel the order, auto-cancel the parent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderConditionFlags {
    /// The `conditions_ignore_rth` switch.
    pub conditions_ignore_rth: bool,
    /// The `conditions_cancel_order` switch.
    pub conditions_cancel_order: bool,
    /// The `auto_cancel_parent` switch.
    pub auto_cancel_parent: bool,
}

/// The switches of scale orders and the percentage constraints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderScaleFlags {
    /// The `scale_auto_reset` switch.
    pub scale_auto_reset: bool,
    /// The `scale_random_percent` switch.
    pub scale_random_percent: bool,
    /// The `override_percentage_constraints` switch.
    pub override_percentage_constraints: bool,
}

/// Hedge, OMS container and discretionary price switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderHedgeFlags {
    /// The `dont_use_auto_price_for_hedge` switch.
    pub dont_use_auto_price_for_hedge: bool,
    /// The `is_oms_container` switch.
    pub is_oms_container: bool,
    /// The `discretionary_up_to_limit_price` switch.
    pub discretionary_up_to_limit_price: bool,
}

/// Switches about the customer and the session: professional customer, overnight, imbalance only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderCustomer {
    /// The `professional_customer` switch.
    pub professional_customer: bool,
    /// The `include_overnight` switch.
    pub include_overnight: bool,
    /// The `imbalance_only` switch.
    pub imbalance_only: bool,
}

impl Default for OrderHandling {
    /// The real `Order()` default sends the order at once; this app's ticket sets its own value.
    fn default() -> Self {
        Self { transmit: true, what_if: false, outside_rth: false }
    }
}

/// Mirrors every `IBApi.Order` field `placeOrder` writes to the wire, defaulted exactly as the
/// real `Order()` constructor does. Fields this app's Instant Order ticket does not expose
/// (algos, scale orders, combo/FA params, `MiFID` II attribution...) keep that real default.
#[derive(Clone)]
pub struct IbOrder {
    /// `IBApi.Order.Action`.
    pub action: String,
    /// `IBApi.Order.TotalQuantity`.
    pub total_quantity: f64,
    /// `IBApi.Order.OrderType`.
    pub order_type: String,
    /// `IBApi.Order.LmtPrice`.
    pub lmt_price: f64,
    /// `IBApi.Order.AuxPrice`.
    pub aux_price: f64,
    /// `IBApi.Order.Tif`.
    pub tif: String,
    /// `IBApi.Order.OcaGroup`.
    pub oca_group: String,
    /// `IBApi.Order.OcaType`.
    pub oca_type: i32,
    /// `IBApi.Order.Account`.
    pub account: String,
    /// `IBApi.Order.OpenClose`.
    pub open_close: String,
    /// `IBApi.Order.Origin`.
    pub origin: i32,
    /// `IBApi.Order.OrderRef`.
    pub order_ref: String,
    /// `IBApi.Order.ParentId`.
    pub parent_id: i32,
    /// `IBApi.Order.DisplaySize`.
    pub display_size: i32,
    /// `IBApi.Order.TriggerMethod`.
    pub trigger_method: i32,
    /// `IBApi.Order.DiscretionaryAmt`.
    pub discretionary_amt: f64,
    /// `IBApi.Order.GoodAfterTime`.
    pub good_after_time: String,
    /// `IBApi.Order.GoodTillDate`.
    pub good_till_date: String,
    /// `IBApi.Order.FaGroup`.
    pub fa_group: String,
    /// `IBApi.Order.FaMethod`.
    pub fa_method: String,
    /// `IBApi.Order.FaPercentage`.
    pub fa_percentage: String,
    /// `IBApi.Order.ModelCode`.
    pub model_code: String,
    /// `IBApi.Order.ShortSaleSlot`.
    pub short_sale_slot: i32,
    /// `IBApi.Order.DesignatedLocation`.
    pub designated_location: String,
    /// `IBApi.Order.ExemptCode`.
    pub exempt_code: i32,
    /// `IBApi.Order.Rule80A`.
    pub rule80a: String,
    /// `IBApi.Order.SettlingFirm`.
    pub settling_firm: String,
    /// `IBApi.Order.MinQty`.
    pub min_qty: i32,
    /// `IBApi.Order.PercentOffset`.
    pub percent_offset: f64,
    /// `IBApi.Order.AuctionStrategy`.
    pub auction_strategy: i32,
    /// `IBApi.Order.StartingPrice`.
    pub starting_price: f64,
    /// `IBApi.Order.StockRefPrice`.
    pub stock_ref_price: f64,
    /// `IBApi.Order.Delta`.
    pub delta: f64,
    /// `IBApi.Order.StockRangeLower`.
    pub stock_range_lower: f64,
    /// `IBApi.Order.StockRangeUpper`.
    pub stock_range_upper: f64,
    /// `IBApi.Order.Volatility`.
    pub volatility: f64,
    /// `IBApi.Order.VolatilityType`.
    pub volatility_type: i32,
    /// `IBApi.Order.DeltaNeutralOrderType`.
    pub delta_neutral_order_type: String,
    /// `IBApi.Order.DeltaNeutralAuxPrice`.
    pub delta_neutral_aux_price: f64,
    /// `IBApi.Order.ContinuousUpdate`.
    pub continuous_update: i32,
    /// `IBApi.Order.ReferencePriceType`.
    pub reference_price_type: i32,
    /// `IBApi.Order.TrailStopPrice`.
    pub trail_stop_price: f64,
    /// `IBApi.Order.TrailingPercent`.
    pub trailing_percent: f64,
    /// The conditions the order waits for (`Conditions`), empty for an ordinary order.
    pub conditions: Vec<IbCondition>,
    /// `IBApi.Order.ScaleInitLevelSize`.
    pub scale_init_level_size: i32,
    /// `IBApi.Order.ScaleSubsLevelSize`.
    pub scale_subs_level_size: i32,
    /// `IBApi.Order.ScalePriceIncrement`.
    pub scale_price_increment: f64,
    /// `IBApi.Order.ScalePriceAdjustValue`.
    pub scale_price_adjust_value: f64,
    /// `IBApi.Order.ScalePriceAdjustInterval`.
    pub scale_price_adjust_interval: i32,
    /// `IBApi.Order.ScaleProfitOffset`.
    pub scale_profit_offset: f64,
    /// `IBApi.Order.ScaleInitPosition`.
    pub scale_init_position: i32,
    /// `IBApi.Order.ScaleInitFillQty`.
    pub scale_init_fill_qty: i32,
    /// `IBApi.Order.ScaleTable`.
    pub scale_table: String,
    /// `IBApi.Order.ActiveStartTime`.
    pub active_start_time: String,
    /// `IBApi.Order.ActiveStopTime`.
    pub active_stop_time: String,
    /// `IBApi.Order.HedgeType`.
    pub hedge_type: String,
    /// `IBApi.Order.HedgeParam`.
    pub hedge_param: String,
    /// `IBApi.Order.ClearingAccount`.
    pub clearing_account: String,
    /// `IBApi.Order.ClearingIntent`.
    pub clearing_intent: String,
    /// `IBApi.Order.AlgoStrategy`.
    pub algo_strategy: String,
    /// `IBApi.Order.AlgoId`.
    pub algo_id: String,
    /// `IBApi.Order.ExtOperator`.
    pub ext_operator: String,
    /// `IBApi.Order.CashQty`.
    pub cash_qty: f64,
    /// `IBApi.Order.Mifid2DecisionMaker`.
    pub mifid2_decision_maker: String,
    /// `IBApi.Order.Mifid2DecisionAlgo`.
    pub mifid2_decision_algo: String,
    /// `IBApi.Order.Mifid2ExecutionTrader`.
    pub mifid2_execution_trader: String,
    /// `IBApi.Order.Mifid2ExecutionAlgo`.
    pub mifid2_execution_algo: String,
    /// `IBApi.Order.UsePriceMgmtAlgo`.
    pub use_price_mgmt_algo: Option<bool>,
    /// `IBApi.Order.Duration`.
    pub duration: i32,
    /// `IBApi.Order.PostToAts`.
    pub post_to_ats: i32,
    /// `IBApi.Order.AdvancedErrorOverride`.
    pub advanced_error_override: String,
    /// `IBApi.Order.ManualOrderTime`.
    pub manual_order_time: String,
    /// `IBApi.Order.MinTradeQty`.
    pub min_trade_qty: i32,
    /// `IBApi.Order.CustomerAccount`.
    pub customer_account: String,
    /// `IBApi.Order.ManualOrderIndicator`.
    pub manual_order_indicator: i32,
    /// `IBApi.Order.AdjustedOrderType`.
    pub adjusted_order_type: String,
    /// `IBApi.Order.TriggerPrice`.
    pub trigger_price: f64,
    /// `IBApi.Order.LmtPriceOffset`.
    pub lmt_price_offset: f64,
    /// `IBApi.Order.AdjustedStopPrice`.
    pub adjusted_stop_price: f64,
    /// `IBApi.Order.AdjustedStopLimitPrice`.
    pub adjusted_stop_limit_price: f64,
    /// `IBApi.Order.AdjustedTrailingAmount`.
    pub adjusted_trailing_amount: f64,
    /// `IBApi.Order.AdjustableTrailingUnit`.
    pub adjustable_trailing_unit: i32,
    /// How the order is handled: sent at once or held, what-if, outside regular hours.
    pub handling: OrderHandling,
    /// How the order may fill: all or none, block, sweep to fill.
    pub fill: OrderFill,
    /// How the order is routed: hidden, no smart routing, not held.
    pub routing: OrderRouting,
    /// What the conditions of the order do: ignore regular hours, cancel the order, auto-cancel the parent.
    pub condition_flags: OrderConditionFlags,
    /// The switches of scale orders and the percentage constraints.
    pub scale_flags: OrderScaleFlags,
    /// Hedge, OMS container and discretionary price switches.
    pub hedge_flags: OrderHedgeFlags,
    /// Switches about the customer and the session: professional customer, overnight, imbalance only.
    pub customer: OrderCustomer,
}

impl Default for IbOrder {
    fn default() -> Self {
        Self {
            handling: OrderHandling::default(),
            fill: OrderFill::default(),
            routing: OrderRouting::default(),
            condition_flags: OrderConditionFlags::default(),
            scale_flags: OrderScaleFlags::default(),
            hedge_flags: OrderHedgeFlags::default(),
            customer: OrderCustomer::default(),
            action: String::new(),
            total_quantity: 0.0,
            order_type: String::new(),
            lmt_price: f64::MAX,
            aux_price: f64::MAX,
            tif: String::new(),
            oca_group: String::new(),
            oca_type: 0,
            account: String::new(),
            open_close: String::new(),
            origin: 0, // Order.CUSTOMER
            order_ref: String::new(),
            parent_id: 0,
            display_size: 0,
            trigger_method: 0,
            discretionary_amt: 0.0,
            good_after_time: String::new(),
            good_till_date: String::new(),
            fa_group: String::new(),
            fa_method: String::new(),
            fa_percentage: String::new(),
            model_code: String::new(),
            short_sale_slot: 0,
            designated_location: String::new(),
            exempt_code: -1,
            rule80a: String::new(),
            settling_firm: String::new(),
            min_qty: i32::MAX,
            percent_offset: f64::MAX,
            auction_strategy: 0,
            starting_price: f64::MAX,
            stock_ref_price: f64::MAX,
            delta: f64::MAX,
            stock_range_lower: f64::MAX,
            stock_range_upper: f64::MAX,
            volatility: f64::MAX,
            volatility_type: i32::MAX,
            delta_neutral_order_type: String::new(),
            delta_neutral_aux_price: f64::MAX,
            continuous_update: 0,
            reference_price_type: i32::MAX,
            trail_stop_price: f64::MAX,
            trailing_percent: f64::MAX,
            conditions: Vec::new(),
            scale_init_level_size: i32::MAX,
            scale_subs_level_size: i32::MAX,
            scale_price_increment: f64::MAX,
            scale_price_adjust_value: f64::MAX,
            scale_price_adjust_interval: i32::MAX,
            scale_profit_offset: f64::MAX,
            scale_init_position: i32::MAX,
            scale_init_fill_qty: i32::MAX,
            scale_table: String::new(),
            active_start_time: String::new(),
            active_stop_time: String::new(),
            hedge_type: String::new(),
            hedge_param: String::new(),
            clearing_account: String::new(),
            clearing_intent: String::new(),
            algo_strategy: String::new(),
            algo_id: String::new(),
            ext_operator: String::new(),
            cash_qty: f64::MAX,
            mifid2_decision_maker: String::new(),
            mifid2_decision_algo: String::new(),
            mifid2_execution_trader: String::new(),
            mifid2_execution_algo: String::new(),
            use_price_mgmt_algo: None,
            duration: i32::MAX,
            post_to_ats: i32::MAX,
            advanced_error_override: String::new(),
            manual_order_time: String::new(),
            min_trade_qty: i32::MAX,
            customer_account: String::new(),
            manual_order_indicator: i32::MAX,
            adjusted_order_type: String::new(),
            trigger_price: f64::MAX,
            lmt_price_offset: f64::MAX,
            adjusted_stop_price: f64::MAX,
            adjusted_stop_limit_price: f64::MAX,
            adjusted_trailing_amount: f64::MAX,
            adjustable_trailing_unit: 0,
        }
    }
}
