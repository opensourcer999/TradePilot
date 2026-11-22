//! Closing a trade by hand: the close dialog, the close order, Mark Closed, and the actions under the list of open trades.

use super::{
    attach, check_order_quantity, fix_price, muted_text, trade_exit, App, CloseKind, Color32, Decimal, Duration, IbContract, IbOrder, OrderHandling,
    PendingClose, RichText, StoredTrade, TradeStatus, ACCENT_BLUE, GREEN_TXT, NOTICE_SECONDS, RED, STATUS_ERROR,
};
use eframe::egui;
use rust_decimal::prelude::ToPrimitive;

impl App {
    /// A click on Close Trade / Close at Market / Mark Closed: opens the confirmation with the
    /// current price to close as the suggestion (when the legs are quoted).
    pub(super) fn request_close(&mut self, index: usize, kind: CloseKind) {
        let Some(trade) = self.trades_stored.get(index) else {
            return;
        };
        if !trade.is_open() {
            return;
        }
        let leg_count = trade_exit::close_legs(trade).len();
        let suggestion = self.closing_price(trade).and_then(|ask| fix_price(ask, &trade.symbol, leg_count)).filter(|_| kind != CloseKind::Market);
        let price = suggestion.map_or_else(String::new, |p| p.normalize().to_string());
        self.close_pending = Some(PendingClose { index, kind, price });
    }

    /// Shows `text` below the Enter Trade button for `NOTICE_SECONDS`; a newer status replaces it earlier.
    pub(super) fn show_notice(&mut self, text: String) {
        self.instant.template_notice = Some((text.clone(), self.clock.instant() + Duration::from_secs(NOTICE_SECONDS)));
        self.instant.template_status = Some(text);
    }

    /// Takes a message away once its time is up (only if it is still the one shown), and wakes the screen for that moment.
    pub(super) fn expire_notice(&mut self, ctx: &egui::Context) {
        let Some((text, until)) = self.instant.template_notice.clone() else { return };
        let now = self.clock.instant();
        if self.instant.template_status.as_ref() != Some(&text) {
            self.instant.template_notice = None;
        } else if now >= until {
            self.instant.template_status = None;
            self.instant.template_notice = None;
        } else {
            ctx.request_repaint_after(until - now);
        }
    }

    fn block_close(&mut self, reason: &str) {
        self.log(&format!("Blocked close: {reason}"));
        self.show_notice(format!("{}: {reason}", self.tr("Not closed", "Nicht geschlossen")));
    }

    /// "Mark Closed": the trade was closed outside the app at `price`; it is booked as closed.
    fn mark_closed(&mut self, index: usize, trade: &StoredTrade, price: Option<Decimal>) {
        let Some(paid) = price else {
            self.block_close("enter the price the trade was closed at (0 or more)");
            return;
        };
        // The position is gone: its stop, profit target and close order must not stay working
        // (cancelling only reduces risk, so it needs no safety gate).
        self.cancel_orders_of(trade);
        let close = if trade.price_open > Decimal::ZERO { -paid } else { paid };
        // The typed price is for what was still open; levels hit before count in the average.
        let Some(average) = self.average_close_price(trade, close) else {
            self.block_close("the closing price cannot be averaged with the profit targets hit so far");
            return;
        };
        self.finish_trade(index, TradeStatus::ClosedOther, average, "Mark Closed");
    }
    /// "Send now" / "Mark closed" of the close confirmation.
    pub(super) fn confirm_close(&mut self) {
        let Some(pending) = self.close_pending.take() else {
            return;
        };
        let Some(trade) = self.trades_stored.get(pending.index).filter(|t| t.is_open()).cloned() else {
            return;
        };
        let price = crate::pricing::parse_decimal(&pending.price).filter(|p| *p >= Decimal::ZERO);
        if pending.kind == CloseKind::Mark {
            self.mark_closed(pending.index, &trade, price);
            return;
        }

        // A close order: the typed limit (paid to close), or a market order.
        let limit = if pending.kind == CloseKind::Limit {
            let Some(raw) = price.filter(|p| *p > Decimal::ZERO) else {
                self.block_close("enter a limit price above 0");
                return;
            };
            Some(raw)
        } else {
            None
        };
        match self.place_close_order(pending.index, limit) {
            Ok(order_id) => {
                self.instant.template_status = Some(format!("#{order_id}: {}", self.tr("close order sent to TWS", "Close-Order an die TWS gesendet")));
            }
            Err(reason) => self.block_close(&reason),
        }
    }

