//! The cards of the Add/Edit Trade Template form. `App::template_editor_page` lays them out in
//! order; each card is a function of the form being edited and what the cards share
//! (`EditorContext`). Labels, sections and which parts show for which selections are defined in
//! `template_form`; the labels are English-only.

use super::{
    custom_target_types, egui, form_combo, form_number, form_text, form_toggle, labeled, muted_text, navy_bar, time_picker, validated, white_card, CalendarLeg,
    CalendarLegForm, Field, LongSideForm, ShortSideForm, Side, TemplateEditorOutcome, TemplateError, TemplateForm, TimeField, ADJUSTMENT_CHANGE_TYPES,
    ADJUSTMENT_ORDER_TYPES, ADJUSTMENT_TRIGGERS, CALENDAR_LONG_TARGET_TYPES, CALENDAR_SHORT_TARGET_TYPES, COUNT_CHOICES, CUSTOM_ACTIONS, CUSTOM_LEG1_ACTIONS,
    EXCESS_LONG_BEHAVIORS, LONG_TYPES, MIN_MAX_DEBIT, PREFERENCES, PROFIT_TARGET_TYPES, PUT_CALL, STOP_BASES, STOP_ORDER_TYPES, STOP_TARGET_TYPES,
    STOP_TRIGGERS, STOP_TYPES, STRIKE_TARGET_TYPES, SYMBOLS, TRADE_TYPES,
};
use super::{Decimal, RichText, ORANGE_WARN};
use crate::template_form::{Part, TemplateLayout};

/// What the cards of the editor share: the errors of the last save (each faulty field is outlined
/// in red and the first one is scrolled to), the layout of the trade type and the names of the
/// other templates (for the re-entries).
pub(super) struct EditorContext<'a> {
    /// The errors of the last save, at most one per field.
    pub errors: &'a [TemplateError],
    /// Whether the first faulty field still has to be scrolled into view.
    pub scroll_to_error: &'a std::cell::Cell<bool>,
    /// Which parts the trade type shows.
    pub layout: &'a TemplateLayout,
    /// The id and name of every saved template.
    pub other_templates: &'a [(i32, String)],
    /// The id and name of every saved trade condition.
    pub conditions: &'a [(u32, String)],
}

impl EditorContext<'_> {
    /// The message of the last save's error on `field`, if there is one.
    fn error_for(&self, field: Field) -> Option<String> {
        self.errors.iter().find(|e| e.field == field).map(|e| e.message.clone())
    }

    /// `checked` with a list of values to pick from.
    fn number(&self, ui: &mut egui::Ui, value: &mut String, width: f32, field: Field, list: &[&str]) {
        validated(ui, self.error_for(field).as_deref(), self.scroll_to_error, |ui| form_number(ui, value, width, list));
    }

    /// `number` for a distance in points that has to land on a strike (a width): with the distance between the strikes TWS reported
    /// (`step`) the list offers only the distances that fit it; without one it offers them all.
    fn strike_distance(&self, ui: &mut egui::Ui, value: &mut String, width: f32, field: Field, list: &[&str], step: Option<Decimal>) {
        let offer = |item: &str| crate::strikes::fits_strike_step_text(step, item);
        validated(ui, self.error_for(field).as_deref(), self.scroll_to_error, |ui| {
            crate::widgets::form_number_offering(ui, value, width, list, &offer);
        });
    }

    /// A text field outlined in red, with its message, when the last save found it faulty.
    fn checked(&self, ui: &mut egui::Ui, value: &mut String, width: f32, field: Field) {
        validated(ui, self.error_for(field).as_deref(), self.scroll_to_error, |ui| form_text(ui, value, width));
    }
}
/// The Template Details card: name, symbol, trade type and Long Reuse Only.
pub(super) fn details_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, "Template Details");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Name", |ui| ctx.checked(ui, &mut form.name, 300.0, Field::Name));
            labeled(ui, "Symbol", |ui| form_combo(ui, "tt_symbol", &mut form.symbol, &SYMBOLS, 120.0));
            let mut types: Vec<&str> = TRADE_TYPES.to_vec();
            if form.trade_type == "Custom" {
                types.push("Custom");
            }
            if labeled(ui, "Trade Type", |ui| form_combo(ui, "tt_trade_type", &mut form.trade_type, &types, 170.0)) {
                form.on_trade_type_changed();
            }
            if ctx.layout.shows(Part::LongReuseOnly) {
                labeled(ui, "Long Reuse Only", |ui| form_toggle(ui, &mut form.entry.long_reuse_only, "On", "Off"));
            }
        });
    });
}

