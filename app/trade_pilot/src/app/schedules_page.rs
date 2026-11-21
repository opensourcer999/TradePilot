//! The Schedules page and the runs of the schedules: the list, the sets, import and export, the editor, and each run's attempts.

use super::{
    card_bg, choose_template, data_table_rows, editor_page, empty_state, expire_runs, exports_dir, form_text, group_name, linked_done, mask_account,
    muted_text, navy_bar, next_run, next_selection, run_of, runs_path, save_active_group, schedule_export, schedules_path, setup_runs,
    template_strategy_and_name, trade_entry, upsert, visible_templates, white_card, write_to, App, CheckContext, Decimal, Duration, EditorData, EditorOutcome,
    EntrySelection, Page, PendingTemplateOrder, Plan, RichText, RunStatus, Schedule, ScheduleForm, Skip, TableCell, TableColumn, TemplateChoice, TradeTemplate,
    Verdict, GROUPS, RED,
};
use eframe::egui;

impl App {
    /// Brings the runs of today up to date (once a second): makes the run of every schedule that
    /// runs today, lets a run whose window is over expire, and saves the runs when something changed.
    pub(super) fn update_schedule_runs(&mut self) {
        let instant = self.clock.instant();
        if !crate::clock::due(&mut self.schedule.runs_checked_at, instant, Duration::from_secs(1)) {
            return;
        }
        let (now, today) = (self.clock.utc(), self.clock.new_york().date());
        let shown = self.shown_schedules();
        let mut changed = setup_runs(&mut self.schedule.runs, &shown, today, now);
        for id in expire_runs(&mut self.schedule.runs, now) {
            self.log(&format!("Schedule run {id} expired: its window ended without an entry"));
            self.notify_expired_run(id);
            changed = true;
        }
        if changed {
            self.save_schedule_runs();
        }
    }

    /// Tries the next run of the schedules (every 2 seconds, one at a time): see `attempt_schedule_run`.
    pub(super) fn run_schedules(&mut self) {
        let instant = self.clock.instant();
        if !crate::clock::due(&mut self.schedule.runner_checked_at, instant, Duration::from_secs(2)) {
            return;
        }
        // The daily profit target is reached: no entry is made any more today.
        if self.daily_target_reached_today() {
            return;
        }
        // One entry at a time: a run with an order in TWS is finished (or sent back) by its order status.
        if self.schedule.runs.iter().any(|run| run.status == RunStatus::Working) {
            return;
        }
        let Some(index) = next_run(&self.schedule.runs, self.clock.utc()) else {
            return;
        };
        self.attempt_schedule_run(index);
        self.save_schedule_runs();
    }

