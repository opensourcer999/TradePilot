//! Encoders: the fields of the messages sent to TWS.

use crate::{IbContract, IbOrder, REQ_MKT_DATA_GENERIC_TICKS};

// ------------------------------------------------------------------------
// Real order/market-data wire encoding (IBApi.EClient/Order/Contract/IBParamsList).
// Written field-for-field, including every
// version gate, so no field this app doesn't expose is silently omitted: it is written
// with its real "not set" default from the API's Order()/Contract() constructors.
// Template trades send BAG/combo contracts: the BAG blocks of `placeOrder` are
// written field for field below; `reqMktData`'s BAG block is still not needed.
// ------------------------------------------------------------------------

/// A boolean field as the API writes it: "1" or "0".
#[must_use]
pub fn param_bool(value: bool) -> String {
    if value { "1" } else { "0" }.to_string()
}

/// An optional boolean field: "1", "0", or empty when unset.
#[must_use]
pub fn param_bool_opt(value: Option<bool>) -> String {
    match value {
        Some(true) => "1".to_string(),
        Some(false) => "0".to_string(),
        None => String::new(),
    }
}

/// A number field as the API writes it.
#[must_use]
pub fn param_f64(value: f64) -> String {
    format!("{value}")
}

/// A number field that is empty when unset (`f64::MAX`, the API's "no value").
#[must_use]
pub fn param_f64_max(value: f64) -> String {
    if value.total_cmp(&f64::MAX).is_eq() {
        String::new()
    } else if value == f64::INFINITY {
        "Infinity".to_string()
    } else {
        param_f64(value)
    }
}

/// An integer field that is empty when unset (`i32::MAX`, the API's "no value").
#[must_use]
pub fn param_i32_max(value: i32) -> String {
    if value == i32::MAX {
        String::new()
    } else {
        value.to_string()
    }
}

/// A handful of `Order` fields are written with plain `AddParameter(double)` (not the "Max"
/// variant) despite defaulting to `double.MaxValue`, so the API client really does send
/// the literal text of the largest double (`1.7976931348623157E+308`) for them when unset.
#[must_use]
pub fn param_f64_plain(value: f64) -> String {
    if value.total_cmp(&f64::MAX).is_eq() {
        "1.7976931348623157E+308".to_string()
    } else if value.total_cmp(&f64::MIN).is_eq() {
        "-1.7976931348623157E+308".to_string()
    } else {
        param_f64(value)
    }
}

/// `IBApi.EClient.reqMktData`'s real field order.
#[must_use]
pub fn encode_req_mkt_data(server_version: i32, ticker_id: i32, contract: &IbContract) -> Vec<String> {
    let mut f = Vec::new();
    f.push("11".to_string()); // version
    f.push(ticker_id.to_string());
    if server_version >= 37 {
        f.push(contract.con_id.to_string());
    }
    f.push(contract.symbol.clone());
    f.push(contract.sec_type.clone());
    f.push(contract.last_trade_date_or_contract_month.clone());
    f.push(param_f64_max(contract.strike));
    f.push(contract.right.clone());
    if server_version >= 15 {
        f.push(contract.multiplier.clone());
    }
    f.push(contract.exchange.clone());
    if server_version >= 14 {
        f.push(contract.primary_exch.clone());
    }
    f.push(contract.currency.clone());
    if server_version >= 2 {
        f.push(contract.local_symbol.clone());
    }
    if server_version >= 68 {
        f.push(contract.trading_class.clone());
    }
    // contract.SecType == "BAG" combo-leg block omitted: this app never sends BAG contracts.
    if server_version >= 40 {
        f.push(param_bool(false)); // no DeltaNeutralContract
    }
    if server_version >= 31 {
        f.push(REQ_MKT_DATA_GENERIC_TICKS.to_string());
    }
    if server_version >= 35 {
        f.push(param_bool(false)); // snapshot
    }
    if server_version >= 114 {
        f.push(param_bool(false)); // regulatorySnapshot
    }
    if server_version >= 70 {
        f.push(String::new()); // mktDataOptions: none
    }
    f
}

