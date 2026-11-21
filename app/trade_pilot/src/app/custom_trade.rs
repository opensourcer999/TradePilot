//! Custom trades in the app: the preview that finds the legs of a custom template in the live chains (the entry, the order and
//! the record of the trade are the common path of all template trades). See `trade_entry::custom` for the selection.

use super::{dec, expiry_for_dte, template_strategy_and_name, App, Duration, EntrySelection, TradeTemplate};
use crate::trade_entry::custom::{select_custom, CustomInput};
use std::fmt::Write as _;

impl App {
    /// PREVIEW Trade for a `Custom` template: the expiration of each leg is worked out from its DTE (moved to the
    /// next trading day when the template allows it), the chains that are not loaded yet are asked for, and `select_custom`
    /// finds the legs. No order.
    pub(super) fn build_custom_preview(&mut self, template: TradeTemplate, symbol: &str, selection: &mut EntrySelection, log: &mut Vec<String>) -> String {
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
        let legs = [
            (&template.custom_leg1_action, template.custom_leg1_dte, template.leg_dte_custom.cleg1_dte_adjust),
            (&template.custom_leg2_action, template.custom_leg2_dte, template.leg_dte_custom.cleg2_dte_adjust),
            (&template.custom_leg3_action, template.custom_leg3_dte, template.leg_dte_custom.cleg3_dte_adjust),
            (&template.custom_leg4_action, template.custom_leg4_dte, template.leg_dte_a.cleg4_dte_adjust),
        ];
        let mut expiries: [String; 4] = Default::default();
        let mut needed: Vec<(i32, String)> = Vec::new();
        for (slot, (action, dte, adjust)) in legs.into_iter().enumerate() {
            if action == "None" {
                continue;
            }
            let mut current = dte;
            let mut expiry = expiry_for_dte(today, current);
            if expiry.is_none() && adjust {
                while expiry.is_none() && current < dte.saturating_add(7) {
                    current += 1;
                    expiry = expiry_for_dte(today, current);
                }
                log.push(format!("{symbol} {dte} DTE Expiration is not available. Changing DTE to {current}"));
            }
            let Some(expiry) = expiry else {
                return format!("{symbol} {dte} DTE Expiration is not available. Cancelling trade.");
            };
            if let Some(entry) = expiries.get_mut(slot) {
                entry.clone_from(&expiry);
            }
            if !needed.iter().any(|(_, e)| *e == expiry) {
                needed.push((current, expiry));
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
        let chain = self.chain_legs(symbol, &wanted, now, &selection.account);
        let underlying_outdated = self
            .chains
            .underlyings
            .iter()
            .find(|u| u.symbol == symbol)
            .and_then(|u| u.price_at)
            .is_none_or(|at| now.saturating_duration_since(at) > Duration::from_secs(60));
        let open_price = self.quotes.iter().find(|q| q.label == symbol).and_then(|q| q.open).and_then(dec);
        log.push(format!("Previewing custom trade {}", template_strategy_and_name(&template)));
        let input = CustomInput {
            template: &template,
            price,
            open_price,
            legs: chain,
            expiries,
            market_open: self.market_is_open(),
            underlying_outdated,
            test_trade: selection.test_trade,
        };
        let outcome = select_custom(&input);
        log.extend(outcome.log.iter().cloned());
        let mut text = outcome.preview.clone();
        if let Some(entry) = outcome.entry_price {
            let _ = write!(text, "\n{}: {}", self.tr("Entry price (limit)", "Einstiegspreis (Limit)"), entry.normalize());
            selection.outcome = Some((template, outcome));
        }
        text
    }
}
