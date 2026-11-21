//! The trade template: every setting of an entry and its exit orders, the file format templates are exported and imported in, and
//! the operations on a list of templates (what is shown, new ids, import, upgrading templates saved by earlier versions). Reading and
//! writing the files is the job of the app.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Whether the DTE of custom legs 1 to 3 may be adjusted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomLegDte {
    /// The `custom_leg1_dte_adjust` switch.
    #[serde(rename = "custom_leg1_dte_adjust")]
    pub cleg1_dte_adjust: bool,
    /// The `custom_leg2_dte_adjust` switch.
    #[serde(rename = "custom_leg2_dte_adjust")]
    pub cleg2_dte_adjust: bool,
    /// The `custom_leg3_dte_adjust` switch.
    #[serde(rename = "custom_leg3_dte_adjust")]
    pub cleg3_dte_adjust: bool,
}

/// Whether the DTE of custom leg 4, of the trade and of the short call may be adjusted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegDteSwitchesA {
    /// The `custom_leg4_dte_adjust` switch.
    #[serde(rename = "custom_leg4_dte_adjust")]
    pub cleg4_dte_adjust: bool,
    /// The `dte_adjust` switch.
    #[serde(rename = "dte_adjust")]
    pub dte_adjust: bool,
    /// The `short_call_dte_adjust` switch.
    #[serde(rename = "short_call_dte_adjust")]
    pub short_call_dte_adjust: bool,
}

/// Whether the DTE of the short put, the long call and the long put may be adjusted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegDteSwitchesB {
    /// The `short_put_dte_adjust` switch.
    #[serde(rename = "short_put_dte_adjust")]
    pub short_put_dte_adjust: bool,
    /// The `long_call_dte_adjust` switch.
    #[serde(rename = "long_call_dte_adjust")]
    pub long_call_dte_adjust: bool,
    /// The `long_put_dte_adjust` switch.
    #[serde(rename = "long_put_dte_adjust")]
    pub long_put_dte_adjust: bool,
}

/// What triggers the automatic re-entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReEnterSwitches {
    /// The `reentry_close` switch.
    #[serde(rename = "reentry_close")]
    pub re_enter_close: bool,
    /// The `reentry_stop` switch.
    #[serde(rename = "reentry_stop")]
    pub re_enter_stop: bool,
    /// The `reentry_profit_target` switch.
    #[serde(rename = "reentry_profit_target")]
    pub re_enter_profit_target: bool,
}

/// The switches of the long strikes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LongSwitches {
    /// The `disable_narrower_long` switch.
    #[serde(rename = "disable_narrower_long")]
    pub disable_narrower_long: bool,
    /// The `disable_narrower_long_call` switch.
    #[serde(rename = "disable_narrower_long_call")]
    pub disable_narrower_long_call: bool,
    /// The `long_reuse_only` switch.
    #[serde(rename = "long_reuse_only")]
    pub long_reuse_only: bool,
}

/// The switches of the orders: extended hours and all or none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OrderSwitches {
    /// The switch for the stop outside regular trading hours.
    #[serde(rename = "extended_hour_stop")]
    pub extended_hour_stop: bool,
    /// The switch for the profit target outside regular trading hours.
    #[serde(rename = "extended_hour_target")]
    pub extended_hour_pt: bool,
    /// The `all_or_none` switch.
    #[serde(rename = "all_or_none")]
    pub all_or_none: bool,
}

/// The switches of the stop: REL, the ITM-only rule and the guardrail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StopSwitches {
    /// The `stop_rel` switch.
    #[serde(rename = "stop_rel")]
    pub stop_rel: bool,
    /// The `stop_rel_itm_only` switch.
    #[serde(rename = "stop_rel_itm_only")]
    pub stop_rel_itm_only: bool,
    /// The `stop_guardrail` switch.
    #[serde(rename = "stop_guardrail")]
    pub stop_guardrail: bool,
}

/// The switches of the exit and the entry conflicts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExitSwitches {
    /// The `exit_order_limit` switch.
    #[serde(rename = "exit_order_limit")]
    pub exit_order_limit: bool,
    /// The `force_close_all_legs` switch.
    #[serde(rename = "force_close_all_legs")]
    pub force_close_all_legs: bool,
    /// The `allow_strike_conflict` switch.
    #[serde(rename = "allow_strike_conflict")]
    pub allow_strike_conflict: bool,
}

