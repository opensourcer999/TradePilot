//! Reads files that earlier versions of the app wrote: their field names are renamed to the current ones.
//! Only files are read this way; nothing is written with these names.

use serde_json::Value;

/// (old name, current name) of every field of a template file.
const FIELDS: &[(&str, &str)] = &[
    ("Adjustment1", "adjustment1"),
    ("Adjustment1Change", "adjustment1_change"),
    ("Adjustment1ChangeOffset", "adjustment1_change_offset"),
    ("Adjustment1ChangeType", "adjustment1_change_type"),
    ("Adjustment1Hour", "adjustment1_hour"),
    ("Adjustment1Minute", "adjustment1_minute"),
    ("Adjustment1OrderType", "adjustment1_order_type"),
    ("Adjustment1Type", "adjustment1_type"),
    ("Adjustment2", "adjustment2"),
    ("Adjustment2Change", "adjustment2_change"),
    ("Adjustment2ChangeOffset", "adjustment2_change_offset"),
    ("Adjustment2ChangeType", "adjustment2_change_type"),
    ("Adjustment2Hour", "adjustment2_hour"),
    ("Adjustment2Minute", "adjustment2_minute"),
    ("Adjustment2OrderType", "adjustment2_order_type"),
    ("Adjustment2Type", "adjustment2_type"),
    ("Adjustment3", "adjustment3"),
    ("Adjustment3Change", "adjustment3_change"),
    ("Adjustment3ChangeOffset", "adjustment3_change_offset"),
    ("Adjustment3ChangeType", "adjustment3_change_type"),
    ("Adjustment3Hour", "adjustment3_hour"),
    ("Adjustment3Minute", "adjustment3_minute"),
    ("Adjustment3OrderType", "adjustment3_order_type"),
    ("Adjustment3Type", "adjustment3_type"),
    ("AllOrNone", "all_or_none"),
    ("AllowStrikeConflict", "allow_strike_conflict"),
    ("CLeg1Action", "custom_leg1_action"),
    ("CLeg1DTE", "custom_leg1_dte"),
    ("CLeg1DTEAdjust", "custom_leg1_dte_adjust"),
    ("CLeg1PutCall", "custom_leg1_put_call"),
    ("CLeg1Ratio", "custom_leg1_ratio"),
    ("CLeg1Target", "custom_leg1_target"),
    ("CLeg1TargetType", "custom_leg1_target_type"),
    ("CLeg2Action", "custom_leg2_action"),
    ("CLeg2DTE", "custom_leg2_dte"),
    ("CLeg2DTEAdjust", "custom_leg2_dte_adjust"),
    ("CLeg2PutCall", "custom_leg2_put_call"),
    ("CLeg2Ratio", "custom_leg2_ratio"),
    ("CLeg2Target", "custom_leg2_target"),
    ("CLeg2TargetType", "custom_leg2_target_type"),
    ("CLeg3Action", "custom_leg3_action"),
    ("CLeg3DTE", "custom_leg3_dte"),
    ("CLeg3DTEAdjust", "custom_leg3_dte_adjust"),
    ("CLeg3PutCall", "custom_leg3_put_call"),
    ("CLeg3Ratio", "custom_leg3_ratio"),
    ("CLeg3Target", "custom_leg3_target"),
    ("CLeg3TargetType", "custom_leg3_target_type"),
    ("CLeg4Action", "custom_leg4_action"),
    ("CLeg4DTE", "custom_leg4_dte"),
    ("CLeg4DTEAdjust", "custom_leg4_dte_adjust"),
    ("CLeg4PutCall", "custom_leg4_put_call"),
    ("CLeg4Ratio", "custom_leg4_ratio"),
    ("CLeg4Target", "custom_leg4_target"),
    ("CLeg4TargetType", "custom_leg4_target_type"),
    ("CallRatio", "call_ratio"),
    ("ConditionDetails", "condition_details"),
    ("ConditionIDMap", "condition_id_map"),
    ("Conditions", "conditions"),
    ("DTEAdjust", "dte_adjust"),
    ("DisableNarrowerLong", "disable_narrower_long"),
    ("DisableNarrowerLongCall", "disable_narrower_long_call"),
    ("ExcessLongBehavior", "excess_long_behavior"),
    ("ExitConditionID", "exit_condition_id"),
    ("ExitDTE", "exit_dte"),
    ("ExitHour", "exit_hour"),
    ("ExitMinute", "exit_minute"),
    ("ExitMinutesInTrade", "exit_minutes_in_trade"),
    ("ExitOrderLimit", "exit_order_limit"),
    ("ExtendedHourPT", "extended_hour_target"),
    ("ExtendedHourStop", "extended_hour_stop"),
    ("FillAdjustment", "fill_adjustment"),
    ("FillAttempts", "fill_attempts"),
    ("FillWait", "fill_wait"),
    ("ForceCloseAllLegs", "force_close_all_legs"),
    ("IsDeleted", "is_deleted"),
    ("LongCallDTE", "long_call_dte"),
    ("LongCallDTEAdjust", "long_call_dte_adjust"),
    ("LongCallTarget", "long_call_target"),
    ("LongCallTargetType", "long_call_target_type"),
    ("LongMaxPremium", "long_max_premium"),
    ("LongMaxPremiumCall", "long_max_premium_call"),
    ("LongMaxWidth", "long_max_width"),
    ("LongMaxWidthCall", "long_max_width_call"),
    ("LongMinPremium", "long_min_premium"),
    ("LongMinPremiumCall", "long_min_premium_call"),
    ("LongPutDTE", "long_put_dte"),
    ("LongPutDTEAdjust", "long_put_dte_adjust"),
    ("LongPutTarget", "long_put_target"),
    ("LongPutTargetType", "long_put_target_type"),
    ("LongReuseOnly", "long_reuse_only"),
    ("LongType", "long_type"),
    ("LongTypeCall", "long_type_call"),
    ("LongWidth", "long_width"),
    ("LongWidthCall", "long_width_call"),
    ("LowerTarget", "lower_target"),
    ("MarketCloseHour", "market_close_hour"),
    ("MaxEntryPrice", "max_entry_price"),
    ("MaxEntrySLRatio", "max_entry_sl_ratio"),
    ("MinEntryPrice", "min_entry_price"),
    ("MinEntrySLRatio", "min_entry_sl_ratio"),
    ("MinOTM", "min_otm"),
    ("MinOTMCall", "min_otm_call"),
    ("Name", "name"),
    ("OrderIDProfitTarget", "order_id_profit_target"),
    ("Preference", "preference"),
    ("PreferenceCall", "preference_call"),
    ("ProfitTarget", "profit_target"),
    ("ProfitTarget2", "profit_target2"),
    ("ProfitTarget2TradePct", "profit_target2_trade_percent"),
    ("ProfitTarget3", "profit_target3"),
    ("ProfitTarget3TradePct", "profit_target3_trade_percent"),
    ("ProfitTarget4", "profit_target4"),
    ("ProfitTarget4TradePct", "profit_target4_trade_percent"),
    ("ProfitTargetExpirationHour", "profit_target_expiration_hour"),
    ("ProfitTargetExpirationMinute", "profit_target_expiration_minute"),
    ("ProfitTargetTradePct", "profit_target_trade_percent"),
    ("ProfitTargetType", "profit_target_type"),
    ("PutRatio", "put_ratio"),
    ("QtyDefault", "quantity_default"),
    ("ReEnterClose", "reentry_close"),
    ("ReEnterCloseTemplateID", "reentry_close_template_id"),
    ("ReEnterCloseTemplateID2", "reentry_close_template_id2"),
    ("ReEnterDelay", "reentry_delay"),
    ("ReEnterExpirationHour", "reentry_expiration_hour"),
    ("ReEnterExpirationMinute", "reentry_expiration_minute"),
    ("ReEnterMaxEntries", "reentry_max_entries"),
    ("ReEnterProfitTarget", "reentry_profit_target"),
    ("ReEnterProfitTargetTemplateID", "reentry_profit_target_template_id"),
    ("ReEnterProfitTargetTemplateID2", "reentry_profit_target_template_id2"),
    ("ReEnterStop", "reentry_stop"),
    ("ReEnterStopTemplateID", "reentry_stop_template_id"),
    ("ReEnterStopTemplateID2", "reentry_stop_template_id2"),
    ("ScheduleMasterIDMap", "schedule_master_id_map"),
    ("Schedules", "schedules"),
    ("ShortCallDTE", "short_call_dte"),
    ("ShortCallDTEAdjust", "short_call_dte_adjust"),
    ("ShortCallTarget", "short_call_target"),
    ("ShortCallTargetType", "short_call_target_type"),
    ("ShortPutDTE", "short_put_dte"),
    ("ShortPutDTEAdjust", "short_put_dte_adjust"),
    ("ShortPutTarget", "short_put_target"),
    ("ShortPutTargetType", "short_put_target_type"),
    ("StopBasis", "stop_basis"),
    ("StopGuardrail", "stop_guardrail"),
    ("StopGuardrailOffsetExp", "stop_guardrail_offset_exp"),
    ("StopGuardrailOffsetOpen", "stop_guardrail_offset_open"),
    ("StopLimitMarketOffset", "stop_limit_market_offset"),
    ("StopLimitOffset", "stop_limit_offset"),
    ("StopMultiple", "stop_multiple"),
    ("StopOffset", "stop_offset"),
    ("StopOrderType", "stop_order_type"),
    ("StopRel", "stop_rel"),
    ("StopRelITM", "stop_rel_itm"),
    ("StopRelITMMinutes", "stop_rel_itm_minutes"),
    ("StopRelITMOnly", "stop_rel_itm_only"),
    ("StopRelLimit", "stop_rel_limit"),
    ("StopRelOffset", "stop_rel_offset"),
    ("StopTargetType", "stop_target_type"),
    ("StopTrigger", "stop_trigger"),
    ("StopType", "stop_type"),
    ("Strategy", "strategy"),
    ("TargetMax", "target_max"),
    ("TargetMaxCall", "target_max_call"),
    ("TargetMin", "target_min"),
    ("TargetMinCall", "target_min_call"),
    ("TargetType", "target_type"),
    ("TargetTypeCall", "target_type_call"),
    ("TemplateIDMap", "template_id_map"),
    ("Templates", "templates"),
    ("TradeTemplateID", "id"),
    ("TradeType", "trade_type"),
    ("UnderlyingSymbol", "underlying_symbol"),
    ("UpgradeFlag", "upgrade_flag"),
];

