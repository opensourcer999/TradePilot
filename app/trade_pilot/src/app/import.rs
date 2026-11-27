//! Taking over positions TWS holds without a trade of this app: the person ticks the legs in the table "Positions Without a Trade",
//! checks the trade they make and its entry price, and stores it. From then on it is an open trade like any other (valuation, stop and
//! target, closing). Nothing is sent to TWS here.

use super::{mask_account, muted_text, App, RichText, StoredTrade, ORANGE_WARN};
use crate::trade_rebuild::import_ref;
use crate::unmanaged::{import_trade, next_import_id, unmanaged_positions, ImportRefusal, UnmanagedPosition};
use eframe::egui;

/// A position of the table "Positions Without a Trade" as the page keeps its selection: account and contract id.
pub(super) type PositionKey = (String, i32);

/// The legs ticked in the table and the window that takes them over.
#[derive(Default)]
pub(super) struct TradeImport {
    /// The ticked rows.
    pub selection: Vec<PositionKey>,
    /// The window, while it is open.
    pub pending: Option<PendingImport>,
}

/// The window "Take over as trade": the legs and the entry price as typed.
pub(super) struct PendingImport {
    /// The legs, by account and contract id.
    pub legs: Vec<PositionKey>,
    /// The entry price per contract as typed (credit positive, debit negative); filled with the proposal when the window opens.
    pub price: String,
}

impl App {
    /// The key of the position at `index` of `self.positions`.
    pub(super) fn position_key(&self, index: usize) -> Option<PositionKey> {
        let position = self.positions.get(index)?;
        Some((position.account.clone(), position.contract_id.trim().parse().ok()?))
    }

    /// The unmanaged positions of the ticked `legs`, in the order of the positions.
    fn chosen_positions(&self, legs: &[PositionKey]) -> Vec<UnmanagedPosition> {
        unmanaged_positions(&self.positions, &self.trades_stored)
            .unwrap_or_default()
            .into_iter()
            .filter(|u| self.position_key(u.position).is_some_and(|key| legs.contains(&key)))
            .collect()
    }

    /// The trade the ticked `legs` make now, with a placeholder id (the real one is given when it is stored).
    fn import_preview(&self, legs: &[PositionKey]) -> Result<StoredTrade, ImportRefusal> {
        let time = self.clock.local().format("%Y%m%d %H:%M:%S").to_string();
        import_trade(&self.positions, &self.chosen_positions(legs), -1, &time)
    }

    /// "Take over as trade": opens the window for the ticked rows, with the proposed entry price, and asks TWS what other programs
    /// have working (their orders on these contracts hold back the import, see `other_client_lock`).
    pub(super) fn open_import(&mut self) {
        let legs = self.import.selection.clone();
        let price = self.import_preview(&legs).map(|trade| trade.price_open.normalize().to_string()).unwrap_or_default();
        self.import.pending = Some(PendingImport { legs, price });
        self.request_all_open_orders(true);
    }