/// `IBApi.EClient.reqContractDetails`'s real field order. Used to resolve a real
/// `Contract.ConId` for an underlying symbol before calling
/// `reqSecDefOptParams` - TWS does not reliably return expiries/strikes when `underlyingConId` is
/// `0` ("unresolved"), despite that being a generally valid "not specified" sentinel for `ConId` on
/// other requests (e.g. `reqMktData`); an earlier version of this code assumed it would work there
/// too, untested, and it did not. `RequestContractData`'s Protobuf threshold is server version 205,
/// safely above this app's capped `IB_API_VERSION_MAX` of 200.
#[must_use]
pub fn encode_req_contract_details(server_version: i32, req_id: i32, contract: &IbContract) -> Vec<String> {
    let mut f = Vec::new();
    f.push("8".to_string()); // version
    if server_version >= 40 {
        f.push(req_id.to_string());
    }
    if server_version >= 37 {
        f.push(contract.con_id.to_string());
    }
    f.push(contract.symbol.clone());
    f.push(contract.sec_type.clone());
    f.push(contract.last_trade_date_or_contract_month.clone());
    f.push(param_f64_max(contract.strike));
    f.push(contract.right.clone());
    if server_version >= 15 {
        f.push(contract.multiplier.clone());
    }
    // This app only ever negotiates server_version >= 100 (IB_API_VERSION_MIN), so the real
    // client's older server_version < 75 fallback (combining Exchange/PrimaryExch into one field)
    // can never be reached here; the >= 75 branch below is the only one this app's negotiated
    // range ever takes.
    f.push(contract.exchange.clone());
    f.push(contract.primary_exch.clone());
    f.push(contract.currency.clone());
    f.push(contract.local_symbol.clone());
    if server_version >= 68 {
        f.push(contract.trading_class.clone());
    }
    if server_version >= 31 {
        f.push(param_bool(false)); // IncludeExpired: real Contract() default
    }
    if server_version >= 45 {
        f.push(String::new()); // SecIdType: real Contract() default
        f.push(String::new()); // SecId: real Contract() default
    }
    if server_version >= 176 {
        f.push(String::new()); // IssuerId: real Contract() default
    }
    f
}

/// How the bars of a `reqHistoricalData` request are cut.
pub struct BarsRequest<'a> {
    /// The bar size, as TWS spells it ("1 day", "1 min").
    pub bar_size: &'a str,
    /// How far back the bars reach ("2 D", "1 D").
    pub duration: &'a str,
    /// Keep sending the newest bar while it forms.
    pub keep_up_to_date: bool,
}

/// `IBApi.EClient.reqHistoricalData`'s field order for the daily bars of the last two days (end
/// "" = now, bar size "1 day", duration "2 D", regular trading hours, trades, date format 1, no
/// live updates). Its Protobuf threshold is server version 208, above this app's cap of 200.
#[must_use]
pub fn encode_req_daily_bars(server_version: i32, req_id: i32, contract: &IbContract) -> Vec<String> {
    encode_req_bars(server_version, req_id, contract, &BarsRequest { bar_size: "1 day", duration: "2 D", keep_up_to_date: false })
}

