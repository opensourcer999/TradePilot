//! The risk check of a new position before its opening order is sent (Settings > Safety Features > Risk limits): the contracts open in
//! its symbol, today's loss and the number of new orders within the last minute (`risk::check_entry`). It applies to template trades
//! (entered by hand, by a schedule, a trigger or a re-entry, and sent again after a margin rejection) and to Instant Orders that open a
//! position. Orders that close or protect a position are never checked.
//!
//! Once today's loss has reached its limit, no new position is opened for the rest of the New York day, even if the loss gets smaller
//! again: a recovering market is no reason to trade on after the day's loss limit.

use super::{profit_summary, App, Decimal, IbContract, StoredTrade};
use risk::{check_entry, EntryLimits, EntryRejection, EntryState};
use std::time::Duration;

/// The window of the limit of new orders per minute.
const RATE_WINDOW: Duration = Duration::from_secs(60);

/// A whole number of a risk setting; empty, 0 or unreadable = no limit.
fn count_setting(text: &str) -> u32 {
    text.trim().parse::<u32>().unwrap_or(0)
}

impl App {
    /// The limits of the settings.
    fn entry_limits(&self) -> EntryLimits {
        EntryLimits {
            max_open_per_symbol: count_setting(&self.settings.risk_max_contracts_per_symbol),
            max_daily_loss: crate::pricing::parse_decimal(&self.settings.risk_max_daily_loss).filter(|loss| *loss > Decimal::ZERO).unwrap_or(Decimal::ZERO),
            max_orders_per_minute: count_setting(&self.settings.risk_max_orders_per_minute),
        }
    }

    /// Checks a new position of `contracts` contracts (or combos) in `symbol` against the risk limits. The reason as text when it is
    /// refused; the refusal is logged. A daily loss at its limit stops new positions for the rest of the day.
    pub(super) fn check_entry_risk(&mut self, symbol: &str, contracts: u32) -> Result<(), String> {
        let now = self.clock.instant();
        self.watch.entries_sent.retain(|sent| now.saturating_duration_since(*sent) < RATE_WINDOW);
        let today = self.clock.new_york_today();
        let limits = self.entry_limits();
        if self.watch.loss_stop_day.as_deref() == Some(today.as_str()) && limits.max_daily_loss > Decimal::ZERO {
            let reason = format!("today's loss reached the limit of {} earlier today: no new position today", limits.max_daily_loss);
            self.log(&format!("WARNING: risk check: {symbol} x{contracts} refused - {reason}"));
            return Err(reason);
        }
        let open_contracts =
            self.trades_stored.iter().filter(|t| t.is_open() && t.symbol == symbol).map(StoredTrade::remaining_qty).fold(0_u32, u32::saturating_add);
        // An open trade without a quote makes today's result unknown; the closed trades alone still count.
        let today_profit = profit_summary(&self.trades_today).map(|profit| profit.total().unwrap_or(profit.closed));
        let orders_last_minute = u32::try_from(self.watch.entries_sent.len()).unwrap_or(u32::MAX);
        let state = EntryState { open_contracts, new_contracts: contracts, today_profit, orders_last_minute };
        match check_entry(state, limits) {
            Ok(()) => Ok(()),
            Err(rejection) => {
                if matches!(rejection, EntryRejection::DailyLossReached { .. }) {
                    self.watch.loss_stop_day = Some(today);
                }
                self.log(&format!("WARNING: risk check: {symbol} x{contracts} refused - {rejection}"));
                Err(rejection.to_string())
            }
        }
    }

    /// Counts an opening order that was sent, for the limit of new orders per minute.
    pub(super) fn count_entry_sent(&mut self) {
        let now = self.clock.instant();
        self.watch.entries_sent.push(now);
    }

    /// Whether an Instant Order for `contract` closes a position instead of opening one: it buys back the short of an open trade of
    /// this app, sells the long of one, or the account holds this very contract the other way round.
    pub(super) fn instant_order_closes(&self, contract: &IbContract, action: &str, account: &str) -> bool {
        if action == "BUY" && super::orders::closes_an_open_short(&self.trades_stored, contract, account) {
            return true;
        }
        if action == "SELL" && super::orders::closes_an_open_long(&self.trades_stored, contract, account) {
            return true;
        }
        let con_id = contract.con_id.to_string();
        contract.con_id != 0
            && self.positions.iter().any(|position| {
                position.account == account
                    && position.contract_id.trim() == con_id
                    && if action == "BUY" { position.quantity < 0.0 } else { position.quantity > 0.0 }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::count_setting;

    #[test]
    fn a_risk_setting_is_a_whole_number_and_anything_else_is_no_limit() {
        assert_eq!(count_setting(" 20 "), 20);
        assert_eq!(count_setting(""), 0);
        assert_eq!(count_setting("-3"), 0);
        assert_eq!(count_setting("abc"), 0);
    }
}
