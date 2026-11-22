//! Triggers and the webhook receiver in the app: the list and its page, a signal that starts the entry of a trigger or closes
//! its trades, and the receiver that is switched on in Settings > Trading Features. A signal does nothing by itself: it is
//! checked against the trigger (window, days, limits) and then starts a run like a schedule does, so the entry passes the
//! same attempts, conditions, safety gate and risk checks.

use super::{
    data_table_rows, empty_state, mask_account, muted_text, next_selection, visible_templates, white_card, App, EditorData, EditorOutcome, RichText,
    StoredTrade, TableCell, TableColumn,
};
use crate::list_file::write_to;
use crate::schedule_form::{CheckContext, ScheduleError};
use crate::schedule_run::{next_id, RunStatus, ScheduleRun, TriggerRun};
use crate::storage::TimedExit;
use crate::trigger::{load_triggers, triggers_path, upsert, Trigger};
use crate::trigger_form::TriggerForm;
use crate::webhook::{Report, WebhookConfig, WebhookServer};
use crate::widgets::{section_header, setting_row, toggle_switch};
use eframe::egui;
use std::sync::mpsc::Receiver;

/// The triggers of the mode, the page that edits them and the receiver of their signals.
#[derive(Default)]
pub(super) struct TriggerState {
    /// Every saved trigger.
    pub list: Vec<Trigger>,
    /// Why the file's triggers are not loaded.
    pub load_problem: Option<String>,
    /// The file could not be read and is still in place: saving must not overwrite it.
    pub save_blocked: bool,
    /// The trigger selected in the grid.
    pub selected: Option<u32>,
    /// The trigger being added or edited.
    pub editing: Option<TriggerForm>,
    /// The faults of the last save.
    pub errors: Vec<ScheduleError>,
    /// Whether the first faulty field still has to be scrolled into view.
    pub error_scroll: bool,
    /// The trigger that waits for the answer to "Delete this trigger?".
    pub confirm_delete: Option<u32>,
    /// A line under the toolbar.
    pub status: Option<String>,
    /// The running receiver, with its channel and secrets.
    pub server: Option<(WebhookServer, Receiver<Report>, WebhookConfig)>,
    /// The time the receiver checks a request's timestamp against (Unix seconds); `None` = the computer's clock.
    pub webhook_now: Option<fn() -> i64>,
    /// Whether the token is shown in Settings.
    pub show_token: bool,
    /// For each EMA trigger, the last closed bar it looked at (its date and how many bars there were): a crossover counts only on a bar
    /// that came in by itself after that, not on the history that streams in when the bars are requested.
    pub ema_seen: std::collections::HashMap<u32, (String, usize)>,
    /// The number of SPX minute bars and the date of the newest when the EMA triggers last looked: without a new bar there is nothing to work out.
    pub ema_mark: Option<(usize, String)>,
}

impl App {
    /// The schedule a run enters by: the one embedded in a re-entry or a trigger run, else the one of the list.
    pub(super) fn schedule_of_run(&self, run: &ScheduleRun) -> Option<crate::schedule::Schedule> {
        run.reentry
            .as_ref()
            .map(|reentry| reentry.schedule.clone())
            .or_else(|| run.trigger.as_ref().map(|trigger| trigger.schedule.clone()))
            .or_else(|| self.schedule.list.iter().find(|s| s.id == run.schedule_id).cloned())
    }

    /// Loads the triggers of a mode; a file that could not be read is reported and never overwritten.
    pub(super) fn load_mode_triggers(&mut self, live: bool) {
        let loaded = load_triggers(live, &self.clock.local().format("%Y%m%d-%H%M%S").to_string());
        self.triggers = TriggerState { list: loaded.items, load_problem: loaded.problem, save_blocked: loaded.save_blocked, ..TriggerState::default() };
        if let Some(problem) = &self.triggers.load_problem {
            let message = format!("WARNING: triggers: {problem}");
            self.log(&message);
        }
    }

