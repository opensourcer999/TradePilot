//! Attaching a stop and a profit target to an open trade afterwards: a trade that was filled without them, or whose
//! stop was lost. The orders close every leg of the trade, share one OCA group (so that one filling cancels the
//! other) and are recorded on the trade like those placed at the fill, so the reconciliation, the missing-stop mail
//! and the status handling cover them as well. Every order goes through `submit_order`.

use super::alerts::is_order_failure;
use super::{
    fix_price, live_legs, mask_account, muted_text, order_is_final, trade_exit, App, Color32, Decimal, IbOrder, OrderHandling, RichText, StoredTrade,
    ToPrimitive, GREEN_TXT,
};
use eframe::egui;

/// A stop and target to attach, waiting for the user's confirmation.
pub(super) struct PendingExits {
    /// Index in `trades_stored`.
    pub index: usize,
    /// The price per contract that the trade is closed at when the stop is hit (paid to close); empty = no stop.
    pub stop: String,
    /// The price per contract the trade is closed at for its profit (paid to close); empty = no target.
    pub target: String,
}

/// The profit-target orders of a trade that still work: with levels, those of the levels not hit yet (a level that filled keeps its id on
/// record); without, the one profit target.
pub(super) fn working_target_ids(trade: &StoredTrade) -> Vec<i32> {
    match &trade.exit_levels {
        Some(levels) => levels.order_ids.iter().skip(usize::from(levels.level)).flatten().copied().collect(),
        None => trade.profit_order_id.into_iter().collect(),
    }
}

/// What a trade is missing: (a stop, a profit target).
pub(super) fn missing_exits(trade: &StoredTrade) -> (bool, bool) {
    (stop_mode(trade) == ExitMode::Add, target_mode(trade) == ExitMode::Add)
}

/// What the window can set on a trade, new or changed: (the stop, the profit target).
pub(super) fn editable_exits(trade: &StoredTrade) -> (bool, bool) {
    (stop_mode(trade) != ExitMode::Locked, target_mode(trade) != ExitMode::Locked)
}

/// Why a stop or target is not offered in the window.
pub(super) fn locked_note(de: bool) -> &'static str {
    if de {
        "Stop-Limit und gestaffelte Ziele lassen sich hier nicht ändern: erst löschen, dann neu setzen."
    } else {
        "A stop-limit and staged targets cannot be changed here: delete them first, then attach new ones."
    }
}

/// What the window can do with the stop or the target of a trade.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ExitMode {
    /// None yet: a new order is placed.
    Add,
    /// One order that is simply re-sent under its id with the new price (TWS modifies it in place).
    Change,
    /// A shape that cannot be re-priced here (stop-limit, profit levels): delete it first, then attach a new one.
    Locked,
}

/// The mode of the stop of a trade.
pub(super) fn stop_mode(trade: &StoredTrade) -> ExitMode {
    match (trade.stop_order_id, trade.stop_limit_order_id) {
        (None, None) => ExitMode::Add,
        (Some(_), None) if trade.stop_price.is_some() => ExitMode::Change,
        _ => ExitMode::Locked,
    }
}

/// The mode of the profit target of a trade.
pub(super) fn target_mode(trade: &StoredTrade) -> ExitMode {
    if working_target_ids(trade).is_empty() {
        ExitMode::Add
    } else if trade.exit_levels.is_none() && trade.profit_order_id.is_some() && trade.profit_price.is_some() {
        ExitMode::Change
    } else {
        ExitMode::Locked
    }
}

/// A typed price: a decimal comma is accepted; `None` for empty, unreadable or not above 0.
fn typed_price(text: &str) -> Option<Decimal> {
    text.trim().replace(',', ".").parse::<Decimal>().ok().filter(|price| *price > Decimal::ZERO)
}

impl App {
    /// "Stop / target": opens the window for the open trade at `index`.
    pub(super) fn request_exits(&mut self, index: usize) {
        let Some(trade) = self.trades_stored.get(index).filter(|t| t.is_open()) else { return };
        if editable_exits(trade) == (false, false) {
            self.block_exits(locked_note(false));
            return;
        }
        let (stop_mode, target_mode) = (stop_mode(trade), target_mode(trade));
        let shown = |mode: ExitMode, price: Option<Decimal>| {
            if mode == ExitMode::Change {
                price.map(|p| p.normalize().to_string()).unwrap_or_default()
            } else {
                String::new()
            }
        };
        let (stop, target) = (shown(stop_mode, trade.stop_price), shown(target_mode, trade.profit_price));
        self.exits_pending = Some(PendingExits { index, stop, target });
    }