/// The switches of the template list: lower target and deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListSwitches {
    /// The `lower_target` switch.
    #[serde(rename = "lower_target")]
    pub lower_target: bool,
    /// The `is_deleted` switch.
    #[serde(rename = "is_deleted")]
    pub is_deleted: bool,
}

/// One template. Decimal values map to `f64`, `int` to `i32`,
/// nullable values to `Option`; a missing or `null` JSON field takes the type default (see
/// `strip_nulls`).
// The fields are the properties of the template file format; the template editor names and explains each one.
#[allow(missing_docs)]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TradeTemplate {
    pub id: i32,
    pub name: String,
    pub trade_type: String,
    pub target_type: String,
    pub target_type_call: String,
    pub target_min: f64,
    pub target_min_call: Option<f64>,
    pub target_max: f64,
    pub target_max_call: Option<f64>,
    pub long_type: String,
    pub long_type_call: String,
    pub long_width: String,
    pub long_width_call: String,
    pub long_max_premium: Option<f64>,
    pub long_max_premium_call: Option<f64>,
    pub quantity_default: i32,
    pub fill_attempts: i32,
    pub fill_wait: i32,
    pub fill_adjustment: f64,
    pub stop_type: String,
    pub stop_multiple: f64,
    pub stop_offset: f64,
    pub stop_trigger: i32,
    pub stop_order_type: String,
    pub stop_target_type: String,
    pub stop_rel_offset: Option<f64>,
    pub stop_rel_limit: Option<f64>,
    pub stop_limit_offset: Option<f64>,
    pub stop_limit_market_offset: Option<f64>,
    pub order_id_profit_target: Option<i32>,
    pub profit_target_type: String,
    pub profit_target: Option<f64>,
    pub adjustment1_type: String,
    pub adjustment1: Option<f64>,
    pub adjustment1_change_type: String,
    pub adjustment1_change: Option<f64>,
    pub adjustment1_change_offset: Option<f64>,
    pub adjustment1_hour: i32,
    pub adjustment1_minute: i32,
    pub adjustment1_order_type: String,
    pub adjustment2_type: String,
    pub adjustment2: Option<f64>,
    pub adjustment2_change_type: String,
    pub adjustment2_change: Option<f64>,
    pub adjustment2_change_offset: Option<f64>,
    pub adjustment2_hour: i32,
    pub adjustment2_minute: i32,
    pub adjustment2_order_type: String,
    pub exit_hour: i32,
    pub exit_minute: i32,
    pub stop_basis: String,
    pub stop_rel_itm: Option<i32>,
    pub stop_rel_itm_minutes: Option<i32>,
    pub long_max_width: Option<f64>,
    pub long_max_width_call: Option<f64>,
    pub exit_minutes_in_trade: i32,
    pub preference: String,
    pub preference_call: String,
    pub adjustment3_type: String,
    pub adjustment3: Option<f64>,
    pub adjustment3_change_type: String,
    pub adjustment3_change: Option<f64>,
    pub adjustment3_change_offset: Option<f64>,
    pub adjustment3_hour: i32,
    pub adjustment3_minute: i32,
    pub adjustment3_order_type: String,
    pub reentry_close_template_id: i32,
    pub reentry_stop_template_id: i32,
    pub reentry_profit_target_template_id: i32,
    pub reentry_close_template_id2: i32,
    pub reentry_stop_template_id2: i32,
    pub reentry_profit_target_template_id2: i32,
    pub reentry_delay: i32,
    pub reentry_expiration_hour: i32,
    pub reentry_expiration_minute: i32,
    pub reentry_max_entries: i32,
    pub strategy: String,
    pub min_otm: f64,
    pub min_otm_call: Option<f64>,
    pub short_put_target: f64,
    pub short_put_target_type: String,
    pub short_put_dte: i32,
    pub short_call_target: f64,
    pub short_call_target_type: String,
    pub short_call_dte: i32,
    pub long_put_target: f64,
    pub long_put_target_type: String,
    pub long_put_dte: i32,
    pub long_call_target: f64,
    pub long_call_target_type: String,
    pub long_call_dte: i32,
    pub exit_dte: i32,
    pub put_ratio: i32,
    pub call_ratio: i32,
    pub long_min_premium: Option<f64>,
    pub long_min_premium_call: Option<f64>,
    pub profit_target_trade_percent: Option<f64>,
    pub profit_target2: Option<f64>,
    pub profit_target2_trade_percent: Option<f64>,
    pub profit_target3: Option<f64>,
    pub profit_target3_trade_percent: Option<f64>,
    pub profit_target4: Option<f64>,
    pub profit_target4_trade_percent: Option<f64>,
    pub max_entry_price: f64,
    pub min_entry_price: f64,
    pub underlying_symbol: String,
    pub exit_condition_id: i32,
    pub excess_long_behavior: i32,
    pub profit_target_expiration_hour: i32,
    pub profit_target_expiration_minute: i32,
    pub custom_leg1_action: String,
    pub custom_leg1_put_call: String,
    pub custom_leg1_ratio: i32,
    pub custom_leg1_target: f64,
    pub custom_leg1_target_type: String,
    pub custom_leg1_dte: i32,
    pub custom_leg2_action: String,
    pub custom_leg2_put_call: String,
    pub custom_leg2_ratio: i32,
    pub custom_leg2_target: f64,
    pub custom_leg2_target_type: String,
    pub custom_leg2_dte: i32,
    pub custom_leg3_action: String,
    pub custom_leg3_put_call: String,
    pub custom_leg3_ratio: i32,
    pub custom_leg3_target: f64,
    pub custom_leg3_target_type: String,
    pub custom_leg3_dte: i32,
    pub custom_leg4_action: String,
    pub custom_leg4_put_call: String,
    pub custom_leg4_ratio: i32,
    pub custom_leg4_target: f64,
    pub custom_leg4_target_type: String,
    pub custom_leg4_dte: i32,
    pub min_entry_sl_ratio: f64,
    pub max_entry_sl_ratio: f64,
    pub upgrade_flag: f64,
    pub stop_guardrail_offset_open: i32,
    pub stop_guardrail_offset_exp: i32,
    /// Whether the DTE of custom legs 1 to 3 may be adjusted.
    #[serde(flatten)]
    pub leg_dte_custom: CustomLegDte,
    /// Whether the DTE of custom leg 4, of the trade and of the short call may be adjusted.
    #[serde(flatten)]
    pub leg_dte_a: LegDteSwitchesA,
    /// Whether the DTE of the short put, the long call and the long put may be adjusted.
    #[serde(flatten)]
    pub leg_dte_b: LegDteSwitchesB,
    /// What triggers the automatic re-entry.
    #[serde(flatten)]
    pub reenter: ReEnterSwitches,
    /// The switches of the long strikes.
    #[serde(flatten)]
    pub long_flags: LongSwitches,
    /// The switches of the orders: extended hours and all or none.
    #[serde(flatten)]
    pub order_flags: OrderSwitches,
    /// The switches of the stop: REL, the ITM-only rule and the guardrail.
    #[serde(flatten)]
    pub stop_flags: StopSwitches,
    /// The switches of the exit and the entry conflicts.
    #[serde(flatten)]
    pub exit_flags: ExitSwitches,
    /// The switches of the template list: lower target and deleted.
    #[serde(flatten)]
    pub list_flags: ListSwitches,
}