/// `IBApi.EClient.reqHistoricalData`'s field order for the trades of `contract` in regular trading hours, cut
/// as `request` says (end "" = now, date format 1).
#[must_use]
pub fn encode_req_bars(server_version: i32, req_id: i32, contract: &IbContract, request: &BarsRequest<'_>) -> Vec<String> {
    let mut f = Vec::new();
    if server_version < 124 {
        f.push("6".to_string()); // version
    }
    f.push(req_id.to_string());
    if server_version >= 68 {
        f.push(contract.con_id.to_string());
    }
    f.push(contract.symbol.clone());
    f.push(contract.sec_type.clone());
    f.push(contract.last_trade_date_or_contract_month.clone());
    f.push(param_f64_max(contract.strike));
    f.push(contract.right.clone());
    f.push(contract.multiplier.clone());
    f.push(contract.exchange.clone());
    f.push(contract.primary_exch.clone());
    f.push(contract.currency.clone());
    f.push(contract.local_symbol.clone());
    if server_version >= 68 {
        f.push(contract.trading_class.clone());
    }
    f.push(param_bool(false)); // IncludeExpired
    f.push(String::new()); // endDateTime: now
    f.push(request.bar_size.to_string()); // barSizeSetting
    f.push(request.duration.to_string()); // durationStr
    f.push("1".to_string()); // useRTH
    f.push("TRADES".to_string()); // whatToShow
    f.push("1".to_string()); // formatDate
    if server_version >= 124 {
        f.push(param_bool(request.keep_up_to_date)); // keepUpToDate
    }
    if server_version >= 70 {
        f.push(String::new()); // chartOptions: none
    }
    f
}

/// `IBApi.EClient.reqSecDefOptParams`'s real field order - unlike `reqMktData`/`placeOrder`, this
/// message has no leading "version" field at all (see
/// `EClient.reqSecDefOptParams(int, string, string, string, int)`;
/// its Protobuf threshold is server version 212, safely above this app's capped
/// `IB_API_VERSION_MAX` of 200, so the plain-text path below is what a real TWS actually uses).
/// `fut_fop_exchange` of `""` means "all exchanges" per IB's own documented convention (seen the
/// same way `contract.exchange` is sent as-is elsewhere in this file); `underlying_con_id` must be
/// a real, resolved contract id (see `encode_req_contract_details` above) - `0` does not reliably
/// work.
#[must_use]
pub fn encode_req_sec_def_opt_params(
    req_id: i32,
    underlying_symbol: &str,
    fut_fop_exchange: &str,
    underlying_sec_type: &str,
    underlying_con_id: i32,
) -> Vec<String> {
    vec![req_id.to_string(), underlying_symbol.to_string(), fut_fop_exchange.to_string(), underlying_sec_type.to_string(), underlying_con_id.to_string()]
}

/// The leading fields of `placeOrder`: the version, the order id and the contract.
fn encode_order_contract(f: &mut Vec<String>, server_version: i32, order_id: i32, contract: &IbContract) {
    let version = if server_version < 44 { 27 } else { 45 };
    if server_version < 145 {
        f.push(version.to_string());
    }
    f.push(order_id.to_string());
    if server_version >= 46 {
        f.push(contract.con_id.to_string());
    }
    f.push(contract.symbol.clone());
    f.push(contract.sec_type.clone());
    f.push(contract.last_trade_date_or_contract_month.clone());
    f.push(param_f64_max(contract.strike));
    f.push(contract.right.clone());
    if server_version >= 15 {
        f.push(contract.multiplier.clone());
    }
    f.push(contract.exchange.clone());
    if server_version >= 14 {
        f.push(contract.primary_exch.clone());
    }
    f.push(contract.currency.clone());
    if server_version >= 2 {
        f.push(contract.local_symbol.clone());
    }
    if server_version >= 68 {
        f.push(contract.trading_class.clone());
    }
    if server_version >= 45 {
        f.push(String::new()); // Contract.SecIdType
        f.push(String::new()); // Contract.SecId
    }
}

