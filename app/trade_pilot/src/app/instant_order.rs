//! The Instant Order page: an order of one option or of a template trade, its preview, its confirmation and how it is sent.

use super::{
    action_color, card_bg, check_order_quantity, confirm_buttons, dec, expiry_for_dte, fill_attempts, fill_wait, fix_price, format_date, format_strike,
    muted_text, navy_bar, nearest_strike_index, option_chain_lookup, option_trading_class, parse_date_input, parse_dec, send_counted, template_ib_order,
    template_strategy_and_name, trade_entry, underlying_lookup_exchange, visible_templates, white_card, App, Color32, Decimal, Duration, EntrySelection,
    IbContract, IbOrder, Instant, OrderHandling, PendingInstantOrder, PendingTemplateOrder, RichText, TradeTemplate, TwsCommand, ACCENT_BLUE,
    CARD_SCROLLBAR_GAP, GREEN_TXT, INSTANT_LIMIT_REFRESH, RED,
};
use eframe::egui;
use rust_decimal::prelude::ToPrimitive;
use std::fmt::Write as _;

impl App {
    /// The Instant Order panel: template list, Qty, Account, Test Trade?, PREVIEW Trade,
    /// Enter Trade and the "Trade Details" of the last preview.
    fn instant_template_card(&mut self, ui: &mut egui::Ui) {
        let mut templates: Vec<(i32, String, String)> =
            visible_templates(&self.templates).iter().map(|t| (t.id, template_strategy_and_name(t), t.trade_type.clone())).collect();
        // The templates are ordered by their label (strategy, then name).
        templates.sort_by(|a, b| a.1.cmp(&b.1));
        white_card(ui, |ui| {
            navy_bar(ui, self.tr("Trade Template", "Trade Template"));
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(320.0);
                    egui::ScrollArea::vertical().id_salt("instant_templates").max_height(230.0).auto_shrink([false, true]).show(ui, |ui| {
                        if templates.is_empty() {
                            ui.label(RichText::new(self.tr("No trade templates yet.", "Noch keine Trade Templates.")).italics().color(muted_text()));
                        }
                        for (id, name, trade_type) in &templates {
                            let selected = self.instant.template_id == Some(*id);
                            let row = ui.selectable_label(selected, format!("{name}   ({trade_type})"));
                            if row.clicked() {
                                self.instant.template_id = Some(*id);
                                self.instant.template_preview.clear();
                            }
                        }
                    });
                });
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    egui::Grid::new("instant_template_grid").spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label(self.tr("Qty", "Menge"));
                        crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.instant.template_qty).desired_width(60.0));
                        ui.end_row();
                        ui.label(self.tr("Account", "Konto"));
                        let selected = if self.instant.account.is_empty() {
                            self.tr("(connect to TWS)", "(mit der TWS verbinden)").to_string()
                        } else {
                            format!("IB:{}", self.instant.account)
                        };
                        ui.add_enabled_ui(!self.session.managed_accounts.is_empty(), |ui| {
                            crate::widgets::metal_combo(ui, |ui| {
                                egui::ComboBox::from_id_salt("instant_template_account").selected_text(selected).show_ui(ui, |ui| {
                                    for acct in self.session.managed_accounts.clone() {
                                        let label = format!("IB:{acct}");
                                        ui.selectable_value(&mut self.instant.account, acct, label);
                                    }
                                })
                            });
                        });
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    let test_label =
                        self.tr("Test Trade? (prices the order so that it does not fill)", "Test Trade? (Preis so, dass die Order nicht ausgeführt wird)");
                    crate::widgets::metal_checkbox(ui, &mut self.template_order_flags.instant_template_test, test_label);
                    let transmit_label = self
                        .tr("Transmit immediately (off: the order waits in TWS for review)", "Sofort übertragen (aus: Order wartet in der TWS auf Prüfung)");
                    crate::widgets::metal_checkbox(ui, &mut self.template_order_flags.instant_template_transmit, transmit_label);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let can_preview = self.tws_cmd_sender.is_some() && self.instant.template_id.is_some();
                        let preview_label = RichText::new(self.tr("PREVIEW Trade", "PREVIEW Trade")).color(Color32::WHITE).strong();
                        if ui.add_enabled(can_preview, crate::widgets::MetalButton::new(preview_label).fill(ACCENT_BLUE)).clicked() {
                            self.preview_template_trade();
                        }
                        // Paper account only: the same gate as the single option ticket.
                        let can_enter = can_preview && self.trading_allowed();
                        let enter_label = RichText::new(self.tr("Enter Trade", "Trade eingeben")).color(Color32::WHITE).strong();
                        let enter = ui.add_enabled(can_enter, crate::widgets::MetalButton::new(enter_label).fill(GREEN_TXT)).on_disabled_hover_text(self.tr(
                            "Needs a connected TWS account that matches the Live/Paper switch and the kill switch off.",
                            "Braucht ein verbundenes TWS-Konto, das zum Live/Paper-Schalter passt, und einen freien Kill-Switch.",
                        ));
                        if enter.clicked() {
                            self.enter_template_trade();
                        }
                    });
                    self.expire_notice(ui.ctx());
                    if let Some(status) = &self.instant.template_status {
                        ui.add_space(4.0);
                        ui.label(RichText::new(status).color(muted_text()));
                    }
                });
            });
            ui.add_space(10.0);
            ui.label(RichText::new(self.tr("Trade Details", "Trade-Details")).strong());
            egui::Frame::none().fill(card_bg()).inner_margin(8.0).show(ui, |ui| {
                ui.set_width((ui.available_width() - CARD_SCROLLBAR_GAP).max(0.0));
                let text = if self.instant.template_preview.is_empty() {
                    self.tr("Select a template and click PREVIEW Trade.", "Template wählen und PREVIEW Trade klicken.").to_string()
                } else {
                    self.instant.template_preview.clone()
                };
                ui.label(RichText::new(text).monospace());
            });
        });
    }

    pub(super) fn instant_order_page(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new(self.tr("Instant Order", "Sofortorder")).color(self.heading_color()));
        ui.add_space(10.0);
        self.instant_template_card(ui);
        ui.add_space(14.0);
        ui.label(RichText::new(self.tr("Single Option Order", "Einzelne Option")).strong().size(17.0).color(self.heading_color()));
        ui.add_space(6.0);
        self.instant_contract_card(ui);
        ui.add_space(10.0);
        white_card(ui, |ui| {
            navy_bar(ui, self.tr("Order", "Order"));
            ui.add_space(8.0);
            self.instant_order_grid(ui);
            ui.add_space(8.0);
            self.instant_order_buttons(ui);
        });
    }

    /// The Contract card of the single option order: symbol, sec type, exchange, expiry, strike and so on.
    fn instant_contract_card(&mut self, ui: &mut egui::Ui) {
        white_card(ui, |ui| {
            navy_bar(ui, self.tr("Contract", "Kontrakt"));
            ui.add_space(8.0);
            egui::Grid::new("instant_contract_grid").spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label(self.tr("Symbol", "Symbol"));
                crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.instant.symbol).desired_width(80.0));
                ui.label(self.tr("Sec Type", "Kontraktart"));
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("instant_sec_type").selected_text(self.instant.sec_type.clone()).show_ui(ui, |ui| {
                        for t in ["OPT", "STK", "IND", "FUT"] {
                            ui.selectable_value(&mut self.instant.sec_type, t.to_string(), t);
                        }
                    })
                });
                ui.label(self.tr("Exchange", "Börse"));
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("instant_exchange").selected_text(self.instant.exchange.clone()).show_ui(ui, |ui| {
                        for x in ["SMART", "CBOE", "NYSE", "NASDAQ", "ARCA", "GLOBEX"] {
                            ui.selectable_value(&mut self.instant.exchange, x.to_string(), x);
                        }
                    })
                });
                ui.end_row();
                ui.label(self.tr("Currency", "Währung"));
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("instant_currency").selected_text(self.instant.currency.clone()).show_ui(ui, |ui| {
                        for c in ["USD", "EUR", "GBP", "CHF", "JPY", "CAD", "AUD"] {
                            ui.selectable_value(&mut self.instant.currency, c.to_string(), c);
                        }
                    })
                });
                if self.instant.sec_type == "FUT" {
                    ui.label(self.tr("Expiry (DD-MM-YYYY)", "Verfall (TT-MM-JJJJ)"));
                    crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.instant.expiry).desired_width(90.0));
                }
                self.instant_expiry_and_strike(ui);
                ui.end_row();
                // Which side of the contract: BUY = long (or closing a short), SELL = short (or
                // closing a long) - part of the contract choice, next to Right.
                ui.label(self.tr("Action", "Aktion"));
                let selected_action = RichText::new(self.instant.action.clone()).strong().color(action_color(&self.instant.action));
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("instant_action").selected_text(selected_action).show_ui(ui, |ui| {
                        for a in ["BUY", "SELL"] {
                            ui.selectable_value(&mut self.instant.action, a.to_string(), RichText::new(a).strong().color(action_color(a)));
                        }
                    })
                });
                if self.instant.sec_type != "OPT" {
                    ui.end_row();
                }
                if self.instant.sec_type == "OPT" {
                    // TWS calls it the contract's "right" (C/P); shown as what it is.
                    ui.label(self.tr("Option Type", "Optionstyp"));
                    let type_name = |right: &str| if right == "P" { "Put" } else { "Call" };
                    crate::widgets::metal_combo(ui, |ui| {
                        egui::ComboBox::from_id_salt("instant_right").selected_text(type_name(&self.instant.right)).show_ui(ui, |ui| {
                            for r in ["C", "P"] {
                                ui.selectable_value(&mut self.instant.right, r.to_string(), type_name(r));
                            }
                        })
                    });
                    ui.label(self.tr("Underlying Sec Type", "Basiswert-Kontraktart"));
                    crate::widgets::metal_combo(ui, |ui| {
                        egui::ComboBox::from_id_salt("instant_underlying_sec_type").selected_text(self.instant.underlying_sec_type.clone()).show_ui(ui, |ui| {
                            for t in ["STK", "IND", "FUT"] {
                                ui.selectable_value(&mut self.instant.underlying_sec_type, t.to_string(), t);
                            }
                        })
                    });
                    ui.end_row();
                    ui.label("");
                    let can_fetch = self.tws_cmd_sender.is_some() && !self.instant.symbol.trim().is_empty();
                    // Until expiries/strikes are loaded the Expiry/Strike lists are empty, so the
                    // button that fills them stands out (filled accent) while it can be used.
                    let fetch_label = self.tr("Fetch from TWS", "Von der TWS laden");
                    let fetching = self.instant.contract_details_req_id.is_some() || self.instant.option_params_req_id.is_some();
                    let needs_fetch = can_fetch && self.instant.option_expirations.is_empty() && !fetching;
                    let fetch_button = if needs_fetch {
                        crate::widgets::MetalButton::new(RichText::new(format!("⟳ {fetch_label}")).color(Color32::WHITE).strong()).fill(ACCENT_BLUE)
                    } else {
                        crate::widgets::MetalButton::new(fetch_label)
                    };
                    if ui.add_enabled(can_fetch, fetch_button).clicked() {
                        self.request_option_params();
                    }
                    if !self.instant.option_params_status.is_empty() {
                        ui.add(egui::Label::new(RichText::new(self.instant.option_params_status.clone()).italics().color(muted_text())).extend());
                    }
                    ui.end_row();
                }
            });
        });
    }

    /// The Expiry and Strike drop-downs of an option; both lists come from TWS only.
    fn instant_expiry_and_strike(&mut self, ui: &mut egui::Ui) {
        if self.instant.sec_type == "OPT" {
            // Real data only (user requirement): these two lists are populated exclusively
            // from TWS's own `reqSecDefOptParams` response below - never guessed, never
            // hard-coded. Until a fetch completes, both stay empty and disabled.
            // The value stays in TWS's yyyyMMdd; only the display is DD-MM-YYYY.
            ui.label(self.tr("Expiry", "Verfall"));
            let expiry_text = if self.instant.expiry.is_empty() { self.tr("(select)", "(wählen)").to_string() } else { format_date(&self.instant.expiry) };
            crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("instant_expiry_combo").selected_text(expiry_text).show_ui(ui, |ui| {
                    for expiry in self.instant.option_expirations.clone() {
                        let label = format_date(&expiry);
                        ui.selectable_value(&mut self.instant.expiry, expiry, label);
                    }
                })
            });
            ui.label(self.tr("Strike", "Strike"));
            let strike_text = if self.instant.strike.is_empty() { self.tr("(select)", "(wählen)").to_string() } else { self.instant.strike.clone() };
            // On opening, the list is scrolled to (and keyboard focus put on) the strike
            // nearest the underlying's current price.
            let nearest =
                self.underlying_price(&self.instant.symbol.trim().to_uppercase()).and_then(|price| nearest_strike_index(&self.instant.option_strikes, price));
            // A few frames, not one: egui's first popup frame is an invisible sizing pass.
            let just_opened = self.instant.strike_list_frames < 3;
            let popup = crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("instant_strike_combo").selected_text(strike_text).height(300.0).show_ui(ui, |ui| {
                    for (i, strike) in self.instant.option_strikes.clone().into_iter().enumerate() {
                        let label = format_strike(strike);
                        let item = ui.selectable_value(&mut self.instant.strike, label.clone(), label);
                        if just_opened && nearest == Some(i) {
                            item.scroll_to_me(Some(egui::Align::Center));
                            item.request_focus();
                        }
                    }
                })
            });
            self.instant.strike_list_frames = if popup.inner.is_some() { self.instant.strike_list_frames.saturating_add(1) } else { 0 };
        }
    }

    /// The grid of the Order card: quantity, order type, limit price, account and the live quote.
    fn instant_order_grid(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("instant_order_grid").spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label(self.tr("Quantity", "Menge"));
            crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.instant.qty).desired_width(70.0));
            ui.label(self.tr("Order Type", "Ordertyp"));
            crate::widgets::metal_combo(ui, |ui| {
                egui::ComboBox::from_id_salt("instant_order_type").selected_text(self.instant.order_type.clone()).show_ui(ui, |ui| {
                    for t in ["MKT", "LMT"] {
                        ui.selectable_value(&mut self.instant.order_type, t.to_string(), t);
                    }
                })
            });
            ui.end_row();
            if self.instant.order_type == "LMT" {
                ui.label(self.tr("Limit Price", "Limitpreis"));
                ui.horizontal(|ui| {
                    // Filled with the option's live mid while the contract is complete; once
                    // a price is typed, it stays as typed ("Mid" puts the mid back).
                    if crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut self.instant.limit_price).desired_width(80.0)).changed() {
                        self.instant_flags.instant_limit_follows_mid = false;
                    }
                    if let Some(price) = self.instant_mid_on_grid() {
                        if !self.instant_flags.instant_limit_follows_mid
                            && ui.small_button("Mid").on_hover_text(self.tr("Use the mid price again", "Wieder den Mid-Preis verwenden")).clicked()
                        {
                            self.instant_flags.instant_limit_follows_mid = true;
                            self.instant.limit_price = format!("{price:.2}");
                            self.instant.limit_updated_at = Some(self.clock.instant());
                        }
                    }
                });
            }
            ui.label(self.tr("Account", "Konto"));
            let selected = if self.instant.account.is_empty() {
                self.tr("(connect to TWS)", "(mit der TWS verbinden)").to_string()
            } else {
                format!("IB:{}", self.instant.account)
            };
            ui.add_enabled_ui(!self.session.managed_accounts.is_empty(), |ui| {
                crate::widgets::metal_combo(ui, |ui| {
                    egui::ComboBox::from_id_salt("instant_account").selected_text(selected).show_ui(ui, |ui| {
                        for acct in self.session.managed_accounts.clone() {
                            let label = format!("IB:{acct}");
                            ui.selectable_value(&mut self.instant.account, acct, label);
                        }
                    })
                });
            });
            ui.end_row();
            // The live quote of the option on the ticket (see `sync_instant_quote`).
            if self.instant.quote_req_id.is_some() {
                ui.label(self.tr("Quote", "Kurs"));
                let money = |v: Option<Decimal>| v.filter(|v| *v >= Decimal::ZERO).map_or("-".to_string(), |v| format!("{v:.2}"));
                let text = format!(
                    "Bid {}   Ask {}   Mid {}",
                    money(self.instant.bid.and_then(dec)),
                    money(self.instant.ask.and_then(dec)),
                    money(self.instant_mid())
                );
                ui.label(RichText::new(text).strong());
                ui.end_row();
            }
        });
    }

    /// The rest of the Order card: the switches, the kill switch, Enter Trade, the errors and the last order.
    fn instant_order_buttons(&mut self, ui: &mut egui::Ui) {
        let transmit_label = self.tr(
            "Transmit immediately (unchecked: order reaches TWS but stays un-transmitted for manual review there)",
            "Sofort übermitteln (deaktiviert: die Order erreicht die TWS, bleibt dort aber zur manuellen Prüfung liegen)",
        );
        crate::widgets::metal_checkbox(ui, &mut self.instant_flags.instant_transmit, transmit_label);
        ui.add_space(8.0);
        let test_trade_label = self.tr(
            "Test Trade? (What-If: TWS reports margin/commission impact, no order is actually placed)",
            "Test-Trade? (What-If: TWS meldet Margin-/Provisionsauswirkung, es wird keine Order tatsächlich platziert)",
        );
        crate::widgets::metal_checkbox(ui, &mut self.instant_flags.instant_test_trade, test_trade_label);
        ui.add_space(8.0);
        // coding_guideline.md section 8 (MUSS): a kill switch that stops new orders,
        // independent of any other logic - checked inside `trading_allowed()` itself, so
        // tripping it blocks both new orders and cancels below regardless of every other gate.
        let mut switched: Option<bool> = None;
        ui.horizontal(|ui| {
            let mut tripped = self.kill_switch.is_tripped();
            if crate::widgets::metal_checkbox(
                ui,
                &mut tripped,
                self.tr("KILL SWITCH - block and cancel all orders", "KILL-SWITCH - alle Orders blockieren und stornieren"),
            )
            .changed()
            {
                switched = Some(tripped);
            }
        });
        match switched {
            Some(true) => self.trip_kill_switch(),
            Some(false) => {
                self.kill_switch.reset();
                self.log("KILL SWITCH off: orders can be sent again");
            }
            None => {}
        }
        ui.add_space(8.0);
        let can_trade = self.session.server_version.is_some() && self.trading_allowed() && self.next_order_id.is_some();
        if ui.add_enabled(can_trade, crate::widgets::MetalButton::new(self.tr("Enter Trade", "Trade eingeben"))).clicked() {
            self.submit_instant_order();
        }
        if let Some(error) = &self.instant.order_error {
            ui.add_space(6.0);
            ui.colored_label(crate::theme::negative_text(), format!("{}: {error}", self.tr("Not sent", "Nicht gesendet")));
        }
        if !self.trading_allowed() {
            ui.add_space(6.0);
            ui.colored_label(
                RED,
                self.tr(
                    "Blocked: orders need a connected TWS account that matches the Live/Paper switch.",
                    "Blockiert: Orders brauchen ein verbundenes TWS-Konto, das zum Live/Paper-Schalter passt.",
                ),
            );
        }
        if let (Some(status), Some(order_id)) = (self.instant.last_order_status.clone(), self.instant.last_order_id) {
            ui.add_space(6.0);
            let last_order_label = format!("{} #{order_id}: {status}", self.tr("Last order", "Letzte Order"));
            let cancel_label = self.tr("Cancel", "Abbrechen");
            let can_cancel = self.trading_allowed();
            ui.horizontal(|ui| {
                ui.label(last_order_label);
                if ui.add_enabled(can_cancel, crate::widgets::MetalButton::new(cancel_label)).clicked() {
                    self.cancel_instant_order(order_id);
                }
            });
        }
    }

    /// Sends `TwsCommand::CancelOrder` (`IBApi.EClient.cancelOrder`) for the last order placed
    /// from this ticket, gated by the same paper-only safety check as placing one.
    fn cancel_instant_order(&mut self, order_id: i32) {
        if !self.trading_allowed() {
            self.log("Blocked order cancel: paper-account safety gate not satisfied");
            return;
        }
        self.log(&format!("Cancelling order #{order_id}"));
        let _logged = self.submit_cancel(order_id);
    }

    /// Step 1 of fetching real expiries/strikes for the Instant Order Contract card: resolves a
    /// real underlying `Contract.ConId` via `TwsCommand::RequestContractDetails`
    /// (`IBApi.EClient.reqContractDetails`). Not gated by `trading_allowed()`: like live quotes,
    /// this is a read-only market-data/reference-data request, not an order.
    ///
    /// An earlier version of this sent `underlyingConId = 0` ("unresolved") straight to
    /// `reqSecDefOptParams` on the assumption that TWS would match by symbol/secType alone - TWS
    /// does not do that reliably, so the dropdowns stayed empty. This two-step flow (resolve the
    /// real conId first, see `request_option_params_for_con_id`) replaces that guess.
    pub(super) fn request_option_params(&mut self) {
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        let symbol = self.instant.symbol.trim().to_string();
        if symbol.is_empty() {
            return;
        }
        let req_id = self.next_request_id();
        self.instant.contract_details_req_id = Some(req_id);
        self.instant.option_params_req_id = None;
        self.instant.option_expirations.clear();
        self.instant.option_strikes.clear();
        self.instant.expiry.clear();
        self.instant.strike.clear();
        self.instant.option_params_status = self.tr("Resolving underlying contract with TWS...", "Löse Basiswert-Kontrakt mit der TWS auf...").to_string();
        let contract = IbContract {
            symbol: symbol.clone(),
            sec_type: self.instant.underlying_sec_type.clone(),
            exchange: underlying_lookup_exchange(&symbol, &self.instant.underlying_sec_type, self.instant.exchange.trim()),
            currency: self.instant.currency.trim().to_string(),
            ..IbContract::default()
        };
        self.log(&format!(
            "Resolving contract id for {symbol} ({}, {})",
            contract.sec_type,
            if contract.exchange.is_empty() { "any exchange" } else { contract.exchange.as_str() }
        ));
        send_counted(&sender, TwsCommand::RequestContractDetails { req_id, contract }, &mut unsent);
        self.log_unsent_requests("request option params", unsent);
    }

    /// Step 2: once a real `Contract.ConId` has come back from TWS (`TwsEvent::ContractConId`),
    /// sends the actual `TwsCommand::RequestOptionParams` (`IBApi.EClient.reqSecDefOptParams`)
    /// using it. `fut_fop_exchange` of `""` means "all exchanges", IB's own documented convention
    /// for that parameter.
    pub(super) fn request_option_params_for_con_id(&mut self, con_id: i32) {
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        let symbol = self.instant.symbol.trim().to_string();
        let req_id = self.next_request_id();
        self.instant.option_params_req_id = Some(req_id);
        self.instant.option_params_status = self.tr("Fetching expiries/strikes from TWS...", "Lade Verfalltermine/Strikes von der TWS...").to_string();
        self.log(&format!("Resolved {symbol} to conId {con_id}; requesting option expiries/strikes"));
        send_counted(
            &sender,
            TwsCommand::RequestOptionParams {
                req_id,
                underlying_symbol: symbol,
                fut_fop_exchange: String::new(),
                underlying_sec_type: self.instant.underlying_sec_type.clone(),
                underlying_con_id: con_id,
            },
            &mut unsent,
        );
        self.log_unsent_requests("request option params for con id", unsent);
    }

    /// Builds a real `IbContract`/`IbOrder` from the Instant Order ticket and sends it to TWS via
    /// `TwsCommand::PlaceOrder`, after the hard paper-only safety gate. The order id comes from
    /// TWS's own `nextValidId` sequence (incremented locally for each order placed this session).
    /// The underlying's current price: the option-chain underlying, else the Live Data row (last
    /// price, or the close when there is no last yet).
    pub(super) fn underlying_price(&self, symbol: &str) -> Option<f64> {
        self.chains.price(symbol).or_else(|| self.quotes.iter().find(|q| q.label == symbol).and_then(|q| q.last.or(q.close)).filter(|p| *p > 0.0))
    }

    /// The option the Instant Order ticket describes, once symbol, expiry, strike and right are
    /// all set (`None` before that, or for non-option tickets).
    pub(super) fn instant_option_contract(&self) -> Option<IbContract> {
        if self.instant.sec_type != "OPT" {
            return None;
        }
        let symbol = self.instant.symbol.trim().to_uppercase();
        let expiry = parse_date_input(&self.instant.expiry)?;
        let strike = parse_dec(&self.instant.strike)?;
        if symbol.is_empty() || !matches!(self.instant.right.as_str(), "C" | "P") {
            return None;
        }
        Some(IbContract {
            trading_class: option_trading_class(&symbol).unwrap_or_default().to_string(),
            symbol,
            sec_type: "OPT".to_string(),
            last_trade_date_or_contract_month: expiry,
            strike,
            right: self.instant.right.clone(),
            exchange: self.instant.exchange.trim().to_string(),
            currency: self.instant.currency.trim().to_string(),
            ..IbContract::default()
        })
    }

    /// Keeps a live quote on the ticket's option: (re)subscribes when the contract changes, and
    /// drops the subscription when it becomes incomplete or TWS goes away.
    pub(super) fn sync_instant_quote(&mut self) {
        let Some(sender) = self.tws_cmd_sender.clone() else {
            self.instant.quote_req_id = None;
            self.instant.quote_key.clear();
            return;
        };
        let mut unsent = 0;
        let contract = self.instant_option_contract();
        let key = contract
            .as_ref()
            .map_or(String::new(), |c| format!("{} {} {} {} {}", c.symbol, c.last_trade_date_or_contract_month, c.strike, c.right, c.exchange));
        if key == self.instant.quote_key {
            return;
        }
        if let Some(old) = self.instant.quote_req_id.take() {
            send_counted(&sender, TwsCommand::CancelMarketData { req_id: old }, &mut unsent);
        }
        self.instant.quote_key = key;
        self.instant.bid = None;
        self.instant.ask = None;
        self.instant_flags.instant_limit_follows_mid = true;
        self.instant.limit_updated_at = None;
        if let Some(contract) = contract {
            let req_id = self.next_request_id();
            self.instant.quote_req_id = Some(req_id);
            self.log(&format!("Instant Order: requesting the quote of {} (reqId {req_id})", self.instant.quote_key));
            send_counted(&sender, TwsCommand::RequestMarketData { req_id, contract }, &mut unsent);
        }
        self.log_unsent_requests("sync instant quote", unsent);
    }

    /// Puts the (grid) mid into the limit field while it follows the mid: at once when the first
    /// quote is there, then at most every `INSTANT_LIMIT_REFRESH` - not on every tick, so the
    /// field stays readable.
    pub(super) fn refresh_instant_limit(&mut self, now: Instant) {
        if !self.instant_flags.instant_limit_follows_mid {
            return;
        }
        let due = self.instant.limit_updated_at.is_none_or(|at| now.saturating_duration_since(at) >= INSTANT_LIMIT_REFRESH);
        if let (true, Some(price)) = (due, self.instant_mid_on_grid()) {
            self.instant.limit_price = format!("{price:.2}");
            self.instant.limit_updated_at = Some(now);
        }
    }

    /// The mid as a valid limit price: on the symbol's tick grid (`fix_price`).
    fn instant_mid_on_grid(&self) -> Option<Decimal> {
        let mid = self.instant_mid()?;
        fix_price(mid, &self.instant.symbol.trim().to_uppercase(), 1)
    }

    /// (bid + ask) / 2 of the ticket's option, in exact decimals (TWS sends the quotes as f64).
    fn instant_mid(&self) -> Option<Decimal> {
        match (self.instant.bid.and_then(dec), self.instant.ask.and_then(dec)) {
            (Some(bid), Some(ask)) if bid >= Decimal::ZERO && ask > Decimal::ZERO => bid.checked_add(ask)?.checked_div(Decimal::TWO),
            _ => None,
        }
    }

    /// PREVIEW Trade: selects the strikes the
    /// chosen template describes in the live chain and shows them as "Trade Details". No order.
    pub(super) fn preview_template_trade(&mut self) {
        self.instant.template_outcome = None;
        let mut lines: Vec<String> = Vec::new();
        let mut selection =
            EntrySelection { account: self.instant.account.clone(), test_trade: self.template_order_flags.instant_template_test, outcome: None };
        let preview = self.build_template_preview(self.instant.template_id, &mut selection, &mut lines);
        for line in lines {
            self.log(&format!("Instant Preview: {line}"));
        }
        self.instant.template_outcome = selection.outcome;
        self.instant.template_preview = preview;
    }

    /// The preview text for `template_id`; the strikes selected are put into `selection.outcome` when
    /// there is a trade. Nothing of the Instant Order page is touched, so a schedule can use it too.
    pub(super) fn build_template_preview(&mut self, template_id: Option<i32>, selection: &mut EntrySelection, log: &mut Vec<String>) -> String {
        let Some(template) = template_id.and_then(|id| self.live_template(id)).cloned() else {
            return self.tr("Select a trade template.", "Bitte ein Trade Template wählen.").to_string();
        };
        let symbol = template.underlying_symbol.trim().to_uppercase();
        log.push(format!("Previewing Trade  {} ({symbol})", template_strategy_and_name(&template)));
        if trade_entry::uses_calendar_entry(&template.trade_type) {
            return self.build_calendar_preview(template, &symbol, selection, log);
        }
        if template.trade_type == "Custom" {
            return self.build_custom_preview(template, &symbol, selection, log);
        }
        if !trade_entry::uses_tranche_entry(&template.trade_type) {
            // A Straddle template enters nothing.
            return format!(
                "{}: {}",
                template.trade_type,
                self.tr("preview not available for this trade type yet.", "Vorschau für diesen Trade-Typ noch nicht verfügbar.")
            );
        }
        // The symbol must be one of the configured market-data symbols.
        if !self.chains.symbols.iter().any(|s| s.symbol == symbol) {
            return format!("No market data is set up for {symbol}: trade cancelled. Check the market data symbols in Settings.");
        }
        let Some(price) = self.chains.price(&symbol).and_then(dec) else {
            return format!(
                "{symbol}: {}",
                self.tr("no price from TWS yet - please wait and preview again.", "noch kein Kurs von der TWS - bitte warten und erneut Preview.")
            );
        };
        // The expiration of the template's DTE.
        let today = self.clock.local().date_naive();
        let mut dte = template.short_call_dte;
        let mut expiry = expiry_for_dte(today, dte);
        if expiry.is_none() && template.leg_dte_a.dte_adjust {
            // The next DTE that is a trading day.
            while expiry.is_none() && dte < template.short_call_dte + 7 {
                dte += 1;
                expiry = expiry_for_dte(today, dte);
            }
            log.push(format!("{symbol} {} DTE Expiration is not available. Changing DTE to {dte}", template.short_call_dte));
        }
        let Some(expiry) = expiry else {
            return format!("{symbol} {} DTE Expiration is not available. Cancelling trade.", template.short_call_dte);
        };
        if !self.chains.contracts.iter().any(|c| c.symbol == symbol && c.expiry == expiry) {
            // Load this DTE's chain now, preview again in a moment.
            if let Some(config) = self.chains.symbols.iter_mut().find(|s| s.symbol == symbol) {
                if !config.dtes.contains(&dte) {
                    config.dtes.push(dte);
                }
            }
            if !self.chains.pending_lookups.iter().any(|(_, s, e)| *s == symbol && *e == expiry) {
                self.request_chain_lookup(&symbol, &expiry);
            }
            return format!("{symbol} {dte} DTE Market Data requested now. Please wait a few seconds and preview again.");
        }
        let now = self.clock.instant();
        let legs = self.chain_legs(&symbol, &[expiry.as_str()], now, &selection.account);
        let underlying_outdated = self
            .chains
            .underlyings
            .iter()
            .find(|u| u.symbol == symbol)
            .and_then(|u| u.price_at)
            .is_none_or(|at| now.saturating_duration_since(at) > Duration::from_secs(60));
        let open_price = self.quotes.iter().find(|q| q.label == symbol).and_then(|q| q.open).and_then(dec);
        let input = trade_entry::EntryInput {
            template: &template,
            price,
            open_price,
            legs,
            market_open: self.market_is_open(),
            underlying_outdated,
            test_trade: selection.test_trade,
        };
        let outcome = trade_entry::select_trade(&input);
        log.extend(outcome.log.iter().cloned());
        let mut text = outcome.preview.clone();
        if let Some(entry) = outcome.entry_price {
            let _ = write!(text, "\n{}: {}", self.tr("Entry price (limit)", "Einstiegspreis (Limit)"), entry.normalize());
            selection.outcome = Some((template, outcome));
        }
        text
    }

    /// The options an open trade of `account` has an exit order working on, with the side of that order: its short options
    /// are bought back, its long options sold. IB refuses an order on the other side of such an option.
    pub(super) fn working_exit_sides(&self, account: &str) -> std::collections::HashMap<i32, trade_entry::WorkingSide> {
        let mut sides = std::collections::HashMap::new();
        for trade in self.trades_stored.iter().filter(|t| t.is_open() && (account.is_empty() || t.account == account) && !t.exit_order_ids().is_empty()) {
            for (leg, side) in [
                (&trade.short_put, trade_entry::WorkingSide::Buy),
                (&trade.short_call, trade_entry::WorkingSide::Buy),
                (&trade.long_put, trade_entry::WorkingSide::Sell),
                (&trade.long_call, trade_entry::WorkingSide::Sell),
            ] {
                if let Some(leg) = leg {
                    sides.insert(leg.con_id, side);
                }
            }
        }
        sides
    }

    /// The options of `symbol` in `expiries` as the strike selection sees them: quotes, delta and
    /// the account's open position per option.
    pub(super) fn chain_legs(&self, symbol: &str, expiries: &[&str], now: Instant, account: &str) -> Vec<trade_entry::Leg> {
        let positions: Vec<(i32, f64)> = self
            .positions
            .iter()
            .filter(|p| account.is_empty() || p.account == account)
            .filter_map(|p| p.contract_id.trim().parse::<i32>().ok().map(|id| (id, p.quantity)))
            .collect();
        let minus_one = Decimal::NEGATIVE_ONE;
        let working = self.working_exit_sides(account);
        self.chains
            .contracts
            .iter()
            .filter(|c| c.symbol == symbol && expiries.contains(&c.expiry.as_str()))
            .filter_map(|c| {
                Some(trade_entry::Leg {
                    con_id: c.con_id,
                    strike: dec(c.strike)?,
                    right: c.right.clone(),
                    expiry: c.expiry.clone(),
                    bid: c.bid.and_then(dec).unwrap_or(minus_one),
                    ask: c.ask.and_then(dec).unwrap_or(minus_one),
                    delta: c.delta.unwrap_or(0.0),
                    subscribed: c.req_id.is_some(),
                    working: working.get(&c.con_id).copied().unwrap_or_default(),
                    position: positions
                        .iter()
                        .filter(|(id, _)| *id == c.con_id)
                        .filter_map(|(_, q)| dec(*q))
                        .try_fold(Decimal::ZERO, rust_decimal::Decimal::checked_add)?,
                    outdated: c.updated_at.is_none_or(|at| now.saturating_duration_since(at) > Duration::from_secs(120)),
                })
            })
            .collect()
    }

    /// PREVIEW Trade for `CallCalendar`, `PutCalendar` and `DoubleCalendar`: the
    /// near and far expiration of each side are resolved from the template's DTEs, their chains
    /// are loaded if they are not yet, and `select_calendar` picks the strikes. No order.
    fn build_calendar_preview(&mut self, template: TradeTemplate, symbol: &str, selection: &mut EntrySelection, log: &mut Vec<String>) -> String {
        if !self.chains.symbols.iter().any(|s| s.symbol == symbol) {
            return format!("No market data is set up for {symbol}: trade cancelled. Check the market data symbols in Settings.");
        }
        let Some(price) = self.chains.price(symbol).and_then(dec) else {
            return format!(
                "{symbol}: {}",
                self.tr("no price from TWS yet - please wait and preview again.", "noch kein Kurs von der TWS - bitte warten und erneut Preview.")
            );
        };
        let today = self.clock.local().date_naive();
        // The expiration of a DTE; an unavailable one moves to the next trading day when the
        // template allows it.
        let resolve = |dte: i32, adjust: bool, log: &mut Vec<String>| -> Result<(i32, String), String> {
            let mut current = dte;
            let mut expiry = expiry_for_dte(today, current);
            if expiry.is_none() && adjust {
                while expiry.is_none() && current < dte.saturating_add(7) {
                    current += 1;
                    expiry = expiry_for_dte(today, current);
                }
                log.push(format!("{symbol} {dte} DTE Expiration is not available. Changing DTE to {current}"));
            }
            expiry.map(|e| (current, e)).ok_or_else(|| format!("{symbol} {dte} DTE Expiration is not available. Cancelling trade."))
        };
        let (puts, calls) = (template.trade_type != "CallCalendar", template.trade_type != "PutCalendar");
        let mut expiries = trade_entry::CalendarExpiries::default();
        let mut needed: Vec<(i32, String)> = Vec::new();
        let legs_of = [
            (puts, template.short_put_dte, template.leg_dte_b.short_put_dte_adjust, 0),
            (puts, template.long_put_dte, template.leg_dte_b.long_put_dte_adjust, 1),
            (calls, template.short_call_dte, template.leg_dte_a.short_call_dte_adjust, 2),
            (calls, template.long_call_dte, template.leg_dte_b.long_call_dte_adjust, 3),
        ];
        for (used, dte, adjust, slot) in legs_of {
            if !used {
                continue;
            }
            let (dte, expiry) = match resolve(dte, adjust, log) {
                Ok(found) => found,
                Err(message) => return message,
            };
            match slot {
                0 => expiries.short_put.clone_from(&expiry),
                1 => expiries.long_put.clone_from(&expiry),
                2 => expiries.short_call.clone_from(&expiry),
                _ => expiries.long_call.clone_from(&expiry),
            }
            if !needed.iter().any(|(_, e)| *e == expiry) {
                needed.push((dte, expiry));
            }
        }
        // Load every expiration that is not loaded yet, then preview again.
        let missing: Vec<(i32, String)> =
            needed.iter().filter(|(_, e)| !self.chains.contracts.iter().any(|c| c.symbol == symbol && c.expiry == *e)).cloned().collect();
        if !missing.is_empty() {
            for (dte, expiry) in &missing {
                if let Some(config) = self.chains.symbols.iter_mut().find(|s| s.symbol == symbol) {
                    if !config.dtes.contains(dte) {
                        config.dtes.push(*dte);
                    }
                }
                if !self.chains.pending_lookups.iter().any(|(_, s, e)| s == symbol && e == expiry) {
                    self.request_chain_lookup(symbol, expiry);
                }
            }
            let list = missing.iter().map(|(dte, _)| dte.to_string()).collect::<Vec<_>>().join(", ");
            return format!("{symbol} {list} DTE Market Data requested now. Please wait a few seconds and preview again.");
        }
        let now = self.clock.instant();
        let wanted: Vec<&str> = needed.iter().map(|(_, e)| e.as_str()).collect();
        let legs = self.chain_legs(symbol, &wanted, now, &selection.account);
        let underlying_outdated = self
            .chains
            .underlyings
            .iter()
            .find(|u| u.symbol == symbol)
            .and_then(|u| u.price_at)
            .is_none_or(|at| now.saturating_duration_since(at) > Duration::from_secs(60));
        let input = trade_entry::CalendarInput {
            template: &template,
            price,
            legs,
            expiries,
            market_open: self.market_is_open(),
            underlying_outdated,
            test_trade: selection.test_trade,
        };
        let outcome = trade_entry::select_calendar(&input);
        log.extend(outcome.log.iter().cloned());
        let mut text = outcome.preview.clone();
        if let Some(entry) = outcome.entry_price {
            let _ = write!(text, "\n{}: {}", self.tr("Entry price (limit)", "Einstiegspreis (Limit)"), entry.normalize());
            selection.outcome = Some((template, outcome));
        }
        text
    }

    /// "Enter Trade" for a template:
    /// selects the strikes again from the live chain, builds the opening order and shows it for
    /// confirmation. Nothing is sent before "Send now".
    pub(super) fn enter_template_trade(&mut self) {
        self.instant.template_status = None;
        self.template.pending = None;
        let account = self.instant.account.trim().to_string();
        let qty = self.instant.template_qty.trim().parse::<u32>().unwrap_or(0);
        if let Err(reason) = self.check_template_entry(&account, qty, self.instant.template_qty.trim()) {
            self.block_template_order(&reason);
            return;
        }
        // Fresh selection (prices may have moved since PREVIEW).
        self.preview_template_trade();
        let Some((template, outcome)) = self.instant.template_outcome.take() else {
            // The preview says why nothing was found (no price yet, no strike in range, ...): that is what the status shows.
            let why = self.instant.template_preview.lines().next().unwrap_or_default().trim().to_string();
            let reason = if why.is_empty() { "no trade found - see Trade Details".to_string() } else { format!("no trade found - {why}") };
            self.block_template_order(&reason);
            return;
        };
        match Self::build_pending_template_order(template, &outcome, &account, qty, self.template_order_flags.instant_template_transmit) {
            Ok(pending) => self.template.pending = Some(pending),
            Err(reason) => self.block_template_order(&reason),
        }
    }

    /// Whether a template trade may be entered now: the safety gate, the connection, a paper
    /// account, a valid quantity and the risk check (`coding_guideline.md` section 8: it runs for every
    /// order). The reason as text if not; `typed` is the quantity as it was typed, for the message.
    pub(super) fn check_template_entry(&self, account: &str, qty: u32, typed: &str) -> Result<(), String> {
        if !self.trading_allowed() {
            return Err("paper-account safety gate not satisfied".to_string());
        }
        if self.tws_cmd_sender.is_none() || self.next_order_id.is_none() {
            return Err("not connected to TWS".to_string());
        }
        if !self.account_matches_mode(account) {
            return Err(format!("account '{account}' does not match the {} mode", if self.link.trading_live { "Live" } else { "Paper Trading" }));
        }
        if qty == 0 {
            return Err(format!("invalid quantity '{typed}'"));
        }
        let quantity = core_types::Quantity::from_f64(f64::from(qty)).map_err(|error| error.to_string())?;
        check_order_quantity(quantity, &self.order_risk_limits()).map_err(|error| format!("risk check failed: {error}"))
    }

    /// The opening order of a selected trade, waiting to be sent (`confirm_template_order`).
    pub(super) fn build_pending_template_order(
        template: TradeTemplate,
        outcome: &trade_entry::EntryOutcome,
        account: &str,
        qty: u32,
        transmit: bool,
    ) -> Result<PendingTemplateOrder, String> {
        let entry = trade_entry::build_entry_order(&template, outcome, qty, "")?;
        let (contract, mut order) = template_ib_order(&entry, account, transmit).ok_or_else(|| "the order price cannot be sent".to_string())?;
        order.fill.all_or_none = template.order_flags.all_or_none;
        Ok(PendingTemplateOrder { template: template_strategy_and_name(&template), source: template, entry, contract, order, run_id: None })
    }
    pub(super) fn block_template_order(&mut self, reason: &str) {
        self.log(&format!("Blocked template trade: {reason}"));
        self.show_notice(format!("{}: {reason}", self.tr("Not sent", "Nicht gesendet")));
    }

    /// The rows of the confirmation of a template trade: what is sent, and how.
    fn template_confirm_rows(&self, pending: &PendingTemplateOrder, de: bool) -> Vec<(&'static str, String)> {
        let e = &pending.entry;
        let o = &pending.order;
        let mut rows: Vec<(&str, String)> = vec![
            (if de { "Konto" } else { "Account" }, format!("IB:{}", o.account)),
            ("Template", pending.template.clone()),
            (if de { "Menge" } else { "Quantity" }, e.qty.to_string()),
        ];
        if e.sec_type == "BAG" {
            let legs: Vec<String> = e
                .legs
                .iter()
                .map(|l| {
                    let leg = self.chains.contracts.iter().find(|c| c.con_id == l.con_id);
                    let text =
                        leg.map_or_else(|| format!("conId {}", l.con_id), |c| format!("{} {}{}", format_date(&c.expiry), format_strike(c.strike), c.right));
                    format!("{} {}x {text}", l.action, l.ratio)
                })
                .collect();
            rows.push((if de { "Legs (Combo)" } else { "Legs (combo)" }, legs.join("\n")));
            let kind = if e.limit < Decimal::ZERO {
                if de {
                    "Credit"
                } else {
                    "credit"
                }
            } else if de {
                "Debit"
            } else {
                "debit"
            };
            rows.push(("Limit", format!("{} ({kind} {})", e.limit.normalize(), e.limit.abs().normalize())));
        } else {
            let strike = e.strike.map_or_else(String::new, |s| s.normalize().to_string());
            rows.push((
                if de { "Kontrakt" } else { "Contract" },
                format!("{} {} {} {}{} ({})", e.action, e.symbol, format_date(&e.expiry), strike, e.right, e.trading_class),
            ));
            rows.push(("Limit", e.limit.normalize().to_string()));
        }
        rows.push((if de { "Börse" } else { "Exchange" }, e.exchange.clone()));
        let transmit = match (o.handling.transmit, de) {
            (true, true) => "sofort an die Börse",
            (true, false) => "transmitted immediately",
            (false, true) => "nur in TWS, nicht übertragen (dort prüfen und senden)",
            (false, false) => "held in TWS, not transmitted (review and send there)",
        };
        rows.push((if de { "Übertragen" } else { "Transmit" }, transmit.to_string()));
        // Re-pricing only runs for an order that goes out right away (see `SentTemplateOrder`).
        let t = &pending.source;
        let fill_note = match (o.handling.transmit, de) {
            (true, true) => format!(
                "{} Versuche, alle {} s, {} pro Schritt; ohne Ausführung wird die Order abgebrochen",
                fill_attempts(t),
                fill_wait(t).as_secs(),
                t.fill_adjustment
            ),
            (true, false) => {
                format!("{} attempts, {} s apart, {} per step; cancelled if it does not fill", fill_attempts(t), fill_wait(t).as_secs(), t.fill_adjustment)
            }
            (false, true) => "aus - die Order wartet in der TWS und wird nicht verändert".to_string(),
            (false, false) => "off - the order waits in TWS and is not changed".to_string(),
        };
        rows.push((if de { "Nachpreisen" } else { "Fill attempts" }, fill_note));
        if self.template_order_flags.instant_template_test {
            rows.push((
                "Test Trade",
                if de { "ja - Preis so, dass die Order nicht ausgeführt wird".into() } else { "yes - priced so that it does not fill".into() },
            ));
        }
        rows
    }

    /// The rows of the confirmation of a single order: contract, order, quote, cost and transmit mode.
    fn instant_confirm_rows(pending: &PendingInstantOrder, de: bool) -> Vec<(String, String, Option<Color32>)> {
        let c = &pending.contract;
        let o = &pending.order;
        let mut rows: Vec<(String, String, Option<Color32>)> = Vec::new();
        let mut row = |en: &str, de_label: &str, value: String| rows.push((if de { de_label } else { en }.to_string(), value, None));
        row("Account", "Konto", if o.account.is_empty() { "-".into() } else { format!("IB:{}", o.account) });
        let side = match (o.action.as_str(), de) {
            ("BUY", true) => "BUY (Kauf - long)",
            ("BUY", false) => "BUY (long)",
            (_, true) => "SELL (Verkauf - short)",
            (_, false) => "SELL (short)",
        };
        row("Action", "Aktion", side.to_string());
        if let Some(last) = rows.last_mut() {
            last.2 = Some(action_color(&o.action));
        }
        let mut row = |en: &str, de_label: &str, value: String| rows.push((if de { de_label } else { en }.to_string(), value, None));
        row("Quantity", "Menge", format!("{}", o.total_quantity));
        let mut contract_text = format!("{} {}", c.symbol, c.sec_type);
        if c.sec_type == "OPT" {
            let option_type = if c.right == "P" { "Put" } else { "Call" };
            contract_text = format!(
                "{} {} {} {}{}",
                c.symbol,
                format_date(&c.last_trade_date_or_contract_month),
                format_strike(c.strike),
                option_type,
                if c.trading_class.is_empty() { String::new() } else { format!(" ({})", c.trading_class) }
            );
        } else if c.sec_type == "FUT" {
            contract_text = format!("{} FUT {}", c.symbol, format_date(&c.last_trade_date_or_contract_month));
        }
        row("Contract", "Kontrakt", contract_text);
        row("Exchange", "Börse", format!("{} ({})", c.exchange, c.currency));
        let price_text = if o.order_type == "LMT" { format!("LMT {:.2}", o.lmt_price) } else { "MKT".to_string() };
        row("Order", "Order", format!("{price_text}, {}", o.tif));
        if let Some((bid, ask, mid)) = pending.quote {
            let money = |v: Option<Decimal>| v.map_or("-".to_string(), |v| format!("{v:.2}"));
            row("Quote now", "Kurs jetzt", format!("Bid {} / Ask {} / Mid {}", money(bid), money(ask), money(mid)));
        }
        // Limit x quantity x 100 (option multiplier), exact and overflow-checked.
        let total =
            dec(o.lmt_price).zip(dec(o.total_quantity)).and_then(|(price, qty)| price.checked_mul(qty)).and_then(|v| v.checked_mul(Decimal::ONE_HUNDRED));
        if let (true, Some(total)) = (o.order_type == "LMT" && c.sec_type == "OPT", total) {
            let what = match (o.action.as_str(), de) {
                ("BUY", true) => "Kosten",
                ("BUY", false) => "Cost",
                (_, true) => "Prämie",
                (_, false) => "Premium",
            };
            row(what, what, format!("{total:.2} {} (x100)", c.currency));
        }
        let transmit = match (o.handling.transmit, de) {
            (true, true) => "sofort an die Börse",
            (true, false) => "transmitted immediately",
            (false, true) => "nur in TWS, nicht übertragen (dort prüfen und senden)",
            (false, false) => "held in TWS, not transmitted (review and send there)",
        };
        row("Transmit", "Übertragen", transmit.to_string());
        if o.handling.what_if {
            row("Test Trade", "Test-Trade", if de { "ja - What-If, keine echte Order".into() } else { "yes - what-if, no real order".into() });
        }

        rows
    }

    /// The confirmation window for a template trade: legs, price, quantity, then Send/Cancel.
    pub(super) fn template_confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.template.pending else {
            return;
        };
        let de = self.german();
        let rows = self.template_confirm_rows(pending, de);
        let mut send = false;
        let mut cancel = false;
        egui::Window::new(if de { "Template-Trade übertragen?" } else { "Send this template trade?" })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::Grid::new("template_confirm_grid").spacing([14.0, 6.0]).show(ui, |ui| {
                    for (label, value) in &rows {
                        ui.label(RichText::new(*label).color(muted_text()));
                        ui.label(RichText::new(value).strong());
                        ui.end_row();
                    }
                });
                (send, cancel) = confirm_buttons(ui, de);
            });
        if send {
            self.confirm_template_order();
        } else if cancel {
            self.template.pending = None;
            self.log("Template trade cancelled in the confirmation");
        }
    }

    /// One contract lookup for an expiry the chain does not have yet (see `request_chain_lookups`).
    pub(super) fn request_chain_lookup(&mut self, symbol: &str, expiry: &str) {
        let Some(sender) = self.tws_cmd_sender.clone() else {
            return;
        };
        let mut unsent = 0;
        let req_id = self.next_request_id();
        self.chains.pending_lookups.push((req_id, symbol.to_string(), expiry.to_string()));
        send_counted(&sender, TwsCommand::RequestContractDetails { req_id, contract: option_chain_lookup(symbol, expiry) }, &mut unsent);
        self.log(&format!("Requesting contracts and market data {symbol} {} (reqId {req_id})", format_date(expiry)));
        self.log_unsent_requests("request chain lookup", unsent);
    }

    /// Stops an Instant Order before it is sent: logged and shown under "Enter Trade".
    fn block_instant_order(&mut self, reason: &str) {
        self.log(&format!("Blocked instant order: {reason}"));
        self.instant.order_error = Some(reason.to_string());
    }

    /// "Enter Trade": checks and builds the order, then asks for confirmation with a summary
    /// (`instant_pending`, see `instant_confirm_dialog`) - nothing is sent yet.
    pub(super) fn submit_instant_order(&mut self) {
        self.instant.order_error = None;
        self.instant.pending = self.prepare_instant_order();
    }

    /// "Send" in the confirmation: re-checks the safety gate (it may have changed while the dialog
    /// was open) and sends the confirmed order to TWS.
    pub(super) fn confirm_instant_order(&mut self) {
        let Some(pending) = self.instant.pending.take() else {
            return;
        };
        if !self.trading_allowed() {
            self.block_instant_order("paper-account safety gate not satisfied");
            return;
        }
        let (true, Some(order_id)) = (self.tws_cmd_sender.is_some(), self.next_order_id) else {
            self.block_instant_order("not connected to TWS");
            return;
        };
        let PendingInstantOrder { contract, order, .. } = pending;
        // An order that opens a position goes through the risk limits; one that closes a position never does.
        let opens = !self.instant_order_closes(&contract, &order.action, &order.account);
        if opens {
            let contracts = order.total_quantity.round().to_u32().unwrap_or(u32::MAX);
            if let Err(reason) = self.check_entry_risk(&contract.symbol, contracts) {
                self.block_instant_order(&format!("risk limit - {reason}"));
                return;
            }
        }
        let sent = (contract.clone(), order.clone());
        self.next_order_id = Some(order_id + 1);
        self.log(&format!(
            "Sending instant order #{order_id}: {} {} {} {} {} {} {}",
            order.action,
            order.total_quantity,
            contract.symbol,
            contract.sec_type,
            if contract.sec_type == "OPT" {
                format!("{} {}{}", format_date(&contract.last_trade_date_or_contract_month), format_strike(contract.strike), contract.right)
            } else {
                String::new()
            },
            order.order_type,
            if order.order_type == "LMT" { format!("{:.2}", order.lmt_price) } else { String::new() },
        ));
        if let Err(reason) = self.submit_order(order_id, contract, order) {
            self.block_instant_order(&reason.to_string());
            return;
        }
        if opens {
            self.count_entry_sent();
        }
        self.instant.last_order_id = Some(order_id);
        self.instant.last_order_status = Some("Sent to TWS".to_string());
        self.instant.sent = Some(sent);
    }

    /// Everything "Enter Trade" checks, producing the order to confirm - or `None` with the
    /// reason shown under the button.
    fn prepare_instant_order(&mut self) -> Option<PendingInstantOrder> {
        if !self.trading_allowed() {
            self.block_instant_order("paper-account safety gate not satisfied");
            return None;
        }
        if self.tws_cmd_sender.is_none() || self.next_order_id.is_none() {
            self.block_instant_order("not connected to TWS");
            return None;
        }
        let quantity: f64 = match parse_dec(&self.instant.qty) {
            Some(value) if value > 0.0 => value,
            _ => {
                self.block_instant_order(&format!("invalid quantity '{}'", self.instant.qty.trim()));
                return None;
            }
        };
        // coding_guideline.md section 8 (MUSS): every order runs through a risk check before
        // being sent - here, the order quantity against a configured limit - independent of the
        // paper-account gate above.
        let risk_quantity = match core_types::Quantity::from_f64(quantity) {
            Ok(value) => value,
            Err(error) => {
                self.block_instant_order(&error.to_string());
                return None;
            }
        };
        if let Err(error) = check_order_quantity(risk_quantity, &self.order_risk_limits()) {
            self.block_instant_order(&format!("risk check failed: {error}"));
            return None;
        }
        let expiry = if matches!(self.instant.sec_type.as_str(), "OPT" | "FUT") {
            if let Some(value) = parse_date_input(&self.instant.expiry) {
                value
            } else {
                self.block_instant_order("no expiry selected");
                return None;
            }
        } else {
            String::new()
        };
        let strike: f64 = if self.instant.sec_type == "OPT" {
            if let Some(value) = parse_dec(&self.instant.strike) {
                value
            } else {
                self.block_instant_order("no strike selected");
                return None;
            }
        } else {
            f64::MAX
        };
        let limit_price = if self.instant.order_type == "LMT" {
            let typed = parse_dec(&self.instant.limit_price).and_then(dec).filter(|v| *v >= Decimal::ZERO);
            // Onto the tick grid (`fix_price`) - off-grid prices are rejected by
            // TWS with error 110. The field shows the price actually sent.
            let on_grid = typed.and_then(|value| fix_price(value, &self.instant.symbol.trim().to_uppercase(), 1).map(|g| (value, g)));
            let Some((value, on_grid)) = on_grid else {
                self.block_instant_order(&format!("invalid limit price '{}'", self.instant.limit_price.trim()));
                return None;
            };
            if on_grid != value {
                self.log(&format!("Instant Order: limit {value} adjusted to the tick grid: {on_grid:.2}"));
                self.instant.limit_price = format!("{on_grid:.2}");
            }
            // f64 only on the TWS wire (`IBApi.Order.LmtPrice` is a double).
            let Some(wire) = on_grid.to_f64() else {
                self.block_instant_order(&format!("limit price {on_grid} cannot be sent"));
                return None;
            };
            wire
        } else {
            f64::MAX
        };
        let contract = IbContract {
            symbol: self.instant.symbol.trim().to_string(),
            sec_type: self.instant.sec_type.clone(),
            last_trade_date_or_contract_month: expiry,
            strike,
            right: if self.instant.sec_type == "OPT" { self.instant.right.clone() } else { String::new() },
            exchange: self.instant.exchange.trim().to_string(),
            currency: self.instant.currency.trim().to_string(),
            // Same contract as the quote shown on the ticket (SPXW, not the AM-settled SPX).
            trading_class: if self.instant.sec_type == "OPT" {
                option_trading_class(&self.instant.symbol.trim().to_uppercase()).unwrap_or_default().to_string()
            } else {
                String::new()
            },
            ..IbContract::default()
        };
        let order = IbOrder {
            action: self.instant.action.clone(),
            total_quantity: quantity,
            order_type: self.instant.order_type.clone(),
            lmt_price: limit_price,
            tif: "DAY".to_string(),
            account: self.instant.account.trim().to_string(),
            handling: OrderHandling {
                transmit: self.instant_flags.instant_transmit,
                what_if: self.instant_flags.instant_test_trade,
                ..OrderHandling::default()
            },
            ..IbOrder::default()
        };
        // The quote as of now, for the summary.
        let quote = (self.instant.quote_req_id.is_some()).then(|| (self.instant.bid.and_then(dec), self.instant.ask.and_then(dec), self.instant_mid()));
        Some(PendingInstantOrder { contract, order, quote })
    }

    /// The confirmation window after "Enter Trade": what exactly will be sent, then Send/Cancel.
    pub(super) fn instant_confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.instant.pending else {
            return;
        };
        let de = self.german();
        let rows = Self::instant_confirm_rows(pending, de);
        let mut send = false;
        let mut cancel = false;
        egui::Window::new(if de { "Trade übertragen?" } else { "Send this trade?" })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::Grid::new("instant_confirm_grid").spacing([14.0, 6.0]).show(ui, |ui| {
                    for (label, value, color) in &rows {
                        ui.label(RichText::new(label).color(muted_text()));
                        let text = RichText::new(value).strong();
                        ui.label(match color {
                            Some(color) => text.color(*color),
                            None => text,
                        });
                        ui.end_row();
                    }
                });
                (send, cancel) = confirm_buttons(ui, de);
            });
        if send {
            self.confirm_instant_order();
        } else if cancel {
            self.instant.pending = None;
            self.log("Instant order cancelled in the confirmation");
        }
    }
}