/// The `.tpt` export file (JSON). This app
/// has no conditions or schedules yet, so those lists are carried as raw JSON and left empty on
/// export.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfigExport {
    /// The exported templates.
    pub templates: Vec<TradeTemplate>,
    /// Conditions as raw JSON.
    pub conditions: Vec<Value>,
    /// Condition details as raw JSON.
    pub condition_details: Vec<Value>,
    /// Schedules as raw JSON.
    pub schedules: Vec<Value>,
    /// Old condition id to new id.
    pub condition_id_map: Option<HashMap<String, i32>>,
    /// Old template id to new id.
    pub template_id_map: Option<HashMap<String, i32>>,
    /// Old schedule master id to new id.
    pub schedule_master_id_map: Option<HashMap<String, i32>>,
    /// The hour (New York) the market closes.
    pub market_close_hour: i32,
}

/// What an import did: how many templates came in and why others did not.
#[derive(Debug, Default, PartialEq)]
pub struct ImportReport {
    /// Templates that came in.
    pub imported: usize,
    /// Templates whose "Early Exit Condition" was dropped: conditions are not implemented here yet.
    pub dropped_exit_conditions: usize,
    /// Templates left out because an identical one (same name and settings) already exists.
    pub skipped_duplicates: usize,
    /// New names of templates imported under a different name because the name was taken.
    pub renamed: Vec<String>,
}