    /// The contract and the order that close `trade` under the id `order_id`: a limit order at `limit` (put on the exchange's price grid) or,
    /// without one, a market order; also the limit as it goes to TWS (0 for a market order). Only what is still open is closed. The order
    /// joins the OCA group of the trade's exit orders. An error says why it cannot be built.
    fn close_order_parts(trade: &StoredTrade, order_id: i32, limit: Option<Decimal>) -> Result<(IbContract, IbOrder, f64), String> {
        let legs = trade_exit::close_legs(trade);
        let contract = trade_exit::exit_contract(trade, &legs)
            .ok_or_else(|| "the closing order cannot be built (legs missing - a trade from before V0.97?)".to_string())?;
        let limit_price = match limit {
            Some(raw) => {
                let fixed = fix_price(raw, &trade.symbol, legs.len()).ok_or_else(|| "the limit price cannot be computed".to_string())?;
                // A combo bought as a debit is closed by buying its legs the other way round for a credit: the typed price is what it
                // brings, so the limit of the BUY combo is negative. Any other close pays the typed price (a single long is sold at it).
                let receives = trade.price_open < Decimal::ZERO && legs.len() > 1;
                let sent = if receives { -fixed } else { fixed };
                sent.to_f64().ok_or_else(|| "the limit price cannot be sent".to_string())?
            }
            None => 0.0,
        };
        // A combo is bought (its legs carry their own actions); a single option is bought back
        // (a sold one) or sold (a bought one).
        let action = trade_exit::closing_action(&legs);
        let (oca_group, oca_type) = trade_exit::trade_oca_group(trade).map_or((String::new(), 0), |group| (group, 3));
        let order = IbOrder {
            action: action.to_string(),
            total_quantity: f64::from(trade.remaining_qty()),
            order_type: if limit.is_some() { "LMT" } else { "MKT" }.to_string(),
            lmt_price: limit_price,
            tif: "DAY".to_string(),
            account: trade.account.clone(),
            order_ref: crate::trade_rebuild::order_ref(order_id, "Close"),
            oca_group,
            oca_type,
            handling: OrderHandling { transmit: true, ..OrderHandling::default() },
            ..IbOrder::default()
        };
        Ok((contract, order, limit_price))
    }

    /// Sends the close order `order_id` of the trade at `index` again with the limit `limit`: TWS changes the price of the working order.
    /// Goes through `submit_order` like every change of an order, so the safety gate holds for it too.
    pub(super) fn move_close_limit(&mut self, index: usize, order_id: i32, limit: Decimal) -> Result<(), String> {
        let trade = self.trades_stored.get(index).filter(|t| t.is_open()).cloned().ok_or_else(|| "the trade is not open".to_string())?;
        let (contract, order, limit_price) = Self::close_order_parts(&trade, order_id, Some(limit))?;
        self.log(&format!("Adjust {order_id} close price to {limit_price} ({})", trade.label()));
        self.submit_order(order_id, contract, order).map_err(|reason| reason.to_string())
    }