/// The action, quantity, type, prices and the basic order fields of `placeOrder`.
fn encode_order_action(f: &mut Vec<String>, server_version: i32, order: &IbOrder) {
    f.push(order.action.clone());
    if server_version >= 101 {
        f.push(param_f64(order.total_quantity));
    } else {
        f.push(format!("{:.0}", order.total_quantity));
    }
    f.push(order.order_type.clone());
    if server_version < 61 {
        f.push(param_f64(if order.lmt_price.total_cmp(&f64::MAX).is_eq() { 0.0 } else { order.lmt_price }));
    } else {
        f.push(param_f64_max(order.lmt_price));
    }
    if server_version < 62 {
        f.push(param_f64(if order.aux_price.total_cmp(&f64::MAX).is_eq() { 0.0 } else { order.aux_price }));
    } else {
        f.push(param_f64_max(order.aux_price));
    }
    f.push(order.tif.clone());
    f.push(order.oca_group.clone());
    f.push(order.account.clone());
    f.push(order.open_close.clone());
    f.push(order.origin.to_string());
    f.push(order.order_ref.clone());
    f.push(param_bool(order.handling.transmit));
    if server_version >= 4 {
        f.push(order.parent_id.to_string());
    }
    if server_version >= 5 {
        f.push(param_bool(order.fill.block_order));
        f.push(param_bool(order.fill.sweep_to_fill));
        f.push(order.display_size.to_string());
        f.push(order.trigger_method.to_string());
        if server_version < 38 {
            f.push(param_bool(false));
        } else {
            f.push(param_bool(order.handling.outside_rth));
        }
    }
    if server_version >= 7 {
        f.push(param_bool(order.routing.hidden));
    }
}

/// The BAG/combo fields of `placeOrder`: the combo legs, the order legs and the smart-routing parameters.
fn encode_order_combo(f: &mut Vec<String>, server_version: i32, contract: &IbContract, order: &IbOrder) {
    let is_bag = contract.sec_type == "BAG";
    if server_version >= 8 && is_bag {
        f.push(contract.combo_legs.len().to_string());
        for leg in &contract.combo_legs {
            f.push(leg.con_id.to_string());
            f.push(leg.ratio.to_string());
            f.push(leg.action.clone());
            f.push(leg.exchange.clone());
            f.push("0".to_string()); // ComboLeg.OpenClose (default 0 = SAME)
            if server_version >= 35 {
                f.push("0".to_string()); // ComboLeg.ShortSaleSlot
                f.push(String::new()); // ComboLeg.DesignatedLocation (null -> empty)
            }
            if server_version >= 51 {
                f.push("0".to_string()); // ComboLeg.ExemptCode (parameterless ctor: 0)
            }
        }
    }
    if server_version >= 61 && is_bag {
        f.push("0".to_string()); // Order.OrderComboLegs: none (no per-leg prices)
    }
    if server_version >= 57 && is_bag {
        f.push("0".to_string()); // Order.SmartComboRoutingParams: none
    }
    if server_version >= 9 {
        f.push(String::new()); // deprecated shares-allocation field; always empty
    }
    if server_version >= 10 {
        f.push(param_f64(order.discretionary_amt));
    }
    if server_version >= 11 {
        f.push(order.good_after_time.clone());
    }
    if server_version >= 12 {
        f.push(order.good_till_date.clone());
    }
    if server_version >= 13 {
        f.push(order.fa_group.clone());
        f.push(order.fa_method.clone());
        f.push(order.fa_percentage.clone());
        if server_version < 177 {
            f.push(String::new());
        }
    }
}