/// The Calendar Setup card: the strike target, DTE and Exact DTE of each calendar leg.
pub(super) fn calendar_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, "Calendar Setup");
        ui.add_space(8.0);
        egui::Grid::new("tt_calendar_grid").spacing([10.0, 8.0]).show(ui, |ui| {
            for h in ["", "", "Strike Target", "DTE", "Exact DTE"] {
                ui.label(h);
            }
            ui.end_row();
            let mut rows: Vec<(&str, &mut CalendarLegForm, bool, CalendarLeg)> = Vec::new();
            if ctx.layout.shows(Part::CalendarPutRows) {
                rows.push(("Short Put", &mut form.short_put, true, CalendarLeg::ShortPut));
                rows.push(("Long Put", &mut form.long_put, false, CalendarLeg::LongPut));
            }
            if ctx.layout.shows(Part::CalendarCallRows) {
                rows.push(("Short Call", &mut form.short_call, true, CalendarLeg::ShortCall));
                rows.push(("Long Call", &mut form.long_call, false, CalendarLeg::LongCall));
            }
            for (label, leg, short, which) in rows {
                ui.label(label);
                ui.add_enabled_ui(short, |ui| {
                    let options: &[&str] = if short { &CALENDAR_SHORT_TARGET_TYPES } else { &CALENDAR_LONG_TARGET_TYPES };
                    form_combo(ui, ("tt_cal_type", label), &mut leg.target_type, options, 120.0);
                });
                ui.add_enabled_ui(short, |ui| {
                    ctx.number(ui, &mut leg.target, 80.0, Field::CalendarTarget(which), crate::number_lists::for_short_target(&leg.target_type));
                });
                ctx.number(ui, &mut leg.dte, 60.0, Field::CalendarDte(which), &crate::number_lists::DTES);
                form_toggle(ui, &mut leg.exact_dte, "On", "Off");
                ui.end_row();
            }
        });
    });
}

/// The Custom Trade Setup card: up to four legs.
pub(super) fn custom_card(ui: &mut egui::Ui, form: &mut TemplateForm) {
    white_card(ui, |ui| {
        navy_bar(ui, "Custom Trade Setup");
        ui.add_space(8.0);
        egui::Grid::new("tt_custom_grid").spacing([10.0, 8.0]).show(ui, |ui| {
            for h in ["", "Action", "Put/Call", "Ratio", "Target", "Target Type", "DTE", "Exact DTE"] {
                ui.label(h);
            }
            ui.end_row();
            for (i, leg) in form.custom_legs.iter_mut().enumerate() {
                let n = i + 1;
                ui.label(format!("Leg {n}"));
                let actions: &[&str] = if n == 1 { &CUSTOM_LEG1_ACTIONS } else { &CUSTOM_ACTIONS };
                form_combo(ui, ("tt_cleg_action", n), &mut leg.action, actions, 80.0);
                if n > 1 && leg.action == "None" {
                    ui.end_row();
                    continue;
                }
                form_combo(ui, ("tt_cleg_pc", n), &mut leg.put_call, &PUT_CALL, 50.0);
                form_number(ui, &mut leg.ratio, 50.0, &crate::number_lists::RATIO);
                form_number(ui, &mut leg.target, 70.0, crate::number_lists::for_custom_target(&leg.target_type));
                let types = custom_target_types(n);
                let type_refs: Vec<&str> = types.iter().map(String::as_str).collect();
                form_combo(ui, ("tt_cleg_type", n), &mut leg.target_type, &type_refs, 150.0);
                form_number(ui, &mut leg.dte, 50.0, &crate::number_lists::DTES);
                form_toggle(ui, &mut leg.exact_dte, "On", "Off");
                ui.end_row();
            }
        });
    });
}