/// Renames the old keys of `object` to the current ones; a key that already has its current name is left alone.
fn upgrade_object(object: &mut serde_json::Map<String, Value>) {
    for (old, new) in FIELDS {
        if !object.contains_key(*new) {
            if let Some(value) = object.remove(*old) {
                object.insert((*new).to_string(), value);
            }
        }
    }
    upgrade_template_values(object);
}

/// A templates file or an export file as `value`: the keys of the export and of every template are brought to the current names.
pub fn upgrade_templates(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().filter_map(Value::as_object_mut).for_each(upgrade_object),
        Value::Object(map) => {
            upgrade_object(map);
            if let Some(Value::Array(items)) = map.get_mut("templates") {
                items.iter_mut().filter_map(Value::as_object_mut).for_each(upgrade_object);
            }
        }
        _ => {}
    }
}

/// (old key, current key) of every value a rule of a condition can look at.
const INPUTS: &[(&str, &str)] = &[
    ("DeltaAccount", "delta_account"),
    ("DeltaAccountSymbol", "delta_account_symbol"),
    ("DeltaTotal", "delta_total"),
    ("DeltaTotalSymbol", "delta_total_symbol"),
    ("EMA", "ema"),
    ("EMA20", "ema20"),
    ("EMA40", "ema40"),
    ("EMA5", "ema5"),
    ("EMA750", "ema750"),
    ("ES", "es"),
    ("QQQ", "qqq"),
    ("RSI", "rsi"),
    ("SMADaily", "sma_daily"),
    ("SPX", "spx"),
    ("SPXChangePct", "spx_change_percent"),
    ("SPXChangePts", "spx_change_points"),
    ("SPXGapPct", "spx_gap_percent"),
    ("SPXGapPts", "spx_gap_points"),
    ("SPXIntradayPct", "spx_intraday_percent"),
    ("SPXIntradayPts", "spx_intraday_points"),
    ("SPXOpen", "spx_open"),
    ("SPXPriorClose", "spx_prior_close"),
    ("SPY", "spy"),
    ("Time_MinutesFromMarketOpen", "minutes_from_open"),
    ("Time_MinutesToMarketClose", "minutes_to_close"),
    ("Trade_AskPrice", "trade_ask_price"),
    ("Trade_BidPrice", "trade_bid_price"),
    ("Trade_CustomLeg1Delta", "trade_custom_leg1_delta"),
    ("Trade_CustomLeg2Delta", "trade_custom_leg2_delta"),
    ("Trade_CustomLeg3Delta", "trade_custom_leg3_delta"),
    ("Trade_CustomLeg4Delta", "trade_custom_leg4_delta"),
    ("Trade_LongCallDelta", "trade_long_call_delta"),
    ("Trade_LongCallOTMPct", "trade_long_call_otm_percent"),
    ("Trade_LongCallOTMPts", "trade_long_call_otm_points"),
    ("Trade_LongPutDelta", "trade_long_put_delta"),
    ("Trade_LongPutOTMPct", "trade_long_put_otm_percent"),
    ("Trade_LongPutOTMPts", "trade_long_put_otm_points"),
    ("Trade_PositionDelta", "trade_position_delta"),
    ("Trade_PositionTotalDelta", "trade_position_total_delta"),
    ("Trade_ProfitAmt", "trade_profit_amount"),
    ("Trade_ProfitPct", "trade_profit_percent"),
    ("Trade_ShortCallDelta", "trade_short_call_delta"),
    ("Trade_ShortCallOTMPct", "trade_short_call_otm_percent"),
    ("Trade_ShortCallOTMPts", "trade_short_call_otm_points"),
    ("Trade_ShortLongRatio", "trade_short_long_ratio"),
    ("Trade_ShortLongRatioChangePct", "trade_short_long_ratio_change_percent"),
    ("Trade_ShortPutDelta", "trade_short_put_delta"),
    ("Trade_ShortPutOTMPct", "trade_short_put_otm_percent"),
    ("Trade_ShortPutOTMPts", "trade_short_put_otm_points"),
    ("UnderlyingPrice", "underlying_price"),
    ("VIX", "vix"),
    ("VIX9D/VIX", "vix9d_over_vix"),
    ("VIXChangePct", "vix_change_percent"),
    ("VIXChangePts", "vix_change_points"),
    ("VIXGapPct", "vix_gap_percent"),
    ("VIXGapPts", "vix_gap_points"),
    ("VIXIntradayPct", "vix_intraday_percent"),
    ("VIXIntradayPts", "vix_intraday_points"),
    ("VIXOpen", "vix_open"),
    ("VIXPercentile", "vix_percentile"),
    ("VIXPriorClose", "vix_prior_close"),
    ("VIXRank", "vix_rank"),
];

