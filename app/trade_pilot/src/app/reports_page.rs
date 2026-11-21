//! The Reports page and the CSV export: the trades of a period, their figures, the equity curve, the days and the trade log.

use super::{
    credit_summary, data_table, display_currency, display_percent, draw_gauge, empty_state, line_chart, mask_account, mode_dir, muted_text, profit_loss,
    reports, show_stored_day, text_primary, white_card, App, Decimal, GaugeSpec, PathBuf, ReportsTab, RichText, Session, StoredTrade, TableCell, TableColumn,
    TradeRecord, TradeStatus, PCR_TICKS, STATUS_ERROR, STATUS_OK, WIN_TICKS,
};
use eframe::egui;
use rust_decimal::prelude::ToPrimitive;

impl App {
    /// The stored trades whose opening day lies in the report period (both days included).
    pub(super) fn report_trades(&self) -> Vec<&StoredTrade> {
        let (start, end) = (self.reports.start.format("%Y-%m-%d").to_string(), self.reports.end.format("%Y-%m-%d").to_string());
        self.trade_log
            .iter()
            .filter(|t| {
                !t.is_split()
                    && t.day >= start
                    && t.day <= end
                    && self.reports.account.as_deref().is_none_or(|a| t.account == a)
                    && self.reports.strategy.as_deref().is_none_or(|s| t.strategy == s)
            })
            .collect()
    }

    /// The report period's trades as the figures count them (with their day and side): live-valued
    /// while open, unknown entries left out.
    pub(super) fn report_rows(&self) -> Vec<reports::ReportRow> {
        self.report_trades()
            .into_iter()
            .filter_map(|t| Some(reports::ReportRow { day: t.day.clone(), side: reports::side_of(&t.trade_type), record: self.record_of(t)? }))
            .collect()
    }

    /// The same records for the Overview's summary - the same code as the Dashboard, over a
    /// different set of trades.
    pub(super) fn report_records(&self) -> Vec<TradeRecord> {
        self.report_rows().into_iter().map(|r| r.record).collect()
    }

    /// The period's trades, newest first, as CSV text.
    pub(super) fn report_csv(&self) -> String {
        self.csv_of(self.report_trades())
    }

    /// `trades` newest first as CSV text.
    pub(super) fn csv_of(&self, mut trades: Vec<&StoredTrade>) -> String {
        trades.sort_by(|a, b| b.day.cmp(&a.day).then(b.order_id.cmp(&a.order_id)));
        let rows: Vec<reports::CsvTrade<'_>> = trades
            .iter()
            .map(|t| reports::CsvTrade {
                trade: t,
                profit: self.record_of(t).and_then(|r| reports::profit(&r)),
                fees: crate::fees::trade_fees(t, &self.exec_history, &self.commissions).total(),
            })
            .collect();
        reports::trades_csv(&rows, &|account| if account.chars().count() >= 8 { mask_account(&format!("IB:{account}")) } else { account.to_string() })
    }

    /// Writes `csv` as `name` to the `exports` folder of the mode (there is no file dialog) and returns where it is.
    fn write_export(&self, name: &str, csv: &str) -> std::io::Result<PathBuf> {
        let folder = mode_dir(self.link.trading_live)?.join("exports");
        std::fs::create_dir_all(&folder)?;
        let path = folder.join(name);
        std::fs::write(&path, csv)?;
        Ok(path)
    }

    /// "Export to CSV": writes the period's trades to the `exports` folder of the mode and says where.
    fn export_report_csv(&mut self) {
        let count = self.report_trades().len();
        let name = format!("TradePilot-Trades-{}_{}.csv", self.reports.start.format("%d-%m-%Y"), self.reports.end.format("%d-%m-%Y"));
        let message = match self.write_export(&name, &self.report_csv()) {
            Ok(path) => format!("{} {count} {} {}", self.tr("Exported", "Exportiert:"), self.tr("trade(s) to", "Trade(s) nach"), path.display()),
            Err(error) => format!("{}: {error}", self.tr("Export failed", "Export fehlgeschlagen")),
        };
        self.log(&message);
        self.reports.export_status = Some(message);
    }

