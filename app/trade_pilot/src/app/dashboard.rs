//! The Dashboard: the figures and gauges of the day, today's trades with their actions and the scheduled tasks.

use super::{
    credit_summary, delta_total, display_currency, display_percent, empty_state, execution_time_text, format_date, gauge_card, gauge_card_height, group_name,
    kpi_card, mask_account, money_text, muted_text, newest_first, next_selection, profit_summary, segmented, show_date, sign_color_of, stat_tile, text_primary,
    trade_table, App, DashboardFigures, Decimal, GaugeSpec, Lang, Page, ProfitSummary, RichText, RunStatus, ScheduleForm, ScheduleRun, StoredTrade, TableCell,
    TableColumn, ACCENT_BLUE, GROUPS, PCR_TICKS, STATUS_ERROR, WIN_TICKS,
};
use crate::widgets::{arranged_table, framed_card, plate_label, RowSelection};
use eframe::egui;
use rust_decimal::prelude::ToPrimitive;

/// How many seconds before its start a scheduled trade shows in the top bar.
const STARTING_SOON_SECS: i64 = 300;

impl App {
    /// The figures of the dashboard's first row, worked out from today's trades.
    fn dashboard_figures(&self) -> DashboardFigures {
        let summary = credit_summary(&self.trades_today);
        let (premium_sold, premium_captured, pcr_text, win_text, pcr, win_rate) = match &summary {
            // An open trade without a quote yet: Captured and PCR are unknown ("-"), not guessed.
            Some(s) if s.captured_unknown => (
                display_currency(s.premium_sold),
                "-".into(),
                "-".into(),
                display_percent(s.win_rate),
                0.0,
                s.win_rate.checked_mul(Decimal::ONE_HUNDRED).and_then(|v| v.to_f64()).unwrap_or(0.0),
            ),
            Some(s) => (
                display_currency(s.premium_sold),
                display_currency(s.premium_captured),
                display_percent(s.pcr),
                display_percent(s.win_rate),
                // Gauge needle positions only (screen geometry, not money).
                s.pcr.checked_mul(Decimal::ONE_HUNDRED).and_then(|v| v.to_f64()).unwrap_or(0.0),
                s.win_rate.checked_mul(Decimal::ONE_HUNDRED).and_then(|v| v.to_f64()).unwrap_or(0.0),
            ),
            None => ("-".into(), "-".into(), "-".into(), "-".into(), 0.0, 0.0),
        };
        let trade_count = summary.as_ref().map_or(0, |s| s.trade_count);
        let captured_negative = summary.as_ref().is_some_and(|s| s.premium_captured < Decimal::ZERO);
        // Open and closed P/L and the symbol's total delta over today's trades.
        let profit = profit_summary(&self.trades_today);
        let live_total = profit.as_ref().and_then(ProfitSummary::total);
        let delta_symbol = if self.settings.ticker_delta_symbol.is_empty() { "SPX".to_string() } else { self.settings.ticker_delta_symbol.clone() };
        let delta = delta_total(&self.trades_today, &delta_symbol).unwrap_or(0.0);
        DashboardFigures {
            premium_sold,
            premium_captured,
            pcr_text,
            win_text,
            pcr,
            win_rate,
            trade_count,
            captured_negative,
            profit,
            live_total,
            delta,
            delta_symbol,
            sweep: self.gauge_sweep(),
        }
    }

