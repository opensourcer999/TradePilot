//! The Conditions page and the conditions file: the list, the editor and the snapshot a condition is checked against.

use super::{
    card_bg, condition_editor, condition_summary, conditions_path, data_table_rows, dec, early_close, empty_state, evaluate_condition, load_conditions,
    muted_text, next_selection, session_minutes, stored_record, upsert_condition, write_to, App, ConditionForm, ConditionState, Decimal, QuoteRow, RichText,
    Snapshot, TableCell, TableColumn, Verdict, RED,
};
use eframe::egui;

impl App {
    /// Loads the conditions of a mode (`condition::load_conditions`); a file that could not be loaded is
    /// reported in the log and on the Conditions page instead of ending up as an empty list that the next
    /// save would write over.
    pub(super) fn load_mode_conditions(&mut self, live: bool) {
        let loaded = load_conditions(live, &self.clock.local().format("%Y%m%d-%H%M%S").to_string());
        self.conditions = ConditionState { list: loaded.items, load_problem: loaded.problem, save_blocked: loaded.save_blocked, ..ConditionState::default() };
        if let Some(problem) = &self.conditions.load_problem {
            let message = format!("WARNING: conditions: {problem}");
            self.log(&message);
        }
    }

    /// Writes the conditions to their file (not in tests, and never over a file that could not be read).
    pub(super) fn save_conditions(&mut self) {
        if self.conditions.save_blocked {
            let message = "Conditions NOT saved: the conditions file could not be read and would be overwritten - repair or move it, then restart";
            self.log(&format!("WARNING: {message}"));
            self.conditions.status = Some(message.to_string());
            return;
        }
        if cfg!(test) {
            return;
        }
        let result = conditions_path(self.link.trading_live).and_then(|path| write_to(&path, &self.conditions.list));
        if let Err(error) = result {
            let message = format!("The conditions could not be saved: {error}");
            self.log(&format!("WARNING: {message}"));
            self.conditions.status = Some(message);
        }
    }

    /// "Save Condition": stores the condition, or keeps the page open with the faulty fields outlined.
    pub(super) fn save_condition_form(&mut self, form: ConditionForm) {
        let other_names: Vec<String> = self.conditions.list.iter().filter(|c| c.id != form.id).map(|c| c.name.clone()).collect();
        match form.to_condition(&other_names) {
            Ok(condition) => {
                let name = condition.name.clone();
                let id = upsert_condition(&mut self.conditions.list, condition);
                self.conditions.selected = Some(id);
                self.conditions.errors.clear();
                self.save_conditions();
                self.log(&format!("Condition {id} saved: {name}"));
            }
            Err(errors) => {
                for error in &errors {
                    self.log(&format!("Condition not saved - {}", error.message));
                }
                self.conditions.errors = errors;
                self.conditions.error_scroll = true;
                self.conditions.editing = Some(form);
            }
        }
    }

    /// Opens the Add/Edit page for the condition `id`.
    fn edit_condition(&mut self, id: u32, duplicate: bool) {
        let Some(condition) = self.conditions.list.iter().find(|c| c.id == id) else {
            return;
        };
        self.conditions.editing = Some(ConditionForm::from_condition(condition, duplicate));
        self.conditions.errors.clear();
    }

    /// Deletes a condition, unless a schedule still uses it (then the status line says which).
    pub(super) fn delete_condition(&mut self, id: u32) {
        if let Some(schedule) = self.schedule.list.iter().find(|s| s.condition_id == id) {
            let message =
                format!("The condition is used by the schedule {} and is not deleted - choose another condition there first", self.schedule_label(schedule));
            self.log(&message);
            self.conditions.status = Some(message);
            return;
        }
        if let Some(template) = self.templates.iter().find(|t| !t.list_flags.is_deleted && i64::from(t.exit_condition_id) == i64::from(id)) {
            let message = format!(
                "The condition is the early exit condition of the template {} and is not deleted - choose another condition there first",
                template.name
            );
            self.log(&message);
            self.conditions.status = Some(message);
            return;
        }
        self.conditions.list.retain(|c| c.id != id);
        self.conditions.selected = None;
        self.save_conditions();
        self.log(&format!("Condition {id} deleted"));
    }