/// (old text, current text) of the values a template field can hold.
const VALUES: &[(&str, &str)] = &[
    ("Highest Premium/Delta", "Highest premium"),
    ("Mid Premium/Delta", "Middle premium"),
    ("Lowest Premium/Delta", "Lowest premium"),
    ("Narrow Width", "Narrowest"),
    ("Wide Width", "Widest"),
    ("Matching Width", "Same width"),
    ("Most OTM", "Furthest OTM"),
    ("Least OTM", "Nearest OTM"),
    ("ShortPremium", "Short premium"),
    ("PercentageOTM", "Percent OTM"),
    ("StrikeOffset_Current", "Offset from price"),
    ("StrikeOffset_Open", "Offset from open"),
    ("StrikeOffset_Leg1", "Offset from leg 1"),
    ("StrikeOffset_Leg2", "Offset from leg 2"),
    ("StrikeOffset_Leg3", "Offset from leg 3"),
    ("FixedPremium", "Fixed premium"),
    ("PointsOTM", "Points OTM"),
    ("SpreadWidth", "Spread width"),
    ("Max Premium", "Premium cap"),
    ("StopMarket", "Market stop"),
    ("StopLimitOCO", "Limit stop with OCO"),
    ("StopLimit", "Limit stop"),
    ("Exact Price", "Given price"),
    ("Average Price", "Mean price"),
    ("Short Price", "Short leg price"),
    ("Long Price", "Long leg price"),
    ("Profit Level", "Profit percent"),
    ("ProfitTarget1", "Target 1"),
    ("ProfitTarget2", "Target 2"),
    ("ProfitTarget3", "Target 3"),
    ("Stop Multiple", "Stop factor"),
    ("Fixed Amount", "Fixed price"),
    ("Exit Price", "Target price"),
];