    /// One attempt of the run at `index`: the window is open and the run is waiting. The schedule
    /// enters its trade through the same steps as a template trade entered by hand - selection from
    /// the live chain, the checks of the safety gate and the risk limit, the order, the fill attempts,
    /// stop and profit target - only without the confirmation window: the schedule is the user's
    /// approval. Every outcome is logged with the run's number:
    /// - not connected: the run keeps waiting (and no attempt is counted);
    /// - no trade found yet: the run keeps waiting, the next attempt is at least 5 seconds later;
    /// - the schedule is gone or switched off, the template is missing, the attempts are used up, no
    ///   contracts, the safety gate or the risk check refuses, the order cannot be sent: the run is cancelled;
    /// - the order is sent: the run is working until TWS reports the order filled (`Complete`) or over (waiting again).
    fn attempt_schedule_run(&mut self, index: usize) {
        let now = self.clock.utc();
        let Some(run) = self.schedule.runs.get_mut(index) else {
            return;
        };
        run.started = Some(now);
        let run = run.clone();
        let Some(schedule) = self.schedule_of_run(&run) else {
            self.end_schedule_run(index, RunStatus::Cancelled, "the schedule does not exist any more");
            return;
        };
        if !schedule.active {
            self.end_schedule_run(index, RunStatus::Cancelled, "the schedule is switched off");
            return;
        }
        if !linked_done(&self.schedule.runs, &schedule, run.day) {
            self.note_schedule_run(index, "waiting: the linked schedule has not entered its trade yet");
            return;
        }
        let template_id = match choose_template(&schedule, run.attempts) {
            TemplateChoice::Main => schedule.template_id,
            TemplateChoice::Alternate => {
                self.log(&format!("Schedule run {}: using the alternate template after {} attempts", run.id, run.attempts));
                schedule.alternate_template_id
            }
            TemplateChoice::GiveUp => {
                let last = if run.notes.is_empty() { String::new() } else { format!(" - the last attempt: {}", run.notes) };
                self.end_schedule_run(index, RunStatus::Cancelled, &format!("the maximum of {} attempts is reached{last}", schedule.max_attempts));
                return;
            }
        };
        if !self.templates.iter().any(|t| t.id == template_id && !t.list_flags.is_deleted) {
            self.end_schedule_run(index, RunStatus::Cancelled, &format!("template {template_id} does not exist (or is deleted)"));
            return;
        }
        if self.tws_cmd_sender.is_none() || self.next_order_id.is_none() {
            self.note_schedule_run(index, "waiting: not connected to TWS");
            return;
        }
        let Some(template_id) = self.schedule_condition_step(index, &schedule, run.id, template_id) else {
            return;
        };
        if let Some(run) = self.schedule.runs.get_mut(index) {
            run.attempts += 1;
        }
        let tag = format!("Schedule run {}", run.id);
        let mut selection = EntrySelection { account: schedule.account.clone(), test_trade: false, outcome: None };
        let mut lines = Vec::new();
        let preview = self.build_template_preview(Some(template_id), &mut selection, &mut lines);
        for line in lines {
            self.log(&format!("{tag}: {line}"));
        }
        let Some((template, outcome)) = selection.outcome else {
            let first_line = preview.lines().next().unwrap_or_default().to_string();
            self.log(&format!("{tag}: no trade yet - {first_line}"));
            self.note_schedule_run(index, &format!("waiting: no trade yet - {first_line}"));
            return;
        };
        let template = match run.trigger.as_ref().filter(|trigger| !trigger.strategy.is_empty()) {
            Some(trigger) => TradeTemplate { strategy: trigger.strategy.clone(), ..template },
            None => template,
        };
        let Some(entry_price) = outcome.entry_price else {
            self.note_schedule_run(index, "waiting: no entry price yet");
            return;
        };
        let buying_power = trade_entry::buying_power_per_contract(&template.trade_type, &outcome);
        let net_liquidation = self.net_liquidation_of(&schedule.account);
        let Some(qty) = schedule.quantity.quantity(entry_price, buying_power, net_liquidation) else {
            self.end_schedule_run(index, RunStatus::Cancelled, "no contracts to enter: the quantity rule gives none for this trade");
            return;
        };
        self.log(&format!(
            "{tag}: entering {} x{qty} for account {} ({})",
            template_strategy_and_name(&template),
            mask_account(&schedule.account),
            schedule.quantity.label()
        ));
        if let Err(reason) = self.check_template_entry(&schedule.account, qty, &qty.to_string()) {
            self.end_schedule_run(index, RunStatus::Cancelled, &reason);
            return;
        }
        // The order goes out at once: a schedule that holds its order for a review nobody does would be pointless.
        let pending = match Self::build_pending_template_order(template, &outcome, &schedule.account, qty, true) {
            Ok(pending) => PendingTemplateOrder { run_id: Some(run.id), ..pending },
            Err(reason) => {
                self.end_schedule_run(index, RunStatus::Cancelled, &reason);
                return;
            }
        };
        self.template.pending = Some(pending);
        self.confirm_template_order();
        let order_id = self.template.orders.iter().rev().find(|sent| sent.run_id == Some(run.id)).map(|sent| sent.order_id);
        match order_id {
            Some(order_id) => {
                if let Some(run) = self.schedule.runs.get_mut(index) {
                    run.status = RunStatus::Working;
                }
                self.note_schedule_run(index, &format!("order {order_id}: {qty} contracts sent to TWS"));
            }
            None => self.end_schedule_run(index, RunStatus::Cancelled, "the order was not sent (see the log)"),
        }
    }