/// The Short Strike Selection card: the put and the call side.
pub(super) fn short_strikes_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, ctx.layout.short_strike_title);
        ui.add_space(8.0);
        let mut sides: Vec<(&str, &str, &mut ShortSideForm, Side)> = Vec::new();
        if ctx.layout.shows(Part::ShortPut) {
            sides.push(("PUT Strike Selection", "Put Ratio", &mut form.short_put_side, Side::Put));
        }
        if ctx.layout.shows(Part::ShortCall) {
            sides.push(("CALL Strike Selection", "Call Ratio", &mut form.short_call_side, Side::Call));
        }
        for (title, ratio_label, side, which) in sides {
            ui.horizontal(|ui| {
                labeled(ui, title, |ui| form_combo(ui, ("tt_target_type", title), &mut side.target_type, &STRIKE_TARGET_TYPES, 170.0));
                labeled(ui, "Target Minimum", |ui| {
                    ctx.number(ui, &mut side.target_min, 80.0, Field::TargetMin(which), crate::number_lists::for_short_target(&side.target_type));
                });
                labeled(ui, "Target Maximum", |ui| {
                    ctx.number(ui, &mut side.target_max, 80.0, Field::TargetMax(which), crate::number_lists::for_short_target(&side.target_type));
                });
                labeled(ui, "Selection Preference", |ui| form_combo(ui, ("tt_pref", title), &mut side.preference, &PREFERENCES, 190.0));
                labeled(ui, "Min Distance OTM", |ui| ctx.number(ui, &mut side.min_otm, 80.0, Field::MinOtm(which), &crate::number_lists::MIN_OTM));
                if ctx.layout.shows(Part::Ratios) {
                    labeled(ui, ratio_label, |ui| ctx.number(ui, &mut side.ratio, 50.0, Field::Ratio(which), &crate::number_lists::RATIO));
                }
            });
            ui.add_space(6.0);
        }
    });
}

/// The Long Strike Selection card: width or premium of the long of each side.
pub(super) fn long_strikes_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, ctx.layout.long_strike_title);
        ui.add_space(8.0);
        let step = form.strike_step;
        // Where the check of the widths stands: what TWS reported, or that nothing is known and so nothing is checked.
        let note = match step {
            Some(step) => format!("{} strikes are {} apart (from the TWS option chain): only widths that fit are offered.", form.symbol, step.normalize()),
            None => format!(
                "Strike distance of {} unknown: TWS has reported no strikes of it in this session (connect, or load its option chain) - the widths are not checked.",
                form.symbol
            ),
        };
        ui.label(RichText::new(note).small().color(if step.is_some() { muted_text() } else { ORANGE_WARN }));
        ui.add_space(4.0);
        let mut sides: Vec<(&str, &mut LongSideForm, Side)> = Vec::new();
        if ctx.layout.shows(Part::LongPut) {
            sides.push(("PUT Selection", &mut form.long_put_side, Side::Put));
        }
        if ctx.layout.shows(Part::LongCall) {
            sides.push(("CALL Selection", &mut form.long_call_side, Side::Call));
        }
        for (title, side, which) in sides {
            ui.horizontal(|ui| {
                labeled(ui, title, |ui| form_combo(ui, ("tt_long_type", title), &mut side.long_type, &LONG_TYPES, 130.0));
                if side.long_type == "Width" {
                    labeled(ui, "Width(s)", |ui| {
                        ctx.strike_distance(ui, &mut side.width, 110.0, Field::LongWidth(which), &crate::number_lists::WIDTHS, step);
                    });
                    labeled(ui, "Use Closer .05 strike width, if available", |ui| form_toggle(ui, &mut side.use_closer_width, "On", "Off"));
                } else {
                    labeled(ui, "Min Premium", |ui| form_number(ui, &mut side.min_premium, 80.0, &crate::number_lists::LONG_PREMIUMS));
                    labeled(ui, "Max Premium", |ui| {
                        ctx.number(ui, &mut side.max_premium, 80.0, Field::LongMaxPremium(which), &crate::number_lists::LONG_PREMIUMS);
                    });
                    labeled(ui, "Max Width", |ui| {
                        ctx.strike_distance(ui, &mut side.max_width, 80.0, Field::LongMaxWidth(which), &crate::number_lists::WIDTHS, step);
                    });
                }
            });
            ui.add_space(6.0);
        }
    });
}