    /// Writes the triggers (not in tests, and never over a file that could not be read).
    fn save_triggers(&mut self) {
        if self.triggers.save_blocked {
            let message = "Triggers NOT saved: the triggers file could not be read and would be overwritten - repair or move it, then restart";
            self.log(&format!("WARNING: {message}"));
            self.triggers.status = Some(message.to_string());
            return;
        }
        if cfg!(test) {
            return;
        }
        if let Err(error) = triggers_path(self.link.trading_live).and_then(|path| write_to(&path, &self.triggers.list)) {
            let message = format!("The triggers could not be saved: {error}");
            self.log(&format!("WARNING: {message}"));
            self.triggers.status = Some(message);
        }
    }

    /// A trigger as the person knows it: its name (or id) and template.
    fn trigger_label(&self, trigger: &Trigger) -> String {
        let template = self.template(trigger.base.template_id).map_or("?", |t| t.name.as_str());
        let name = if trigger.base.name.is_empty() { format!("Trigger {}", trigger.base.id) } else { trigger.base.name.clone() };
        format!("{name} - {template}")
    }

    /// "Save Trigger": stores it, or keeps the page open with the faulty fields outlined.
    fn save_trigger_form(&mut self, form: TriggerForm) {
        let template_ids: Vec<i32> = visible_templates(&self.templates).iter().map(|t| t.id).collect();
        let condition_ids: Vec<u32> = self.conditions.list.iter().map(|c| c.id).collect();
        let context = CheckContext {
            template_ids: &template_ids,
            accounts: &self.session.managed_accounts,
            condition_ids: &condition_ids,
            other_schedule_ids: &[],
            local_offset_minutes: 0,
        };
        match form.to_trigger(&context) {
            Ok(trigger) => {
                let label = self.trigger_label(&trigger);
                let id = upsert(&mut self.triggers.list, trigger);
                self.triggers.selected = Some(id);
                self.triggers.errors.clear();
                self.save_triggers();
                self.log(&format!("Trigger {id} saved: {label}"));
            }
            Err(errors) => {
                for error in &errors {
                    self.log(&format!("Trigger not saved - {}", error.message));
                }
                self.triggers.errors = errors;
                self.triggers.error_scroll = true;
                self.triggers.editing = Some(form);
            }
        }
    }

    /// Switches the selected trigger on or off.
    fn set_trigger_active(&mut self, active: bool) {
        let Some(id) = self.triggers.selected else { return };
        if let Some(trigger) = self.triggers.list.iter_mut().find(|t| t.base.id == id) {
            trigger.base.active = active;
        }
        self.save_triggers();
        self.log(&format!("Trigger {id} {}", if active { "enabled" } else { "disabled" }));
    }