/// The templates as shown in the list: not deleted, a real id, sorted by
/// name ignoring case.
#[must_use]
pub fn visible_templates(all: &[TradeTemplate]) -> Vec<&TradeTemplate> {
    let mut list: Vec<&TradeTemplate> = all.iter().filter(|t| !t.list_flags.is_deleted && t.id > 0).collect();
    list.sort_by_key(|t| t.name.to_lowercase());
    list
}

/// The id a new template gets: one above the highest in `all`.
#[must_use]
pub fn next_template_id(all: &[TradeTemplate]) -> i32 {
    all.iter().map(|t| t.id).max().unwrap_or(0).max(0) + 1
}

/// Inserts or replaces (by `id`) a template saved from the editor; a new template
/// (id 0) gets the next free id. Returns the id it was saved under.
pub fn upsert_template(all: &mut Vec<TradeTemplate>, mut template: TradeTemplate) -> i32 {
    if template.id <= 0 {
        template.id = next_template_id(all);
    }
    let id = template.id;
    match all.iter_mut().find(|t| t.id == id) {
        Some(existing) => *existing = template,
        None => all.push(template),
    }
    id
}

/// A template plus, recursively, every template its re-entry
/// settings point at, each only once.
fn add_template_to_export(id: i32, all: &[TradeTemplate], export: &mut ConfigExport) {
    let Some(template) = all.iter().find(|t| t.id == id) else { return };
    if export.templates.iter().any(|t| t.id == id) {
        return;
    }
    export.templates.push(template.clone());
    for linked in [
        template.reentry_close_template_id,
        template.reentry_close_template_id2,
        template.reentry_profit_target_template_id,
        template.reentry_profit_target_template_id2,
        template.reentry_stop_template_id,
        template.reentry_stop_template_id2,
    ] {
        if linked > 0 {
            add_template_to_export(linked, all, export);
        }
    }
}

/// The export of the templates with the ids `selected_ids` (and what they refer to) for `.tpt` files.
#[must_use]
pub fn build_export(all: &[TradeTemplate], selected_ids: &[i32], market_close_hour: i32) -> ConfigExport {
    let mut export = ConfigExport { market_close_hour, ..ConfigExport::default() };
    for id in selected_ids {
        add_template_to_export(*id, all, &mut export);
    }
    export
}

/// The template without what differs between otherwise identical copies: its id,
/// its re-entry links and its list flags.
fn comparable(template: &TradeTemplate) -> TradeTemplate {
    let mut t = template.clone();
    t.id = 0;
    t.reentry_close_template_id = 0;
    t.reentry_close_template_id2 = 0;
    t.reentry_profit_target_template_id = 0;
    t.reentry_profit_target_template_id2 = 0;
    t.reentry_stop_template_id = 0;
    t.reentry_stop_template_id2 = 0;
    t.list_flags = ListSwitches::default();
    t
}