/// The cards of one row: Days To Expiration, Min/Max Entry Price and Short/Long Ratio (each as the trade type shows it).
pub(super) fn entry_cards(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    ui.horizontal(|ui| {
        if ctx.layout.shows(Part::DteAtEntry) {
            white_card(ui, |ui| {
                navy_bar(ui, "Days To Expiration");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    labeled(ui, "DTE at trade entry", |ui| ctx.number(ui, &mut form.dte_all, 80.0, Field::DteAll, &crate::number_lists::DTES));
                    labeled(ui, "Exact DTE", |ui| form_toggle(ui, &mut form.entry.exact_dte, "On", "Off"));
                });
            });
            ui.add_space(10.0);
        }
        white_card(ui, |ui| {
            navy_bar(ui, "Min/Max Entry Price");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                labeled(ui, "Minimum", |ui| ctx.number(ui, &mut form.min_entry_price, 70.0, Field::MinEntryPrice, &crate::number_lists::ENTRY_PRICES));
                labeled(ui, "Maximum", |ui| ctx.number(ui, &mut form.max_entry_price, 70.0, Field::MaxEntryPrice, &crate::number_lists::ENTRY_PRICES));
                if ctx.layout.shows(Part::MinMaxDebit) {
                    labeled(ui, "Debit/Credit", |ui| form_combo(ui, "tt_min_max_debit", &mut form.min_max_debit, &MIN_MAX_DEBIT, 90.0));
                }
            });
        });
        if ctx.layout.shows(Part::SlRatio) {
            ui.add_space(10.0);
            white_card(ui, |ui| {
                navy_bar(ui, "Short/Long Ratio");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    labeled(ui, "Minimum", |ui| form_number(ui, &mut form.min_sl_ratio, 70.0, &crate::number_lists::SHORT_LONG_RATIOS));
                    labeled(ui, "Maximum", |ui| form_number(ui, &mut form.max_sl_ratio, 70.0, &crate::number_lists::SHORT_LONG_RATIOS));
                });
            });
        }
    });
}