    /// Sends the order that closes what is still open of the trade at `index`: a limit order at `limit` (the price to pay,
    /// or for a custom combo entered as a debit the price it brings,
    /// put on the exchange's price grid) or, without one, a market order. It goes through the same checks as every order
    /// of this app and joins the OCA group of the trade's exit orders, so a fill of it reduces them and a stop cannot fire
    /// while the trade is being closed. Returns the order id; the order is recorded on the trade as its close order.
    pub(super) fn place_close_order(&mut self, index: usize, limit: Option<Decimal>) -> Result<i32, String> {
        let trade = self.trades_stored.get(index).filter(|t| t.is_open()).cloned().ok_or_else(|| "the trade is not open".to_string())?;
        if !self.trading_allowed() {
            return Err("paper-account safety gate not satisfied".to_string());
        }
        let (true, Some(order_id)) = (self.tws_cmd_sender.is_some(), self.next_order_id) else {
            return Err("not connected to TWS".to_string());
        };
        if !self.account_matches_mode(&trade.account) {
            return Err(format!("account '{}' does not match the {} mode", trade.account, if self.link.trading_live { "Live" } else { "Paper Trading" }));
        }
        let quantity = core_types::Quantity::from_f64(f64::from(trade.remaining_qty())).map_err(|error| error.to_string())?;
        check_order_quantity(quantity, &self.order_risk_limits()).map_err(|error| format!("risk check failed: {error}"))?;
        let (contract, order, limit_price) = Self::close_order_parts(&trade, order_id, limit)?;
        self.next_order_id = Some(order_id.saturating_add(1));
        self.log(&format!(
            "Place {} close ClientOrderID: {order_id} {} {} {} x{}{}",
            if limit.is_some() { "Limit" } else { "MKT" },
            trade.symbol,
            trade.trade_type,
            trade.strikes,
            trade.qty,
            if limit.is_some() { format!(" Price: {limit_price}") } else { String::new() }
        ));
        self.submit_order(order_id, contract, order).map_err(|reason| reason.to_string())?;
        let mut updated = trade;
        updated.close_order_id = Some(order_id);
        self.save_trade(index, updated);
        Ok(order_id)
    }