    /// The page "Webhooks / Triggers": the list, or the Add/Edit page.
    pub(super) fn webhooks_page(&mut self, ui: &mut egui::Ui) {
        if let Some(mut form) = self.triggers.editing.take() {
            match self.trigger_editor_page(ui, &mut form) {
                EditorOutcome::Keep => self.triggers.editing = Some(form),
                EditorOutcome::Save => self.save_trigger_form(form),
                EditorOutcome::Cancel => self.triggers.errors.clear(),
            }
            return;
        }
        ui.heading(RichText::new(self.tr("Webhooks / Triggers", "Webhooks / Trigger")).color(self.heading_color()));
        ui.add_space(8.0);
        if let Some(problem) = &self.triggers.load_problem {
            ui.colored_label(
                crate::theme::negative_text(),
                format!("{}: {problem}", self.tr("The triggers file could not be loaded", "Die Trigger-Datei konnte nicht geladen werden")),
            );
            ui.add_space(6.0);
        }
        self.triggers_toolbar(ui);
        if let Some(status) = &self.triggers.status {
            ui.colored_label(muted_text(), status.as_str());
        }
        ui.label(RichText::new(self.webhook_summary()).small().color(muted_text()));
        ui.add_space(8.0);
        if self.triggers.list.is_empty() {
            empty_state(
                ui,
                self.tr("No webhooks or triggers yet", "Noch keine Webhooks oder Trigger"),
                self.tr(
                    "Use Add to create one. A webhook signal names its TriggerID.",
                    "Mit Hinzufügen einen erstellen. Ein Webhook-Signal nennt seine TriggerID.",
                ),
            );
            return;
        }
        let columns = [
            TableColumn::new("Trigger", 1.0, false),
            TableColumn::new(self.tr("Qty", "Menge"), 0.6, true),
            TableColumn::new(self.tr("Account", "Konto"), 1.4, false),
            TableColumn::new(self.tr("Active", "Aktiv"), 0.7, false),
            TableColumn::new("Template", 2.0, false),
            TableColumn::new(self.tr("Strategy", "Strategie"), 1.4, false),
            TableColumn::new(self.tr("Start Time", "Startzeit"), 0.9, false),
            TableColumn::new(self.tr("End Time", "Endzeit"), 0.9, false),
            TableColumn::new(self.tr("Days", "Tage"), 1.2, false),
            TableColumn::new(self.tr("Condition", "Bedingung"), 1.4, false),
            TableColumn::new("Name", 1.6, false),
            TableColumn::new("TriggerID", 0.8, true),
        ];
        let rows: Vec<Vec<TableCell>> = self
            .triggers
            .list
            .iter()
            .map(|t| {
                let template = self.templates.iter().find(|template| template.id == t.base.template_id);
                let condition = self.conditions.list.iter().find(|c| c.id == t.base.condition_id).map_or_else(String::new, |c| c.name.clone());
                let (start, end) = t.window_label();
                vec![
                    TableCell::new(t.entry_label()),
                    TableCell::new(t.base.quantity.label()),
                    TableCell::new(mask_account(&t.base.account)),
                    TableCell::new(if t.base.active { self.tr("Yes", "Ja") } else { self.tr("No", "Nein") }),
                    TableCell::new(
                        template.map_or_else(|| format!("#{} ({})", t.base.template_id, self.tr("missing", "fehlt")), |template| template.name.clone()),
                    ),
                    TableCell::new(if t.strategy.is_empty() {
                        template.map_or_else(String::new, |template| template.strategy.clone())
                    } else {
                        t.strategy.clone()
                    }),
                    TableCell::new(start),
                    TableCell::new(end),
                    TableCell::new(t.base.days.label()),
                    TableCell::new(condition),
                    TableCell::new(t.base.name.clone()),
                    TableCell::new(t.base.id.to_string()),
                ]
            })
            .collect();
        let selected = self.triggers.selected.and_then(|id| self.triggers.list.iter().position(|t| t.base.id == id));
        let clicks = data_table_rows(ui, "triggers_table", &columns, &rows, selected);
        let id_of = |index: Option<usize>| index.and_then(|i| self.triggers.list.get(i)).map(|t| t.base.id);
        self.triggers.selected = next_selection(self.triggers.selected, clicks, |index| id_of(Some(index)));
        if let Some(id) = id_of(clicks.double_clicked) {
            if let Some(trigger) = self.triggers.list.iter().find(|t| t.base.id == id) {
                self.triggers.editing = Some(TriggerForm::from_trigger(trigger, false));
                self.triggers.errors.clear();
            }
        }
    }