    /// The automatic daily export (Settings > Database Backup): once the market has closed, the trades of the day are written to
    /// `TradePilot-Trades-<DD-MM-YYYY>.csv` in the `exports` folder, whatever the filters of the Reports page. Asked every second; it does
    /// its work once a day (an app started after the close exports once as well, the file of the day is then written again).
    pub(super) fn daily_csv_tick(&mut self) {
        let today = self.clock.local().format("%Y-%m-%d").to_string();
        if !self.settings.daily_csv_export || self.market_session() != Session::AfterClose || self.csv_exported_day.as_deref() == Some(today.as_str()) {
            return;
        }
        self.csv_exported_day = Some(today.clone());
        let trades: Vec<&StoredTrade> = self.trade_log.iter().filter(|t| t.day == today).collect();
        let (count, csv) = (trades.len(), self.csv_of(trades));
        let name = format!("TradePilot-Trades-{}.csv", self.clock.local().format("%d-%m-%Y"));
        if cfg!(test) {
            return;
        }
        match self.write_export(&name, &csv) {
            Ok(path) => self.log(&format!("Daily export: {count} trade(s) of today written to {}", path.display())),
            Err(error) => self.log(&format!("WARNING: Daily export of the trades failed: {error}")),
        }
    }

    /// The period's days, oldest first (see `reports::daily_totals`).
    fn report_days(&self) -> Vec<reports::DayTotal> {
        reports::daily_totals(&self.report_rows()).unwrap_or_default()
    }