    /// The condition step of an attempt: a schedule with a condition enters its trade only if the condition is
    /// true. Returns the template to enter, or `None` if the run is left waiting (the condition cannot be
    /// decided yet, or is false and retried until the window ends) or ended (the condition is false and there
    /// is no template for that case, or that template does not exist).
    /// - true: the template that was chosen;
    /// - false: the template for a false condition if the schedule has one, else the run is cancelled;
    /// - cannot be decided: the run keeps waiting and the attempt is not counted.
    fn schedule_condition_step(&mut self, index: usize, schedule: &Schedule, run_id: u32, template_id: i32) -> Option<i32> {
        if schedule.condition_id == 0 {
            return Some(template_id);
        }
        let symbol = self.template(template_id).map_or_else(String::new, |t| t.underlying_symbol.clone());
        match self.check_schedule_condition(schedule.condition_id, run_id, &symbol, &schedule.account) {
            Verdict::Pass => Some(template_id),
            Verdict::Retry => {
                self.note_schedule_run(index, "waiting: the condition is not true yet or cannot be decided");
                None
            }
            Verdict::Fail if schedule.condition_failure_template_id == 0 => {
                self.end_schedule_run(index, RunStatus::Cancelled, "the condition is false");
                None
            }
            Verdict::Fail => {
                let failure = schedule.condition_failure_template_id;
                if !self.templates.iter().any(|t| t.id == failure && !t.list_flags.is_deleted) {
                    self.end_schedule_run(
                        index,
                        RunStatus::Cancelled,
                        &format!("the condition is false and template {failure} for that case does not exist (or is deleted)"),
                    );
                    return None;
                }
                self.log(&format!("Schedule run {run_id}: the condition is false - using template {failure} instead"));
                Some(failure)
            }
        }
    }

    /// The net liquidation value TWS reported for `account`; the override of the settings counts if it is
    /// above 0. `None` if neither is known.
    fn net_liquidation_of(&self, account: &str) -> Option<Decimal> {
        let overridden = self.settings.net_liquidity_override.trim().replace(',', ".").parse::<Decimal>().ok().filter(|value| *value > Decimal::ZERO);
        overridden.or_else(|| {
            self.account_summary
                .iter()
                .find(|(row_account, tag, _, _)| row_account == account && tag == "NetLiquidation")
                .and_then(|(_, _, value, _)| value.trim().parse::<Decimal>().ok())
        })
    }

    /// Ends the run at `index` with a final status and the reason, and logs it.
    pub(super) fn end_schedule_run(&mut self, index: usize, status: RunStatus, reason: &str) {
        let now = self.clock.utc();
        let Some(run) = self.schedule.runs.get_mut(index) else {
            return;
        };
        run.status = status;
        run.finished = Some(now);
        run.notes = reason.to_string();
        let message = format!("Schedule run {} {}: {reason}", run.id, Self::run_status_text_en(status));
        self.log(&message);
    }

    /// Records what the run at `index` is doing now (for the task log) without changing its status.
    fn note_schedule_run(&mut self, index: usize, note: &str) {
        let Some(run) = self.schedule.runs.get_mut(index) else {
            return;
        };
        if run.notes != note {
            run.notes = note.to_string();
            let message = format!("Schedule run {}: {note}", run.id);
            self.log(&message);
        }
    }