    /// Why the ticked legs cannot be taken over, in the language of the screen.
    fn refusal_text(&self, refusal: ImportRefusal) -> &'static str {
        match refusal {
            ImportRefusal::NoLegs => {
                self.tr("No position is ticked (or it now belongs to a trade).", "Keine Position angehakt (oder sie gehört inzwischen zu einem Trade).")
            }
            ImportRefusal::Accounts => self.tr("The positions are on different accounts.", "Die Positionen liegen auf verschiedenen Konten."),
            ImportRefusal::Symbols => self.tr("The positions are on different underlyings.", "Die Positionen haben verschiedene Basiswerte."),
            ImportRefusal::Multiplier => self.tr(
                "A position has no multiplier, or the positions have different ones.",
                "Eine Position hat keinen Multiplikator, oder die Positionen haben verschiedene.",
            ),
            ImportRefusal::AverageCost => self.tr(
                "TWS sent no average cost for a position: no entry price can be proposed.",
                "Die TWS hat für eine Position keine Einstandskosten gesendet: kein Einstiegspreis möglich.",
            ),
            ImportRefusal::NotATrade => self.tr("The ticked legs do not make a trade.", "Die angehakten Beine ergeben keinen Trade."),
        }
    }

    /// The window "Take over as trade".
    pub(super) fn import_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.import.pending.take() else { return };
        let de = self.german();
        let preview = self.import_preview(&pending.legs);
        let blocked = match &preview {
            Ok(trade) => self.other_client_lock(trade, de),
            Err(refusal) => Some(self.refusal_text(*refusal).to_string()),
        };
        let (mut take_over, mut cancel) = (false, false);
        egui::Window::new(self.tr("Take over as trade?", "Als Trade übernehmen?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                if let Ok(trade) = &preview {
                    egui::Grid::new("import_grid").spacing([14.0, 6.0]).show(ui, |ui| {
                        let rows = [
                            (self.tr("Account", "Konto"), mask_account(&format!("IB:{}", trade.account))),
                            ("Trade", trade.label()),
                            (self.tr("Quantity", "Menge"), trade.qty.to_string()),
                            (self.tr("Multiplier", "Multiplikator"), trade.multiplier.to_string()),
                            (self.tr("Expiry", "Verfall"), trade.expiry.clone()),
                        ];
                        for (label, value) in rows {
                            ui.label(RichText::new(label).color(muted_text()));
                            ui.label(RichText::new(value).strong());
                            ui.end_row();
                        }
                        ui.label(RichText::new(self.tr("Entry price", "Einstiegspreis")).color(muted_text()));
                        ui.add(egui::TextEdit::singleline(&mut pending.price).desired_width(90.0));
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(self.tr(
                            "Per contract, as for every trade: a credit positive, a debit negative. Proposed from the average cost TWS reports (its average price); fees may be in it. Change it if the trade was opened at another price.",
                            "Pro Kontrakt wie bei jedem Trade: Kredit positiv, Debit negativ. Vorgeschlagen aus den Einstandskosten der TWS (ihr Durchschnittspreis); Gebühren können darin stecken. Ändern, wenn der Trade zu einem anderen Preis eröffnet wurde.",
                        ))
                        .small()
                        .color(muted_text()),
                    );
                }
                (take_over, cancel) = super::send_or_cancel(ui, de, self.tr("Take over", "Übernehmen"), blocked.as_deref());
            });
        if take_over {
            self.import.pending = Some(pending);
            self.confirm_import();
        } else if !cancel {
            self.import.pending = Some(pending);
        }
    }

    /// "Take over": checks again what the window showed, then stores the trade under the next import id. It is an open trade from
    /// now on; no order is sent.
    pub(super) fn confirm_import(&mut self) {
        let Some(pending) = self.import.pending.take() else { return };
        let mut trade = match self.import_preview(&pending.legs) {
            Ok(trade) => trade,
            Err(refusal) => {
                self.refuse_import(refusal_log_text(refusal));
                self.import.pending = Some(pending);
                return;
            }
        };
        if let Some(reason) = self.other_client_lock(&trade, false) {
            self.refuse_import(&reason);
            self.import.pending = Some(pending);
            return;
        }
        let Some(price) = crate::pricing::parse_decimal(&pending.price).filter(|p| !p.is_zero()) else {
            self.refuse_import("enter the entry price per contract (a credit positive, a debit negative)");
            self.import.pending = Some(pending);
            return;
        };
        let proposed = trade.price_open;
        let mut known = self.known_trades();
        let Some(order_id) = next_import_id(&known) else {
            self.refuse_import("no import id is left");
            return;
        };
        trade.order_id = order_id;
        trade.price_open = price.normalize();
        let name = import_ref(order_id).unwrap_or_default();
        let source = if price == proposed { "the average cost TWS reports".to_string() } else { format!("typed; {} proposed", proposed.normalize()) };
        self.log(&format!(
            "Trade imported from TWS positions: {name} {} x{} ({}), entry {} ({source})",
            trade.label(),
            trade.qty,
            mask_account(&format!("IB:{}", trade.account)),
            trade.price_open
        ));
        self.store_new_trade(&trade);
        known.push(trade);
        self.import.selection.clear();
        self.adopt_trades(known);
    }

    fn refuse_import(&mut self, reason: &str) {
        self.log(&format!("Import from TWS positions refused: {reason}"));
        self.show_notice(format!("{}: {reason}", self.tr("Not taken over", "Nicht übernommen")));
    }

    /// A short note under the table while rows are ticked: how many and what they make.
    pub(super) fn import_selection_note(&self) -> Option<(String, bool)> {
        if self.import.selection.is_empty() {
            return None;
        }
        Some(match self.import_preview(&self.import.selection) {
            Ok(trade) => (format!("{} x{}", trade.label(), trade.qty), true),
            Err(refusal) => (self.refusal_text(refusal).to_string(), false),
        })
    }
}

/// The reason in English for the log.
fn refusal_log_text(refusal: ImportRefusal) -> &'static str {
    match refusal {
        ImportRefusal::NoLegs => "no position is ticked, or it belongs to a trade by now",
        ImportRefusal::Accounts => "the positions are on different accounts",
        ImportRefusal::Symbols => "the positions are on different underlyings",
        ImportRefusal::Multiplier => "a position has no multiplier, or the positions have different ones",
        ImportRefusal::AverageCost => "TWS sent no average cost for a position",
        ImportRefusal::NotATrade => "the ticked legs do not make a trade",
    }
}

/// The colour of the note under the table.
pub(super) fn note_color(ok: bool) -> egui::Color32 {
    if ok {
        muted_text()
    } else {
        ORANGE_WARN
    }
}
