//! The Add/Edit page of a schedule: the cards with their
//! fields. What the fields mean and how they are checked is in `schedule_form`; this module only
//! lays them out, outlines every faulty field in red and says which button was clicked.

use crate::schedule_form::{QuantityKind, ScheduleError, ScheduleField, ScheduleForm};
use crate::widgets::{form_number, form_text, form_toggle, labeled, navy_bar, time_picker, validated, white_card};
use chrono::Weekday;
use eframe::egui;

/// What the person did on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorOutcome {
    /// Nothing yet: the page stays open.
    Keep,
    /// "Save Schedule" was clicked.
    Save,
    /// "Cancel" was clicked.
    Cancel,
}

/// What the page needs to know besides the form.
pub struct EditorData<'a> {
    /// The id and name of every saved template.
    pub templates: &'a [(i32, String)],
    /// The accounts known from TWS (empty while not connected).
    pub accounts: &'a [String],
    /// The id and label of every other schedule.
    pub other_schedules: &'a [(u32, String)],
    /// The id and name of every condition.
    pub conditions: &'a [(u32, String)],
    /// The faults of the last save, at most one per field.
    pub errors: &'a [ScheduleError],
    /// Whether the first faulty field still has to be scrolled into view.
    pub scroll_to_error: &'a std::cell::Cell<bool>,
    /// Minutes the local time of this computer is ahead of New York time now.
    pub local_offset_minutes: i64,
    /// Today in New York: the month the calendars open at.
    pub today: chrono::NaiveDate,
}

impl EditorData<'_> {
    /// The message of the last save's fault on `field`, if there is one.
    pub(crate) fn error_for(&self, field: ScheduleField) -> Option<&str> {
        self.errors.iter().find(|e| e.field == field).map(|e| e.message.as_str())
    }

    /// `add_contents`, outlined in red if the last save found `field` faulty.
    pub(crate) fn checked<R>(&self, ui: &mut egui::Ui, field: ScheduleField, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
        validated(ui, self.error_for(field), self.scroll_to_error, add_contents)
    }

    /// A number field outlined in red, with its message, when the last save found it faulty; it offers a list of values to pick from.
    pub(crate) fn number(&self, ui: &mut egui::Ui, value: &mut String, width: f32, field: ScheduleField, list: &[&str]) {
        self.checked(ui, field, |ui| form_number(ui, value, width, list));
    }
}

/// A drop-down over `options` (value and name); `value` is the chosen one, shown as `none` if it is not among them.
fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, id: &str, value: &mut T, options: &[(T, String)], none: &str, width: f32) {
    let shown = options.iter().find(|(v, _)| v == value).map_or(none, |(_, name)| name.as_str());
    crate::widgets::metal_combo(ui, |ui| {
        egui::ComboBox::from_id_salt(id).width(width).selected_text(shown).show_ui(ui, |ui| {
            for (option, name) in options {
                ui.selectable_value(value, *option, name.as_str());
            }
        })
    });
}