    /// The status in English for the log.
    fn run_status_text_en(status: RunStatus) -> &'static str {
        match status {
            RunStatus::Waiting => "waiting",
            RunStatus::Paused => "paused",
            RunStatus::Working => "working",
            RunStatus::Complete => "complete",
            RunStatus::Partial => "partial",
            RunStatus::Expired => "expired",
            RunStatus::Cancelled => "cancelled",
        }
    }

    /// TWS reported the order of a schedule run completely filled: the run is complete.
    pub(super) fn schedule_order_filled(&mut self, run_id: Option<u32>) {
        let Some(index) = run_id.and_then(|id| self.schedule.runs.iter().position(|run| run.id == id)) else {
            return;
        };
        self.end_schedule_run(index, RunStatus::Complete, "the order is filled");
        self.save_schedule_runs();
    }

    /// The order of a schedule run is over without a fill (cancelled after the fill attempts, rejected):
    /// the run waits again, so the next attempt can follow while its window is open.
    pub(super) fn schedule_order_ended(&mut self, run_id: Option<u32>, status: &str) {
        let Some(index) = run_id.and_then(|id| self.schedule.runs.iter().position(|run| run.id == id)) else {
            return;
        };
        if self.schedule.runs.get(index).is_some_and(|run| run.status == RunStatus::Working) {
            if let Some(run) = self.schedule.runs.get_mut(index) {
                run.status = RunStatus::Waiting;
            }
            self.note_schedule_run(index, &format!("waiting: the order ended without a fill ({status})"));
            self.save_schedule_runs();
        }
    }
    /// Writes the runs to their file (not in tests, and never over a file that could not be read).
    pub(super) fn save_schedule_runs(&mut self) {
        if cfg!(test) || self.schedule.runs_save_blocked {
            return;
        }
        let result = runs_path(self.link.trading_live).and_then(|path| write_to(&path, &self.schedule.runs));
        if let Err(error) = result {
            self.log(&format!("WARNING: the schedule runs could not be saved: {error}"));
        }
    }
    /// The status of a run in the language of the page.
    pub(super) fn run_status_text(&self, status: RunStatus) -> String {
        match status {
            RunStatus::Waiting => self.tr("Waiting", "Wartet"),
            RunStatus::Paused => self.tr("Paused", "Pausiert"),
            RunStatus::Working => self.tr("Working", "Läuft"),
            RunStatus::Complete => self.tr("Complete", "Fertig"),
            RunStatus::Partial => self.tr("Partial", "Teilweise"),
            RunStatus::Expired => self.tr("Expired", "Abgelaufen"),
            RunStatus::Cancelled => self.tr("Cancelled", "Abgebrochen"),
        }
        .to_string()
    }
    /// What a schedule does today (a New York day), for the grid: its window, or why it does not run.
    fn schedule_today_text(&self, schedule: &Schedule, today: chrono::NaiveDate) -> String {
        match schedule.plan_for(today) {
            Plan::Runs { start, end } => {
                let ny = |instant: chrono::DateTime<chrono::Utc>| instant.with_timezone(&chrono_tz::America::New_York).format("%H:%M:%S").to_string();
                format!("{} - {}", ny(start), ny(end))
            }
            Plan::Skipped(Skip::Inactive) => self.tr("Inactive", "Inaktiv").to_string(),
            Plan::Skipped(Skip::Holiday(english, german)) => self.tr(english, german).to_string(),
            Plan::Skipped(Skip::NotThisDay) => self.tr("Not today", "Nicht heute").to_string(),
            Plan::Skipped(Skip::InvalidTime) => self.tr("Invalid time", "Ungültige Zeit").to_string(),
        }
    }
    /// Writes the schedules to their file (not in tests, and never over a file that could not be read).
    pub(super) fn save_schedules(&mut self) {
        if self.schedule.save_blocked {
            let message = "Schedules NOT saved: the schedules file could not be read and would be overwritten - repair or move it, then restart";
            self.log(&format!("WARNING: {message}"));
            self.schedule.status = Some(message.to_string());
            return;
        }
        if cfg!(test) {
            return;
        }
        let result = schedules_path(self.link.trading_live).and_then(|path| write_to(&path, &self.schedule.list));
        if let Err(error) = result {
            let message = format!("The schedules could not be saved: {error}");
            self.log(&format!("WARNING: {message}"));
            self.schedule.status = Some(message);
        }
    }

    /// A schedule as the person knows it: its time and its template, e.g. `09:35:00 - IC 0DTE`.
    pub(super) fn schedule_label(&self, schedule: &Schedule) -> String {
        let template = self.template(schedule.template_id).map_or("?", |t| t.name.as_str());
        let name = if schedule.name.is_empty() { String::new() } else { format!("{} - ", schedule.name) };
        format!("{name}{} - {template}", schedule.time_label())
    }

    /// "Save Schedule": stores the schedule, or keeps the page open with the faulty fields outlined.
    pub(super) fn save_schedule_form(&mut self, form: ScheduleForm) {
        let template_ids: Vec<i32> = visible_templates(&self.templates).iter().map(|t| t.id).collect();
        let other_ids: Vec<u32> = self.schedule.list.iter().filter(|s| s.id != form.id && s.group == form.group).map(|s| s.id).collect();
        let condition_ids: Vec<u32> = self.conditions.list.iter().map(|c| c.id).collect();
        let context = CheckContext {
            template_ids: &template_ids,
            accounts: &self.session.managed_accounts,
            condition_ids: &condition_ids,
            other_schedule_ids: &other_ids,
            local_offset_minutes: self.local_offset_minutes(),
        };
        match form.to_schedule(&context) {
            Ok(schedule) => {
                let label = self.schedule_label(&schedule);
                let id = upsert(&mut self.schedule.list, schedule);
                self.schedule.selected = Some(id);
                self.schedule.errors.clear();
                self.save_schedules();
                self.log(&format!("Schedule {id} saved: {label}"));
                // A waiting run of today picks up the change at once.
                self.schedule.runs_checked_at = None;
            }
            Err(errors) => {
                for error in &errors {
                    self.log(&format!("Schedule not saved - {}", error.message));
                }
                self.schedule.errors = errors;
                self.schedule.error_scroll = true;
                self.schedule.editing = Some(form);
            }
        }
    }

    /// `difference` in minutes, rounded: the two clocks are read a moment apart, so 6 hours come out as 5:59:59.99 and must not read as 359 minutes.
    pub(super) fn whole_minutes(difference: chrono::Duration) -> i64 {
        (difference.num_seconds() + 30).div_euclid(60)
    }

    /// Minutes the local time of this computer is ahead of New York time right now (negative: behind).
    fn local_offset_minutes(&self) -> i64 {
        Self::whole_minutes(self.clock.local().naive_local() - self.clock.new_york())
    }

    /// The Add/Edit page of a schedule.
    fn schedule_editor_page(&mut self, ui: &mut egui::Ui, form: &mut ScheduleForm) -> EditorOutcome {
        ui.heading(RichText::new(self.tr("Scheduled Trades", "Geplante Trades")).color(self.heading_color()));
        ui.add_space(8.0);
        let templates = self.template_choices();
        let others: Vec<(u32, String)> =
            self.schedule.list.iter().filter(|s| s.id != form.id && s.group == form.group).map(|s| (s.id, self.schedule_label(s))).collect();
        let conditions: Vec<(u32, String)> = self.conditions.list.iter().map(|c| (c.id, c.name.clone())).collect();
        let errors = self.schedule.errors.clone();
        let scroll_to_error = std::cell::Cell::new(self.schedule.error_scroll);
        let local_offset_minutes = self.local_offset_minutes();

        let data = EditorData {
            templates: &templates,
            accounts: &self.session.managed_accounts,
            conditions: &conditions,
            other_schedules: &others,
            errors: &errors,
            scroll_to_error: &scroll_to_error,
            local_offset_minutes,
            today: self.clock.new_york().date(),
        };
        let outcome = editor_page(ui, form, &data);
        self.schedule.error_scroll = scroll_to_error.get();
        outcome
    }

    /// Add, then Edit, Copy, Enable or Disable and Delete over the schedule selected in the list.
    fn schedules_toolbar(&mut self, ui: &mut egui::Ui) {
        let shown = self.shown_schedules();
        let selected = self.schedule.selected.filter(|id| shown.iter().any(|s| s.id == *id));
        egui::Frame::none().fill(card_bg()).inner_margin(egui::vec2(10.0, 8.0)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add(crate::widgets::MetalButton::new(self.tr("+ Add", "+ Hinzufügen"))).clicked() {
                    self.schedule.editing = Some(ScheduleForm { group: self.schedule.group, ..ScheduleForm::new_default() });
                    self.schedule.errors.clear();
                }
                ui.separator();
                self.schedule_action_buttons(ui, selected);
                self.schedule_set_controls(ui, selected.is_some());
            });
            self.schedule_delete_question(ui);
        });
    }

    /// The schedules of the active set (the ones the page shows and the runs are made for).
    pub(super) fn shown_schedules(&self) -> Vec<Schedule> {
        self.schedule.list.iter().filter(|s| s.group == self.schedule.group).cloned().collect()
    }

    /// Switches to another set of schedules: the waiting runs of the old set are dropped, the ones of the new
    /// set are made, and the choice is kept for the next start.
    pub(super) fn set_active_group(&mut self, group: u8) {
        if group == self.schedule.group || group >= GROUPS {
            return;
        }
        self.schedule.group = group;
        self.schedule.selected = None;
        self.schedule.confirm_delete = None;
        if let Err(error) = save_active_group(self.link.trading_live, group) {
            self.log(&format!("WARNING: the active schedule set could not be saved: {error}"));
        }
        self.log(&format!("Schedule set {} is active", group_name(group)));
        self.schedule.runs_checked_at = None;
        self.update_schedule_runs();
        self.save_schedule_runs();
    }

    /// A drop-down with the sets; choosing one makes it the active set.
    fn group_choice(&mut self, ui: &mut egui::Ui, id: &str) {
        let mut group = self.schedule.group;
        crate::widgets::metal_combo(ui, |ui| {
            egui::ComboBox::from_id_salt(id).width(130.0).selected_text(group_name(group)).show_ui(ui, |ui| {
                for candidate in 0..GROUPS {
                    ui.selectable_value(&mut group, candidate, group_name(candidate));
                }
            })
        });
        self.set_active_group(group);
    }

    /// Copies the selected schedule into another set as a new schedule.
    pub(super) fn copy_selected_to_group(&mut self, group: u8) {
        let Some(schedule) = self.schedule.selected.and_then(|id| self.schedule.list.iter().find(|s| s.id == id)).cloned() else {
            return;
        };
        if schedule.group == group || group >= GROUPS {
            return;
        }
        let id = upsert(&mut self.schedule.list, Schedule { id: 0, group, last_run: None, ..schedule });
        self.save_schedules();
        let message = format!("Schedule copied to set {} as schedule {id}", group_name(group));
        self.log(&message);
        self.schedule.status = Some(message);
    }

    /// The name of a template that exists, for the export.
    pub(super) fn template_name_of(&self, id: i32) -> Option<String> {
        self.live_template(id).map(|t| t.name.clone())
    }

    /// Writes the schedules of the active set to an export file.
    fn export_schedules(&mut self) {
        let export = schedule_export::build_export(&self.shown_schedules(), |id| self.template_name_of(id));
        let stamp = self.clock.local().format("%Y%m%d-%H%M%S").to_string();
        let message = match exports_dir().and_then(|dir| schedule_export::write_export(&export, &dir, &stamp)) {
            Ok(path) => format!("Exported {} schedule(s) to {}", export.schedules.len(), path.display()),
            Err(error) => format!("Could not export the schedules: {error}"),
        };
        self.log(&message);
        self.schedule.status = Some(message);
    }

    /// Imports an export file into the active set: new schedules, switched off.
    pub(super) fn import_schedule_file(&mut self, path: &std::path::Path) {
        let message = match schedule_export::read_export(path) {
            Ok(export) => {
                let templates = self.template_choices();
                let report = schedule_export::import_schedules(&mut self.schedule.list, export, &templates, self.schedule.group);
                self.save_schedules();
                self.schedule.import.open = false;
                format!(
                    "Imported {} schedule(s) into set {} from {}; they are switched off, {} without a template",
                    report.imported,
                    group_name(self.schedule.group),
                    path.display(),
                    report.without_template
                )
            }
            Err(error) => format!("Could not import {}: {error}", path.display()),
        };
        self.log(&message);
        self.schedule.status = Some(message);
    }

    /// The card with the export files to import (and a path field), like the one of the templates.
    fn schedule_import_card(&mut self, ui: &mut egui::Ui) {
        if !self.schedule.import.open {
            return;
        }
        let mut import_path: Option<std::path::PathBuf> = None;
        white_card(ui, |ui| {
            navy_bar(ui, self.tr("Import Schedules", "Zeitpläne importieren"));
            ui.add_space(8.0);
            let folder = exports_dir().unwrap_or_default();
            ui.colored_label(muted_text(), format!("Schedule exports in {}:", folder.display()));
            let candidates = schedule_export::import_candidates(&folder);
            if candidates.is_empty() {
                ui.colored_label(muted_text(), "No schedule exports found.");
            }
            for path in candidates {
                ui.horizontal(|ui| {
                    if ui.add(crate::widgets::MetalButton::new(self.tr("Import", "Importieren"))).clicked() {
                        import_path = Some(path.clone());
                    }
                    ui.label(path.display().to_string());
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(self.tr("Or file path:", "Oder Dateipfad:"));
                form_text(ui, &mut self.schedule.import.path, 420.0);
                let can_import = !self.schedule.import.path.trim().is_empty();
                if ui.add_enabled(can_import, crate::widgets::MetalButton::new(self.tr("Import", "Importieren"))).clicked() {
                    import_path = Some(std::path::PathBuf::from(self.schedule.import.path.trim()));
                }
            });
        });
        if let Some(path) = import_path {
            self.import_schedule_file(&path);
        }
        ui.add_space(8.0);
    }

    /// The controls of the sets in the toolbar: the active set, copy to another set, export and import.
    fn schedule_set_controls(&mut self, ui: &mut egui::Ui, has_selection: bool) {
        ui.separator();
        ui.label(self.tr("Set:", "Satz:"));
        self.group_choice(ui, "schedule_group");
        let mut copy_to = None;
        ui.add_enabled_ui(has_selection, |ui| {
            crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("schedule_copy_to").width(150.0).selected_text(self.tr("Copy to set...", "In Satz kopieren...")).show_ui(
                    ui,
                    |ui| {
                        for candidate in (0..GROUPS).filter(|g| *g != self.schedule.group) {
                            if ui.selectable_label(false, group_name(candidate)).clicked() {
                                copy_to = Some(candidate);
                            }
                        }
                    },
                )
            });
        });
        if let Some(group) = copy_to {
            self.copy_selected_to_group(group);
        }
        if ui.add(crate::widgets::MetalButton::new(self.tr("Export", "Exportieren"))).clicked() {
            self.export_schedules();
        }
        if ui.add(crate::widgets::MetalButton::new(self.tr("Import", "Importieren"))).clicked() {
            self.schedule.import.open = !self.schedule.import.open;
        }
    }

    /// Switches the selected schedule on or off and saves.
    pub(super) fn set_schedule_active(&mut self, active: bool) {
        let Some(id) = self.schedule.selected else {
            return;
        };
        if let Some(schedule) = self.schedule.list.iter_mut().find(|s| s.id == id) {
            schedule.active = active;
            self.log(&format!("Schedule {id} {}", if active { "enabled" } else { "disabled" }));
            self.save_schedules();
            self.schedule.runs_checked_at = None;
        }
    }

    /// Opens the Add/Edit page for the schedule `id` (a double-click on a task of the dashboard).
    pub(super) fn edit_schedule(&mut self, id: u32) {
        let Some(schedule) = self.schedule.list.iter().find(|s| s.id == id) else {
            return;
        };
        self.schedule.editing = Some(ScheduleForm::from_schedule(schedule, false));
        self.schedule.selected = Some(id);
        self.schedule.errors.clear();
        self.page = Page::Schedules;
    }

    /// Deletes a schedule; the schedules that waited for it no longer wait.
    pub(super) fn delete_schedule(&mut self, id: u32) {
        self.schedule.list.retain(|s| s.id != id);
        for other in &mut self.schedule.list {
            if other.linked_schedule_id == id {
                other.linked_schedule_id = 0;
            }
        }
        self.schedule.selected = None;
        self.schedule.confirm_delete = None;
        self.log(&format!("Schedule {id} deleted"));
        self.save_schedules();
    }

    /// The question before a schedule is deleted, and what its answer does.
    pub(super) fn schedule_delete_question(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.schedule.confirm_delete else {
            return;
        };
        let Some(schedule) = self.schedule.list.iter().find(|s| s.id == id).cloned() else {
            self.schedule.confirm_delete = None;
            return;
        };
        ui.horizontal(|ui| {
            ui.colored_label(
                crate::theme::negative_text(),
                format!("{} {}?", self.tr("Delete the schedule", "Zeitplan löschen:"), self.schedule_label(&schedule)),
            );
            if ui.add(crate::widgets::MetalButton::new(self.tr("Yes, delete", "Ja, löschen"))).clicked() {
                self.delete_schedule(id);
            }
            if ui.add(crate::widgets::MetalButton::new(self.tr("No", "Nein"))).clicked() {
                self.schedule.confirm_delete = None;
            }
        });
    }

    pub(super) fn schedules_page(&mut self, ui: &mut egui::Ui) {
        if let Some(mut form) = self.schedule.editing.take() {
            match self.schedule_editor_page(ui, &mut form) {
                EditorOutcome::Keep => self.schedule.editing = Some(form),
                EditorOutcome::Save => self.save_schedule_form(form),
                EditorOutcome::Cancel => self.schedule.errors.clear(),
            }
            return;
        }
        ui.heading(RichText::new(self.tr("Scheduled Trades", "Geplante Trades")).color(self.heading_color()));
        ui.add_space(8.0);
        if let Some(problem) = &self.schedule.load_problem {
            ui.colored_label(
                RED,
                format!("{}: {problem}", self.tr("The schedules file could not be loaded", "Die Zeitplan-Datei konnte nicht geladen werden")),
            );
            ui.add_space(6.0);
        }
        if self.schedule.save_blocked {
            ui.colored_label(
                RED,
                self.tr(
                    "Saving schedules is blocked so the file on disk is not overwritten.",
                    "Das Speichern der Zeitpläne ist gesperrt, damit die Datei nicht überschrieben wird.",
                ),
            );
            ui.add_space(6.0);
        }
        self.schedules_toolbar(ui);
        if let Some(status) = &self.schedule.status {
            ui.colored_label(muted_text(), status.as_str());
        }
        self.schedule_import_card(ui);
        ui.add_space(8.0);
        if self.shown_schedules().is_empty() {
            empty_state(
                ui,
                self.tr("No scheduled trades in this set", "Keine geplanten Trades in diesem Satz"),
                self.tr("Use Add to create one.", "Mit Hinzufügen einen erstellen."),
            );
            return;
        }
        let columns = [
            TableColumn::new(self.tr("Time (NY)", "Zeit (NY)"), 0.9, false),
            TableColumn::new(self.tr("Local time", "Ortszeit"), 0.9, false),
            TableColumn::new("Template", 2.0, false),
            TableColumn::new(self.tr("Qty", "Menge"), 0.6, true),
            TableColumn::new(self.tr("Account", "Konto"), 1.4, false),
            TableColumn::new(self.tr("Active", "Aktiv"), 0.7, false),
            TableColumn::new(self.tr("Days", "Tage"), 1.4, false),
            TableColumn::new(self.tr("Today (NY)", "Heute (NY)"), 1.8, false),
            TableColumn::new("Status", 1.0, false),
            TableColumn::new("Symbol", 1.0, false),
            TableColumn::new(self.tr("Strategy", "Strategie"), 1.4, false),
        ];
        let today = self.clock.new_york().date();
        let shown = self.shown_schedules();
        let rows: Vec<Vec<TableCell>> = shown
            .iter()
            .map(|s| {
                let template = self.template(s.template_id);
                vec![
                    TableCell::new(s.time_label()),
                    TableCell::new(
                        s.start_on(today).map_or_else(|| "-".to_string(), |start| start.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()),
                    ),
                    TableCell::new(template.map_or_else(|| format!("#{} ({})", s.template_id, self.tr("missing", "fehlt")), |t| t.name.clone())),
                    TableCell::new(s.quantity.label()),
                    TableCell::new(mask_account(&s.account)),
                    TableCell::new(if s.active { self.tr("Yes", "Ja") } else { self.tr("No", "Nein") }),
                    TableCell::new(s.days.label()),
                    TableCell::new(self.schedule_today_text(s, today)),
                    TableCell::new(run_of(&self.schedule.runs, s.id, today).map_or_else(|| "-".to_string(), |run| self.run_status_text(run.status))),
                    TableCell::new(template.map_or_else(String::new, |t| t.underlying_symbol.clone())),
                    TableCell::new(template.map_or_else(String::new, |t| t.strategy.clone())),
                ]
            })
            .collect();
        // A click selects a schedule (like the list in the toolbar), a double-click opens it for editing.
        let selected = self.schedule.selected.and_then(|id| shown.iter().position(|s| s.id == id));
        let clicks = data_table_rows(ui, "schedules_table", &columns, &rows, selected);
        let id_of = |index: Option<usize>| index.and_then(|i| shown.get(i)).map(|s| s.id);
        self.schedule.selected = next_selection(self.schedule.selected, clicks, |index| id_of(Some(index)));
        if let Some(id) = id_of(clicks.double_clicked) {
            self.edit_schedule(id);
        }
    }
}