/// The order details of `placeOrder` from the OCA group to the scale orders.
fn encode_order_details(f: &mut Vec<String>, server_version: i32, order: &IbOrder) {
    if server_version >= 103 {
        f.push(order.model_code.clone());
    }
    if server_version >= 18 {
        f.push(order.short_sale_slot.to_string());
        f.push(order.designated_location.clone());
    }
    if server_version >= 51 {
        f.push(order.exempt_code.to_string());
    }
    if server_version >= 19 {
        f.push(order.oca_type.to_string());
        if server_version < 38 {
            f.push(param_bool(false));
        }
        f.push(order.rule80a.clone());
        f.push(order.settling_firm.clone());
        f.push(param_bool(order.fill.all_or_none));
        f.push(param_i32_max(order.min_qty));
        f.push(param_f64_max(order.percent_offset));
        f.push(param_bool(false));
        f.push(param_bool(false));
        f.push(param_f64_max(f64::MAX)); // unconditionally empty in the real client too
        f.push(param_i32_max(order.auction_strategy));
        f.push(param_f64_max(order.starting_price));
        f.push(param_f64_max(order.stock_ref_price));
        f.push(param_f64_max(order.delta));
        f.push(param_f64_max(order.stock_range_lower));
        f.push(param_f64_max(order.stock_range_upper));
    }
    if server_version >= 22 {
        f.push(param_bool(order.scale_flags.override_percentage_constraints));
    }
    if server_version >= 26 {
        f.push(param_f64_max(order.volatility));
        f.push(param_i32_max(order.volatility_type));
        // serverVersion is always >= 100 here, so the real client's "< 28" MKT-string branch
        // (a different, older wire shape) can never be taken; only the >= 28 shape applies.
        f.push(order.delta_neutral_order_type.clone());
        f.push(param_f64_max(order.delta_neutral_aux_price));
        if server_version >= 58 && !order.delta_neutral_order_type.is_empty() {
            f.push("0".to_string()); // DeltaNeutralConId
            f.push(String::new()); // DeltaNeutralSettlingFirm
            f.push(String::new()); // DeltaNeutralClearingAccount
            f.push(String::new()); // DeltaNeutralClearingIntent
        }
        if server_version >= 66 && !order.delta_neutral_order_type.is_empty() {
            f.push(String::new()); // DeltaNeutralOpenClose
            f.push(param_bool(false)); // DeltaNeutralShortSale
            f.push("0".to_string()); // DeltaNeutralShortSaleSlot
            f.push(String::new()); // DeltaNeutralDesignatedLocation
        }
        f.push(order.continuous_update.to_string());
        f.push(param_i32_max(order.reference_price_type));
    }
    if server_version >= 30 {
        f.push(param_f64_max(order.trail_stop_price));
    }
    if server_version >= 62 {
        f.push(param_f64_max(order.trailing_percent));
    }
    if server_version >= 35 {
        f.push(param_i32_max(order.scale_init_level_size));
        f.push(param_i32_max(order.scale_subs_level_size));
        f.push(param_f64_max(order.scale_price_increment));
    }
    if server_version >= 60 && order.scale_price_increment > 0.0 && order.scale_price_increment.total_cmp(&f64::MAX).is_ne() {
        f.push(param_f64_max(order.scale_price_adjust_value));
        f.push(param_i32_max(order.scale_price_adjust_interval));
        f.push(param_f64_max(order.scale_profit_offset));
        f.push(param_bool(order.scale_flags.scale_auto_reset));
        f.push(param_i32_max(order.scale_init_position));
        f.push(param_i32_max(order.scale_init_fill_qty));
        f.push(param_bool(order.scale_flags.scale_random_percent));
    }
}