    /// The total delta of every open trade, for the delta totals of the conditions. Every open trade is valued, so
    /// `with_bars` asks for it only when a rule of the condition uses a delta total.
    pub(super) fn open_deltas(&self) -> Vec<crate::condition::OpenDelta> {
        self.trades_stored
            .iter()
            .filter(|t| t.is_open())
            .map(|t| crate::condition::OpenDelta {
                account: t.account.clone(),
                symbol: t.symbol.clone(),
                total: stored_record(t, &self.chains).and_then(|record| dec(record.total_delta)).unwrap_or(Decimal::ZERO),
            })
            .collect()
    }

    /// What the conditions look at, as far as the app has it: the quotes of SPX, VIX and `VIX9D` and the
    /// time of the session. A zero or missing quote is unknown.
    pub(super) fn condition_snapshot(&self) -> Snapshot {
        let quote = |label: &str, pick: fn(&QuoteRow) -> Option<f64>| {
            self.quotes.iter().find(|q| q.label == label).and_then(pick).and_then(dec).filter(|value| *value > Decimal::ZERO)
        };
        let now = self.clock.new_york();
        let (from_open, to_close) = session_minutes(now, early_close(now.date()));
        Snapshot {
            vix: quote("VIX", |q| q.last),
            vix_open: quote("VIX", |q| q.open),
            vix_prior_close: quote("VIX", |q| q.close),
            vix9d: quote("VIX9D", |q| q.last),
            spx: quote("SPX", |q| q.last),
            spx_open: quote("SPX", |q| q.open),
            spx_prior_close: quote("SPX", |q| q.close),
            minutes_from_open: Some(Decimal::from(from_open)),
            minutes_to_close: Some(Decimal::from(to_close)),
            spy: self.chains.price("SPY").and_then(dec).filter(|price| *price > Decimal::ZERO),
            qqq: self.chains.price("QQQ").and_then(dec).filter(|price| *price > Decimal::ZERO),
            ..Snapshot::default()
        }
    }

    /// Checks the condition of a schedule for its run `run_id`; every line of the check goes to the log.
    /// A condition that does not exist is false (no trade). Prices that TWS no longer sends (see
    /// `market_data_warning`) are not used: the answer is `Retry`.
    pub(super) fn check_schedule_condition(&mut self, condition_id: u32, run_id: u32, symbol: &str, account: &str) -> Verdict {
        let tag = format!("Schedule run {run_id}");
        let Some(condition) = self.conditions.list.iter().find(|c| c.id == condition_id).cloned() else {
            self.log(&format!("{tag}: condition {condition_id} does not exist - treated as false"));
            return Verdict::Fail;
        };
        if let Some(warning) = self.market_data_warning(self.clock.instant(), self.market_is_open()) {
            self.log(&format!("{tag}: condition {} not checked, the prices are not current ({warning})", condition.name));
            return Verdict::Retry;
        }
        let snapshot = Snapshot { account: account.to_string(), ..self.condition_snapshot() };
        let snapshot = self.with_bars(snapshot, &condition, symbol);
        let evaluation = evaluate_condition(&condition, &snapshot);
        for line in &evaluation.lines {
            self.log(&format!("{tag}: {line}"));
        }
        evaluation.verdict
    }