    fn block_exits(&mut self, reason: &str) {
        self.log(&format!("Blocked stop/target: {reason}"));
        self.show_notice(format!("{}: {reason}", self.tr("No stop/target placed", "Kein Stop/Ziel gesetzt")));
    }

    /// The window: the price fields of what the trade is missing and what the market is now.
    pub(super) fn exits_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.exits_pending.take() else { return };
        let Some(trade) = self.trades_stored.get(pending.index).filter(|t| t.is_open()).cloned() else { return };
        let de = self.german();
        let (needs_stop, needs_target) = editable_exits(&trade);
        let ask = self.closing_price(&trade);
        let (mut send, mut cancel) = (false, false);
        egui::Window::new(if de { "Stop / Ziel setzen oder ändern?" } else { "Attach or change a stop / target?" })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::Grid::new("exits_grid").spacing([14.0, 6.0]).show(ui, |ui| {
                    let rows = [
                        (if de { "Konto" } else { "Account" }, mask_account(&format!("IB:{}", trade.account))),
                        ("Trade", trade.label()),
                        (if de { "Menge" } else { "Quantity" }, trade.remaining_qty().to_string()),
                        (if de { "Eröffnet zu" } else { "Opened at" }, trade.price_open.normalize().to_string()),
                        (if de { "Schließkurs jetzt" } else { "Cost to close now" }, ask.map_or_else(|| "-".to_string(), |a| a.normalize().to_string())),
                    ];
                    for (label, value) in rows {
                        ui.label(RichText::new(label).color(muted_text()));
                        ui.label(RichText::new(value).strong());
                        ui.end_row();
                    }
                    if needs_stop {
                        ui.label(RichText::new(if de { "Stop (Schließkurs)" } else { "Stop (price to close)" }).color(muted_text()));
                        crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut pending.stop).desired_width(80.0));
                        ui.end_row();
                    }
                    if needs_target {
                        ui.label(RichText::new(if de { "Ziel (Schließkurs)" } else { "Target (price to close)" }).color(muted_text()));
                        crate::widgets::metal_field(ui, egui::TextEdit::singleline(&mut pending.target).desired_width(80.0));
                        ui.end_row();
                    }
                });
                ui.add_space(8.0);
                ui.label(if de {
                    "Die Orders schließen alle Legs, laufen als OCA-Gruppe (eine füllt, die andere wird storniert) und werden am Trade vermerkt. Leer lassen = nicht setzen. Ein vorhandener Stop/Ziel wird mit dem neuen Preis geändert."
                } else {
                    "The orders close every leg and run as one OCA group (one fills, the other is cancelled); they are recorded on the trade. Leave a field empty to skip it. An existing stop/target is changed to the new price."
                });
                if !needs_stop || !needs_target {
                    ui.label(RichText::new(locked_note(de)).small().color(muted_text()));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(crate::widgets::MetalButton::new(RichText::new(if de { "Jetzt übertragen" } else { "Send now" }).color(Color32::WHITE).strong()).fill(GREEN_TXT)).clicked() {
                        send = true;
                    }
                    if ui.add(crate::widgets::MetalButton::new(if de { "Abbrechen" } else { "Cancel" })).clicked() {
                        cancel = true;
                    }
                });
            });
        if send {
            self.exits_pending = Some(pending);
            self.confirm_exits();
        } else if cancel {
            self.log("Attaching a stop/target cancelled in the confirmation");
        } else {
            self.exits_pending = Some(pending);
        }
    }

    /// "Send now": checks the prices against the market, then places the stop and the target.
    pub(super) fn confirm_exits(&mut self) {
        let Some(pending) = self.exits_pending.take() else { return };
        let Some(trade) = self.trades_stored.get(pending.index).filter(|t| t.is_open()).cloned() else { return };
        let (stop_editable, target_editable) = editable_exits(&trade);
        let stop = stop_editable.then(|| typed_price(&pending.stop)).flatten();
        let target = target_editable.then(|| typed_price(&pending.target)).flatten();
        if stop.is_none() && target.is_none() {
            self.block_exits("enter a price above 0 for the stop or the target");
            return;
        }
        let legs = trade_exit::close_legs(&trade);
        if trade.price_open <= Decimal::ZERO {
            self.block_exits("only credit trades are supported - set the stop of a debit trade in TWS");
            return;
        }
        let Some(contract) = trade_exit::exit_contract(&trade, &legs) else {
            self.block_exits("the closing order cannot be built (legs missing - a trade from before V0.97?)");
            return;
        };
        let (true, Some(first_id)) = (self.tws_cmd_sender.is_some(), self.next_order_id) else {
            self.block_exits("not connected to TWS");
            return;
        };
        let leg_count = legs.len();
        let on_grid = |price: Option<Decimal>| match price {
            None => Ok(None),
            Some(price) => fix_price(price, &trade.symbol, leg_count).map(Some).ok_or(()),
        };
        let (Ok(stop), Ok(target)) = (on_grid(stop), on_grid(target)) else {
            self.block_exits("a price cannot be put on the exchange's price grid");
            return;
        };
        // A price that is the one on record once it is on the grid changes nothing and is left out.
        let stop = stop.filter(|p| Some(*p) != trade.stop_price);
        let target = target.filter(|p| Some(*p) != trade.profit_price);
        if stop.is_none() && target.is_none() {
            self.block_exits("the prices are the ones on record (on the exchange's price grid) - nothing to change");
            return;
        }
        // A stop that is passed already or a target that is reached already would trigger at once.
        if let Some(ask) = live_legs(&trade, &self.chains).and_then(|legs| legs.current_ask(&trade.trade_type)) {
            if stop.is_some_and(|stop| ask >= stop) {
                self.block_exits(&format!("the stop is passed already (the trade costs {} to close now)", ask.normalize()));
                return;
            }
            if target.is_some_and(|target| ask <= target) {
                self.block_exits(&format!("the target is reached already (the trade costs {} to close now) - it would fill at once", ask.normalize()));
                return;
            }
        }
        self.place_attached_exits(pending.index, &trade, contract, stop, target, first_id, legs.as_slice());
    }

    /// Sends the orders. A new order is recorded on the trade once it was handed to TWS; a changed one keeps its old price on
    /// record until TWS confirms the change (`on_exit_change_status`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_attached_exits(
        &mut self,
        index: usize,
        trade: &StoredTrade,
        contract: super::IbContract,
        stop: Option<Decimal>,
        target: Option<Decimal>,
        first_id: i32,
        legs: &[super::ComboLegSpec],
    ) {
        let label = trade.label();
        // A stop or target that exists is sent again under its own id: TWS then changes the working order.
        let stop_id = trade.stop_order_id.filter(|_| stop_mode(trade) == ExitMode::Change).unwrap_or(first_id);
        let target_id = trade.profit_order_id.filter(|_| target_mode(trade) == ExitMode::Change).unwrap_or(first_id.saturating_add(1));
        self.next_order_id = Some(first_id.saturating_add(2));
        // An existing group is joined (its other order is then reduced or cancelled with these); else a new one starts.
        let group = trade_exit::trade_oca_group(trade).unwrap_or_else(|| trade_exit::oca_group(first_id));
        let tif = if trade.expiry == self.clock.new_york_today() { "DAY" } else { "GTC" };
        let action = trade_exit::closing_action(legs).as_str();
        let order = |kind: &str, id: i32| IbOrder {
            action: action.to_string(),
            total_quantity: f64::from(trade.remaining_qty()),
            tif: tif.to_string(),
            account: trade.account.clone(),
            order_ref: crate::trade_rebuild::order_ref(id, kind),
            oca_group: group.clone(),
            oca_type: 3,
            handling: OrderHandling { transmit: true, ..OrderHandling::default() },
            ..IbOrder::default()
        };
        let mut updated = trade.clone();
        updated.oca_group = Some(group.clone());
        let mut placed = false;
        if let Some(price) = stop.and_then(|p| p.to_f64().map(|f| (p, f))) {
            let change = stop_mode(trade) == ExitMode::Change;
            let stop_order = IbOrder { order_type: "STP".to_string(), aux_price: price.1, ..order("SL", stop_id) };
            let old = trade.stop_price.filter(|_| change);
            self.log(&exit_order_line("STP", stop_id, old, price.1, stop_order.total_quantity, &label));
            if self.submit_order(stop_id, contract.clone(), stop_order).is_ok() {
                placed = true;
                if change {
                    self.watch.exit_changes.insert(stop_id, price.0.normalize());
                } else {
                    updated.stop_order_id = Some(stop_id);
                    updated.stop_price = Some(price.0.normalize());
                }
            }
        }
        if let Some(price) = target.and_then(|p| p.to_f64().map(|f| (p, f))) {
            let change = target_mode(trade) == ExitMode::Change;
            let target_order = IbOrder { order_type: "LMT".to_string(), lmt_price: price.1, ..order("TP", target_id) };
            let old = trade.profit_price.filter(|_| change);
            self.log(&exit_order_line("Profit Limit", target_id, old, price.1, target_order.total_quantity, &label));
            if self.submit_order(target_id, contract, target_order).is_ok() {
                placed = true;
                if change {
                    self.watch.exit_changes.insert(target_id, price.0.normalize());
                } else {
                    updated.profit_order_id = Some(target_id);
                    updated.profit_price = Some(price.0.normalize());
                }
            }
        }
        if !placed {
            self.block_exits("TWS did not take the orders - see the log");
            return;
        }
        self.save_trade(index, updated);
        self.instant.template_status = Some(format!("{label}: {}", self.tr("stop/target sent to TWS", "Stop/Ziel an die TWS gesendet")));
    }

    /// The status of an order: a change sent from the window takes effect on the trade once TWS reports the order as
    /// working again. An order that ended instead was not changed; the status handling of the exit orders deals with it.
    pub(super) fn on_exit_change_status(&mut self, order_id: i32, status: &str) {
        let confirmed = matches!(status, "Submitted" | "PreSubmitted");
        if !confirmed && !order_is_final(status) {
            return;
        }
        let Some(price) = self.watch.exit_changes.remove(&order_id) else { return };
        if !confirmed {
            self.log(&format!("WARNING: the change of order {order_id} to {price} was not made: the order is {status}"));
            return;
        }
        let found = self.trades_stored.iter().position(|t| t.is_open() && (t.stop_order_id == Some(order_id) || t.profit_order_id == Some(order_id)));
        let Some((index, trade)) = found.and_then(|index| self.trades_stored.get(index).cloned().map(|trade| (index, trade))) else {
            self.log(&format!("WARNING: TWS confirmed the change of order {order_id} to {price}, but no open trade has that order any more"));
            return;
        };
        let label = trade.label();
        let mut updated = trade.clone();
        if trade.stop_order_id == Some(order_id) {
            updated.stop_price = Some(price);
        } else {
            updated.profit_price = Some(price);
        }
        self.log(&format!("Order {order_id} of {label} changed to {price}, confirmed by TWS"));
        self.save_trade(index, updated);
    }

    /// TWS refused a change sent from the window: the order keeps its old price, if it still works at all.
    pub(super) fn on_exit_change_error(&mut self, order_id: i32, code: i32, message: &str) {
        if !is_order_failure(code) {
            return;
        }
        let Some(price) = self.watch.exit_changes.remove(&order_id) else { return };
        self.log(&format!(
            "WARNING: TWS refused the change of order {order_id} to {price} (TWS {code}: {message}) - the order keeps its old price if it is still working: check it in TWS"
        ));
        self.show_notice(format!("{}: TWS {code}: {message}", self.tr("Change not made", "Änderung nicht übernommen")));
    }
}

/// The log line of a stop or target sent from the window: a new order, or the change of a working one from `old` to `price`.
fn exit_order_line(kind: &str, id: i32, old: Option<Decimal>, price: f64, quantity: f64, label: &str) -> String {
    match old {
        Some(old) => format!("Modify {kind} ClientOrderID: {id} Price: {} -> {price} Qty: {quantity} ({label})", old.normalize()),
        None => format!("Place {kind} ClientOrderID: {id} Price: {price} Qty: {quantity} (attached to {label})"),
    }
}