/// The hedge, algo and conditions fields of `placeOrder`.
fn encode_order_hedge_and_algo(f: &mut Vec<String>, server_version: i32, order: &IbOrder) {
    if server_version >= 69 {
        f.push(order.scale_table.clone());
        f.push(order.active_start_time.clone());
        f.push(order.active_stop_time.clone());
    }
    if server_version >= 54 {
        f.push(order.hedge_type.clone());
        if !order.hedge_type.is_empty() {
            f.push(order.hedge_param.clone());
        }
    }
    if server_version >= 56 {
        f.push(param_bool(order.routing.opt_out_smart_routing));
    }
    if server_version >= 39 {
        f.push(order.clearing_account.clone());
        f.push(order.clearing_intent.clone());
    }
    if server_version >= 44 {
        f.push(param_bool(order.routing.not_held));
    }
    if server_version >= 40 {
        f.push(param_bool(false)); // no DeltaNeutralContract on the Contract
    }
    if server_version >= 41 {
        f.push(order.algo_strategy.clone());
        if !order.algo_strategy.is_empty() {
            f.push("0".to_string()); // AlgoParams count: none
        }
    }
    if server_version >= 71 {
        f.push(order.algo_id.clone());
    }
    if server_version >= 36 {
        f.push(param_bool(order.handling.what_if));
    }
    if server_version >= 70 {
        f.push(String::new()); // OrderMiscOptions: empty
    }
    if server_version >= 73 {
        f.push(param_bool(false)); // Solicited
    }
    if server_version >= 76 {
        f.push(param_bool(false)); // RandomizeSize
        f.push(param_bool(false)); // RandomizePrice
    }
    if server_version >= 102 {
        // OrderType is never a peg-bench order type in this app. The conditions: their number, then
        // for each its type and fields, and - only when there are any - the two flags.
        f.push(order.conditions.len().to_string());
        if !order.conditions.is_empty() {
            for condition in &order.conditions {
                f.push(condition.type_code().to_string());
                f.extend(condition.fields());
            }
            f.push(param_bool(order.condition_flags.conditions_ignore_rth));
            f.push(param_bool(order.condition_flags.conditions_cancel_order));
        }
        f.push(order.adjusted_order_type.clone());
        f.push(param_f64_plain(order.trigger_price));
        f.push(param_f64_plain(order.lmt_price_offset));
        f.push(param_f64_plain(order.adjusted_stop_price));
        f.push(param_f64_plain(order.adjusted_stop_limit_price));
        f.push(param_f64_plain(order.adjusted_trailing_amount));
        f.push(order.adjustable_trailing_unit.to_string());
    }
}

/// The fields of `placeOrder` that newer server versions added at the end.
fn encode_order_newer_fields(f: &mut Vec<String>, server_version: i32, contract: &IbContract, order: &IbOrder) {
    if server_version >= 105 {
        f.push(order.ext_operator.clone());
    }
    if server_version >= 106 {
        f.push(String::new()); // Tier.Name
        f.push(String::new()); // Tier.Value
    }
    if server_version >= 111 {
        f.push(param_f64_max(order.cash_qty));
    }
    if server_version >= 138 {
        f.push(order.mifid2_decision_maker.clone());
        f.push(order.mifid2_decision_algo.clone());
    }
    if server_version >= 139 {
        f.push(order.mifid2_execution_trader.clone());
        f.push(order.mifid2_execution_algo.clone());
    }
    if server_version >= 141 {
        f.push(param_bool(order.hedge_flags.dont_use_auto_price_for_hedge));
    }
    if server_version >= 145 {
        f.push(param_bool(order.hedge_flags.is_oms_container));
    }
    if server_version >= 148 {
        f.push(param_bool(order.hedge_flags.discretionary_up_to_limit_price));
    }
    if server_version >= 151 {
        f.push(param_bool_opt(order.use_price_mgmt_algo));
    }
    if server_version >= 158 {
        f.push(order.duration.to_string()); // plain AddParameter: writes the literal sentinel if unset
    }
    if server_version >= 160 {
        f.push(order.post_to_ats.to_string());
    }
    if server_version >= 162 {
        f.push(param_bool(order.condition_flags.auto_cancel_parent));
    }
    if server_version >= 166 {
        f.push(order.advanced_error_override.clone());
    }
    if server_version >= 169 {
        f.push(order.manual_order_time.clone());
    }
    // OrderType is always MKT/LMT here, never a peg-best/peg-mid order type, so only the
    // IBKRATS min-trade-qty field (also gated by serverVersion >= 170) ever applies.
    if server_version >= 170 && contract.exchange == "IBKRATS" {
        f.push(param_i32_max(order.min_trade_qty));
    }
    if server_version >= 183 {
        f.push(order.customer_account.clone());
    }
    if server_version >= 184 {
        f.push(param_bool(order.customer.professional_customer));
    }
    if (187..190).contains(&server_version) {
        f.push(String::new());
        f.push(i32::MAX.to_string());
    }
    if server_version >= 189 {
        f.push(param_bool(order.customer.include_overnight));
    }
    if server_version >= 192 {
        f.push(order.manual_order_indicator.to_string());
    }
    if server_version >= 199 {
        f.push(param_bool(order.customer.imbalance_only));
    }
}