/// The whole page: the cards, then the Save and Cancel buttons.
pub fn editor_page(ui: &mut egui::Ui, form: &mut ScheduleForm, data: &EditorData) -> EditorOutcome {
    let mut outcome = EditorOutcome::Keep;
    egui::ScrollArea::vertical().id_salt("schedule_editor_scroll").show(ui, |ui| {
        what_card(ui, form, data);
        ui.add_space(10.0);
        when_card(ui, form, data);
        ui.add_space(10.0);
        days_card(ui, form, data);
        ui.add_space(10.0);
        attempts_card(ui, form, data);
        ui.add_space(14.0);
        for error in data.errors {
            ui.colored_label(crate::theme::negative_text(), &error.message);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(crate::widgets::MetalButton::new("Save Schedule")).clicked() {
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

/// The "What to enter" card: name, account, template, quantity and Active.
pub(crate) fn what_card(ui: &mut egui::Ui, form: &mut ScheduleForm, data: &EditorData) {
    white_card(ui, |ui| {
        navy_bar(ui, "What to Enter");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Name", |ui| form_text(ui, &mut form.name, 220.0));
            labeled(ui, "Account", |ui| {
                data.checked(ui, ScheduleField::Account, |ui| {
                    if data.accounts.is_empty() {
                        form_text(ui, &mut form.account, 150.0);
                    } else {
                        let accounts: Vec<(String, String)> = data.accounts.iter().map(|a| (a.clone(), a.clone())).collect();
                        choice_text(ui, &mut form.account, &accounts, 150.0);
                    }
                });
            });
            labeled(ui, "Template", |ui| {
                data.checked(ui, ScheduleField::Template, |ui| choice(ui, "sched_template", &mut form.template_id, data.templates, "Choose a template", 260.0));
            });
            labeled(ui, "Active", |ui| form_toggle(ui, &mut form.active, "On", "Off"));
            labeled(ui, "Report when expired", |ui| form_toggle(ui, &mut form.notify_on_expiry, "On", "Off"));
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Quantity", |ui| {
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("sched_quantity_kind").width(220.0).selected_text(form.quantity_kind.label()).show_ui(ui, |ui| {
                        for kind in QuantityKind::ALL {
                            ui.selectable_value(&mut form.quantity_kind, kind, kind.label());
                        }
                    })
                });
            });
            data.checked(ui, ScheduleField::Quantity, |ui| quantity_fields(ui, form));
        });
    });
}

/// A drop-down over accounts (text values).
fn choice_text(ui: &mut egui::Ui, value: &mut String, options: &[(String, String)], width: f32) {
    crate::widgets::metal_combo(ui, |ui| {
        egui::ComboBox::from_id_salt("sched_account").width(width).selected_text(value.as_str()).show_ui(ui, |ui| {
            for (option, name) in options {
                ui.selectable_value(value, option.clone(), name.as_str());
            }
        })
    });
}

/// The fields of the quantity that belong to the chosen kind.
fn quantity_fields(ui: &mut egui::Ui, form: &mut ScheduleForm) {
    ui.horizontal(|ui| match form.quantity_kind {
        QuantityKind::Fixed => {
            labeled(ui, "Contracts", |ui| form_number(ui, &mut form.count, 70.0, &crate::number_lists::CONTRACTS));
        }
        QuantityKind::PremiumAllocation | QuantityKind::PctOfNetLiq => {
            let caption = if form.quantity_kind == QuantityKind::PremiumAllocation { "Budget ($)" } else { "Percent" };
            labeled(ui, caption, |ui| {
                form_number(
                    ui,
                    &mut form.amount,
                    90.0,
                    if form.quantity_kind == QuantityKind::PremiumAllocation {
                        &crate::number_lists::PREMIUM_BUDGET
                    } else {
                        &crate::number_lists::NET_LIQ_PERCENT
                    },
                );
            });
            labeled(ui, "Max (0 = no limit)", |ui| form_number(ui, &mut form.max, 70.0, &crate::number_lists::COUNTS));
            labeled(ui, "Min", |ui| form_number(ui, &mut form.min, 70.0, &crate::number_lists::CONTRACTS));
        }
    });
}

/// The "When" card: time of day (New York), the minutes of the window.
fn when_card(ui: &mut egui::Ui, form: &mut ScheduleForm, data: &EditorData) {
    white_card(ui, |ui| {
        navy_bar(ui, "When");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Time zone", |ui| {
                let mut local = form.time_is_local;
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("sched_time_zone")
                        .width(170.0)
                        .selected_text(if local { "Local time (this computer)" } else { "New York time" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut local, false, "New York time");
                            ui.selectable_value(&mut local, true, "Local time (this computer)");
                        })
                });
                form.switch_time_zone(local, data.local_offset_minutes);
            });
            labeled(ui, "Time", |ui| {
                data.checked(ui, ScheduleField::Time, |ui| {
                    let mut picked = form.picker_time();
                    let before = picked.clone();
                    time_picker(ui, "sched_time", &mut picked.hour, &mut picked.minute, &mut picked.pm);
                    if picked != before {
                        form.set_picker_time(&picked);
                    }
                });
            });
            labeled(ui, "Second", |ui| form_number(ui, &mut form.second, 50.0, &crate::number_lists::SECONDS));
            labeled(ui, "Window (minutes after the time)", |ui| {
                data.number(ui, &mut form.expiration_minutes, 70.0, ScheduleField::ExpirationMinutes, &crate::number_lists::WINDOW_MINUTES);
            });
        });
        let (other, zone) = if form.time_is_local { ("New York", "") } else { ("Local", " (this computer)") };
        if let Some(time) = form.other_zone_time(data.local_offset_minutes) {
            ui.label(format!("= {time} {other} time{zone} today. The schedule is stored in New York time (the time of the exchange)."));
        }
    });
}