/// The Stop Loss Configuration card: order and target type, REL, protections.
pub(super) fn stop_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, "Stop Loss Configuration");
        ui.add_space(8.0);
        ui.colored_label(muted_text(), "Stops  See the IBKR website for more information on stop configuration options.");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if labeled(ui, "Order Type", |ui| form_combo(ui, "tt_stop_order_type", &mut form.stop_order_type, &STOP_ORDER_TYPES, 130.0)) {
                form.on_stop_order_type_changed();
            }
            if ctx.layout.shows(Part::StopLimitOffset) {
                labeled(ui, "Limit Offset", |ui| ctx.number(ui, &mut form.stop_limit_offset, 70.0, Field::StopLimitOffset, &crate::number_lists::OFFSETS));
            }
            if ctx.layout.shows(Part::StopLimitMarketOffset) {
                labeled(ui, "Stop Market Offset", |ui| {
                    ctx.number(ui, &mut form.stop_limit_market_offset, 70.0, Field::StopLimitMarketOffset, &crate::number_lists::OFFSETS);
                });
            }
            if ctx.layout.shows(Part::StopType)
                && labeled(ui, "Stop Type", |ui| {
                    validated(ui, ctx.error_for(Field::StopType).as_deref(), ctx.scroll_to_error, |ui| {
                        form_combo(ui, "tt_stop_type", &mut form.stop_type, &STOP_TYPES, 100.0)
                    })
                })
            {
                form.on_stop_order_type_changed();
            }
            if ctx.layout.shows(Part::StopTargetType) {
                labeled(ui, "Target Type", |ui| form_combo(ui, "tt_stop_target_type", &mut form.stop_target_type, &STOP_TARGET_TYPES, 100.0));
            }
            if ctx.layout.shows(Part::StopMultiple) {
                labeled(ui, "Stop Multiple (net)", |ui| {
                    ctx.number(ui, &mut form.stop_multiple, 70.0, Field::StopMultiple, &crate::number_lists::STOP_MULTIPLES);
                });
            }
            if ctx.layout.shows(Part::StopAmount) {
                labeled(ui, "Stop Amount", |ui| ctx.number(ui, &mut form.stop_amount, 70.0, Field::StopAmount, &crate::number_lists::STOP_AMOUNTS));
            }
            if ctx.layout.shows(Part::StopOffset) {
                labeled(ui, "Trigger Offset (optional)", |ui| ctx.number(ui, &mut form.stop_offset, 70.0, Field::StopOffset, &crate::number_lists::OFFSETS));
            }
            if ctx.layout.shows(Part::StopTrigger) {
                let triggers: Vec<&str> = STOP_TRIGGERS.iter().map(|(label, _)| *label).collect();
                labeled(ui, "Trigger", |ui| form_combo(ui, "tt_stop_trigger", &mut form.stop_trigger, &triggers, 140.0));
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            labeled(ui, "Extended Hours Stop", |ui| {
                validated(ui, ctx.error_for(Field::ExtendedHourStop).as_deref(), ctx.scroll_to_error, |ui| {
                    form_toggle(ui, &mut form.order_flags.extended_hour_stop, "Active", "Not Active")
                })
            });
            if ctx.layout.shows(Part::StrikeConflict)
                && labeled(ui, "Allow Conflicting Strikes", |ui| form_toggle(ui, &mut form.entry.allow_strike_conflict, "Allowed", "Not Allowed"))
            {
                form.on_strike_conflict_toggled();
            }
            if ctx.layout.shows(Part::ExcessLongBehavior) {
                labeled(ui, "Leftover Long Behavior", |ui| {
                    let current = EXCESS_LONG_BEHAVIORS.iter().find(|(_, v)| *v == form.excess_long_behavior).map_or("Ignore", |(l, _)| *l);
                    crate::widgets::metal_combo(ui, |ui| {
                        egui::ComboBox::from_id_salt("tt_excess_long").width(90.0).selected_text(current).show_ui(ui, |ui| {
                            for (label, value) in EXCESS_LONG_BEHAVIORS {
                                ui.selectable_value(&mut form.excess_long_behavior, value, label);
                            }
                        })
                    });
                });
            }
            labeled(ui, "Stop Price Basis", |ui| form_combo(ui, "tt_stop_basis", &mut form.stop_basis, &STOP_BASES, 120.0));
        });
        rel_panel(ui, form, ctx);
    });
}

/// The REL panel of the Stop Loss Configuration card: the REL stop, its protections and the guardrail.
fn rel_panel(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    if ctx.layout.shows(Part::RelPanel) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if labeled(ui, "Use REL order for Short Stop Market?", |ui| form_toggle(ui, &mut form.rel_flags.rel, "On", "Off")) {
                form.on_protection_toggled();
            }
            if ctx.layout.shows(Part::RelFields) {
                labeled(ui, "REL Offset", |ui| ctx.number(ui, &mut form.rel_offset, 70.0, Field::RelOffset, &crate::number_lists::OFFSETS));
                labeled(ui, "REL Limit Price", |ui| form_number(ui, &mut form.rel_limit, 70.0, &crate::number_lists::REL_LIMITS));
                if labeled(ui, "Use ITM Protection (REL)", |ui| form_toggle(ui, &mut form.rel_flags.rel_itm_protection, "On", "Off")) {
                    form.on_protection_toggled();
                }
            }
        });
        if ctx.layout.shows(Part::RelItmFields) {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                labeled(ui, "ITM Protection Distance", |ui| ctx.number(ui, &mut form.rel_itm, 70.0, Field::RelItm, &crate::number_lists::ITM_DISTANCES));
                labeled(ui, "Minutes Before Close", |ui| form_number(ui, &mut form.rel_itm_minutes, 70.0, &crate::number_lists::MINUTES_BEFORE_CLOSE));
                labeled(ui, "ITM Protection Only", |ui| form_toggle(ui, &mut form.rel_flags.rel_itm_only, "On", "Off"));
            });
        }
        if ctx.layout.shows(Part::Guardrail) {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if labeled(ui, "Use Guardrail (Volatility Spike Protection - REL)", |ui| form_toggle(ui, &mut form.guardrail, "On", "Off")) {
                    form.on_protection_toggled();
                }
                if ctx.layout.shows(Part::GuardrailOffsets) {
                    labeled(ui, "Offset at Trade Open", |ui| {
                        ctx.number(ui, &mut form.guardrail_offset_open, 60.0, Field::GuardrailOffsetOpen, &crate::number_lists::GUARDRAIL_OFFSETS);
                    });
                    labeled(ui, "Offset at Expiration", |ui| {
                        ctx.number(ui, &mut form.guardrail_offset_exp, 60.0, Field::GuardrailOffsetExp, &crate::number_lists::GUARDRAIL_OFFSETS);
                    });
                }
            });
        }
    }
}