/// `IBApi.EClient.placeOrder`'s real field order, including the BAG/combo blocks.
#[must_use]
pub fn encode_place_order(server_version: i32, order_id: i32, contract: &IbContract, order: &IbOrder) -> Vec<String> {
    let mut f = Vec::new();
    encode_order_contract(&mut f, server_version, order_id, contract);
    encode_order_action(&mut f, server_version, order);
    encode_order_combo(&mut f, server_version, contract, order);
    encode_order_details(&mut f, server_version, order);
    encode_order_hedge_and_algo(&mut f, server_version, order);
    encode_order_newer_fields(&mut f, server_version, contract, order);
    f
}

/// `IBApi.EClient.cancelOrder`'s real field order.
#[must_use]
pub fn encode_cancel_order(server_version: i32, order_id: i32) -> Vec<String> {
    let mut f = Vec::new();
    if server_version < 192 {
        f.push("1".to_string());
    }
    f.push(order_id.to_string());
    if server_version >= 169 {
        f.push(String::new()); // OrderCancel.ManualOrderCancelTime
    }
    if (187..190).contains(&server_version) {
        f.push(String::new());
        f.push(String::new());
        f.push(i32::MAX.to_string());
    }
    if server_version >= 192 {
        f.push(String::new()); // OrderCancel.ExtOperator
        f.push(i32::MAX.to_string()); // OrderCancel.ManualOrderIndicator (real default, written literally)
    }
    f
}

/// `IBApi.EClient.reqOpenOrders`'s real fields: only its version, 1. TWS answers with an
/// `openOrder` (and an `orderStatus`) for every order of this client that is still working, then
/// `openOrderEnd`.
#[must_use]
pub fn encode_req_open_orders() -> Vec<String> {
    vec!["1".to_string()]
}

/// Mirrors `IBApi.ExecutionFilter`, defaulted exactly as its real no-arg constructor does.
pub struct IbExecutionFilter {
    client_id: i32,
    acct_code: String,
    time: String,
    symbol: String,
    sec_type: String,
    exchange: String,
    side: String,
    last_n_days: i32,
    specific_dates: Vec<i32>,
}

impl Default for IbExecutionFilter {
    fn default() -> Self {
        Self {
            client_id: 0,
            acct_code: String::new(),
            time: String::new(),
            symbol: String::new(),
            sec_type: String::new(),
            exchange: String::new(),
            side: String::new(),
            last_n_days: i32::MAX, // real ExecutionFilter() default - "not narrowed by day count"
            specific_dates: Vec::new(),
        }
    }
}

/// `IBApi.EClient.reqExecutions`'s real field order: version=3, [v>=42: reqId], [v>=9: clientId,
/// acctCode, time, symbol, secType, exchange, side, [v>=200: lastNDays, `specificDates.len()`,
/// specificDates...]]. A default (empty) filter with `lastNDays` left at its real "unset" sentinel
/// is exactly what makes a real TWS scope the response to the current trading day on its own -
/// the same "Trades today" semantics the Dashboard needs, with no client-side date filtering.
#[must_use]
pub fn encode_req_executions(server_version: i32, req_id: i32, filter: &IbExecutionFilter) -> Vec<String> {
    let mut f = vec!["3".to_string()];
    if server_version >= 42 {
        f.push(req_id.to_string());
    }
    if server_version >= 9 {
        f.push(filter.client_id.to_string());
        f.push(filter.acct_code.clone());
        f.push(filter.time.clone());
        f.push(filter.symbol.clone());
        f.push(filter.sec_type.clone());
        f.push(filter.exchange.clone());
        f.push(filter.side.clone());
        if server_version >= 200 {
            f.push(filter.last_n_days.to_string());
            f.push(filter.specific_dates.len().to_string());
            for date in &filter.specific_dates {
                f.push(date.to_string());
            }
        }
    }
    f
}