    /// Add, then Edit, Copy, Enable or Disable and Delete over the trigger selected in the list.
    fn triggers_toolbar(&mut self, ui: &mut egui::Ui) {
        let selected = self.triggers.selected.and_then(|id| self.triggers.list.iter().find(|t| t.base.id == id)).cloned();
        white_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add(crate::widgets::MetalButton::new(self.tr("+ Add", "+ Hinzufügen"))).clicked() {
                    self.triggers.editing = Some(TriggerForm::new_default());
                    self.triggers.errors.clear();
                }
                let has_selection = selected.is_some();
                let active = selected.as_ref().is_some_and(|t| t.base.active);
                if ui.add_enabled(has_selection, crate::widgets::MetalButton::new(self.tr("Edit", "Bearbeiten"))).clicked() {
                    self.triggers.editing = selected.as_ref().map(|t| TriggerForm::from_trigger(t, false));
                    self.triggers.errors.clear();
                }
                if ui.add_enabled(has_selection, crate::widgets::MetalButton::new(self.tr("Copy", "Kopieren"))).clicked() {
                    self.triggers.editing = selected.as_ref().map(|t| TriggerForm::from_trigger(t, true));
                    self.triggers.errors.clear();
                }
                let toggle = if active { self.tr("Disable", "Deaktivieren") } else { self.tr("Enable", "Aktivieren") };
                if ui.add_enabled(has_selection, crate::widgets::MetalButton::new(toggle)).clicked() {
                    self.set_trigger_active(!active);
                }
                if ui.add_enabled(has_selection, crate::widgets::MetalButton::new(self.tr("Delete", "Löschen"))).clicked() {
                    self.triggers.confirm_delete = self.triggers.selected;
                }
            });
            if let Some(id) = self.triggers.confirm_delete {
                let Some(trigger) = self.triggers.list.iter().find(|t| t.base.id == id).cloned() else {
                    self.triggers.confirm_delete = None;
                    return;
                };
                ui.horizontal(|ui| {
                    ui.colored_label(
                        crate::theme::negative_text(),
                        format!("{} {}?", self.tr("Delete the trigger", "Trigger löschen:"), self.trigger_label(&trigger)),
                    );
                    if ui.add(crate::widgets::MetalButton::new(self.tr("Yes, delete", "Ja, löschen"))).clicked() {
                        self.triggers.list.retain(|t| t.base.id != id);
                        self.triggers.selected = None;
                        self.triggers.confirm_delete = None;
                        self.log(&format!("Trigger {id} deleted"));
                        self.save_triggers();
                    }
                    if ui.add(crate::widgets::MetalButton::new(self.tr("No", "Nein"))).clicked() {
                        self.triggers.confirm_delete = None;
                    }
                });
            }
        });
    }

    /// The Add/Edit page of a trigger.
    fn trigger_editor_page(&mut self, ui: &mut egui::Ui, form: &mut TriggerForm) -> EditorOutcome {
        ui.heading(RichText::new(self.tr("Webhooks / Triggers", "Webhooks / Trigger")).color(self.heading_color()));
        ui.add_space(8.0);
        let templates = self.template_choices();
        let conditions: Vec<(u32, String)> = self.conditions.list.iter().map(|c| (c.id, c.name.clone())).collect();
        let errors = self.triggers.errors.clone();
        let scroll_to_error = std::cell::Cell::new(self.triggers.error_scroll);

        let data = EditorData {
            templates: &templates,
            accounts: &self.session.managed_accounts,
            conditions: &conditions,
            other_schedules: &[],
            errors: &errors,
            scroll_to_error: &scroll_to_error,
            local_offset_minutes: 0,
            today: self.clock.new_york().date(),
        };
        let hint = if form.base.id == 0 {
            "A webhook signal names this trigger by its TriggerID, which it gets when it is saved.".to_string()
        } else {
            format!(
                "A webhook signal starts this trigger with \"TriggerID\": [{}] and closes its open trades with \"CloseTriggerID\": [{}].",
                form.base.id, form.base.id
            )
        };
        let outcome = crate::trigger_editor::editor_page(ui, form, &data, &hint);
        self.triggers.error_scroll = scroll_to_error.get();
        outcome
    }

    /// What the receiver is doing, in a line.
    pub(super) fn webhook_summary(&self) -> String {
        match &self.triggers.server {
            Some((server, _, config)) => format!(
                "Webhook receiver on: POST http://127.0.0.1:{}/signal (this computer only), instance {}. Settings > Trading Features shows the token.",
                server.port(),
                config.instance
            ),
            None => "Webhook receiver off (Settings > Trading Features switches it on).".to_string(),
        }
    }

    /// Starts or stops the receiver as Settings > Trading Features asks.
    pub(super) fn sync_webhook_server(&mut self) {
        let wanted = self.settings.mail_flags.webhook_triggers_enabled;
        if !wanted {
            if self.triggers.server.take().is_some() {
                self.log("Webhook receiver stopped");
            }
            return;
        }
        if self.triggers.server.is_some() || self.triggers.status.as_deref().is_some_and(|s| s.starts_with("The webhook receiver could not start")) {
            return;
        }
        let config = if cfg!(test) {
            WebhookConfig { port: 0, ..WebhookConfig::generated() }
        } else {
            match crate::storage::mode_dir(self.link.trading_live).and_then(|dir| WebhookConfig::load_or_create(&dir.join("webhook.json"))) {
                Ok(config) => config,
                Err(error) => {
                    let message = format!("The webhook receiver could not start: its settings cannot be read or written ({error})");
                    self.log(&format!("WARNING: {message}"));
                    self.triggers.status = Some(message);
                    return;
                }
            }
        };
        match WebhookServer::start(&config, self.triggers.webhook_now.unwrap_or(crate::clock::SystemClock::unix_seconds)) {
            Ok((server, receiver)) => {
                self.log(&format!("Webhook receiver on: POST http://127.0.0.1:{}/signal (this computer only)", server.port()));
                self.triggers.status = None;
                self.triggers.server = Some((server, receiver, config));
            }
            Err(error) => {
                let message = format!("The webhook receiver could not start: port {} is not available ({error})", config.port);
                self.log(&format!("WARNING: {message}"));
                self.triggers.status = Some(message);
            }
        }
    }

    /// Takes what the receiver has received and acts on it.
    pub(super) fn poll_webhook(&mut self) {
        let reports: Vec<Report> = self.triggers.server.as_ref().map(|(_, receiver, _)| receiver.try_iter().collect()).unwrap_or_default();
        for report in reports {
            match report {
                Report::Refused(reason) => self.log(&format!("WARNING: Webhook request refused: {reason}")),
                Report::Signal(signal) => {
                    self.log(&format!("Webhook signal received: TriggerID {:?}, CloseTriggerID {:?}", signal.open, signal.close));
                    self.process_signal(&signal.open, &signal.close);
                }
            }
        }
    }

    /// A signal: `open` triggers start their entry, the open trades of `close` triggers are closed. 0 is a test.
    pub(super) fn process_signal(&mut self, open: &[u32], close: &[u32]) {
        if open.contains(&0) {
            self.log("Webhook test received - the receiver works");
        }
        for id in open.iter().copied().filter(|id| *id != 0) {
            self.process_trigger(id);
        }
        if !close.is_empty() {
            self.process_close_triggers(close);
        }
    }

    /// The EMA triggers (entry trigger "EMA5 Cross Over EMA20" and the like): on every new closed one-minute bar of the SPX the averages
    /// are compared with those of the bar before, and a crossover in the direction of the trigger starts its entry as a signal does
    /// (the window of the day, the limits and the condition then apply as for any signal). Bars that arrive in a batch with the
    /// history are not watched, only those that come in one or two at a time while the app runs.
    pub(super) fn check_ema_triggers(&mut self) {
        let watched: Vec<(u32, crate::trigger::EmaCross)> =
            self.triggers.list.iter().filter(|t| t.base.active).filter_map(|t| t.ema_cross().map(|cross| (t.base.id, cross))).collect();
        if watched.is_empty() {
            return;
        }
        self.watch_minute_bars("SPX");
        let mark = self.minute_bar_mark("SPX");
        if mark.is_none() || mark == self.triggers.ema_mark {
            return;
        }
        self.triggers.ema_mark = mark;
        for (id, cross) in watched {
            let Some(reading) = self.ema_reading("SPX", cross.fast, cross.slow) else { continue };
            let previous = self.triggers.ema_seen.insert(id, (reading.date.clone(), reading.count));
            let Some((date, count)) = previous else { continue };
            if date == reading.date || reading.count.saturating_sub(count) > 2 || !cross.crossed(reading.before, reading.now) {
                continue;
            }
            let name = self.triggers.list.iter().find(|t| t.base.id == id).map_or_else(String::new, |t| t.entry_label().to_string());
            self.log(&format!(
                "Trigger {id}: {name} at the bar of {} (EMA{} {} / EMA{} {}) - starting its entry",
                reading.date, cross.fast, reading.now.0, cross.slow, reading.now.1
            ));
            self.process_trigger(id);
        }
    }

    /// The entry of the trigger `id`, if it is allowed now: a run is started.
    fn process_trigger(&mut self, id: u32) {
        let tag = format!("Trigger {id}");
        if self.daily_target_reached_today() {
            self.log(&format!("{tag}: skipped - the daily profit target is reached"));
            return;
        }
        let Some(trigger) = self.triggers.list.iter().find(|t| t.base.id == id).cloned() else {
            self.log(&format!("{tag}: skipped - there is no such trigger"));
            return;
        };
        let now = self.clock.new_york();
        let started = self.schedule.runs.iter().filter(|run| run.day == now.date() && run.trigger.as_ref().is_some_and(|t| t.trigger_id == id)).count();
        let open = self
            .trades_stored
            .iter()
            .filter(|t| t.is_open() && t.account == trigger.base.account && t.origin.as_ref().is_some_and(|o| o.trigger == id))
            .count();
        if let Err(refusal) = trigger.check(now, started, open) {
            self.log(&format!("{tag}: skipped - {}", refusal.text()));
            return;
        }
        if !self.templates.iter().any(|t| t.id == trigger.base.template_id && !t.list_flags.is_deleted) {
            self.log(&format!("{tag}: skipped - the template {} does not exist (or is deleted)", trigger.base.template_id));
            return;
        }
        let run_id = next_id(&self.schedule.runs);
        let start = self.clock.utc();
        let minutes = i64::from(trigger.base.expiration_minutes.max(1));
        self.schedule.runs.push(ScheduleRun {
            id: run_id,
            schedule_id: 0,
            day: now.date(),
            window_start: start,
            window_end: start + chrono::Duration::minutes(minutes),
            status: RunStatus::Waiting,
            attempts: 0,
            started: None,
            finished: None,
            notes: String::new(),
            reentry: None,
            trigger: Some(TriggerRun {
                trigger_id: id,
                schedule: crate::schedule::Schedule { id: 0, ..trigger.base.clone() },
                strategy: trigger.strategy.clone(),
            }),
        });
        self.save_schedule_runs();
        self.log(&format!("{tag}: processed - schedule run {run_id} created ({})", self.trigger_label(&trigger)));
        self.schedule.runner_checked_at = None;
    }

    /// The open trades entered by the triggers `ids` are closed now (the exit of the template: market, or limit first).
    fn process_close_triggers(&mut self, ids: &[u32]) {
        let now = self.clock.new_york();
        let mut closed = 0;
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index).filter(|t| t.is_open() && t.close_order_id.is_none()).cloned() else { continue };
            let Some(origin) = trade.origin.clone().filter(|o| ids.contains(&o.trigger) && o.trigger != 0) else { continue };
            let limit = self.template(origin.template_id).is_some_and(|t| t.exit_flags.exit_order_limit);
            self.log(&format!("Trigger {} - close {}", origin.trigger, trade.label()));
            let mut updated: StoredTrade = trade;
            updated.timed_exit = Some(TimedExit { at: now.format("%Y-%m-%d %H:%M").to_string(), limit });
            self.save_trade(index, updated);
            closed += 1;
        }
        if closed == 0 {
            self.log(&format!("Close trigger {ids:?}: no open trade of these triggers"));
        }
    }

    /// The card "Webhook Triggers" of Settings > Trading Features: the switch, the test, the token.
    pub(super) fn webhook_settings_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            section_header(ui, self.tr("Webhook Triggers", "Webhook-Trigger"));
            setting_row(ui, self.tr("Webhook triggers", "Webhook-Trigger"), "", |ui| {
                toggle_switch(ui, &mut self.settings.mail_flags.webhook_triggers_enabled);
            });
            let running = self.triggers.server.is_some();
            setting_row(ui, self.tr("Test and token", "Test und Token"), "", |ui| {
                ui.horizontal(|ui| {
                    if ui.add_enabled(running, crate::widgets::MetalButton::new(self.tr("Send Webhook Test", "Webhook-Test senden"))).clicked() {
                        self.send_webhook_test();
                    }
                    if ui.add_enabled(running, crate::widgets::MetalButton::new(self.tr("New Token", "Neues Token"))).clicked() {
                        self.regenerate_webhook_token();
                    }
                });
            });
            ui.add_space(6.0);
            ui.label(RichText::new(self.webhook_summary()).small().color(muted_text()));
            if let Some((_, _, config)) = self.triggers.server.as_ref() {
                let token = if self.triggers.show_token { config.token.clone() } else { "\u{2022}".repeat(config.token.chars().count()) };
                let line = format!("Token: {token}");
                ui.horizontal(|ui| {
                    ui.label(line);
                    if ui
                        .add(crate::widgets::MetalButton::new(if self.triggers.show_token {
                            self.tr("Hide", "Verbergen")
                        } else {
                            self.tr("Show", "Anzeigen")
                        }))
                        .clicked()
                    {
                        self.triggers.show_token = !self.triggers.show_token;
                    }
                });
            }
            ui.label(
                RichText::new(self.tr(
                    "The receiver listens on this computer only (127.0.0.1). A request must carry the token, the instance and a current timestamp; to reach it from outside you forward a port or a tunnel yourself.",
                    "Der Empf\u{e4}nger lauscht nur auf diesem Computer (127.0.0.1). Eine Anfrage braucht Token, Instanz und einen aktuellen Zeitstempel; von au\u{df}en erreichst du ihn nur \u{fc}ber einen Port oder Tunnel, den du selbst einrichtest.",
                ))
                .small()
                .color(muted_text()),
            );
        });
    }

    /// Makes a new token for the receiver (the old one stops working) and restarts it.
    pub(super) fn regenerate_webhook_token(&mut self) {
        let Some((server, _, config)) = self.triggers.server.take() else { return };
        let fresh = WebhookConfig { port: config.port, ..WebhookConfig::generated() };
        drop(server);
        if !cfg!(test) {
            if let Err(error) = crate::storage::mode_dir(self.link.trading_live).and_then(|dir| fresh.save(&dir.join("webhook.json"))) {
                self.log(&format!("WARNING: The new webhook token could not be saved: {error}"));
                return;
            }
        }
        self.log("Webhook token renewed - the old one no longer works");
        self.sync_webhook_server();
    }

    /// Sends a test signal to the receiver (the button in Settings).
    pub(super) fn send_webhook_test(&mut self) {
        let Some((_, _, config)) = self.triggers.server.as_ref() else {
            self.log("Webhook test: the receiver is off - switch it on first");
            return;
        };
        let config = WebhookConfig { port: self.triggers.server.as_ref().map_or(config.port, |(server, _, _)| server.port()), ..config.clone() };
        match crate::webhook::send_test(&config, self.clock.utc()) {
            Ok(answer) => self.log(&format!("Webhook test sent: {answer}")),
            Err(error) => self.log(&format!("WARNING: Webhook test failed: {error}")),
        }
    }
}