/// The Stop Loss Adjustments card: the three adjustments.
pub(super) fn adjustments_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, "Stop Loss Adjustments");
        ui.add_space(8.0);
        for (i, adjustment) in form.adjustments.iter_mut().enumerate() {
            let n = i + 1;
            ui.horizontal(|ui| {
                labeled(ui, &format!("Adjustment {n} Trigger"), |ui| {
                    form_combo(ui, ("tt_adj_trigger", n), &mut adjustment.trigger, &ADJUSTMENT_TRIGGERS, 120.0)
                });
                labeled(ui, "Trigger", |ui| {
                    ctx.number(
                        ui,
                        &mut adjustment.value,
                        60.0,
                        Field::AdjustmentValue(i),
                        if adjustment.trigger == "Profit percent" { &crate::number_lists::ADJUSTMENT_PERCENTS } else { &[] },
                    );
                });
                labeled(ui, "Adjustment Type", |ui| form_combo(ui, ("tt_adj_change_type", n), &mut adjustment.change_type, &ADJUSTMENT_CHANGE_TYPES, 120.0));
                labeled(ui, "New Stop", |ui| {
                    ctx.number(
                        ui,
                        &mut adjustment.change,
                        60.0,
                        Field::AdjustmentChange(i),
                        if adjustment.change_type == "Stop factor" { &crate::number_lists::NEW_STOP_MULTIPLES } else { &[] },
                    );
                });
                labeled(ui, "Offset", |ui| form_number(ui, &mut adjustment.change_offset, 60.0, &crate::number_lists::OFFSETS));
                labeled(ui, "Change Stop Type", |ui| form_combo(ui, ("tt_adj_order_type", n), &mut adjustment.order_type, &ADJUSTMENT_ORDER_TYPES, 120.0));
                labeled(ui, "Activation Time", |ui| {
                    let time = &mut adjustment.activation;
                    time_picker(ui, &format!("tt_adj_time_{n}"), &mut time.hour, &mut time.minute, &mut time.pm);
                });
            });
            ui.add_space(6.0);
        }
    });
}

/// The Profit Target card: type, value, levels and the cancel time.
pub(super) fn profit_target_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, "Profit Target");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Profit Target Type", |ui| {
                validated(ui, ctx.error_for(Field::ProfitTargetType).as_deref(), ctx.scroll_to_error, |ui| {
                    form_combo(ui, "tt_pt_type", &mut form.profit_target_type, &PROFIT_TARGET_TYPES, 110.0)
                })
            });
            labeled(ui, "Target Amount", |ui| {
                ctx.number(
                    ui,
                    &mut form.profit_target,
                    70.0,
                    Field::ProfitTarget,
                    if form.profit_target_type == "Percentage" { &crate::number_lists::TARGET_PERCENTS } else { &[] },
                );
            });
            if form.exit_flags.split_profit_targets {
                labeled(ui, "% to Close", |ui| {
                    ctx.number(ui, &mut form.profit_target_pct[0], 60.0, Field::ProfitTargetPct(0), &crate::number_lists::SHARES_TO_CLOSE);
                });
                let tier_list: &[&str] = if form.profit_target_type == "Percentage" { &crate::number_lists::TARGET_PERCENTS } else { &[] };
                let tiers = form.profit_targets_2_to_4.iter_mut().zip(form.profit_target_pct.iter_mut().skip(1));
                for (i, ((target, pct), label)) in tiers.zip(["Target 2", "Target 3", "Target 4"]).enumerate() {
                    labeled(ui, label, |ui| form_number(ui, target, 60.0, tier_list));
                    labeled(ui, "% to Close", |ui| ctx.number(ui, pct, 60.0, Field::ProfitTargetPct(i + 1), &crate::number_lists::SHARES_TO_CLOSE));
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            labeled(ui, "Split Profit Targets", |ui| form_toggle(ui, &mut form.exit_flags.split_profit_targets, "Split PT", "Single PT"));
            labeled(ui, "Extended Hours PT", |ui| form_toggle(ui, &mut form.order_flags.extended_hour_pt, "Active", "Not Active"));
            labeled(ui, "Profit Target Cancel", |ui| {
                let time = &mut form.profit_target_cancel;
                time_picker(ui, "tt_pt_cancel", &mut time.hour, &mut time.minute, &mut time.pm);
            });
        });
    });
}