/// Imports an export file for an IB account: every template is added
/// under a new id (existing ones are never overwritten), exit/profit-target-cancel hours are
/// shifted by the difference between this app's market close hour and the exporter's, and
/// re-entry links are re-pointed at the newly assigned ids. Finishes with `upgrade_templates`.
pub fn import_templates(all: &mut Vec<TradeTemplate>, export: ConfigExport, market_close_hour: i32) -> ImportReport {
    let shift = if export.market_close_hour > 0 { market_close_hour - export.market_close_hour } else { 0 };
    let mut report = ImportReport::default();
    let mut id_map: HashMap<i32, i32> = HashMap::new();
    let mut imported_ids = Vec::new();
    for mut template in export.templates {
        if template.exit_condition_id > 0 {
            template.exit_condition_id = 0;
            report.dropped_exit_conditions += 1;
        }
        if template.exit_hour > 0 {
            template.exit_hour += shift;
        }
        if template.profit_target_expiration_hour > 0 {
            template.profit_target_expiration_hour += shift;
        }
        let old_id = template.id;
        let wanted = comparable(&template);
        let existing = all.iter().find(|t| !t.list_flags.is_deleted && t.id > 0 && comparable(t) == wanted);
        if let Some(existing) = existing {
            id_map.insert(old_id, existing.id);
            report.skipped_duplicates += 1;
            continue;
        }
        if all.iter().any(|t| !t.list_flags.is_deleted && t.name == template.name) {
            let base = template.name.clone();
            template.name = (2..u32::MAX)
                .map(|n| format!("{base} ({n})"))
                .find(|candidate| !all.iter().any(|t| !t.list_flags.is_deleted && &t.name == candidate))
                .unwrap_or(base);
            report.renamed.push(template.name.clone());
        }
        let new_id = next_template_id(all);
        template.id = new_id;
        id_map.insert(old_id, new_id);
        imported_ids.push(new_id);
        all.push(template);
        report.imported += 1;
    }
    let remap = |id: i32| if id > 0 { id_map.get(&id).copied().unwrap_or(0) } else { id };
    for template in all.iter_mut().filter(|t| imported_ids.contains(&t.id)) {
        template.reentry_close_template_id = remap(template.reentry_close_template_id);
        template.reentry_close_template_id2 = remap(template.reentry_close_template_id2);
        template.reentry_profit_target_template_id = remap(template.reentry_profit_target_template_id);
        template.reentry_profit_target_template_id2 = remap(template.reentry_profit_target_template_id2);
        template.reentry_stop_template_id = remap(template.reentry_stop_template_id);
        template.reentry_stop_template_id2 = remap(template.reentry_stop_template_id2);
    }
    upgrade_templates(all);
    report
}

/// The current text of a value that an older version wrote.
fn current_value(text: &str) -> String {
    crate::legacy_names::upgraded_value(text).unwrap_or(text).to_string()
}

/// Brings templates written by older versions
/// up to the current field conventions.
pub fn upgrade_templates(all: &mut [TradeTemplate]) {
    for item in all.iter_mut() {
        if item.upgrade_flag < 4.0 {
            item.target_type = item.target_type.replace("Credit", "Premium");
            item.target_type_call = item.target_type_call.replace("Credit", "Premium");
            if item.target_type == "PointsFromOpen" {
                item.target_type = "Offset from open".to_string();
            }
            if item.target_type_call == "PointsFromOpen" {
                item.target_type_call = "Offset from open".to_string();
            }
            item.preference = current_value(&item.preference.replace("Credit", "Premium"));
            item.preference_call = current_value(&item.preference_call.replace("Credit", "Premium"));
            if item.long_type_call.is_empty() {
                item.long_type_call = item.long_type.clone();
            }
            if item.long_width_call.is_empty() {
                item.long_width_call = item.long_width.clone();
            }
            if item.long_min_premium_call.is_none() {
                item.long_min_premium_call = item.long_min_premium;
            }
            if item.long_max_premium_call.is_none() {
                item.long_max_premium_call = item.long_max_premium;
            }
            if item.long_max_width_call.is_none() {
                item.long_max_width_call = item.long_max_width;
            }
            item.long_flags.disable_narrower_long_call = item.long_flags.disable_narrower_long;
            item.upgrade_flag = 4.0;
        }
        if item.upgrade_flag < 5.0 {
            let has_pt = !item.profit_target_type.is_empty() && item.profit_target_type != "None";
            let has_stop = !item.stop_order_type.is_empty() && item.stop_order_type != "None";
            if item.exit_flags.allow_strike_conflict && (has_pt || has_stop) {
                item.exit_flags.allow_strike_conflict = false;
            }
            item.upgrade_flag = 5.0;
        }
        if item.upgrade_flag < 6.0 {
            item.stop_guardrail_offset_exp = 10;
            item.stop_guardrail_offset_open = 10;
            item.upgrade_flag = 6.0;
        }
        if item.underlying_symbol.is_empty() {
            item.underlying_symbol = "SPX".to_string();
        }
    }
}
