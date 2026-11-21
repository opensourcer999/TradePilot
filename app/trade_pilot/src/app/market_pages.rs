//! The pages Live Data and Quotes: the index quotes and the quote rows with their toolbar.

use super::{
    card_bg, format_date, format_delta, format_strike, header_text, muted_text, navy_bar, App, ChainSymbol, Color32, OptionContract, RichText, Session,
};
use crate::widgets::PlasticRows;
use eframe::egui;

impl App {
    pub(super) fn live_data_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(self.tr("Live Data", "Live-Daten")).color(self.heading_color()));
            ui.add_space(8.0);
            ui.label(RichText::new(self.tr("(TWS data)", "(TWS-Daten)")).italics().color(muted_text()));
        });
        ui.add_space(10.0);
        navy_bar(ui, self.tr("Index Quotes", "Index-Kurse"));
        egui::Frame::none().fill(card_bg()).show(ui, |ui| {
            let mut plastic = PlasticRows::default();
            let grid = egui::Grid::new("live_data_grid").min_col_width(86.0).spacing([8.0, 10.0]).show(ui, |ui| {
                plastic.row(ui, true);
                ui.label("");
                // English-only column headers for this grid;
                // not run through `self.tr` since there's no verified German translation for them.
                for h in ["Price", "Updated", "Change", "Intraday Move", "Today's Open", "Yesterday's Close", "Gap", "EM Low", "EM High"] {
                    ui.label(RichText::new(h).strong().color(Color32::WHITE));
                }
                ui.end_row();
                for quote in &self.quotes {
                    plastic.row(ui, false);
                    ui.strong(&quote.label);
                    ui.monospace(quote.last.or(quote.history.as_ref().map(|(close, _)| *close)).map_or_else(|| "-".to_string(), |v| format!("{v:.2}")));
                    ui.label(
                        quote
                            .last_update
                            .clone()
                            .or_else(|| quote.history.as_ref().map(|(_, date)| format!("{} {}", self.tr("Close", "Schluss"), format_date(date))))
                            .unwrap_or_else(|| "-".to_string()),
                    );
                    for (current, base) in [(quote.last, quote.reference_close()), (quote.last, quote.open)] {
                        let (text, color) = format_delta(current, base);
                        ui.label(RichText::new(text).color(color));
                    }
                    ui.monospace(quote.open.map_or_else(|| "-".to_string(), |v| format!("{v:.2}")));
                    // Never today's forming daily bar: with the market open that is the price itself, not yesterday's close.
                    ui.monospace(quote.reference_close().map_or_else(|| "-".to_string(), |v| format!("{v:.2}")));
                    let (gap_text, gap_color) = format_delta(quote.open, quote.reference_close());
                    ui.label(RichText::new(gap_text).color(gap_color));
                    // Expected Move needs a real option-chain straddle subscription that is
                    // not set up yet - shown as "-" rather than a made-up number.
                    ui.label("-");
                    ui.label("-");
                    ui.end_row();
                }
            });
            plastic.paint(ui, grid.response.rect, 10.0);
        });
        ui.add_space(18.0);
        ui.strong(self.tr("Live Positions (from TWS)", "Live-Positionen (von der TWS)"));
        ui.add_space(6.0);
        if self.session.server_version.is_none() {
            ui.colored_label(
                muted_text(),
                self.tr("Connect to TWS to retrieve real open positions.", "Mit der TWS verbinden, um echte offene Positionen abzurufen."),
            );
        } else if self.positions.is_empty() && !self.loaded.positions_loaded {
            ui.colored_label(muted_text(), self.tr("Waiting for positions from TWS...", "Warte auf Positionen von der TWS..."));
        } else if self.positions.is_empty() {
            ui.colored_label(muted_text(), self.tr("TWS reports no open positions.", "Die TWS meldet keine offenen Positionen."));
        } else {
            egui::Frame::none().fill(card_bg()).show(ui, |ui| {
                let mut plastic = PlasticRows::default();
                let grid = egui::Grid::new("live_positions_grid").min_col_width(90.0).show(ui, |ui| {
                    plastic.row(ui, true);
                    for h in ["Account", "Symbol", "Type", "Quantity", "Average cost"] {
                        ui.label(RichText::new(h).strong().color(header_text()));
                    }
                    ui.end_row();
                    for position in &self.positions {
                        plastic.row(ui, false);
                        ui.label(&position.account);
                        ui.label(&position.symbol);
                        ui.label(&position.security_type);
                        ui.monospace(format!("{}", position.quantity));
                        ui.monospace(position.average_cost.map_or_else(|| "Not provided".to_string(), |cost| format!("{cost:.2}")));
                        ui.end_row();
                    }
                });
                plastic.paint(ui, grid.response.rect, 4.0);
            });
        }
    }

    pub(super) fn quotes_page(&mut self, ui: &mut egui::Ui) {
        self.quotes_toolbar(ui);
        ui.add_space(10.0);
        let market_closed = self.market_session() != Session::Open;
        let empty_text = if self.tws_cmd_sender.is_none() {
            self.tr("Not connected to TWS.", "Nicht mit der TWS verbunden.")
        } else if market_closed && self.chains.contracts.is_empty() {
            self.tr("The US market is closed: no live option quotes.", "Die US-Börse ist geschlossen: keine Live-Optionskurse.")
        } else if self.chains.contracts.is_empty() {
            self.tr("Waiting for the option chain from TWS...", "Warte auf die Optionskette der TWS...")
        } else {
            self.tr("No active subscription.", "Kein aktives Abonnement.")
        };
        let positions: Vec<(String, f64)> = self.positions.iter().map(|p| (p.contract_id.trim().to_string(), p.quantity)).collect();
        let mut puts: Vec<&OptionContract> = Vec::new();
        let mut calls: Vec<&OptionContract> = Vec::new();
        for contract in &self.chains.contracts {
            if contract.req_id.is_none() || contract.symbol != self.quotes_view.symbol || contract.expiry != self.quotes_view.expiry {
                continue;
            }
            match contract.right.as_str() {
                "P" => puts.push(contract),
                "C" => calls.push(contract),
                _ => {}
            }
        }
        // Puts by strike descending, calls ascending.
        puts.sort_by(|a, b| b.strike.total_cmp(&a.strike));
        calls.sort_by(|a, b| a.strike.total_cmp(&b.strike));
        let price = self.chains.price(&self.quotes_view.symbol);
        ui.columns(2, |columns| {
            for (col, (title, rows)) in columns.iter_mut().zip([("Calls", &calls), ("Puts", &puts)]) {
                navy_bar(col, title);
                egui::Frame::none().fill(card_bg()).show(col, |ui| {
                    egui::ScrollArea::vertical().id_salt(format!("quotes_scroll_{title}")).auto_shrink([false, true]).show(ui, |ui| {
                        let mut plastic = PlasticRows::default();
                        let grid = egui::Grid::new(format!("quotes_grid_{title}")).min_col_width(58.0).spacing([6.0, 4.0]).show(ui, |ui| {
                            plastic.row(ui, true);
                            for h in ["Strike", "Bid", "Ask", "Position", "Delta", "Last", "Mark"] {
                                ui.label(RichText::new(h).strong().color(header_text()));
                            }
                            ui.end_row();
                            for contract in rows {
                                plastic.row(ui, false);
                                let in_the_money = price.is_some_and(|p| if title == "Calls" { contract.strike < p } else { contract.strike > p });
                                let strike = RichText::new(format_strike(contract.strike)).strong();
                                ui.label(if in_the_money { strike.color(muted_text()) } else { strike });
                                let money = |v: Option<f64>| v.filter(|v| *v >= 0.0).map_or("-".to_string(), |v| format!("{v:.2}"));
                                ui.label(money(contract.bid));
                                ui.label(money(contract.ask));
                                let position = positions
                                    .iter()
                                    .find(|(id, _)| id.parse::<i32>().ok() == Some(contract.con_id))
                                    .map_or(String::new(), |(_, qty)| format!("{qty}"));
                                ui.label(position);
                                ui.label(contract.delta.map_or("-".to_string(), |d| format!("{d:.3}")));
                                ui.label(contract.updated.clone().unwrap_or_default());
                                ui.label(money(contract.mid()));
                                ui.end_row();
                            }
                        });
                        plastic.paint(ui, grid.response.rect, 4.0);
                        if rows.is_empty() {
                            ui.add_space(26.0);
                            ui.vertical_centered(|ui| ui.label(RichText::new(empty_text).italics().color(muted_text())));
                            ui.add_space(26.0);
                        }
                    });
                });
            }
        });
    }

    /// The toolbar of the Quotes page: title, symbol, expiry and the buttons.
    fn quotes_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(self.tr("Quotes", "Kurse")).color(self.heading_color()));
            ui.add_space(8.0);
            ui.label(RichText::new(self.tr("(Broker data)", "(Broker-Daten)")).italics().color(muted_text()));
            ui.add_space(12.0);
            let connected = self.tws_cmd_sender.is_some();
            if ui.add_enabled(connected, crate::widgets::MetalButton::new(self.tr("Refresh Quote Subscriptions", "Kurs-Abonnements aktualisieren"))).clicked() {
                self.refresh_chain_subscriptions();
            }
            // The symbol list: the configured ones (or the SPX default) - shown even before TWS
            // has answered.
            let symbols: Vec<String> = if self.chains.symbols.is_empty() {
                ChainSymbol::from_settings(&self.settings.symbol_configs).into_iter().map(|s| s.symbol).collect()
            } else {
                self.chains.symbols.iter().map(|s| s.symbol.clone()).collect()
            };
            if !symbols.contains(&self.quotes_view.symbol) {
                self.quotes_view.symbol = symbols.first().cloned().unwrap_or_default();
            }
            crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("quotes_symbol").selected_text(self.quotes_view.symbol.clone()).width(70.0).show_ui(ui, |ui| {
                    for symbol in &symbols {
                        ui.selectable_value(&mut self.quotes_view.symbol, symbol.clone(), symbol);
                    }
                })
            });
            let expiries = self.chains.expiries(&self.quotes_view.symbol);
            if !expiries.contains(&self.quotes_view.expiry) {
                self.quotes_view.expiry = expiries.first().cloned().unwrap_or_default();
            }
            crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("quotes_expiry")
                    .selected_text(if self.quotes_view.expiry.is_empty() { "-".to_string() } else { format_date(&self.quotes_view.expiry) })
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for expiry in &expiries {
                            ui.selectable_value(&mut self.quotes_view.expiry, expiry.clone(), format_date(expiry));
                        }
                    })
            });
            if let Some(price) = self.chains.price(&self.quotes_view.symbol) {
                ui.label(RichText::new(format!("{} {price:.2}", self.quotes_view.symbol)).strong());
            }
            ui.label(format!("{} {}", self.chains.subscribed_count(), self.tr("data lines subscribed", "Datenzeilen abonniert")));
        });
    }
}