/// The current text of a list value an earlier version stored under its old text.
#[must_use]
pub fn upgraded_value(text: &str) -> Option<&'static str> {
    VALUES.iter().find(|(old, _)| *old == text).map(|(_, new)| *new)
}

fn upgraded_input(text: &str) -> Option<&'static str> {
    INPUTS.iter().find(|(old, _)| *old == text).map(|(_, new)| *new)
}

/// The texts a stored template holds in the fields that take one of a list of values.
fn upgrade_template_values(object: &mut serde_json::Map<String, Value>) {
    for (key, value) in object.iter_mut() {
        let is_choice = key.ends_with("_type") || key.starts_with("preference") || key == "stop_basis" || key.starts_with("long_width");
        if let (true, Value::String(text)) = (is_choice, &mut *value) {
            if let Some(new) = upgraded_value(text) {
                *text = new.to_string();
            }
        }
    }
}

/// The conditions of a conditions file: the keys of the values the rules look at are brought to the current ones.
pub fn upgrade_conditions(value: &mut Value) {
    let Value::Array(conditions) = value else { return };
    for rule in conditions.iter_mut().filter_map(|c| c.get_mut("rules")).filter_map(Value::as_array_mut).flatten() {
        if let Some(Value::String(input)) = rule.get_mut("input") {
            if let Some(new) = upgraded_input(input) {
                *input = new.to_string();
            }
        }
        if let Some(Value::String(input)) = rule.get_mut("comparison").and_then(|c| c.get_mut("Input")) {
            if let Some(new) = upgraded_input(input) {
                *input = new.to_string();
            }
        }
    }
}