    /// The confirmation window of a manual close.
    pub(super) fn close_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.close_pending.take() else {
            return;
        };
        let Some(trade) = self.trades_stored.get(pending.index).filter(|t| t.is_open()).cloned() else {
            return;
        };
        let de = self.german();
        let title = match (pending.kind, de) {
            (CloseKind::Limit, true) => "Trade mit Limit schließen?",
            (CloseKind::Limit, false) => "Close this trade with a limit order?",
            (CloseKind::Market, true) => "Trade zum Marktpreis schließen?",
            (CloseKind::Market, false) => "Close this trade at market?",
            (CloseKind::Mark, true) => "Trade als geschlossen markieren?",
            (CloseKind::Mark, false) => "Mark this trade as closed?",
        };
        let mut send = false;
        let mut cancel = false;
        egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            egui::Grid::new("close_confirm_grid").spacing([14.0, 6.0]).show(ui, |ui| {
                let rows = [
                    (if de { "Konto" } else { "Account" }, format!("IB:{}", trade.account)),
                    ("Trade", trade.label()),
                    (if de { "Menge" } else { "Quantity" }, trade.qty.to_string()),
                    (if de { "Eröffnet zu" } else { "Opened at" }, trade.price_open.normalize().to_string()),
                ];
                for (label, value) in rows {
                    ui.label(RichText::new(label).color(muted_text()));
                    ui.label(RichText::new(value).strong());
                    ui.end_row();
                }
                match pending.kind {
                    CloseKind::Limit => {
                        ui.label(RichText::new(if de { "Limit (zu zahlen)" } else { "Limit (to pay)" }).color(muted_text()));
                        crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut pending.price).desired_width(80.0));
                        ui.end_row();
                    }
                    CloseKind::Mark => {
                        ui.label(RichText::new(if de { "Geschlossen zu" } else { "Closed at" }).color(muted_text()));
                        crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut pending.price).desired_width(80.0));
                        ui.end_row();
                    }
                    CloseKind::Market => {}
                }
            });
            ui.add_space(8.0);
            let note = match (pending.kind, de) {
                (CloseKind::Mark, true) => "Es wird keine Order gesendet. Stop, Profit-Target und Close-Order dieses Trades werden in TWS storniert.",
                (CloseKind::Mark, false) => "No order is sent. This trade's stop, profit target and close order are cancelled in TWS.",
                (_, true) => "Die Order schließt alle Legs. Stop und Profit-Target bleiben bis zur Ausführung aktiv und werden dann reduziert.",
                (_, false) => "The order closes every leg. The stop and profit target stay active until it fills, then they are reduced.",
            };
            ui.label(note);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let label = match (pending.kind, de) {
                    (CloseKind::Mark, true) => "Als geschlossen markieren",
                    (CloseKind::Mark, false) => "Mark closed",
                    (_, true) => "Jetzt übertragen",
                    (_, false) => "Send now",
                };
                if ui.add(crate::widgets::MetalButton::new(RichText::new(label).color(Color32::WHITE).strong()).fill(GREEN_TXT)).clicked() {
                    send = true;
                }
                if ui.add(crate::widgets::MetalButton::new(if de { "Abbrechen" } else { "Cancel" })).clicked() {
                    cancel = true;
                }
            });
        });
        if send {
            self.close_pending = Some(pending);
            self.confirm_close();
        } else if cancel {
            self.log("Close cancelled in the confirmation");
        } else {
            self.close_pending = Some(pending);
        }
    }

    /// The open trades with what protects them and the buttons to end them by hand.
    /// The buttons of an open trade: the three that close it (the one clicked is returned) and the one that attaches a stop
    /// and a target (`exits_clicked`).
    fn close_buttons(&self, ui: &mut egui::Ui, can_attach: bool, exits_clicked: &mut bool, deletable: (bool, bool, &mut Option<bool>)) -> Option<CloseKind> {
        let mut clicked = None;
        let (has_stop, has_target, delete) = deletable;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(has_target, crate::widgets::MetalButton::new(self.tr("Delete target", "Ziel löschen"))).clicked() {
                *delete = Some(false);
            }
            if ui.add_enabled(has_stop, crate::widgets::MetalButton::new(self.tr("Delete stop", "Stop löschen"))).clicked() {
                *delete = Some(true);
            }
            if ui.add(crate::widgets::MetalButton::new(self.tr("Mark closed", "Als geschlossen markieren"))).clicked() {
                clicked = Some(CloseKind::Mark);
            }
            if ui.add(crate::widgets::MetalButton::new(self.tr("Close at market", "Zum Markt schließen"))).clicked() {
                clicked = Some(CloseKind::Market);
            }
            if ui.add(crate::widgets::MetalButton::new(self.tr("Close (limit)", "Schließen (Limit)"))).clicked() {
                clicked = Some(CloseKind::Limit);
            }
            // Greyed out for a trade whose stop and target can neither be attached nor changed here.
            let hint = if can_attach {
                self.tr("Attach or change a stop and a profit target in TWS", "Stop und Profit Target in der TWS anhängen oder ändern")
            } else {
                attach::locked_note(self.german())
            };
            if ui
                .add_enabled(can_attach, crate::widgets::MetalButton::new(self.tr("Stop / target", "Stop / Ziel")))
                .on_hover_text(hint)
                .on_disabled_hover_text(hint)
                .clicked()
            {
                *exits_clicked = true;
            }
        });
        clicked
    }

    /// The question before a stop or a profit target is deleted: without it the trade is no longer protected (or no longer has its target).
    fn delete_exit_question(&mut self, ui: &mut egui::Ui) {
        let Some((index, stop)) = self.watch.delete_asked else { return };
        let Some(trade) = self.trades_stored.get(index).filter(|t| t.is_open()).cloned() else {
            self.watch.delete_asked = None;
            return;
        };
        let what = if stop { self.tr("the stop", "den Stop") } else { self.tr("the profit target", "das Profit Target") };
        let warning = if stop { self.tr("The trade then has no stop.", "Der Trade hat dann keinen Stop.") } else { "" };
        ui.horizontal(|ui| {
            ui.colored_label(RED, format!("{} {what} {} {}? {warning}", self.tr("Delete", "Löschen:"), self.tr("of", "von"), trade.label()));
            if ui.add(crate::widgets::MetalButton::new(self.tr("Yes, delete", "Ja, löschen"))).clicked() {
                self.watch.delete_asked = None;
                self.delete_exit_orders(index, stop);
            }
            if ui.add(crate::widgets::MetalButton::new(self.tr("No", "Nein"))).clicked() {
                self.watch.delete_asked = None;
            }
        });
    }

    pub(super) fn open_trade_actions(&mut self, ui: &mut egui::Ui) {
        let rows: Vec<(usize, String, String, bool)> = self
            .trades_stored
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_open())
            .map(|(index, t)| {
                let stop = t.stop_price.map(|p| format!("Stop {}", p.normalize()));
                let target = t.profit_price.map(|p| format!("{} {}", self.tr("Target", "Ziel"), p.normalize()));
                let parts: Vec<String> = [stop, target].into_iter().flatten().collect();
                let protected = !parts.is_empty();
                let mut info = if protected { parts.join(" · ") } else { self.tr("no stop or target", "kein Stop, kein Ziel").to_string() };
                // The exit time of the template, New York time.
                if let Some(time) = t.timed_exit.as_ref().and_then(|exit| exit.at.split_whitespace().last()) {
                    info = format!("{info} · {} {time} ET", self.tr("exit", "Ausstieg"));
                }
                // A stop that covers fewer contracts than the trade holds is the most urgent thing to say about it.
                let gap = crate::reconcile::stop_coverage(t, &self.exit_coverage.remaining);
                if let Some((covered, held)) = gap {
                    info = format!("{info} · {} {covered}/{held}", self.tr("stop covers only", "Stop deckt nur"));
                }
                let close = t.close_order_id.map(|id| format!(" · {} #{id}", self.tr("close order", "Close-Order")));
                (index, t.label(), format!("{info}{}", close.unwrap_or_default()), protected && gap.is_none())
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        ui.add_space(12.0);
        let heading = format!("{} ({})", self.tr("Manage open trades", "Offene Trades verwalten"), rows.len());
        let mut request: Option<(usize, CloseKind)> = None;
        let mut exits_request: Option<usize> = None;
        let mut delete_request: Option<(usize, bool)> = None;
        egui::CollapsingHeader::new(RichText::new(heading).size(13.0).color(muted_text())).default_open(true).show(ui, |ui| {
            if !rows.iter().any(|(index, ..)| self.ui_state.selected_trade == Some(*index)) {
                ui.label(
                    RichText::new(self.tr(
                        "Click a trade in the list above to select it, then close it here.",
                        "Klicke oben auf einen Trade, um ihn auszuwählen, und schließe ihn hier.",
                    ))
                    .color(muted_text())
                    .italics(),
                );
            }
            let chosen = self.ui_state.selected_trade;
            for (index, label, info, protected) in rows.iter().filter(|(index, ..)| chosen == Some(*index)) {
                ui.horizontal(|ui| {
                    let selected = self.ui_state.selected_trade == Some(*index);
                    let text = RichText::new(if selected { format!("> {label}") } else { label.clone() }).strong();
                    ui.label(if selected { text.color(ACCENT_BLUE) } else { text });
                    ui.label(RichText::new(info).color(if *protected { muted_text() } else { STATUS_ERROR }));
                    let mut exits = false;
                    let can_attach = self.trades_stored.get(*index).is_some_and(|t| attach::editable_exits(t) != (false, false));
                    let (no_stop, no_target) = self.trades_stored.get(*index).map_or((true, true), attach::missing_exits);
                    let mut delete = None;
                    if let Some(kind) = self.close_buttons(ui, can_attach, &mut exits, (!no_stop, !no_target, &mut delete)) {
                        request = Some((*index, kind));
                    }
                    if exits {
                        exits_request = Some(*index);
                    }
                    if let Some(stop) = delete {
                        delete_request = Some((*index, stop));
                    }
                });
            }
        });
        if let Some((index, kind)) = request {
            self.request_close(index, kind);
        }
        if let Some(index) = exits_request {
            self.request_exits(index);
        }
        if let Some(asked) = delete_request {
            self.watch.delete_asked = Some(asked);
        }
        self.delete_exit_question(ui);
    }
}