    /// The Conditions page: the list, or the Add/Edit page.
    pub(super) fn conditions_page(&mut self, ui: &mut egui::Ui) {
        if let Some(mut form) = self.conditions.editing.take() {
            ui.heading(RichText::new(self.tr("Trade Conditions", "Trade-Bedingungen")).color(self.heading_color()));
            ui.add_space(8.0);
            let errors = self.conditions.errors.clone();
            let scroll_to_error = std::cell::Cell::new(self.conditions.error_scroll);
            let data = condition_editor::EditorData { errors: &errors, scroll_to_error: &scroll_to_error };
            let outcome = condition_editor::editor_page(ui, &mut form, &data);
            self.conditions.error_scroll = scroll_to_error.get();
            match outcome {
                condition_editor::EditorOutcome::Keep => self.conditions.editing = Some(form),
                condition_editor::EditorOutcome::Save => self.save_condition_form(form),
                condition_editor::EditorOutcome::Cancel => self.conditions.errors.clear(),
            }
            return;
        }
        ui.heading(RichText::new(self.tr("Trade Conditions", "Trade-Bedingungen")).color(self.heading_color()));
        ui.add_space(8.0);
        if let Some(problem) = &self.conditions.load_problem {
            ui.colored_label(
                RED,
                format!("{}: {problem}", self.tr("The conditions file could not be loaded", "Die Bedingungs-Datei konnte nicht geladen werden")),
            );
            ui.add_space(6.0);
        }
        let selected = self.conditions.selected.filter(|id| self.conditions.list.iter().any(|c| c.id == *id));
        egui::Frame::none().fill(card_bg()).inner_margin(egui::vec2(10.0, 8.0)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add(crate::widgets::MetalButton::new(self.tr("+ Add", "+ Hinzufügen"))).clicked() {
                    self.conditions.editing = Some(ConditionForm::new_default());
                    self.conditions.errors.clear();
                }
                if ui.add_enabled(selected.is_some(), crate::widgets::MetalButton::new(self.tr("Edit", "Bearbeiten"))).clicked() {
                    if let Some(id) = selected {
                        self.edit_condition(id, false);
                    }
                }
                if ui.add_enabled(selected.is_some(), crate::widgets::MetalButton::new(self.tr("Copy", "Kopieren"))).clicked() {
                    if let Some(id) = selected {
                        self.edit_condition(id, true);
                    }
                }
                if ui.add_enabled(selected.is_some(), crate::widgets::MetalButton::new(self.tr("Delete", "Löschen"))).clicked() {
                    if let Some(id) = selected {
                        self.delete_condition(id);
                    }
                }
            });
        });
        if let Some(status) = &self.conditions.status {
            ui.colored_label(muted_text(), status.as_str());
        }
        ui.add_space(8.0);
        if self.conditions.list.is_empty() {
            empty_state(
                ui,
                self.tr("No saved trade conditions yet", "Noch keine gespeicherten Bedingungen"),
                self.tr(
                    "Use Add to create one. A schedule can check it before it enters its trade.",
                    "Mit Hinzufügen eine erstellen. Ein Zeitplan kann sie prüfen, bevor er seinen Trade eingibt.",
                ),
            );
            return;
        }
        let columns = [
            TableColumn::new("Name", 2.0, false),
            TableColumn::new(self.tr("Rules", "Regeln"), 5.0, false),
            TableColumn::new(self.tr("Used by", "Verwendet von"), 1.2, true),
        ];
        let rows: Vec<Vec<TableCell>> = self
            .conditions
            .list
            .iter()
            .map(|c| {
                let used = self.schedule.list.iter().filter(|s| s.condition_id == c.id).count();
                vec![TableCell::new(c.name.clone()), TableCell::new(condition_summary(c)), TableCell::new(used.to_string())]
            })
            .collect();
        let position = selected.and_then(|id| self.conditions.list.iter().position(|c| c.id == id));
        let clicks = data_table_rows(ui, "conditions_table", &columns, &rows, position);
        let id_of = |index: Option<usize>| index.and_then(|i| self.conditions.list.get(i)).map(|c| c.id);
        self.conditions.selected = next_selection(self.conditions.selected, clicks, |index| id_of(Some(index)));
        if let Some(id) = id_of(clicks.double_clicked) {
            self.edit_condition(id, false);
        }
    }
}
