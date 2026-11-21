//! The Add/Edit page of a trigger: the cards of the schedule page that fit (what to enter, the days), a card for the window of
//! the day and the limits, and one for the attempts and the condition. What the fields mean and how they are checked is in
//! `trigger_form`; this module only lays them out.

use crate::schedule_editor::{choice_with_none, days_card, what_card, EditorData, EditorOutcome};
use crate::schedule_form::{apply_picker, picker_of, ScheduleField};
use crate::trigger_form::TriggerForm;
use crate::widgets::{form_combo, form_text, labeled, navy_bar, time_picker, white_card};
use eframe::egui;

/// The whole page: the cards, then the Save and Cancel buttons. `webhook` says how a signal names the trigger, for the card.
pub fn editor_page(ui: &mut egui::Ui, form: &mut TriggerForm, data: &EditorData, webhook: &str) -> EditorOutcome {
    let mut outcome = EditorOutcome::Keep;
    egui::ScrollArea::vertical().id_salt("trigger_editor_scroll").show(ui, |ui| {
        what_card(ui, &mut form.base, data);
        ui.add_space(10.0);
        window_card(ui, form, data, webhook);
        ui.add_space(10.0);
        days_card(ui, &mut form.base, data);
        ui.add_space(10.0);
        attempts_card(ui, form, data);
        ui.add_space(14.0);
        for error in data.errors {
            ui.colored_label(crate::theme::negative_text(), &error.message);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(crate::widgets::MetalButton::new("Save Trigger")).clicked() {
                outcome = EditorOutcome::Save;
            }
            if ui.add(crate::widgets::MetalButton::new("Cancel")).clicked() {
                outcome = EditorOutcome::Cancel;
            }
        });
        ui.add_space(14.0);
    });
    outcome
}

/// The time picker over a typed hour (0-23) and minute.
fn picker(ui: &mut egui::Ui, id: &str, hour: &mut String, minute: &mut String) {
    let mut picked = picker_of(hour, minute);
    let before = picked.clone();
    time_picker(ui, id, &mut picked.hour, &mut picked.minute, &mut picked.pm);
    if picked != before {
        apply_picker(hour, minute, &picked);
    }
}

/// The "When" card: the window of the day (New York time), how long an entry may take, the limits and the strategy.
fn window_card(ui: &mut egui::Ui, form: &mut TriggerForm, data: &EditorData, webhook: &str) {
    white_card(ui, |ui| {
        navy_bar(ui, "When a Signal Counts");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Window start (New York)", |ui| {
                data.checked(ui, ScheduleField::Time, |ui| picker(ui, "trigger_start", &mut form.base.hour, &mut form.base.minute));
            });
            labeled(ui, "Window end (New York)", |ui| {
                data.checked(ui, ScheduleField::Window, |ui| picker(ui, "trigger_end", &mut form.end_hour, &mut form.end_minute));
            });
            labeled(ui, "Entry may take (minutes)", |ui| {
                data.number(ui, &mut form.base.expiration_minutes, 70.0, ScheduleField::ExpirationMinutes, &crate::number_lists::WINDOW_MINUTES);
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Max entries a day (0 = no limit)", |ui| {
                data.number(ui, &mut form.max_executions, 70.0, ScheduleField::MaxExecutions, &crate::number_lists::COUNTS);
            });
            labeled(ui, "Max open trades (0 = no limit)", |ui| data.number(ui, &mut form.max_open, 70.0, ScheduleField::MaxOpen, &crate::number_lists::COUNTS));
            labeled(ui, "Strategy (empty = that of the template)", |ui| form_text(ui, &mut form.strategy, 180.0));
        });
        ui.add_space(8.0);
        labeled(ui, "Entry Trigger", |ui| {
            form_combo(ui, "trigger_entry", &mut form.entry, &crate::trigger::ENTRY_TRIGGERS, 200.0);
        });
        if form.entry == crate::trigger::ENTRY_TRIGGERS[0] {
            ui.label(webhook);
        } else {
            ui.label("An EMA crossover is watched on the one-minute SPX bars while the window is open; each crossover starts one entry.");
        }
    });
}

/// The "Attempts and Condition" card.
fn attempts_card(ui: &mut egui::Ui, form: &mut TriggerForm, data: &EditorData) {
    white_card(ui, |ui| {
        navy_bar(ui, "Attempts and Condition");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Max attempts (0 = until the time is up)", |ui| {
                data.number(ui, &mut form.base.max_attempts, 70.0, ScheduleField::Attempts, &crate::number_lists::ATTEMPTS);
            });
            labeled(ui, "Alternate template", |ui| {
                data.checked(ui, ScheduleField::AlternateTemplate, |ui| {
                    choice_with_none(ui, "trigger_alternate", &mut form.base.alternate_template_id, data.templates, "None", 260.0);
                });
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Condition (must be true before the trade is entered)", |ui| {
                data.checked(ui, ScheduleField::Condition, |ui| {
                    choice_with_none(ui, "trigger_condition", &mut form.base.condition_id, data.conditions, "None", 260.0);
                });
            });
            labeled(ui, "Template if the condition is false (None = cancel)", |ui| {
                data.checked(ui, ScheduleField::ConditionFailureTemplate, |ui| {
                    choice_with_none(ui, "trigger_condition_failure", &mut form.base.condition_failure_template_id, data.templates, "None", 260.0);
                });
            });
        });
    });
}