    /// Row 1 of the dashboard: four cards of the same size - today's result, the credit figures, two gauges.
    fn dashboard_kpi_row(&mut self, ui: &mut egui::Ui, figures: DashboardFigures) {
        let DashboardFigures {
            premium_sold,
            premium_captured,
            pcr_text,
            win_text,
            pcr,
            win_rate,
            trade_count,
            captured_negative,
            profit,
            live_total,
            delta,
            delta_symbol,
            sweep,
        } = figures;
        let gap = 12.0;
        let card_w = ((ui.available_width() - 3.0 * gap - 4.0) / 4.0).floor().clamp(120.0, 260.0);
        let card_h = gauge_card_height(card_w);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            kpi_card(ui, card_w, card_h, |ui| {
                plate_label(ui, RichText::new(self.tr("Today's P/L", "Heutiges G/V")).size(12.0).color(muted_text()));
                ui.add_space(2.0);
                plate_label(ui, RichText::new(money_text(live_total)).size(26.0).strong().color(sign_color_of(live_total)));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    let open = profit.as_ref().and_then(|p| p.open);
                    let closed = profit.as_ref().map(|p| p.closed);
                    stat_tile(ui, self.tr("Open", "Offen"), &money_text(open), sign_color_of(open));
                    stat_tile(ui, self.tr("Closed", "Geschlossen"), &money_text(closed), sign_color_of(closed));
                });
                ui.add_space(8.0);
                plate_label(ui, RichText::new(format!("Delta  {delta:.2} {delta_symbol}")).size(12.0).color(muted_text()));
            });
            kpi_card(ui, card_w, card_h, |ui| {
                let count = match self.settings.lang {
                    Lang::De => format!("Credit-Trades · {trade_count} heute"),
                    Lang::En => format!("Credit trades · {trade_count} today"),
                };
                plate_label(ui, RichText::new(count).size(12.0).color(muted_text()))
                    .on_hover_text(self.tr("Credit trades TradePilot opened today", "Credit-Trades, die TradePilot heute eröffnet hat"));
                ui.add_space(8.0);
                let captured_color = if captured_negative { STATUS_ERROR } else { text_primary() };
                stat_tile(ui, self.tr("Premium Sold", "Prämie verkauft"), &premium_sold, text_primary());
                ui.add_space(12.0);
                stat_tile(ui, self.tr("Premium Captured", "Prämie vereinnahmt"), &premium_captured, captured_color);
            });
            let win_label = self.tr("Win Rate", "Gewinnrate");
            let pcr_spec = GaugeSpec { min: -100.0, max: 100.0, ticks: &PCR_TICKS, title: "PCR", value_text: &pcr_text, sweep };
            gauge_card(ui, card_w, pcr, &pcr_spec);
            let win_spec = GaugeSpec { min: 0.0, max: 100.0, ticks: &WIN_TICKS, title: win_label, value_text: &win_text, sweep };
            gauge_card(ui, card_w, win_rate, &win_spec);
        });
    }

    /// Row 2 of the dashboard: today's trades - title, count, All / Open / Closed, then the list.
    fn dashboard_trades_card(&mut self, ui: &mut egui::Ui) {
        framed_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.tr("Trades today", "Heutige Trades")).size(16.0).strong().color(text_primary()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Each choice says how many trades it shows, so a click visibly changes something even when two choices
                    // show the same trades.
                    let (open, closed) = self.trade_filter_counts();
                    let labels = [
                        format!("{} ({})", self.tr("All", "Alle"), open + closed),
                        format!("{} ({open})", self.tr("Open", "Offen")),
                        format!("{} ({closed})", self.tr("Closed", "Geschlossen")),
                    ];
                    let options = labels.each_ref().map(|label| (label.as_str(), ACCENT_BLUE));
                    let current = match self.ui_state.trade_filter.as_str() {
                        "Open" => 1,
                        "Closed" => 2,
                        _ => 0,
                    };
                    let (clicked, _) = segmented(ui, &options, current, 64.0);
                    if let Some(index) = clicked {
                        self.ui_state.trade_filter = ["All", "Open", "Closed"].get(index).copied().unwrap_or("All").to_string();
                        ui.ctx().request_repaint();
                    }
                });
            });
            // The list after the choice above, so a click shows its trades in the same frame.
            let views = self.trade_row_views();
            ui.add_space(8.0);
            if views.is_empty() && !self.trades_stored.iter().all(StoredTrade::is_split) {
                // There are trades, the filter hides them all.
                let (title, hint) = if self.ui_state.trade_filter == "Closed" {
                    (self.tr("No closed trades", "Keine geschlossenen Trades"), self.tr("All trades are still open.", "Alle Trades sind noch offen."))
                } else {
                    (
                        self.tr("No open trades", "Keine offenen Trades"),
                        self.tr("\"All\" also shows the closed trades.", "„Alle“ zeigt auch die geschlossenen Trades."),
                    )
                };
                empty_state(ui, title, hint);
            } else if views.is_empty() {
                empty_state(
                    ui,
                    self.tr("No trades yet", "Noch keine Trades"),
                    self.tr(
                        "Trades you enter on Instant Order (a template trade or a single option) appear here, valued live.",
                        "Trades, die du auf Sofortorder eingibst (aus einem Template oder eine einzelne Option), erscheinen hier mit Live-Bewertung.",
                    ),
                );
            } else {
                let titles = [
                    self.tr("Trade", "Trade"),
                    self.tr("Order Ref", "Order-Ref"),
                    self.tr("Strikes", "Strikes"),
                    self.tr("Expiry", "Verfall"),
                    self.tr("Qty", "Menge"),
                    self.tr("Open", "Einstieg"),
                    self.tr("Stop", "Stop"),
                    self.tr("Target", "Ziel"),
                    "Bid",
                    "Mid",
                    "Ask",
                    self.tr("P/L", "G/V"),
                ];
                let clicks = trade_table(ui, &views, titles, self.selected_open_index());
                self.apply_trade_clicks(clicks);
            }
            self.open_trade_actions(ui);
            // The raw executions TWS reports for today: collapsed once the list above has trades.
            if !self.today_trades.is_empty() {
                self.executions_section(ui, views.is_empty());
            }
        });
    }

    /// The raw executions of today as a collapsible table; open when the list of trades is empty.
    fn executions_section(&self, ui: &mut egui::Ui, open: bool) {
        ui.add_space(12.0);
        let heading = format!("{} ({})", self.tr("Executions from TWS", "Ausführungen der TWS"), self.today_trades.len());
        egui::CollapsingHeader::new(RichText::new(heading).size(13.0).color(muted_text())).default_open(open).show(ui, |ui| {
            ui.add_space(4.0);
            let columns = [
                TableColumn::new(self.tr("Time", "Zeit"), 1.8, false),
                TableColumn::new(self.tr("Account", "Konto"), 1.6, false),
                TableColumn::new(self.tr("Expiry", "Verfall"), 1.4, false),
                TableColumn::new(self.tr("Type", "Typ"), 1.0, false),
                TableColumn::new("Strike", 1.0, true),
                TableColumn::new(self.tr("Qty", "Menge"), 0.8, true),
                TableColumn::new(self.tr("Price", "Preis"), 1.0, true),
            ];
            let rows: Vec<Vec<TableCell>> = newest_first(&self.today_trades)
                .into_iter()
                .map(|e| {
                    let kind = match (e.sec_type.as_str(), e.right.as_str()) {
                        ("OPT", "P") => "Put".to_string(),
                        ("OPT", "C") => "Call".to_string(),
                        ("BAG", _) => self.tr("Combo (BAG)", "Kombi (BAG)").to_string(),
                        (other, _) => other.to_string(),
                    };
                    let strike = if e.sec_type == "OPT" { format!("{:.0}", e.strike) } else { "-".to_string() };
                    let qty = format!("{}{:.0}", if e.side == "SLD" { "-" } else { "" }, e.quantity);
                    vec![
                        TableCell::new(execution_time_text(&e.time)),
                        TableCell::new(mask_account(&e.account)),
                        TableCell::new(format_date(&e.expiry)),
                        TableCell::new(kind),
                        TableCell::new(strike),
                        TableCell::new(qty),
                        TableCell::new(format!("{:.2}", e.price)),
                    ]
                })
                .collect();
            let keys = ["time", "account", "expiry", "type", "strike", "qty", "price"];
            let _ = arranged_table(ui, "tws_executions_table", &keys, &columns, &rows, None);
        });
    }

    pub(super) fn dashboard(&mut self, ui: &mut egui::Ui) {
        let figures = self.dashboard_figures();
        ui.heading(RichText::new(self.tr("Dashboard", "Übersicht")).color(self.heading_color()));
        ui.add_space(10.0);
        self.news_bulletin(ui);
        self.dashboard_kpi_row(ui, figures);
        ui.add_space(16.0);
        self.dashboard_trades_card(ui);
        ui.add_space(12.0);

        self.scheduled_tasks_card(ui);
        self.cancel_run_dialog(ui.ctx());
    }

    /// The name of the template of a run's schedule, for the lists of runs.
    pub(super) fn run_template_name(&self, run: &ScheduleRun) -> String {
        let template_id = self.schedule_of_run(run).map(|schedule| schedule.template_id);
        match template_id {
            None => self.tr("(schedule deleted)", "(Zeitplan gelöscht)").to_string(),
            Some(id) => self.template(id).map_or_else(|| self.tr("(template deleted)", "(Template gelöscht)").to_string(), |t| t.name.clone()),
        }
    }

    /// A run's window as text in New York time, e.g. `09:35:00`.
    fn run_time_text(run: &ScheduleRun) -> String {
        run.window_start.with_timezone(&chrono_tz::America::New_York).format("%H:%M:%S").to_string()
    }

    /// A run's window start as text in the local time of this computer, e.g. `15:35:00`.
    fn run_local_time_text(run: &ScheduleRun) -> String {
        run.window_start.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()
    }

    /// The place in `trades_stored` of the selected trade, while it is open; a trade that closed is no longer selected.
    pub(super) fn selected_open_index(&self) -> Option<usize> {
        let key = self.ui_state.selected_trade.as_deref()?;
        self.trades_stored.iter().position(|t| t.is_open() && t.key() == key)
    }

    /// A selected trade was edited (stop/target sent, closed or marked closed, an exit deleted): the selection ends with it, so the list
    /// and the actions below it are free for the next trade.
    pub(super) fn trade_edit_done(&mut self) {
        self.ui_state.selected_trade = None;
    }

    /// The clicks on the list of today's trades: only an open trade can be selected, a click on a closed one changes nothing.
    pub(super) fn apply_trade_clicks(&mut self, clicks: crate::widgets::RowClicks) {
        let current = self.selected_open_index().and(self.ui_state.selected_trade.clone());
        let trades = &self.trades_stored;
        self.ui_state.selected_trade = next_selection(current, clicks, |index| trades.get(index).filter(|t| t.is_open()).map(StoredTrade::key));
    }

    /// The waiting run that starts next within `STARTING_SOON_SECS`, with the seconds left until its start.
    pub(super) fn run_starting_soon(runs: &[ScheduleRun], now: chrono::DateTime<chrono::Utc>) -> Option<(&ScheduleRun, i64)> {
        runs.iter()
            .filter(|run| run.status == RunStatus::Waiting)
            .map(|run| (run, (run.window_start - now).num_seconds()))
            .filter(|(_, left)| (1..=STARTING_SOON_SECS).contains(left))
            .min_by_key(|(_, left)| *left)
    }

    /// The sign of the top bar that a scheduled trade starts within five minutes: a clock at five to twelve and the time left; the
    /// template on hover. A click shows the run on the Dashboard. Nothing while no run starts that soon.
    pub(super) fn starting_soon_item(&mut self, ui: &mut egui::Ui) {
        let now = self.clock.utc();
        let runs = self.dashboard_runs();
        let Some((run, left)) = Self::run_starting_soon(&runs, now) else { return };
        let template = self.run_template_name(run);
        let left_text = format!("{}:{:02}", left / 60, left % 60);
        crate::widgets::bar_separator(ui);
        let size = egui::vec2(22.0, 22.0);
        let (rect, clock) = ui.allocate_exact_size(size, egui::Sense::click());
        let seconds = f32::from(u8::try_from(now.timestamp().rem_euclid(60)).unwrap_or(0));
        ui.painter().extend(crate::metal::five_to_twelve_clock(ui.ctx(), rect.center(), 10.0, super::ORANGE_WARN, seconds));
        let label = ui.add(egui::Label::new(RichText::new(left_text.clone()).color(super::ORANGE_WARN).strong().size(13.0)).sense(egui::Sense::click()));
        let hover = match self.settings.lang {
            Lang::De => format!("Geplanter Trade „{template}“ startet in {left_text} (NY {}).", Self::run_time_text(run)),
            Lang::En => format!("Scheduled trade \"{template}\" starts in {left_text} (NY {}).", Self::run_time_text(run)),
        };
        let clicked = clock.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&hover).clicked()
            || label.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&hover).clicked();
        if clicked {
            self.page = Page::Dashboard;
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }

    /// The time left until a waiting run starts as `HH:MM:SS`; `None` once it has started or for a run that is not waiting.
    pub(super) fn run_countdown(run: &ScheduleRun, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
        if run.status != RunStatus::Waiting {
            return None;
        }
        let left = (run.window_start - now).num_seconds();
        (left > 0).then(|| format!("{:02}:{:02}:{:02}", left / 3600, left % 3600 / 60, left % 60))
    }

    /// Today's runs as the Dashboard lists them, by their time. A run of a deleted schedule is listed only while it has an order or a
    /// trade to show; else it is gone with its schedule (the task log of the reports keeps it).
    pub(super) fn dashboard_runs(&self) -> Vec<ScheduleRun> {
        let today = self.clock.new_york().date();
        let mut runs: Vec<ScheduleRun> = self
            .schedule
            .runs
            .iter()
            .filter(|run| run.day == today)
            .filter(|run| self.schedule_of_run(run).is_some() || matches!(run.status, RunStatus::Working | RunStatus::Complete | RunStatus::Partial))
            .cloned()
            .collect();
        runs.sort_by_key(|run| run.window_start);
        runs
    }

    /// The card "Scheduled Tasks" of the dashboard: today's runs (time, type, status, template). A click
    /// selects a run, the bar below acts on it and its schedule; a double-click opens the schedule.
    fn scheduled_tasks_card(&mut self, ui: &mut egui::Ui) {
        let runs = self.dashboard_runs();
        let now = self.clock.utc();
        let mut group = self.schedule.group;
        framed_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.tr("Scheduled Tasks", "Geplante Aufgaben")).size(16.0).strong().color(text_primary()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    crate::widgets::metal_combo(ui, |ui| {
                        egui::ComboBox::from_id_salt("tasks_group").width(130.0).selected_text(group_name(group)).show_ui(ui, |ui| {
                            for candidate in 0..GROUPS {
                                ui.selectable_value(&mut group, candidate, group_name(candidate));
                            }
                        })
                    });
                    ui.label(RichText::new(self.tr("Set:", "Satz:")).color(muted_text()));
                });
            });
            ui.add_space(6.0);
            if runs.is_empty() {
                empty_state(
                    ui,
                    self.tr("No scheduled tasks today", "Heute keine geplanten Aufgaben"),
                    self.tr("Scheduled trades of today are listed here.", "Die geplanten Trades von heute werden hier aufgelistet."),
                );
                return;
            }
            let columns = [
                TableColumn::new(self.tr("Time (NY)", "Zeit (NY)"), 1.0, false),
                TableColumn::new(self.tr("Local time", "Ortszeit"), 1.0, false),
                TableColumn::new(self.tr("Type", "Typ"), 1.3, false),
                TableColumn::new("Status", 1.6, false),
                TableColumn::new("Template", 3.0, false),
            ];
            let rows: Vec<Vec<TableCell>> = runs
                .iter()
                .map(|run| {
                    let kind = if run.reentry.is_some() {
                        self.tr("Re-entry", "Wiedereinstieg")
                    } else if run.trigger.is_some() {
                        "Trigger"
                    } else {
                        self.tr("Scheduled trade", "Geplanter Trade")
                    };
                    let status = match Self::run_countdown(run, now) {
                        Some(left) => {
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                            format!("{} ({left})", self.run_status_text(run.status))
                        }
                        None => self.run_status_text(run.status),
                    };
                    vec![
                        TableCell::new(Self::run_time_text(run)),
                        TableCell::new(Self::run_local_time_text(run)),
                        TableCell::new(kind),
                        TableCell::new(status),
                        TableCell::new(self.run_template_name(run)),
                    ]
                })
                .collect();
            let selected = self.schedule.selected_run.and_then(|id| runs.iter().position(|run| run.id == id));
            let keys = ["time_ny", "local_time", "type", "status", "template"];
            let clicks = arranged_table(ui, "scheduled_tasks_table", &keys, &columns, &rows, Some(RowSelection { selected, ticked: &[], locked: &[] }));
            self.schedule.selected_run = next_selection(self.schedule.selected_run, clicks, |index| runs.get(index).map(|run| run.id));
            if let Some(run) = clicks.double_clicked.and_then(|index| runs.get(index)).filter(|run| run.schedule_id > 0) {
                self.edit_schedule(run.schedule_id);
            }
            let selected = self.schedule.selected_run.and_then(|id| runs.iter().find(|run| run.id == id)).cloned();
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                self.schedule_action_buttons(ui, selected.as_ref().map(|run| run.schedule_id).filter(|id| *id > 0));
                ui.separator();
                let status = selected.as_ref().map(|run| run.status);
                let (pause_text, pause) =
                    if status == Some(RunStatus::Paused) { (self.tr("Resume", "Fortsetzen"), false) } else { (self.tr("Pause", "Pausieren"), true) };
                let can_pause = matches!(status, Some(RunStatus::Waiting | RunStatus::Paused));
                if ui.add_enabled(can_pause, crate::widgets::MetalButton::new(pause_text)).clicked() {
                    if let Some(run) = &selected {
                        self.pause_schedule_run(run.id, pause);
                    }
                }
                if ui.add_enabled(can_pause, crate::widgets::MetalButton::new(self.tr("Cancel task", "Aufgabe abbrechen"))).clicked() {
                    self.schedule.confirm_cancel_run = selected.as_ref().map(|run| run.id);
                }
            });
            ui.colored_label(
                muted_text(),
                self.tr(
                    "Click a row to select it, double-click to edit its schedule.",
                    "Zeile anklicken zum Auswählen, Doppelklick zum Bearbeiten des Zeitplans.",
                ),
            );
            self.schedule_delete_question(ui);
        });
        self.set_active_group(group);
    }

    /// Edit, Copy, Enable or Disable and Delete for the schedule `id`; all greyed out without one.
    pub(super) fn schedule_action_buttons(&mut self, ui: &mut egui::Ui, id: Option<u32>) {
        let schedule = id.and_then(|id| self.schedule.list.iter().find(|s| s.id == id)).cloned();
        let has = schedule.is_some();
        let active = schedule.as_ref().is_some_and(|s| s.active);
        if ui.add_enabled(has, crate::widgets::MetalButton::new(self.tr("Edit", "Bearbeiten"))).clicked() {
            if let Some(s) = &schedule {
                self.edit_schedule(s.id);
            }
        }
        if ui.add_enabled(has, crate::widgets::MetalButton::new(self.tr("Copy", "Kopieren"))).clicked() {
            if let Some(s) = &schedule {
                self.schedule.editing = Some(ScheduleForm::from_schedule(s, true));
                self.schedule.selected = Some(s.id);
                self.schedule.errors.clear();
                self.page = Page::Schedules;
            }
        }
        let toggle = if active { self.tr("Disable", "Deaktivieren") } else { self.tr("Enable", "Aktivieren") };
        if ui.add_enabled(has, crate::widgets::MetalButton::new(toggle)).clicked() {
            if let Some(s) = &schedule {
                self.schedule.selected = Some(s.id);
                self.set_schedule_active(!active);
            }
        }
        if ui.add_enabled(has, crate::widgets::MetalButton::new(self.tr("Delete", "Löschen"))).clicked() {
            self.schedule.confirm_delete = schedule.as_ref().map(|s| s.id);
        }
    }

    /// The person holds a waiting run (`pause`) or lets a paused one go on again. A paused run makes no
    /// attempt, but its window runs out all the same. A run that is neither waiting nor paused is left alone.
    pub(super) fn pause_schedule_run(&mut self, id: u32, pause: bool) {
        let (from, to, word) = if pause { (RunStatus::Waiting, RunStatus::Paused, "paused") } else { (RunStatus::Paused, RunStatus::Waiting, "resumed") };
        let Some(run) = self.schedule.runs.iter_mut().find(|run| run.id == id && run.status == from) else {
            return;
        };
        run.status = to;
        self.log(&format!("Schedule run {id} {word} by the user"));
        self.save_schedule_runs();
    }

    /// The question "Cancel this task?" before a run is cancelled; its answer is applied here.
    fn cancel_run_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.schedule.confirm_cancel_run else {
            return;
        };
        let Some(run) = self.schedule.runs.iter().find(|run| run.id == id && run.status.is_open()).cloned() else {
            self.schedule.confirm_cancel_run = None;
            return;
        };
        let (mut yes, mut no) = (false, false);
        egui::Window::new(self.tr("Cancel this task?", "Diese Aufgabe abbrechen?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("{} (New York)  {}", Self::run_time_text(&run), self.run_template_name(&run)));
                ui.label(self.tr(
                    "The trade is not entered today by this schedule. This cannot be undone.",
                    "Der Trade wird heute von diesem Zeitplan nicht mehr eingegeben. Das lässt sich nicht rückgängig machen.",
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    yes = ui.add(crate::widgets::MetalButton::new(self.tr("Yes, cancel the task", "Ja, Aufgabe abbrechen"))).clicked();
                    no = ui.add(crate::widgets::MetalButton::new(self.tr("No, keep it", "Nein, behalten"))).clicked();
                });
            });
        if yes {
            self.cancel_schedule_run(id);
        }
        if yes || no {
            self.schedule.confirm_cancel_run = None;
        }
    }

    /// The person cancels a run that has not begun (waiting or paused; a run with an order in TWS is not
    /// touched: its order has its own handling). The run ends as `Cancelled` and the schedule does not try
    /// again today.
    pub(super) fn cancel_schedule_run(&mut self, id: u32) {
        let Some(index) = self.schedule.runs.iter().position(|run| run.id == id && run.status.is_open()) else {
            return;
        };
        self.end_schedule_run(index, RunStatus::Cancelled, "cancelled by the user");
        self.save_schedule_runs();
    }

    /// Reports a run that expired without an entry through the notification channels, if its schedule asks
    /// for it, TWS is connected and the market is open (a run that ended while the market was closed or the
    /// app was not connected is not news).
    pub(super) fn notify_expired_run(&self, run_id: u32) {
        if let Some(message) = self.expired_run_report(run_id) {
            self.notify_trade_event("schedule_expired", &message, true, true);
        }
    }

    /// The text to report for an expired run, or `None` if it is not to be reported.
    pub(super) fn expired_run_report(&self, run_id: u32) -> Option<String> {
        let run = self.schedule.runs.iter().find(|run| run.id == run_id)?;
        let schedule = &self.schedule_of_run(run)?;
        if !schedule.notify_on_expiry || self.tws_cmd_sender.is_none() || !self.market_is_open() {
            return None;
        }
        Some(format!("Schedule run {run_id} ({}) expired without an entry: {}", self.schedule_label(schedule), run.notes))
    }

    /// The Reports tab "Task Log": every run of the last week, newest first, with what happened.
    pub(super) fn task_log_tab(&self, ui: &mut egui::Ui) {
        let mut runs: Vec<&ScheduleRun> = self.schedule.runs.iter().collect();
        runs.sort_by_key(|run| std::cmp::Reverse(run.window_start));
        if runs.is_empty() {
            empty_state(
                ui,
                self.tr("No tasks in the last week", "Keine Aufgaben in der letzten Woche"),
                self.tr("Every run of a schedule is listed here with what happened.", "Jeder Lauf eines Zeitplans wird hier mit seinem Ergebnis aufgelistet."),
            );
            return;
        }
        let columns = [
            TableColumn::new(self.tr("Date", "Datum"), 1.1, false),
            TableColumn::new(self.tr("Time (NY)", "Zeit (NY)"), 0.9, false),
            TableColumn::new("Template", 1.8, false),
            TableColumn::new("Status", 1.0, false),
            TableColumn::new(self.tr("Tries", "Versuche"), 0.7, true),
            TableColumn::new(self.tr("What happened", "Was geschah"), 4.0, false),
        ];
        let rows: Vec<Vec<TableCell>> = runs
            .iter()
            .map(|run| {
                vec![
                    TableCell::new(show_date(run.day)),
                    TableCell::new(Self::run_time_text(run)),
                    TableCell::new(self.run_template_name(run)),
                    TableCell::new(self.run_status_text(run.status)),
                    TableCell::new(run.attempts.to_string()),
                    TableCell::new(run.notes.clone()),
                ]
            })
            .collect();
        let keys = ["date", "time_ny", "template", "status", "tries", "what_happened"];
        let _ = arranged_table(ui, "task_log_table", &keys, &columns, &rows, None);
    }
}
