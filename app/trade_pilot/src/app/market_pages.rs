//! The pages Live Data and Quotes: the index quotes and the quote rows with their toolbar.

use super::{
    card_bg, format_date, format_delta, format_strike, header_text, muted_text, navy_bar, App, ChainSymbol, Color32, OptionContract, RichText, Session,
};
use super::{mask_account, TwsCommand, GREEN_TXT, ORANGE_WARN, RED};
use crate::widgets::{PlasticRows, TableCell, TableColumn};
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
        ui.add_space(8.0);
        self.vix_term_ratio_line(ui);
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
            self.live_positions_table(ui);
            ui.add_space(18.0);
            self.unmanaged_positions_table(ui);
        }
    }

    /// The option positions no open trade of this app holds (`unmanaged::unmanaged_positions`), with the part of each that has no trade.
    fn unmanaged_positions_table(&mut self, ui: &mut egui::Ui) {
        ui.strong(self.tr("Positions Without a Trade", "Positionen ohne Trade"));
        ui.add_space(6.0);
        let Some(found) = crate::unmanaged::unmanaged_positions(&self.positions, &self.trades_stored) else {
            ui.colored_label(
                ORANGE_WARN,
                self.tr(
                    "The positions cannot be compared with the trades: a quantity TWS reported cannot be read.",
                    "Die Positionen lassen sich nicht mit den Trades vergleichen: eine Menge der TWS ist nicht lesbar.",
                ),
            );
            return;
        };
        if found.is_empty() {
            ui.colored_label(muted_text(), self.tr("Every option position belongs to an open trade.", "Jede Optionsposition gehört zu einem offenen Trade."));
            return;
        }
        let keys = ["account", "symbol", "contract", "dte", "quantity", "without_trade", "average_cost"];
        let titles = [
            self.tr("Account", "Konto"),
            "Symbol",
            self.tr("Contract", "Kontrakt"),
            "DTE",
            self.tr("Position", "Position"),
            self.tr("Without trade", "Ohne Trade"),
            self.tr("Average cost", "Einstandskosten"),
        ];
        let weights = [1.3, 0.9, 1.6, 0.6, 0.8, 0.9, 1.1];
        let columns: Vec<TableColumn> = titles.iter().zip(weights).enumerate().map(|(i, (title, weight))| TableColumn::new(*title, weight, i >= 3)).collect();
        let today = self.clock.new_york_today();
        let shown: Vec<(&crate::unmanaged::UnmanagedPosition, &broker::Position)> =
            found.iter().filter_map(|u| self.positions.get(u.position).map(|p| (u, p))).collect();
        let row_keys: Vec<Option<super::import::PositionKey>> = shown.iter().map(|(u, _)| self.position_key(u.position)).collect();
        let ticked: Vec<usize> =
            row_keys.iter().enumerate().filter(|(_, key)| key.as_ref().is_some_and(|k| self.import.selection.contains(k))).map(|(i, _)| i).collect();
        let rows: Vec<Vec<TableCell>> = shown
            .iter()
            .map(|(u, p)| {
                vec![
                    TableCell::new(&p.account),
                    TableCell::new(&p.symbol),
                    TableCell::new(contract_text(p)),
                    TableCell::new(days_to_expiry(&p.expiry, &today).map_or_else(|| "-".to_string(), |days| days.to_string())),
                    TableCell::new(format!("{}", p.quantity)),
                    TableCell { text: u.quantity.normalize().to_string(), color: Some(ORANGE_WARN), strong: true, ..TableCell::default() },
                    TableCell::new(p.average_cost.map_or_else(|| "-".to_string(), |cost| format!("{cost:.2}"))),
                ]
            })
            .collect();
        let selection = crate::widgets::RowSelection { selected: None, ticked: &ticked, locked: &[] };
        let clicks = crate::widgets::arranged_table(ui, "unmanaged_positions", &keys, &columns, &rows, Some(selection));
        crate::widgets::toggle_selection(&mut self.import.selection, clicks, |row| row_keys.get(row).cloned().flatten());
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let can_take_over = !self.import.selection.is_empty();
            if ui.add_enabled(can_take_over, crate::widgets::MetalButton::new(self.tr("Take over as trade ...", "Als Trade übernehmen ..."))).clicked() {
                self.open_import();
            }
            match self.import_selection_note() {
                Some((note, ok)) => {
                    ui.label(RichText::new(note).color(super::import::note_color(ok)));
                }
                None => {
                    ui.label(RichText::new(self.tr("Tick the legs of one trade.", "Die Beine eines Trades anhaken.")).color(muted_text()));
                }
            }
        });
        ui.label(
            RichText::new(self.tr(
                "Option positions TWS holds that no open trade of this app accounts for: opened by hand in TWS, by another program or on another computer, or long options kept for reuse.",
                "Optionspositionen der TWS, die zu keinem offenen Trade dieser App gehören: von Hand in der TWS, von einem anderen Programm oder auf einem anderen Rechner eröffnet, oder Long-Optionen, die zur Weiterverwendung liegen bleiben.",
            ))
            .small()
            .color(muted_text()),
        );
    }

    /// Asks TWS once per session for the portfolio values of the account (`reqAccountUpdates`): of the account the positions are on,
    /// else the first account TWS manages. TWS then values every position itself, stocks and bills included.
    pub(super) fn subscribe_portfolio(&mut self) {
        if self.portfolio_account.is_some() {
            return;
        }
        let account = self.positions.first().map(|p| p.account.clone()).or_else(|| self.session.managed_accounts.first().cloned());
        let (Some(account), Some(sender)) = (account.filter(|a| !a.trim().is_empty()), self.tws_cmd_sender.as_ref()) else { return };
        if sender.send(TwsCommand::SubscribePortfolio { account: account.clone() }).is_ok() {
            self.debug(&format!("Asked TWS for the portfolio values of {}", mask_account(&account)));
            self.portfolio_account = Some(account);
        }
    }

    /// One position as TWS values it: it replaces what was known of it; a position that is closed (quantity 0) is dropped. A row that
    /// does not match the position TWS reported for the same contract (quantity or average cost) is reported once: its values are
    /// then not used (see `portfolio_row_of`), as the fields of the message may not be where the app reads them.
    pub(super) fn on_portfolio(&mut self, row: broker::PortfolioRow) {
        let position = self.positions.iter().find(|p| p.account == row.account && p.contract_id.trim().parse::<i32>().ok() == Some(row.con_id));
        if let Some(position) = position.filter(|p| !portfolio_matches(p, &row)) {
            let key = format!("portfolio {}", row.con_id);
            if self.watch.mismatch_reported.insert(key) {
                self.log(&format!(
                    "WARNING: TWS portfolio value of contract {} does not match its position (quantity {} vs {}, average cost {} vs {}) - values ignored",
                    row.con_id,
                    row.quantity,
                    position.quantity,
                    row.average_cost,
                    position.average_cost.map_or_else(|| "-".to_string(), |c| c.to_string())
                ));
            }
        }
        self.portfolio.retain(|known| known.account != row.account || known.con_id != row.con_id);
        if row.quantity != 0.0 {
            self.portfolio.push(row);
        }
    }

    /// The portfolio row TWS gave for `position`, only when it matches the position (`portfolio_matches`).
    fn portfolio_row_of(&self, position: &broker::Position) -> Option<&broker::PortfolioRow> {
        let con_id: i32 = position.contract_id.trim().parse().ok()?;
        self.portfolio.iter().find(|row| row.con_id == con_id && row.account == position.account).filter(|row| portfolio_matches(position, row))
    }

    /// The price, market value and profit of a position: as TWS values it when it has (the portfolio), else worked out from the quote
    /// of its option while the app has one; `None` for the parts nothing is known of.
    pub(super) fn position_figures(&self, position: &broker::Position) -> (Option<f64>, Option<crate::position_value::PositionValue>) {
        let con_id: Option<i32> = position.contract_id.trim().parse().ok();
        if let Some(row) = self.portfolio_row_of(position) {
            let value = crate::position_value::tws_position_value(position.quantity, row.average_cost, row.market_value, row.unrealized_pnl);
            return (Some(row.market_price), value);
        }
        let price =
            con_id.and_then(|con_id| self.chains.contracts.iter().find(|c| c.con_id == con_id)).and_then(|contract| match (contract.bid, contract.ask) {
                (Some(bid), Some(ask)) if bid >= 0.0 && ask > 0.0 => Some(f64::midpoint(bid, ask)),
                _ => None,
            });
        (price, crate::position_value::position_value(position.quantity, position.average_cost, position.multiplier, price))
    }

    /// The positions TWS reports with their value at the market and their profit; the columns can be moved by drag and drop and keep
    /// their order.
    fn live_positions_table(&self, ui: &mut egui::Ui) {
        let keys = ["account", "symbol", "contract", "type", "quantity", "average_cost", "price", "market_value", "profit", "profit_percent"];
        let titles = [
            self.tr("Account", "Konto"),
            "Symbol",
            self.tr("Contract", "Kontrakt"),
            self.tr("Type", "Typ"),
            self.tr("Quantity", "Menge"),
            self.tr("Average cost", "Einstandskosten"),
            self.tr("Price", "Kurs"),
            self.tr("Market value", "Marktwert"),
            self.tr("P/L", "G/V"),
            self.tr("P/L %", "G/V %"),
        ];
        let weights = [1.3, 0.9, 1.6, 0.6, 0.8, 1.1, 0.8, 1.1, 1.0, 0.8];
        let columns: Vec<TableColumn> = titles.iter().zip(weights).enumerate().map(|(i, (title, weight))| TableColumn::new(*title, weight, i >= 4)).collect();
        let money = |value: rust_decimal::Decimal| format!("{:.2}", value.round_dp(2));
        let rows: Vec<Vec<TableCell>> = self
            .positions
            .iter()
            .map(|p| {
                let (price, value) = self.position_figures(p);
                let color = value.map(|v| if v.profit < rust_decimal::Decimal::ZERO { RED } else { GREEN_TXT });
                vec![
                    TableCell::new(&p.account),
                    TableCell::new(&p.symbol),
                    TableCell::new(contract_text(p)),
                    TableCell::new(&p.security_type),
                    TableCell::new(format!("{}", p.quantity)),
                    TableCell::new(p.average_cost.map_or_else(|| "-".to_string(), |cost| format!("{cost:.2}"))),
                    TableCell::new(price.map_or_else(|| "-".to_string(), |price| format!("{price:.2}"))),
                    TableCell::new(value.map_or_else(|| "-".to_string(), |v| money(v.market_value))),
                    TableCell { text: value.map_or_else(|| "-".to_string(), |v| money(v.profit)), color, strong: true, ..TableCell::default() },
                    TableCell {
                        text: value.and_then(|v| v.percent).map_or_else(|| "-".to_string(), |p| format!("{:+.1} %", p.round_dp(1))),
                        color,
                        ..TableCell::default()
                    },
                ]
            })
            .collect();
        let _ = crate::widgets::arranged_table(ui, "live_positions", &keys, &columns, &rows, None);
        ui.label(
            RichText::new(self.tr(
                "Price, market value and P/L as TWS values the positions. Columns: drag a header to move it, drag the border between two headers to size them.",
                "Kurs, Marktwert und G/V so, wie die TWS die Positionen bewertet. Spalten: Kopf ziehen zum Verschieben, Grenze zwischen zwei Köpfen ziehen für die Breite.",
            ))
            .small()
            .color(muted_text()),
        );
    }

    /// VIX9D divided by VIX, the same value the conditions use (`VIX9D/VIX`); `None` until both are quoted.
    pub(super) fn vix9d_over_vix(&self) -> Option<rust_decimal::Decimal> {
        self.condition_snapshot().value_with(crate::condition::Input::Vix9dOverVix, 1)
    }

    /// The line under the index quotes with the ratio of VIX9D to VIX and what it says: below 1 the next nine days are expected
    /// calmer than the next thirty (the usual shape), above 1 the curve is inverted (short-term stress) and the ratio turns orange.
    fn vix_term_ratio_line(&self, ui: &mut egui::Ui) {
        let ratio = self.vix9d_over_vix();
        let inverted = ratio.is_some_and(|r| r > rust_decimal::Decimal::ONE);
        let (value, note) = match ratio {
            Some(r) => (
                format!("{:.2}", r.round_dp(2)),
                if inverted {
                    self.tr(
                        "inverted: the next 9 days are expected more volatile than the next 30",
                        "invertiert: die nächsten 9 Tage werden unruhiger erwartet als die nächsten 30",
                    )
                } else {
                    self.tr(
                        "normal: the next 9 days are expected calmer than the next 30",
                        "normal: die nächsten 9 Tage werden ruhiger erwartet als die nächsten 30",
                    )
                },
            ),
            None => ("-".to_string(), self.tr("needs live quotes of VIX and VIX9D", "braucht Live-Kurse von VIX und VIX9D")),
        };
        ui.horizontal(|ui| {
            ui.strong("VIX9D / VIX");
            ui.add_space(6.0);
            let color = if inverted { crate::theme::ORANGE_WARN } else { crate::theme::text_primary() };
            ui.label(RichText::new(value).monospace().strong().size(15.0).color(color));
            ui.add_space(6.0);
            ui.label(RichText::new(note).color(muted_text()));
        });
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

/// The contract of a position as the position tables show it: "20261218 6650 P" for an option, empty otherwise.
fn contract_text(position: &broker::Position) -> String {
    if position.security_type == "OPT" {
        format!("{} {} {}", position.expiry, format_strike(position.strike), position.right)
    } else {
        String::new()
    }
}

/// The calendar days from `today` to `expiry` (both yyyyMMdd), counted like the DTE of the option chain; `None` when one of them
/// cannot be read (a stock has no expiry).
fn days_to_expiry(expiry: &str, today: &str) -> Option<i64> {
    let parse = |day: &str| chrono::NaiveDate::parse_from_str(day.trim(), "%Y%m%d").ok();
    Some((parse(expiry)? - parse(today)?).num_days())
}

/// Whether a portfolio row of TWS agrees with the position TWS reported for the same contract: the same quantity and, when both
/// carry one, the same average cost. Both come from TWS, so a difference means the app did not read the message where its values are.
fn portfolio_matches(position: &broker::Position, row: &broker::PortfolioRow) -> bool {
    let same_quantity = (position.quantity - row.quantity).abs() < 1e-9;
    let same_cost = position.average_cost.is_none_or(|cost| (cost - row.average_cost).abs() < 0.01);
    same_quantity && same_cost
}

#[cfg(test)]
mod tests {
    use super::{days_to_expiry, portfolio_matches};

    #[test]
    fn days_to_expiry_counts_calendar_days() {
        assert_eq!(days_to_expiry("20261127", "20261127"), Some(0), "expires today");
        assert_eq!(days_to_expiry("20261130", "20261127"), Some(3), "over the weekend");
        assert_eq!(days_to_expiry("", "20261127"), None, "a stock");
    }

    fn position(quantity: f64, average_cost: Option<f64>) -> broker::Position {
        broker::Position {
            account: "U1".into(),
            contract_id: "11".into(),
            symbol: "MU".into(),
            security_type: "STK".into(),
            expiry: String::new(),
            strike: 0.0,
            right: String::new(),
            multiplier: None,
            quantity,
            average_cost,
        }
    }

    fn row(quantity: f64, average_cost: f64) -> broker::PortfolioRow {
        broker::PortfolioRow { account: "U1".into(), con_id: 11, quantity, market_price: 900.0, market_value: 1800.0, average_cost, unrealized_pnl: 91.28 }
    }

    #[test]
    fn a_portfolio_row_is_used_only_when_it_agrees_with_the_position_of_the_same_contract() {
        assert!(portfolio_matches(&position(2.0, Some(854.36)), &row(2.0, 854.36)));
        assert!(portfolio_matches(&position(2.0, None), &row(2.0, 854.36)), "a position without a cost: only the quantity counts");
        assert!(!portfolio_matches(&position(2.0, Some(854.36)), &row(3.0, 854.36)), "another quantity");
        assert!(!portfolio_matches(&position(2.0, Some(854.36)), &row(2.0, 900.0)), "another cost: the fields were not read where they are");
    }
}