/// The Timed Exit card: exit time, minutes in the trade, DTE and the exit options.
pub(super) fn timed_exit_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, "Timed Exit");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Minutes In Trade", |ui| ctx.number(ui, &mut form.exit_minutes, 70.0, Field::ExitMinutes, &crate::number_lists::MINUTES_IN_TRADE));
            labeled(ui, "Exit Time", |ui| {
                ui.horizontal(|ui| {
                    let time = &mut form.exit_time;
                    time_picker(ui, "tt_exit_time", &mut time.hour, &mut time.minute, &mut time.pm);
                    if ui.add(crate::widgets::MetalButton::new("Reset")).clicked() {
                        form.exit_time = TimeField::default();
                    }
                });
            });
            labeled(ui, "DTE", |ui| ctx.number(ui, &mut form.exit_dte, 50.0, Field::ExitDte, &crate::number_lists::DTES));
            labeled(ui, "Early Exit Condition", |ui| {
                let chosen = ctx.conditions.iter().find(|(id, _)| i64::from(*id) == i64::from(form.exit_condition_id)).map_or_else(
                    || if form.exit_condition_id == 0 { "None".to_string() } else { format!("#{}", form.exit_condition_id) },
                    |(_, name)| name.clone(),
                );
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("tt_exit_condition").width(160.0).selected_text(chosen).show_ui(ui, |ui| {
                        ui.selectable_value(&mut form.exit_condition_id, 0, "None");
                        for (id, name) in ctx.conditions {
                            ui.selectable_value(&mut form.exit_condition_id, i32::try_from(*id).unwrap_or(0), name);
                        }
                    })
                });
            });
            labeled(ui, "Closing Order", |ui| form_toggle(ui, &mut form.exit_flags.exit_order_limit, "Limit (progressive)", "Market"));
            labeled(ui, "Close All Legs", |ui| form_toggle(ui, &mut form.exit_flags.force_close_all_legs, "On", "Off"));
        });
    });
}