/// The "Days" card: the weekdays, the blackout dates and the exact dates.
pub(crate) fn days_card(ui: &mut egui::Ui, form: &mut ScheduleForm, data: &EditorData) {
    white_card(ui, |ui| {
        navy_bar(ui, "Days");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (name, day) in [("Mon", Weekday::Mon), ("Tue", Weekday::Tue), ("Wed", Weekday::Wed), ("Thu", Weekday::Thu), ("Fri", Weekday::Fri)] {
                let mut on = form.days.contains(day);
                if crate::widgets::metal_checkbox(ui, &mut on, name).changed() {
                    form.days = form.days.with(day, on);
                }
            }
            ui.label("(none chosen = Monday to Friday)");
        });
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            labeled(ui, "Blackout dates (no trade on these days; text that is no date is a comment)", |ui| {
                crate::widgets::metal_field(ui, egui::TextEdit::multiline(&mut form.blackout_dates).desired_rows(3).desired_width(340.0));
                crate::calendar::calendar_button(ui, "sched_blackout_calendar", &mut form.blackout_dates, data.today);
            });
            labeled(ui, "Exact dates (only these days; overrides everything above)", |ui| {
                data.checked(ui, ScheduleField::Dates, |ui| {
                    crate::widgets::metal_field(ui, egui::TextEdit::multiline(&mut form.exact_dates).desired_rows(3).desired_width(340.0));
                });
            });
        });
        ui.label("Dates as YYYY-MM-DD or DD-MM-YYYY, separated by commas or lines.");
    });
}

/// The "Attempts and Links" card: maximum attempts, alternate template and linked schedule.
fn attempts_card(ui: &mut egui::Ui, form: &mut ScheduleForm, data: &EditorData) {
    white_card(ui, |ui| {
        navy_bar(ui, "Attempts, Links and Condition");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Max attempts (0 = until the window ends)", |ui| {
                data.number(ui, &mut form.max_attempts, 70.0, ScheduleField::Attempts, &crate::number_lists::ATTEMPTS);
            });
            labeled(ui, "Alternate template", |ui| {
                data.checked(ui, ScheduleField::AlternateTemplate, |ui| {
                    choice_with_none(ui, "sched_alternate", &mut form.alternate_template_id, data.templates, "None", 260.0);
                });
            });
            labeled(ui, "Only after schedule", |ui| {
                data.checked(ui, ScheduleField::LinkedSchedule, |ui| {
                    choice_with_none(ui, "sched_linked", &mut form.linked_schedule_id, data.other_schedules, "None", 260.0);
                });
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            labeled(ui, "Condition (must be true before the trade is entered)", |ui| {
                data.checked(ui, ScheduleField::Condition, |ui| {
                    choice_with_none(ui, "sched_condition", &mut form.condition_id, data.conditions, "None", 260.0);
                });
            });
            labeled(ui, "Template if the condition is false (None = cancel)", |ui| {
                data.checked(ui, ScheduleField::ConditionFailureTemplate, |ui| {
                    choice_with_none(ui, "sched_condition_failure", &mut form.condition_failure_template_id, data.templates, "None", 260.0);
                });
            });
        });
    });
}

/// A drop-down with "None" (0) in front of `options`.
pub(crate) fn choice_with_none<T: Copy + PartialEq + Default>(ui: &mut egui::Ui, id: &str, value: &mut T, options: &[(T, String)], none: &str, width: f32) {
    let mut all = vec![(T::default(), none.to_string())];
    all.extend(options.iter().cloned());
    choice(ui, id, value, &all, none, width);
}