/// Every text in `value` that is an old list value, replaced by the current one.
fn upgrade_texts(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some(new) = upgraded_value(text) {
                *text = new.to_string();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(upgrade_texts),
        Value::Object(map) => map.values_mut().for_each(upgrade_texts),
        _ => {}
    }
}

/// The stored stop adjustments of a trade (JSON text): the texts of the trigger, the order type and the way the stop is worked out.
#[must_use]
pub fn upgrade_adjustments(text: &str) -> String {
    let mut value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return text.to_string(),
    };
    upgrade_texts(&mut value);
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_conditions_template_values_and_adjustments_are_brought_to_the_current_names() {
        let mut conditions = json!([{"id":1,"rules":[{"group":1,"input":"VIXOpen","operator":">","comparison":{"Input":"SPXPriorClose"}}]}]);
        upgrade_conditions(&mut conditions);
        assert_eq!(conditions[0]["rules"][0]["input"], "vix_open");
        assert_eq!(conditions[0]["rules"][0]["comparison"]["Input"], "spx_prior_close");

        let mut templates = json!([{"Name":"Wide Width","PreferenceCall":"Narrow Width","StopOrderType":"StopLimitOCO","Adjustment2Type":"ProfitTarget2"}]);
        upgrade_templates(&mut templates);
        let template = &templates[0];
        assert_eq!(template["name"], "Wide Width", "a name typed by the person is not a list value");
        assert_eq!(
            (&template["preference_call"], &template["stop_order_type"], &template["adjustment2_type"]),
            (&json!("Narrowest"), &json!("Limit stop with OCO"), &json!("Target 2"))
        );

        let adjustments = upgrade_adjustments(r#"{"items":[{"kind":"Profit Level","order_type":"StopLimit","change_type":"Stop Multiple"}]}"#);
        assert!(adjustments.contains("Profit percent") && adjustments.contains("Limit stop") && adjustments.contains("Stop factor"), "{adjustments}");
        assert_eq!(upgrade_adjustments("not json"), "not json");
    }
}