/// The Automatic Trade Re-Entry card.
pub(super) fn reentry_card(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    white_card(ui, |ui| {
        navy_bar(ui, "Automatic Trade Re-Entry");
        ui.add_space(8.0);
        let template_name = |id: i32, zero: &str| -> String {
            if id == 0 {
                zero.to_string()
            } else {
                ctx.other_templates.iter().find(|(tid, _)| *tid == id).map_or_else(|| format!("#{id}"), |(_, name)| name.clone())
            }
        };
        egui::Grid::new("tt_reentry_grid").spacing([10.0, 8.0]).show(ui, |ui| {
            for h in ["", "", "Re-Entry Template 1", "Re-Entry Template 2"] {
                ui.label(h);
            }
            ui.end_row();
            for (label, enabled, ids) in [
                ("After Stop", &mut form.reenter_flags.reenter_stop, &mut form.reenter_stop_ids),
                ("After Close", &mut form.reenter_flags.reenter_close, &mut form.reenter_close_ids),
                ("After Profit Target", &mut form.reenter_flags.reenter_profit_target, &mut form.reenter_profit_target_ids),
            ] {
                ui.label(label);
                form_toggle(ui, enabled, "On", "Off");
                // Re-Entry Template 1 offers "Same Template" (id 0), Template 2 "Disabled" (id 0).
                for (slot, zero) in ids.iter_mut().zip(["Same Template", "Disabled"]) {
                    let current = template_name(*slot, zero);
                    crate::widgets::metal_combo(ui, |ui| {
                        egui::ComboBox::from_id_salt(("tt_reentry", label, zero)).width(200.0).selected_text(current).show_ui(ui, |ui| {
                            ui.selectable_value(slot, 0, zero);
                            for (id, name) in ctx.other_templates {
                                ui.selectable_value(slot, *id, name.as_str());
                            }
                        })
                    });
                }
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            labeled(ui, "Delay Minutes", |ui| form_number(ui, &mut form.reenter_delay, 70.0, &crate::number_lists::DELAY_MINUTES));
            labeled(ui, "Max ReEntries", |ui| {
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("tt_reentry_max").width(60.0).selected_text(form.reenter_max.to_string()).show_ui(ui, |ui| {
                        for n in COUNT_CHOICES {
                            ui.selectable_value(&mut form.reenter_max, n, n.to_string());
                        }
                    })
                });
            });
            labeled(ui, "Don't Re-Enter After", |ui| {
                let time = &mut form.reenter_expiration;
                time_picker(ui, "tt_reentry_expiration", &mut time.hour, &mut time.minute, &mut time.pm);
            });
        });
    });
}

/// The cards of the last row: Fill Progression and Strategy Tracking.
pub(super) fn fill_and_strategy_cards(ui: &mut egui::Ui, form: &mut TemplateForm, ctx: &EditorContext) {
    ui.horizontal(|ui| {
        white_card(ui, |ui| {
            navy_bar(ui, "Fill Progression");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                labeled(ui, "# of Attempts", |ui| {
                    crate::widgets::metal_combo(ui, |ui| {
                        egui::ComboBox::from_id_salt("tt_fill_attempts").width(80.0).selected_text(form.fill_attempts.to_string()).show_ui(ui, |ui| {
                            for n in COUNT_CHOICES {
                                ui.selectable_value(&mut form.fill_attempts, n, n.to_string());
                            }
                        })
                    });
                });
                labeled(ui, "Wait (seconds)", |ui| ctx.number(ui, &mut form.fill_wait, 70.0, Field::FillWait, &crate::number_lists::WAIT_SECONDS));
                labeled(ui, "Adjustment", |ui| ctx.number(ui, &mut form.fill_adjustment, 70.0, Field::FillAdjustment, &crate::number_lists::FILL_STEPS));
                labeled(ui, "All or None", |ui| form_toggle(ui, &mut form.order_flags.all_or_none, "On", "Off"));
            });
        });
        ui.add_space(10.0);
        white_card(ui, |ui| {
            navy_bar(ui, "Strategy Tracking");
            ui.add_space(8.0);
            labeled(ui, "Strategy Name", |ui| form_text(ui, &mut form.strategy, 180.0));
        });
    });
}

/// The bottom of the form: the errors of the last save, and the Save and
/// Cancel buttons (what was clicked).
pub(super) fn footer(ui: &mut egui::Ui, ctx: &EditorContext) -> TemplateEditorOutcome {
    let mut outcome = TemplateEditorOutcome::Keep;
    if !ctx.errors.is_empty() {
        ui.colored_label(crate::theme::negative_text(), format!("{} field(s) need your attention (marked in red above):", ctx.errors.len()));
        for error in ctx.errors {
            ui.colored_label(crate::theme::negative_text(), format!("- {}", error.message));
        }
        ui.add_space(6.0);
    }
    ui.horizontal(|ui| {
        if ui.add(crate::widgets::MetalButton::new("Save Template")).clicked() {
            outcome = TemplateEditorOutcome::Save;
        }
        if ui.add(crate::widgets::MetalButton::new("Cancel")).clicked() {
            outcome = TemplateEditorOutcome::Cancel;
        }
    });
    outcome
}
