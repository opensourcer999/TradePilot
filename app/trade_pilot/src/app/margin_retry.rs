//! Settings > Safety > "Retry trade entry on margin rejection": an entry order that TWS refused for margin is sent again under a new
//! order id after a pause, a few times, before the entry is given up. Margin is often freed a moment later, when another trade closes
//! or an order is cancelled.

use super::{App, FillAttempts, Instant, SentTemplateOrder, ToPrimitive};
use std::time::Duration;

/// How often an entry is sent again after a margin rejection.
pub(super) const MARGIN_RETRY_MAX: u32 = 3;
/// The pause before an entry is sent again.
const MARGIN_RETRY_WAIT: Duration = Duration::from_secs(20);

/// Whether an entry refused for margin is sent again: the setting is on, the order was sent right away (one held in TWS for review is
/// never touched), and it has been sent again fewer than `MARGIN_RETRY_MAX` times.
pub(super) fn retries_entry(enabled: bool, transmitted: bool, attempt: u32) -> bool {
    enabled && transmitted && attempt < MARGIN_RETRY_MAX
}

impl App {
    /// TWS refused `order_id` for margin: if it is an entry that is to be retried, its next attempt is planned; the schedule run it
    /// belongs to keeps working meanwhile.
    pub(super) fn plan_margin_retry(&mut self, order_id: i32) {
        let enabled = self.settings.alert_flags.retry_on_margin_rejection;
        let now = self.clock.instant();
        let Some(sent) = self.template.orders.iter_mut().find(|o| o.order_id == order_id && !o.recorded) else { return };
        if !retries_entry(enabled, sent.fill.is_some(), sent.margin_attempt) {
            if enabled {
                self.log(&format!("Entry {order_id} refused for margin: not sent again (attempts used up, or the order was held in TWS)"));
            }
            return;
        }
        sent.retry_due = Some(now + MARGIN_RETRY_WAIT);
        if let Some(fill) = sent.fill.as_mut() {
            fill.finished = true;
        }
        let attempt = sent.margin_attempt + 1;
        self.log(&format!(
            "Entry {order_id} refused for margin: sending it again in {} s (attempt {attempt} of {MARGIN_RETRY_MAX})",
            MARGIN_RETRY_WAIT.as_secs()
        ));
    }

    /// Sends the entries whose retry is due again under a new order id, at the limit the entry started with. An entry that cannot be
    /// sent ends as a refused one would.
    pub(super) fn margin_retry_tick(&mut self, now: Instant) {
        let due: Vec<usize> =
            self.template.orders.iter().enumerate().filter(|(_, o)| o.retry_due.is_some_and(|at| at <= now)).map(|(index, _)| index).collect();
        for index in due {
            self.send_margin_retry(index);
        }
    }

    fn send_margin_retry(&mut self, index: usize) {
        let Some(sent) = self.template.orders.get_mut(index) else { return };
        sent.retry_due = None;
        let (old_id, run_id, attempt) = (sent.order_id, sent.run_id, sent.margin_attempt + 1);
        let Some(fill) = sent.fill.as_ref() else { return };
        let (contract, mut order, limit) = (fill.contract.clone(), fill.order.clone(), sent.entry.limit);
        let (entry, template, account) = (sent.entry.clone(), sent.template.clone(), sent.account.clone());
        let source = fill.template.clone();
        let allowed = self.trading_allowed();
        let (true, Some(order_id), true) = (self.tws_cmd_sender.is_some(), self.next_order_id, allowed) else {
            self.log(&format!("Entry {old_id} NOT sent again after the margin rejection: not connected, or the safety gate is shut"));
            self.schedule_order_ended(run_id, "Cancelled");
            return;
        };
        let Some(wire) = limit.to_f64() else {
            self.log(&format!("Entry {old_id} NOT sent again after the margin rejection: the limit {limit} cannot be sent"));
            self.schedule_order_ended(run_id, "Cancelled");
            return;
        };
        if let Err(reason) = self.check_entry_risk(&entry.symbol, entry.qty) {
            self.log(&format!("Entry {old_id} NOT sent again after the margin rejection: risk limit - {reason}"));
            self.schedule_order_ended(run_id, "Cancelled");
            return;
        }
        self.next_order_id = Some(order_id + 1);
        order.lmt_price = wire;
        order.order_ref = crate::trade_rebuild::order_ref(order_id, "");
        self.log(&format!(
            "Entry {old_id} sent again as order {order_id} after the margin rejection (attempt {attempt} of {MARGIN_RETRY_MAX}), price {}",
            limit.normalize()
        ));
        if let Err(reason) = self.submit_order(order_id, contract.clone(), order.clone()) {
            self.log(&format!("Entry {old_id} NOT sent again: {reason}"));
            self.schedule_order_ended(run_id, "Cancelled");
            return;
        }
        self.count_entry_sent();
        let fill = FillAttempts { template: source, contract, order, limit, attempt: 1, next_tick: None, finished: false, partly_filled: false };
        self.template.orders.push(SentTemplateOrder {
            order_id,
            account,
            entry,
            template,
            recorded: false,
            run_id,
            fill: Some(fill),
            margin_attempt: attempt,
            retry_due: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{retries_entry, MARGIN_RETRY_MAX};

    #[test]
    fn an_entry_is_sent_again_only_when_switched_on_sent_right_away_and_not_used_up() {
        assert!(retries_entry(true, true, 0));
        assert!(retries_entry(true, true, MARGIN_RETRY_MAX - 1));
        assert!(!retries_entry(true, true, MARGIN_RETRY_MAX), "used up");
        assert!(!retries_entry(false, true, 0), "switched off: the entry is given up");
        assert!(!retries_entry(true, false, 0), "held in TWS for review");
    }
}
