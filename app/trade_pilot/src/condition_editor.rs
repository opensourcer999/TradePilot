//! The Add/Edit page of a condition: the cards with their
//! fields. What the fields mean and how they are checked is in `condition_form`; this module only lays
//! them out, outlines every faulty field in red and says which button was clicked.

use crate::condition::{Input, Operator};
use crate::condition_form::{ConditionError, ConditionField, ConditionForm};
use crate::widgets::{form_number, form_text, form_toggle, labeled, navy_bar, validated, white_card};
use eframe::egui;

/// What the person did on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorOutcome {
    /// Nothing yet: the page stays open.
    Keep,
    /// "Save Condition" was clicked.
    Save,
    /// "Cancel" was clicked.
    Cancel,
}

/// The faults of the last save and whether the first faulty field still has to be scrolled into view.
pub struct EditorData<'a> {
    /// The faults of the last save, at most one per field.
    pub errors: &'a [ConditionError],
    /// Whether the first faulty field still has to be scrolled into view.
    pub scroll_to_error: &'a std::cell::Cell<bool>,
}

impl EditorData<'_> {
    /// The message of the last save's fault on `field`, if there is one.
    fn error_for(&self, field: ConditionField) -> Option<&str> {
        self.errors.iter().find(|e| e.field == field).map(|e| e.message.as_str())
    }
}

/// A drop-down over `options`; `name` gives the text of one.
fn pick<T: Copy + PartialEq>(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut T, options: &[T], name: fn(T) -> &'static str, width: f32) {
    crate::widgets::metal_combo(ui, |ui| {
        egui::ComboBox::from_id_salt(id).width(width).selected_text(name(*value)).show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(value, *option, name(*option));
            }
        })
    });
}

/// The whole page: the cards, then the Save and Cancel buttons.
pub fn editor_page(ui: &mut egui::Ui, form: &mut ConditionForm, data: &EditorData) -> EditorOutcome {
    let mut outcome = EditorOutcome::Keep;
    egui::ScrollArea::vertical().id_salt("condition_editor_scroll").show(ui, |ui| {
        white_card(ui, |ui| {
            navy_bar(ui, "Condition");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                labeled(ui, "Name", |ui| {
                    validated(ui, data.error_for(ConditionField::Name), data.scroll_to_error, |ui| form_text(ui, &mut form.name, 280.0));
                });
                labeled(ui, "If no group is true", |ui| {
                    form_toggle(ui, &mut form.retry_until_expiration, "Keep checking until the window ends", "Give up at once")
                });
            });
        });
        ui.add_space(10.0);
        rules_card(ui, form, data);
        ui.add_space(14.0);
        for error in data.errors {
            ui.colored_label(crate::theme::negative_text(), &error.message);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(crate::widgets::MetalButton::new("Save Condition")).clicked() {
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

/// The "Rules" card: one row per rule.
fn rules_card(ui: &mut egui::Ui, form: &mut ConditionForm, data: &EditorData) {
    white_card(ui, |ui| {
        navy_bar(ui, "Rules");
        ui.add_space(6.0);
        ui.label("Rules of the same group must all be true; the condition is true if any group is. Start another group by typing another group number.");
        ui.add_space(8.0);
        if form.rules.is_empty() {
            validated(ui, data.error_for(ConditionField::Rules), data.scroll_to_error, |ui| {
                ui.label("No rules yet.");
            });
        }
        let mut remove = None;
        egui::Grid::new("condition_rules_grid").spacing([10.0, 8.0]).show(ui, |ui| {
            for title in ["Group", "Value", "", "Compare with", "", ""] {
                ui.label(title);
            }
            ui.end_row();
            for (index, row) in form.rules.iter_mut().enumerate() {
                validated(ui, data.error_for(ConditionField::Rule(index)), data.scroll_to_error, |ui| {
                    form_number(ui, &mut row.group, 40.0, &crate::number_lists::GROUPS);
                });
                ui.horizontal(|ui| {
                    pick(ui, ("rule_input", index), &mut row.input, &Input::ALL, Input::name, 230.0);
                    if row.input.takes_length() {
                        ui.label("bars");
                        form_number(ui, &mut row.input_length, 50.0, &crate::number_lists::BAR_COUNTS);
                    }
                });
                pick(ui, ("rule_operator", index), &mut row.operator, &Operator::ALL, Operator::sign, 50.0);
                let mut with_input = row.compare_with_input;
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt(("rule_kind", index)).width(110.0).selected_text(if with_input { "a value" } else { "a number" }).show_ui(
                        ui,
                        |ui| {
                            ui.selectable_value(&mut with_input, false, "a number");
                            ui.selectable_value(&mut with_input, true, "a value");
                        },
                    )
                });
                row.compare_with_input = with_input;
                if row.compare_with_input {
                    ui.horizontal(|ui| {
                        pick(ui, ("rule_other", index), &mut row.comparison_input, &Input::ALL, Input::name, 230.0);
                        if row.comparison_input.takes_length() {
                            ui.label("bars");
                            form_number(ui, &mut row.comparison_length, 50.0, &crate::number_lists::BAR_COUNTS);
                        }
                    });
                } else {
                    form_text(ui, &mut row.comparison_value, 100.0);
                }
                if ui.add(crate::widgets::MetalButton::new("Remove")).clicked() {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });
        if let Some(index) = remove {
            form.remove_rule(index);
        }
        ui.add_space(6.0);
        if ui.add(crate::widgets::MetalButton::new("+ Add rule")).clicked() {
            form.add_rule();
        }
    });
}