    /// Reports > Equity Curve: the running total of the profit over the days of the period.
    fn equity_curve_tab(&mut self, ui: &mut egui::Ui) {
        let days = self.report_days();
        let curve = reports::equity_curve(&days).unwrap_or_default();
        if curve.is_empty() {
            empty_state(
                ui,
                self.tr("No profit to show", "Kein Ergebnis anzuzeigen"),
                self.tr(
                    "No day of this period has a known profit yet. Pick another period.",
                    "Für keinen Tag dieses Zeitraums ist schon ein Ergebnis bekannt. Wähle einen anderen Zeitraum.",
                ),
            );
            return;
        }
        let points: Vec<(String, f64)> = curve.iter().map(|(day, total)| (day.clone(), total.to_f64().unwrap_or(0.0))).collect();
        let unknown = days.iter().filter(|d| d.profit.is_none()).count();
        ui.label(
            RichText::new(self.tr("Running total of the profit per day (credit trades).", "Laufende Summe des Ergebnisses je Tag (Credit-Trades)."))
                .color(muted_text()),
        );
        ui.add_space(6.0);
        line_chart(ui, &points, 280.0);
        if unknown > 0 {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "{unknown} {}",
                    self.tr("day(s) with a trade that has no price yet are not on the curve.", "Tag(e) mit einem noch unbewerteten Trade fehlen in der Kurve.")
                ))
                .color(muted_text()),
            );
        }
    }

    /// Reports > Daily View: one line per day of the period with trades, premium and profit.
    fn daily_view_tab(&mut self, ui: &mut egui::Ui) {
        let days = self.report_days();
        if days.is_empty() {
            empty_state(
                ui,
                self.tr("No trades in this period", "Keine Trades in diesem Zeitraum"),
                self.tr("Pick another start or end date.", "Wähle ein anderes Start- oder Enddatum."),
            );
            return;
        }
        let columns = [
            TableColumn::new(self.tr("Date", "Datum"), 1.4, false),
            TableColumn::new("Trades", 0.8, true),
            TableColumn::new(self.tr("Premium Sold", "Prämie verkauft"), 1.4, true),
            TableColumn::new(self.tr("Wins", "Gewinne"), 0.8, true),
            TableColumn::new(self.tr("Losses", "Verluste"), 0.8, true),
            TableColumn::new(self.tr("P/L", "G/V"), 1.4, true),
            TableColumn::new(self.tr("Running total", "Summe"), 1.4, true),
        ];
        // The same rule as the curve: a day without a known profit adds nothing (and shows "-").
        let mut running = Decimal::ZERO;
        let mut rows: Vec<Vec<TableCell>> = days
            .iter()
            .map(|d| {
                let shown = d.profit.and_then(|p| running.checked_add(p));
                running = shown.unwrap_or(running);
                let color = match d.profit {
                    Some(p) if p > Decimal::ZERO => STATUS_OK,
                    Some(p) if p < Decimal::ZERO => STATUS_ERROR,
                    _ => text_primary(),
                };
                vec![
                    TableCell::new(show_stored_day(&d.day)),
                    TableCell::new(d.trades.to_string()),
                    TableCell::new(display_currency(d.premium_sold)),
                    TableCell::new(d.wins.to_string()),
                    TableCell::new(d.losses.to_string()),
                    TableCell { text: d.profit.map_or_else(|| "-".to_string(), display_currency), color: Some(color), strong: true, ..TableCell::default() },
                    TableCell::new(shown.map_or_else(|| "-".to_string(), display_currency)),
                ]
            })
            .collect();
        // Newest day first, like the trade log (the running total was built oldest first).
        rows.reverse();
        data_table(ui, "daily_view_table", &columns, &rows);
    }

    /// Reports > Trade Log: the period's stored trades, newest first, with entry, close and profit.
    fn trade_log_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.add(crate::widgets::MetalButton::new(self.tr("Refresh", "Aktualisieren"))).clicked() {
                self.reload_trade_log();
            }
            ui.label(
                RichText::new(self.tr(
                    "Trades opened by TradePilot, plus positions recovered from TWS bookings (entry unknown = '?').",
                    "Von TradePilot eröffnete Trades und aus TWS-Buchungen wiederhergestellte Positionen (Einstieg unbekannt = '?').",
                ))
                .color(muted_text()),
            );
        });
        ui.add_space(8.0);
        if self.report_trades().is_empty() {
            empty_state(
                ui,
                self.tr("No trades in this period", "Keine Trades in diesem Zeitraum"),
                self.tr(
                    "Pick another start or end date. Trades appear once TradePilot has seen their fills or expiry.",
                    "Wähle ein anderes Start- oder Enddatum. Trades erscheinen, sobald TradePilot ihre Fills oder ihren Verfall gesehen hat.",
                ),
            );
            return;
        }
        let columns = [
            TableColumn::new(self.tr("Date", "Datum"), 1.2, false),
            TableColumn::new("Trade", 1.6, false),
            TableColumn::new("Strikes", 2.4, false),
            TableColumn::new(self.tr("Qty", "Menge"), 0.7, true),
            TableColumn::new(self.tr("Entry", "Einstieg"), 1.0, true),
            TableColumn::new(self.tr("Close", "Ausstieg"), 1.0, true),
            TableColumn::new(self.tr("P/L", "G/V"), 1.4, true),
            TableColumn::new("Status", 1.2, false),
        ];
        let money = |v: Option<Decimal>| {
            v.map_or_else(|| "-".to_string(), |v| format!("{:.2}", v.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)))
        };
        let mut trades: Vec<&StoredTrade> = self.report_trades();
        trades.sort_by(|a, b| b.day.cmp(&a.day).then(b.order_id.cmp(&a.order_id)));
        let rows: Vec<Vec<TableCell>> = trades
            .iter()
            .map(|t| {
                let open: Option<Decimal> = Some(t.price_open);
                let close: Option<Decimal> = t.price_close;
                let profit = if t.entry_unknown { None } else { open.zip(close).and_then(|(o, c)| profit_loss(o, c, t.qty, t.multiplier)) };
                let color = match profit {
                    Some(p) if p > Decimal::ZERO => STATUS_OK,
                    Some(p) if p < Decimal::ZERO => STATUS_ERROR,
                    _ => text_primary(),
                };
                let day = show_stored_day(&t.day);
                vec![
                    TableCell::new(day),
                    TableCell::new(t.trade_type.clone()),
                    TableCell::new(t.strikes.clone()),
                    TableCell::new(t.qty.to_string()),
                    TableCell::new(if t.entry_unknown { "?".to_string() } else { money(open) }),
                    TableCell::new(money(close)),
                    TableCell { text: profit.map_or_else(|| "-".to_string(), display_currency), color: Some(color), strong: true, ..TableCell::default() },
                    TableCell::new(t.status.clone()),
                ]
            })
            .collect();
        data_table(ui, "trade_log_table", &columns, &rows);
    }

    fn preset_label(&self, preset: reports::Preset) -> &'static str {
        use reports::Preset;
        match preset {
            Preset::Today => self.tr("Today", "Heute"),
            Preset::Yesterday => self.tr("Yesterday", "Gestern"),
            Preset::ThisWeek => self.tr("This week", "Diese Woche"),
            Preset::Last7Days => self.tr("Last 7 days", "Letzte 7 Tage"),
            Preset::ThisMonth => self.tr("This month", "Dieser Monat"),
        }
    }

    /// "Report Date" (a ready-made period that sets the two dates) and "Account" (limits the report to one account).
    fn report_period_and_account_pickers(&mut self, ui: &mut egui::Ui) {
        let today = self.clock.local().date_naive();
        let current = reports::Preset::of(self.reports.start, self.reports.end, today);
        ui.vertical(|ui| {
            ui.label(self.tr("Report Date", "Berichtsdatum"));
            let shown = current.map_or(self.tr("Custom", "Eigener Zeitraum"), |p| self.preset_label(p));
            crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("reports_preset").selected_text(shown).show_ui(ui, |ui| {
                    for preset in reports::Preset::ALL {
                        if ui.selectable_label(current == Some(preset), self.preset_label(preset)).clicked() {
                            (self.reports.start, self.reports.end) = preset.period(today);
                        }
                    }
                })
            });
        });
        ui.add_space(14.0);
        let mut accounts: Vec<String> = self.trade_log.iter().map(|t| t.account.clone()).collect();
        accounts.sort();
        accounts.dedup();
        ui.vertical(|ui| {
            ui.label(self.tr("Account", "Konto"));
            let shown = self.reports.account.as_deref().map_or_else(|| self.tr("All", "Alle").to_string(), mask_account);
            crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("reports_account").selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(self.reports.account.is_none(), self.tr("All", "Alle")).clicked() {
                        self.reports.account = None;
                    }
                    for account in accounts {
                        let selected = self.reports.account.as_deref() == Some(account.as_str());
                        if ui.selectable_label(selected, mask_account(&account)).clicked() {
                            self.reports.account = Some(account);
                        }
                    }
                })
            });
        });
        ui.add_space(14.0);
    }

    /// "Strategy": limits the report to the trades of one strategy (the strategy of the template they were entered with);
    /// trades stored before the strategy was kept are listed as "(none)".
    fn report_strategy_picker(&mut self, ui: &mut egui::Ui) {
        let mut strategies: Vec<String> = self.trade_log.iter().map(|t| t.strategy.clone()).collect();
        strategies.sort();
        strategies.dedup();
        let none = self.tr("(none)", "(keine)");
        let name_of = |strategy: &str| if strategy.is_empty() { none.to_string() } else { strategy.to_string() };
        ui.vertical(|ui| {
            ui.label(self.tr("Strategy", "Strategie"));
            let shown = self.reports.strategy.as_deref().map_or_else(|| self.tr("All", "Alle").to_string(), name_of);
            crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("reports_strategy").selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(self.reports.strategy.is_none(), self.tr("All", "Alle")).clicked() {
                        self.reports.strategy = None;
                    }
                    for strategy in strategies {
                        let selected = self.reports.strategy.as_deref() == Some(strategy.as_str());
                        if ui.selectable_label(selected, name_of(&strategy)).clicked() {
                            self.reports.strategy = Some(strategy);
                        }
                    }
                })
            });
        });
        ui.add_space(14.0);
    }

    pub(super) fn reports_page(&mut self, ui: &mut egui::Ui) {
        if self.pending.trade_log_dirty {
            self.reload_trade_log();
        }
        let period_before = (self.reports.start, self.reports.end);
        ui.heading(RichText::new(self.tr("Reports", "Berichte")).color(self.heading_color()));
        ui.add_space(10.0);
        white_card(ui, |ui| {
            ui.horizontal(|ui| {
                self.report_period_and_account_pickers(ui);
                self.report_strategy_picker(ui);
                ui.vertical(|ui| {
                    ui.label(self.tr("Start Date", "Startdatum"));
                    ui.add(egui_extras::DatePickerButton::new(&mut self.reports.start).id_salt("reports_start").format(crate::i18n::date_pattern()));
                });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.label(self.tr("End Date", "Enddatum"));
                    ui.add(egui_extras::DatePickerButton::new(&mut self.reports.end).id_salt("reports_end").format(crate::i18n::date_pattern()));
                });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.add_space(18.0);
                    if ui.add(crate::widgets::MetalButton::new(self.tr("Export to CSV", "Als CSV exportieren"))).clicked() {
                        self.export_report_csv();
                    }
                });
            });
        });
        if let Some(status) = &self.reports.export_status {
            ui.label(RichText::new(status).color(muted_text()).size(13.0));
        }
        // Keep the period valid: the date just picked wins over the other one.
        if self.reports.start > self.reports.end {
            if self.reports.start == period_before.0 {
                self.reports.start = self.reports.end;
            } else {
                self.reports.end = self.reports.start;
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            for (tab, label) in [
                (ReportsTab::Overview, self.tr("Overview", "Übersicht")),
                (ReportsTab::EquityCurve, self.tr("Equity Curve", "Kapitalkurve")),
                (ReportsTab::DailyView, self.tr("Daily View", "Tagesansicht")),
                (ReportsTab::TradeLog, self.tr("Trade Log", "Trade-Protokoll")),
                (ReportsTab::TaskLog, self.tr("Task Log", "Aufgaben-Protokoll")),
            ] {
                // The tabs are metal buttons; the chosen one is tinted blue.
                let button = if self.reports.tab == tab {
                    crate::widgets::MetalButton::new(label).fill(crate::theme::ACCENT_BLUE)
                } else {
                    crate::widgets::MetalButton::new(label)
                };
                if ui.add(button).clicked() {
                    self.reports.tab = tab;
                }
            }
        });
        ui.add_space(10.0);
        match self.reports.tab {
            ReportsTab::Overview => self.reports_overview(ui),
            ReportsTab::TradeLog => self.trade_log_tab(ui),
            ReportsTab::EquityCurve => self.equity_curve_tab(ui),
            ReportsTab::DailyView => self.daily_view_tab(ui),
            ReportsTab::TaskLog => self.task_log_tab(ui),
        }
    }

    fn reports_overview(&mut self, ui: &mut egui::Ui) {
        let records = self.report_records();
        // The same live figures as the Dashboard: the credit trades TradePilot opened today.
        let summary = credit_summary(&records);
        let hundred = |v: Decimal| v.checked_mul(Decimal::ONE_HUNDRED).and_then(|v| v.to_f64()).unwrap_or(0.0);
        let (sold, captured, pcr_text, win_text, pcr, win_rate) = match &summary {
            Some(s) if s.captured_unknown => {
                (display_currency(s.premium_sold), "-".to_string(), "-".to_string(), display_percent(s.win_rate), 0.0, hundred(s.win_rate))
            }
            Some(s) => (
                display_currency(s.premium_sold),
                display_currency(s.premium_captured),
                display_percent(s.pcr),
                display_percent(s.win_rate),
                hundred(s.pcr),
                hundred(s.win_rate),
            ),
            None => ("-".into(), "-".into(), "-".into(), "-".into(), 0.0, 0.0),
        };
        let expired = records.iter().filter(|t| t.status == TradeStatus::Expired).count();
        let stopped = records.iter().filter(|t| t.status == TradeStatus::Stopped).count();
        let sweep = self.gauge_sweep();
        ui.horizontal(|ui| {
            white_card(ui, |ui| {
                ui.set_width(240.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(self.tr("CREDIT TRADES", "CREDIT-TRADES")).strong()));
                ui.add_space(6.0);
                egui::Grid::new("reports_credit_grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                    // Avg BP and Return on BP need the account's buying power per trade, which
                    // this app does not track: "-" rather than a made-up number.
                    for (label, value) in [
                        (self.tr("Premium Sold:", "Prämie verkauft:"), sold.as_str()),
                        (self.tr("Premium Captured:", "Prämie vereinnahmt:"), captured.as_str()),
                        (self.tr("PCR:", "PCR:"), pcr_text.as_str()),
                        (self.tr("Win Rate:", "Gewinnrate:"), win_text.as_str()),
                        (self.tr("Avg BP:", "Ø Kaufkraft:"), "-"),
                        (self.tr("Return on BP:", "Rendite auf Kaufkraft:"), "-"),
                    ] {
                        ui.label(label);
                        ui.label(value);
                        ui.end_row();
                    }
                });
            });
            ui.add_space(20.0);
            ui.vertical(|ui| {
                ui.colored_label(crate::theme::positive_text(), format!("{expired} {}", self.tr("Expired Trades", "verfallene Trades")));
                ui.colored_label(crate::theme::negative_text(), format!("{stopped} Stops"));
            });
            ui.add_space(20.0);
            // Size both gauges from whatever width is actually left after the card and the
            // expired/stops labels, rather than a fixed 200px that can exceed it. The floor must
            // stay low enough to never exceed what `available_width` actually reports - a higher
            // floor (tried: 110) can force more total width than exists and push the second
            // gauge off-window entirely, which is worse than a small gauge.
            let gauge_size = ((ui.available_width() - 12.0) / 2.0).clamp(50.0, 200.0);
            let win_label = self.tr("Win Rate", "Gewinnrate");
            draw_gauge(ui, gauge_size, pcr, &GaugeSpec { min: -100.0, max: 100.0, ticks: &PCR_TICKS, title: "PCR", value_text: &pcr_text, sweep });
            ui.add_space(12.0);
            draw_gauge(ui, gauge_size, win_rate, &GaugeSpec { min: 0.0, max: 100.0, ticks: &WIN_TICKS, title: win_label, value_text: &win_text, sweep });
        });
        ui.add_space(12.0);
        self.report_stat_cards(ui);
    }

    /// The four cards under the credit figures of the report: details, daily, puts and calls.
    fn report_stat_cards(&mut self, ui: &mut egui::Ui) {
        let stats = reports::report_stats(&self.report_rows()).unwrap_or_default();
        let money = |v: Option<Decimal>| v.map_or_else(|| "-".to_string(), display_currency);
        let percent = |v: Option<Decimal>| v.map_or_else(|| "-".to_string(), display_percent);
        let side_rows = |s: &reports::SideStats| {
            vec![
                (self.tr("Premium Sold:", "Prämie verkauft:"), money((s.trades > 0).then_some(s.premium_sold))),
                (self.tr("Avg Premium:", "Ø Prämie:"), money(s.avg_premium)),
                (self.tr("Win Rate:", "Gewinnrate:"), percent(s.win_rate)),
            ]
        };
        // The slippage (fill against the quote at order time) is not recorded yet: "-".
        let cards = [
            (
                self.tr("Details", "Details"),
                vec![
                    (self.tr("Avg Premium:", "Ø Prämie:"), money(stats.avg_premium)),
                    (self.tr("Avg Slippage:", "Ø Slippage:"), "-".to_string()),
                    (self.tr("Fees/Commission:", "Gebühren/Provision:"), money(reports::total_fees(&self.report_rows()))),
                ],
            ),
            (
                self.tr("Daily", "Täglich"),
                vec![
                    (self.tr("Days:", "Tage:"), stats.days.to_string()),
                    (self.tr("Wins:", "Gewinne:"), stats.winning_days.to_string()),
                    (self.tr("Losses:", "Verluste:"), stats.losing_days.to_string()),
                ],
            ),
            ("Puts", side_rows(&stats.puts)),
            ("Calls", side_rows(&stats.calls)),
        ];
        ui.columns(4, |columns| {
            for (col, (title, rows)) in columns.iter_mut().zip(cards) {
                white_card(col, |ui| {
                    ui.vertical_centered(|ui| ui.label(RichText::new(title).strong().size(15.0)));
                    ui.add_space(8.0);
                    egui::Grid::new(format!("report_card_{title}")).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        for (label, value) in rows {
                            ui.label(label);
                            ui.label(RichText::new(value).strong());
                            ui.end_row();
                        }
                    });
                });
            }
        });
    }
}
