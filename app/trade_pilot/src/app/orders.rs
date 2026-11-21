//! The order life cycle of `App`: every order goes to TWS through `submit_order` (safety gate,
//! risk check, checked hand-over), the template order and its fill attempts, the exit orders
//! placed at a fill (stop, stop-limit, profit-target levels), the stop adjustments and their
//! replacement stops, the order status TWS reports, closing and finishing a trade, the kill
//! switch and the reconciliation after a connect. The GUI pages stay in `app.rs`.

use super::{
    append_trade, chain_quote, check_order_quantity, dec, display_currency, fill_attempts, fill_wait, mask_account, order_is_final, profit_loss, rel_settings,
    stored_custom_legs, to_stored_leg, trade_entry, trade_exit, trades_path, update_trade, App, ComboLegSpec, Decimal, ExitLevels, FillAttempts, IbContract,
    IbOrder, Instant, OrderBlocked, PendingTemplateOrder, SentTemplateOrder, StopAdjustment, StopAdjustments, StoredTrade, ToPrimitive, TradeStatus,
    TradeTemplate, TwsCommand,
};
use crate::reconcile::{reconcile_trade, Reconciliation};
use crate::trade_mail;

impl App {
    /// The one way an order reaches TWS (`coding_guideline.md` section 8): the safety gate, the
    /// pre-trade risk check on its quantity, then the hand-over to the connection thread - whose
    /// result is checked, so an order that never left is never taken for one that is working.
    /// Every refusal is logged here; the caller records the order (its id on the trade, a stop
    /// price ...) only on `Ok`.
    pub(super) fn submit_order(&mut self, order_id: i32, contract: IbContract, order: IbOrder) -> Result<(), OrderBlocked> {
        let handed_over = self.settings.view_flags.settings_logging_debug.then(|| describe_order(order_id, &contract, &order));
        let result = self.try_submit_order(order_id, contract, order);
        match (&result, handed_over) {
            (Err(reason), _) => self.log(&format!("WARNING: order {order_id} NOT sent: {reason}")),
            (Ok(()), Some(text)) => self.debug(&format!("handed to TWS: {text}")),
            (Ok(()), None) => {}
        }
        result
    }

    pub(super) fn try_submit_order(&self, order_id: i32, contract: IbContract, order: IbOrder) -> Result<(), OrderBlocked> {
        if !self.trading_allowed() {
            return Err(OrderBlocked::SafetyGate);
        }
        let sender = self.tws_cmd_sender.as_ref().ok_or(OrderBlocked::NotConnected)?;
        let quantity = core_types::Quantity::from_f64(order.total_quantity).map_err(|error| OrderBlocked::Risk(error.to_string()))?;
        check_order_quantity(quantity, &self.order_risk_limits()).map_err(|error| OrderBlocked::Risk(error.to_string()))?;
        sender.send(TwsCommand::PlaceOrder { order_id, contract, order: Box::new(order) }).map_err(|_| OrderBlocked::ConnectionLost)
    }

    /// The kill switch (`coding_guideline.md` section 8, MUSS): blocks every new order (see
    /// `trading_allowed`) and cancels every order of this app that may still be working - the
    /// stops, profit targets, levels and close orders of the open trades, the template orders not
    /// filled yet (their fill attempts end) and the last Instant Order. The stops go too: the
    /// switch stops all trading, and the log says which trades are left without a stop. Orders TWS
    /// already finished are cancelled harmlessly (TWS rejects the cancel).
    pub(super) fn trip_kill_switch(&mut self) {
        self.kill_switch.trip();
        let mut ids: Vec<i32> = Vec::new();
        let mut unprotected = Vec::new();
        for trade in self.trades_stored.iter().filter(|t| t.is_open()) {
            let exits = trade.exit_order_ids();
            if trade.stop_order_id.is_some() || trade.stop_limit_order_id.is_some() {
                unprotected.push(trade.label());
            }
            ids.extend(exits);
        }
        for sent in self.template.orders.iter_mut().filter(|o| !o.recorded) {
            if let Some(fill) = sent.fill.as_mut() {
                fill.finished = true;
            }
            ids.push(sent.order_id);
        }
        if let (Some(id), false) = (self.instant.last_order_id, self.instant.last_order_status.as_deref().is_some_and(order_is_final)) {
            ids.push(id);
        }
        ids.sort_unstable();
        ids.dedup();
        self.log(&format!("KILL SWITCH on: new orders are blocked; cancelling {} order(s) of this app: {ids:?}", ids.len()));
        let mut failed = Vec::new();
        for id in &ids {
            if self.submit_cancel(*id).is_err() {
                failed.push(*id);
            }
        }
        if !unprotected.is_empty() {
            self.log(&format!("WARNING: KILL SWITCH: these open trades lose their stop: {}", unprotected.join("; ")));
        }
        if !failed.is_empty() {
            self.log(&format!("WARNING: KILL SWITCH: the cancel of {failed:?} could not be sent - cancel them in TWS yourself"));
        }
        self.instant.template_status = Some(format!("KILL SWITCH on: {} order(s) cancelled, new orders blocked", ids.len().saturating_sub(failed.len())));
    }

    /// Hands a cancel to the connection thread. Cancelling only reduces risk, so it needs no safety
    /// gate; a cancel that cannot be handed over is logged - the order is then still working in TWS.
    pub(super) fn submit_cancel(&mut self, order_id: i32) -> Result<(), OrderBlocked> {
        let result = match self.tws_cmd_sender.as_ref() {
            None => Err(OrderBlocked::NotConnected),
            Some(sender) => sender.send(TwsCommand::CancelOrder { order_id }).map_err(|_| OrderBlocked::ConnectionLost),
        };
        if let Err(reason) = &result {
            self.log(&format!("WARNING: cancel of order {order_id} NOT sent: {reason} - the order may still be working in TWS"));
        } else {
            self.debug(&format!("cancel of order {order_id} handed to TWS ({})", self.order_role(order_id)));
        }
        result
    }

    /// What an order id is to this app, for the Debug log: an opening order of a template trade, the stop, profit target
    /// or close order of an open trade, an Instant Order, or none of ours (another client, or placed in TWS by hand).
    pub(super) fn order_role(&self, order_id: i32) -> String {
        if let Some(sent) = self.template.orders.iter().find(|o| o.order_id == order_id) {
            return format!("opening order of a template trade{}", sent.run_id.map_or_else(String::new, |run| format!(", schedule run {run}")));
        }
        if let Some(trade) = self.trades_stored.iter().find(|t| t.exit_order_ids().contains(&order_id) || t.close_order_id == Some(order_id)) {
            let kind = match order_id {
                id if trade.stop_order_id == Some(id) => "stop",
                id if trade.stop_limit_order_id == Some(id) => "stop-limit",
                id if trade.profit_order_id == Some(id) => "profit target",
                id if trade.close_order_id == Some(id) => "close order",
                _ => "exit order",
            };
            return format!("{kind} of {} {} {}", trade.symbol, trade.trade_type, trade.strikes);
        }
        if self.instant.last_order_id == Some(order_id) {
            return "Instant Order".to_string();
        }
        "not an order of this app".to_string()
    }

    /// "Send now": re-checks the safety gate and sends the opening order (`placeOrder`).
    pub(super) fn confirm_template_order(&mut self) {
        let Some(pending) = self.template.pending.take() else {
            return;
        };
        if !self.trading_allowed() {
            self.block_template_order("paper-account safety gate not satisfied");
            return;
        }
        let (true, Some(order_id)) = (self.tws_cmd_sender.is_some(), self.next_order_id) else {
            self.block_template_order("not connected to TWS");
            return;
        };
        let PendingTemplateOrder { source, entry, contract, mut order, run_id, .. } = pending;
        // The id is used up even if the order cannot be sent: an id is never given twice.
        self.next_order_id = Some(order_id + 1);
        order.order_ref = crate::trade_rebuild::order_ref(order_id, "");
        // Re-pricing only for an order that goes out right away; see `SentTemplateOrder::fill`.
        let fill = order.handling.transmit.then(|| FillAttempts {
            template: source.clone(),
            contract: contract.clone(),
            order: order.clone(),
            limit: entry.limit,
            attempt: 1,
            next_tick: None,
            finished: false,
        });
        for line in &entry.log {
            self.log(&format!("Template trade: {line}"));
        }
        if let Err(reason) = self.submit_order(order_id, contract, order.clone()) {
            self.block_template_order(&reason.to_string());
            return;
        }
        self.log(&format!(
            "Open {} OrderID:{order_id} Price: {} Qty: {} Account: {}",
            entry.trades.first().map_or("", |t| t.trade_type.as_str()),
            entry.limit.normalize(),
            entry.qty,
            mask_account(&format!("IB:{}", order.account)),
        ));
        self.instant.template_status = Some(format!("#{order_id}: {}", self.tr("sent to TWS", "an TWS gesendet")));
        self.template.orders.push(SentTemplateOrder { order_id, account: order.account.clone(), entry, template: source, recorded: false, run_id, fill });
    }

    /// Fill attempts: an order that is still unfilled
    /// when its timer is due is moved towards a fill; once the attempts are used up it is cancelled.
    pub(super) fn process_fill_attempts(&mut self, now: Instant) {
        let due: Vec<usize> = self
            .template
            .orders
            .iter()
            .enumerate()
            .filter(|(_, o)| o.fill.as_ref().is_some_and(|f| !f.finished && f.next_tick.is_some_and(|t| t <= now)))
            .map(|(index, _)| index)
            .collect();
        for index in due {
            self.fill_tick(index, now);
        }
    }

    pub(super) fn fill_tick(&mut self, index: usize, now: Instant) {
        // Not connected: the tick waits for the connection (a reconnect brings the status back).
        if self.tws_cmd_sender.is_none() {
            return;
        }
        let allowed = self.trading_allowed();
        // What goes to TWS is decided first and sent after (`submit_order` needs `self`).
        let mut command: Option<TwsCommand> = None;
        let Some(sent) = self.template.orders.get_mut(index) else {
            return;
        };
        let order_id = sent.order_id;
        let legs = sent.entry.legs.len().max(1);
        let symbol = sent.entry.symbol.clone();
        let Some(fill) = sent.fill.as_mut() else {
            return;
        };
        let attempts = fill_attempts(&fill.template);
        let mut messages = Vec::new();
        let mut status;
        if !allowed {
            // The gate holds for every change of an order, not only for the first one: with
            // the kill switch on (or the mode switched) the order is cancelled, never moved.
            fill.finished = true;
            command = Some(TwsCommand::CancelOrder { order_id });
            messages.push(format!("Fill attempts for order {order_id} stopped: paper-account safety gate not satisfied - order cancelled"));
            status = format!("#{order_id}: cancelled (safety gate)");
        } else if fill.attempt < attempts {
            let next = trade_entry::next_fill_limit(&fill.template, &symbol, legs, &fill.order.action, fill.limit).unwrap_or(fill.limit);
            if let Some(wire) = next.to_f64() {
                fill.limit = next;
                fill.order.lmt_price = wire;
                fill.attempt += 1;
                fill.next_tick = Some(now + fill_wait(&fill.template));
                command = Some(TwsCommand::PlaceOrder { order_id, contract: fill.contract.clone(), order: Box::new(fill.order.clone()) });
                messages.push(format!("Adjust {order_id} price to {} (Qty: {})", next.normalize(), fill.order.total_quantity));
                status = format!("#{order_id}: price {} (attempt {}/{attempts})", next.normalize(), fill.attempt);
            } else {
                fill.finished = true;
                messages.push(format!("Fill attempts for order {order_id} stopped: the price {next} cannot be sent"));
                status = format!("#{order_id}: fill attempts stopped");
            }
        } else {
            fill.finished = true;
            command = Some(TwsCommand::CancelOrder { order_id });
            messages.push(format!("Cancel {order_id} after all ({attempts}) fill attempts"));
            status = format!("#{order_id}: not filled - cancelled after {attempts} attempts");
        }
        for message in messages {
            self.log(&message);
        }
        let failed = match command {
            Some(TwsCommand::PlaceOrder { order_id: id, contract, order }) => self.submit_order(id, contract, *order).is_err(),
            Some(TwsCommand::CancelOrder { order_id: id }) => self.submit_cancel(id).is_err(),
            _ => false,
        };
        if failed {
            // The order keeps its last price in TWS; it is not moved any further.
            if let Some(fill) = self.template.orders.get_mut(index).and_then(|sent| sent.fill.as_mut()) {
                fill.finished = true;
            }
            status = format!("#{order_id}: fill attempts stopped - the order could not be sent (see the log)");
        }
        self.instant.template_status = Some(status);
    }

    /// TWS's `orderStatus` for a template order: once fully filled, its trades are recorded
    /// (today's trades on the Dashboard, `trades.json`). Partial fills wait for the rest.
    pub(super) fn on_template_order_status(&mut self, order_id: i32, status: &str, remaining: f64, avg_fill_price: f64) {
        let Some(sent) = self.template.orders.iter_mut().find(|o| o.order_id == order_id) else {
            return;
        };
        let run_id = sent.run_id;
        if let Some(fill) = sent.fill.as_mut() {
            if order_is_final(status) {
                fill.finished = true;
            } else if fill.next_tick.is_none() && !fill.finished {
                // The order is active in TWS: the fill timer starts.
                fill.next_tick = Some(self.clock.instant() + fill_wait(&fill.template));
            }
        }
        let complete = status == "Filled" && remaining == 0.0 && !sent.recorded;
        let avg = dec(avg_fill_price);
        if complete && avg.is_some() {
            sent.recorded = true;
        }
        self.instant.template_status = Some(format!("#{order_id}: {status}"));
        self.debug(&format!("template order {order_id}: {status}, remaining {remaining}, average price {avg_fill_price} - {}", fill_outcome(complete, status)));
        if !complete && order_is_final(status) {
            self.schedule_order_ended(run_id, status);
        }
        if !complete {
            return;
        }
        let Some(avg) = avg else {
            self.log(&format!("Template order {order_id} filled, but the fill price {avg_fill_price} is unusable - trade not recorded"));
            return;
        };
        self.schedule_order_filled(run_id);
        let Some(sent) = self.template.orders.iter().find(|o| o.order_id == order_id) else {
            return;
        };
        let now = self.clock.local();
        let price_open = sent.entry.price_open(avg);
        let origin = self.entry_origin(run_id, order_id, &sent.template);
        let mut stored = Vec::new();
        for planned in &sent.entry.trades {
            let Some(price) = share_of_price(price_open, planned.price_share) else { continue };
            stored.push(StoredTrade {
                day: now.format("%Y-%m-%d").to_string(),
                opened: now.format("%d-%m-%Y %H:%M:%S").to_string(),
                account: sent.account.clone(),
                symbol: sent.entry.symbol.clone(),
                trade_type: planned.trade_type.clone(),
                strikes: planned.strikes.clone(),
                order_id,
                qty: planned.qty,
                multiplier: 100,
                price_open: price.normalize(),
                delta: planned.delta,
                short_put: to_stored_leg(planned.legs.short_put.as_ref()),
                long_put: to_stored_leg(planned.legs.long_put.as_ref()),
                short_call: to_stored_leg(planned.legs.short_call.as_ref()),
                long_call: to_stored_leg(planned.legs.long_call.as_ref()),
                put_ratio: planned.legs.put_ratio,
                call_ratio: planned.legs.call_ratio,
                expiry: planned.legs.expiry.clone(),
                status: TradeStatus::Open.as_str().to_string(),
                price_close: None,
                stop_order_id: None,
                profit_order_id: None,
                stop_limit_order_id: None,
                exit_levels: None,
                adjustments: None,
                oca_group: None,
                close_order_id: None,
                stop_price: None,
                profit_price: None,
                entry_unknown: false,
                strategy: sent.template.strategy.clone(),
                timed_exit: trade_exit::timed_exit_of(&sent.template, &planned.legs.expiry, self.clock.new_york()),
                profit_cancel: trade_exit::profit_cancel_of(&sent.template, &planned.legs.expiry, self.clock.new_york()),
                origin: Some(crate::storage::TradeOrigin { ratio_open: self.entry_ratio(planned), ..origin.clone() }),
                custom_legs: stored_custom_legs(planned),
            });
        }
        let template = sent.template.clone();
        let path = trades_path(self.link.trading_live);
        let mut opened = Vec::new();
        for trade in stored {
            self.log(&format!(
                "Trade opened: {} {} {} x{} @ {} (order {order_id})",
                trade.symbol, trade.trade_type, trade.strikes, trade.qty, trade.price_open
            ));
            if !cfg!(test) {
                if let Err(error) = path.as_ref().map_err(std::string::ToString::to_string).and_then(|p| append_trade(p, &trade).map_err(|e| e.to_string())) {
                    self.log(&format!("Could not save the trade: {error}"));
                }
            }
            self.trades_stored.push(trade);
            self.pending.trade_log_dirty = true;
            opened.push(self.trades_stored.len() - 1);
        }
        self.refresh_trades(self.clock.instant(), true);
        let opened = self.mark_long_reuse_trades(opened, &template);
        // The fill is the moment the trade needs its stop and profit target.
        for &index in &opened {
            self.place_exit_orders(index, &template);
        }
        self.notify_trades_opened(&opened);
    }

    /// `Long Reuse Only` of a long trade (`LongCall`, `LongPut`, `LongStrangle`): the trade is booked closed at once at 0, the
    /// long option stays in the account for other trades to reuse, and it gets no stop or profit target. Returns the trades
    /// of `opened` that go on as usual.
    pub(super) fn mark_long_reuse_trades(&mut self, opened: Vec<usize>, template: &TradeTemplate) -> Vec<usize> {
        if !template.long_flags.long_reuse_only {
            return opened;
        }
        let (reused, kept): (Vec<usize>, Vec<usize>) = opened
            .into_iter()
            .partition(|index| self.trades_stored.get(*index).is_some_and(|t| matches!(t.trade_type.as_str(), "LongCall" | "LongPut" | "LongStrangle")));
        for index in reused {
            if let Some(trade) = self.trades_stored.get(index) {
                self.log(&format!("Long Reuse Only - marked trade closed: {} {} {}", trade.symbol, trade.trade_type, trade.strikes));
            }
            self.finish_trade(index, TradeStatus::ClosedOther, Decimal::ZERO, "Long Reuse Only");
        }
        kept
    }

    /// The "Entry filled" notification for the trades at `indices` of `trades_stored`, sent once their exit orders
    /// are placed so the mail can name the stop target.
    fn notify_trades_opened(&self, indices: &[usize]) {
        let opened: Vec<&StoredTrade> = indices.iter().filter_map(|index| self.trades_stored.get(*index)).collect();
        if opened.is_empty() {
            return;
        }
        let today: Vec<&StoredTrade> = self.trades_stored.iter().collect();
        let mail = trade_mail::trade_opened(&opened, &today, &mask_account);
        self.notify_trade_mail(&mail, self.settings.notify_flags.notify_trade_opened_email, self.settings.notify_flags.notify_trade_opened_webhook);
    }

    /// The "Stop filled" or "Trade closed" notification for the finished trade at `index` (a trade marked closed by
    /// hand is not reported).
    fn notify_trade_finished(&self, index: usize, status: TradeStatus) {
        let Some(trade) = self.trades_stored.get(index) else {
            return;
        };
        let today: Vec<&StoredTrade> = self.trades_stored.iter().collect();
        let mail = match status {
            TradeStatus::Stopped => trade_mail::trade_stopped(trade, &today, &mask_account),
            TradeStatus::Closed => trade_mail::trade_closed(trade, &today, &mask_account),
            _ => return,
        };
        self.notify_trade_mail(&mail, self.settings.notify_flags.notify_trade_stopped_email, self.settings.mail_flags.notify_trade_stopped_webhook);
        if status == TradeStatus::Stopped {
            self.notify_high_slippage(index);
        }
    }

    /// "ALERT: Stop filled with high slippage" when the stop of the trade at `index` filled beyond the limit of Settings > Trading (% of
    /// the stop; 0 or empty = off).
    fn notify_high_slippage(&self, index: usize) {
        let Some(trade) = self.trades_stored.get(index) else {
            return;
        };
        let limit: Decimal = self.settings.high_slippage_pct.trim().replace(',', ".").parse().unwrap_or(Decimal::ZERO);
        let Some(slippage) = trade_mail::slippage_percent(trade) else {
            return;
        };
        if !self.settings.alert_flags.high_slippage_notifications_enabled || limit <= Decimal::ZERO || slippage < limit {
            return;
        }
        let today: Vec<&StoredTrade> = self.trades_stored.iter().collect();
        self.notify_trade_mail(&trade_mail::high_slippage(trade, slippage, &today, &mask_account), true, true);
    }

    /// "ALERT: Trade without a stop" for the trades named in `problems`, if Settings > Trading asks for it.
    pub(super) fn notify_missing_stop(&self, problems: &[String]) {
        if self.settings.alert_flags.missing_stop_notifications_enabled {
            self.notify_trade_mail(&trade_mail::missing_stop(problems), true, true);
        }
    }

    /// Places the exit orders of a trade that was just filled: a stop and a profit
    /// target as closing orders, linked in one OCA group. Placed only while the safety gate holds;
    /// whatever cannot be placed is logged, so a missing stop is never silent.
    pub(super) fn place_exit_orders(&mut self, index: usize, template: &TradeTemplate) {
        let Some(trade) = self.trades_stored.get(index).cloned() else {
            return;
        };
        self.place_exit_orders_for(index, &trade, template);
    }

    /// The stop and profit target of `trade`. A custom trade is stopped and targeted as a whole: both orders close every leg, at prices
    /// worked out from what the whole combo was entered at.
    fn place_exit_orders_for(&mut self, index: usize, trade: &StoredTrade, template: &TradeTemplate) {
        let trade = trade.clone();
        let label = trade.label();
        let price_open = trade.price_open;
        if !self.trading_allowed() {
            self.log(&format!("Stop/Profit {label}: paper-account safety gate not satisfied - NO stop or profit target placed"));
            return;
        }
        let (true, Some(first_id)) = (self.tws_cmd_sender.is_some(), self.next_order_id) else {
            self.log(&format!("Stop/Profit {label}: not connected to TWS - NO stop or profit target placed"));
            return;
        };
        // A long without a bid cannot be sold: such a stop is converted to a short one.
        let chains = &self.chains;
        let has_bid = |con_id: i32| chain_quote(chains, con_id).is_none_or(|q| q.bid != Decimal::NEGATIVE_ONE || q.ask.is_none());
        let legs = trade_exit::exit_legs(&trade, template, &has_bid);
        if let Some(note) = trade_exit::partial_debit_exit(&trade, &legs) {
            self.log(&format!("WARNING: Stop/Profit {label}: {note} - NO stop or profit target placed"));
            return;
        }
        // The stop is worked out from the entry price, or - as the template says - from the average
        // of the two sides, or from what the sold or the bought legs filled at.
        let siblings: Vec<&StoredTrade> =
            self.trades_stored.iter().filter(|t| t.order_id == trade.order_id && t.account == trade.account && !t.entry_unknown).collect();
        let (basis, basis_note) =
            trade_exit::stop_basis(&template.stop_basis, price_open, trade_exit::leg_prices(&trade, &self.exec_history), trade_exit::average_entry(&siblings));
        if let Some(note) = basis_note {
            self.log(&format!("Stop/Profit {label}: {note}"));
        }
        let prices = trade_exit::exit_prices_with_basis(template, &trade.symbol, legs.len(), price_open, basis);
        for note in &prices.notes {
            self.log(&format!("Stop/Profit {label}: {note}"));
        }
        self.debug(&format!("exit prices of {label}: {} (entry {price_open}, basis {basis}, {} leg(s) to close)", prices.describe(), legs.len()));
        if prices.market.is_none() && prices.limit.is_none() && prices.trail.is_none() && prices.profit.is_none() {
            return;
        }
        // The three orders take consecutive ids: stop, profit target, stop-limit.
        let (stop_id, profit_id, limit_id) = (first_id, first_id.saturating_add(1), first_id.saturating_add(2));
        // The profit target may be split into levels that close part of the trade each; the first
        // level's order takes its share, the later ones follow when the one before is hit.
        let split = trade_exit::split_profit_qty(trade.qty, template);
        let profit_qty = if split[0] == 0 { trade.qty } else { split[0] };
        let Some(mut orders) =
            trade_exit::exit_orders(&trade, template, &prices, &legs, &self.clock.new_york_today(), stop_id, profit_id, limit_id, profit_qty)
        else {
            self.log(&format!("Stop/Profit {label}: the closing order cannot be built - NO stop or profit target placed"));
            return;
        };
        // A REL stop waits for its conditions; without them it would be active at once, so it is
        // then not placed at all.
        let rel = prices.rel.map(|_| rel_settings(template, &trade, self.chains.price(&trade.symbol).and_then(dec), self.clock.new_york()));
        if let (Some(rel), Some(order), Some(leg), Some(price)) = (&rel, orders.stop.as_mut(), legs.first(), prices.market) {
            if !trade_exit::attach_rel_conditions(order, &trade, leg.con_id, price, rel, template.stop_trigger) {
                orders.stop = None;
                self.log(&format!("Stop/Profit {label}: the conditions of the REL stop cannot be built (underlying, strike or date missing) - NO stop placed"));
            }
        }
        self.next_order_id = Some(first_id.saturating_add(3));
        let mut updated = trade.clone();
        // The group is remembered: once a stop is replaced, its order ids no longer tell it.
        updated.oca_group = Some(trade_exit::oca_group(stop_id));
        // Each order is recorded on the trade only once it was really handed to TWS.
        if let Some(order) = orders.stop {
            self.log(&format!("Place STP ClientOrderID: {stop_id} Price: {} Qty: {}", order.aux_price, order.total_quantity));
            if self.submit_order(stop_id, orders.contract.clone(), order).is_ok() {
                updated.stop_order_id = Some(stop_id);
            }
        }
        if let Some(order) = orders.stop_limit {
            self.log(&format!(
                "Place STP LMT ClientOrderID: {limit_id} Stop Price: {} Limit Price: {} Qty: {}",
                order.aux_price, order.lmt_price, order.total_quantity
            ));
            if self.submit_order(limit_id, orders.contract.clone(), order).is_ok() {
                updated.stop_limit_order_id = Some(limit_id);
            }
        }
        if updated.stop_order_id.is_some() || updated.stop_limit_order_id.is_some() {
            updated.stop_price = prices.stop.map(|p| p.normalize());
        }
        let profit_placed = match orders.profit {
            Some(order) => {
                self.log(&format!("Place Profit Limit ClientOrderID: {profit_id} Profit Limit Price: {} Qty: {}", order.lmt_price, order.total_quantity));
                self.submit_order(profit_id, orders.contract, order).is_ok()
            }
            None => false,
        };
        if profit_placed {
            updated.profit_order_id = Some(profit_id);
            updated.profit_price = prices.profit.map(|p| p.normalize());
            if split.iter().skip(1).any(|qty| *qty > 0) {
                self.log(&format!("Stop/Profit {label}: profit target levels {split:?} contracts, the next one is placed when the one before is hit"));
                updated.exit_levels = Some(profit_levels(template, &trade, &legs, split, profit_id));
            }
        }
        // The template's stop adjustments watch the trade from now on.
        let (notes, adjustments) = stop_adjustments_of(template, prices.stop.map(|p| p.normalize().to_string()), &legs, rel, basis);
        for note in notes {
            self.log(&format!("Stop/Profit {label}: {note}"));
        }
        if updated.stop_order_id.is_some() || updated.stop_limit_order_id.is_some() {
            updated.adjustments = adjustments;
        }
        let stop_wanted = prices.market.is_some() || prices.limit.is_some() || prices.trail.is_some();
        if stop_wanted && updated.stop_order_id.is_none() && updated.stop_limit_order_id.is_none() {
            self.notify_missing_stop(&[format!("{label}: no stop could be placed")]);
        }
        self.save_trade(index, updated);
    }

    /// "Delete stop" / "Delete target": cancels the working stop (and stop-limit) or profit-target orders (every level) of the open trade at
    /// `index`. The trade is left without them; TWS confirms each cancel and the trade forgets the order then.
    pub(super) fn delete_exit_orders(&mut self, index: usize, stop: bool) {
        let Some(trade) = self.trades_stored.get(index).filter(|t| t.is_open()).cloned() else { return };
        let label = trade.label();
        let mut ids: Vec<i32> =
            if stop { [trade.stop_order_id, trade.stop_limit_order_id].into_iter().flatten().collect() } else { super::attach::working_target_ids(&trade) };
        ids.sort_unstable();
        ids.dedup();
        let what = if stop { "stop" } else { "profit target" };
        if ids.is_empty() {
            self.log(&format!("Delete {what} of {label}: the trade has none"));
            return;
        }
        self.log(&format!("Delete {what} of {label}: cancelling the orders {ids:?}"));
        for id in ids {
            if self.submit_cancel(id).is_ok() {
                self.watch.user_cancelled.insert(id);
            }
        }
    }

    /// The stop adjustments: for every open trade whose next adjustment is hit
    /// (its trigger reached, or its profit-target level hit, and its time of day passed) the stop
    /// moves: the old stop is cancelled and, once TWS has confirmed that, the new one is placed
    /// (`place_replacement_stop`). `now` is New York time. Nothing is touched while the safety
    /// gate is shut: a cancelled stop could not be replaced.
    pub(super) fn check_stop_adjustments(&mut self, now: chrono::NaiveDateTime) {
        if !self.trades_stored.iter().any(|t| t.is_open() && t.adjustments.is_some()) || !self.trading_allowed() {
            return;
        }
        if self.tws_cmd_sender.is_none() {
            return;
        }
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index).cloned() else { continue };
            let Some(adjustments) = trade.adjustments.clone() else { continue };
            let Some(item) = adjustments.items.get(usize::from(adjustments.level)).cloned() else { continue };
            if !trade.is_open() || trade.close_order_id.is_some() || adjustments.replace_pending {
                continue;
            }
            let price_open = trade.price_open;
            // A value of the trade now: what it would bring, and the cost to close (not 0).
            let Some((bid, ask)) = self.adjustment_values(&trade) else { continue };
            let profit_level = trade.exit_levels.as_ref().map_or(0, |levels| levels.level);
            if ask == Decimal::ZERO || !trade_exit::adjustment_hit(&item, price_open, bid, profit_level) {
                continue;
            }
            // Only from its time of day on the day of the expiration (hour 0 = at any time).
            if item.hour != 0 {
                let from = chrono::NaiveDate::parse_from_str(&trade.expiry, "%Y%m%d")
                    .ok()
                    .and_then(|date| date.and_hms_opt(u32::try_from(item.hour).unwrap_or(0), u32::try_from(item.minute).unwrap_or(0), 0));
                if from.is_none_or(|from| now <= from) {
                    continue;
                }
            }
            let label = trade.label();
            let number = usize::from(adjustments.level) + 1;
            let new_name = if matches!(item.order_type.as_str(), "" | "Same") { adjustments.order_type.as_str() } else { item.order_type.as_str() };
            // A stop order type this app does not place is never taken for another one: the
            // adjustment is not run (and was reported when the trade's stop was placed).
            let Some(new_type) = trade_exit::StopOrderType::parse(new_name) else { continue };
            // An adjustment by a multiple starts from the price the stop was based on.
            let basis = adjustments.basis.as_deref().and_then(|text| text.parse::<Decimal>().ok()).unwrap_or(price_open);
            let new_stop = trade_exit::adjustment_new_stop(&item, &adjustments.order_type, basis, &trade.symbol, adjustments.legs.len());
            // Without a new price only a trailing stop (which keeps its price) or no stop at all is fine.
            if new_stop.is_none() && !matches!(new_type, trade_exit::StopOrderType::Trailing | trade_exit::StopOrderType::None) {
                continue;
            }
            let mut next = adjustments.clone();
            next.level = adjustments.level.saturating_add(1);
            next.order_type = new_type.as_str().to_string();
            if new_type == trade_exit::StopOrderType::Trailing && item.change_offset.is_some() {
                next.trailing_offset = item.change_offset.map(|offset| offset.to_string());
            } else {
                next.stop = new_stop.map(|price| price.normalize().to_string()).or(next.stop);
                if new_type == trade_exit::StopOrderType::StopLimit && item.change_offset.is_some() {
                    next.limit_offset = item.change_offset;
                }
            }
            self.log(&format!(
                "Stop Adjustment {number} of {label} hit (current value {}): the stop becomes {} at {}",
                bid.normalize(),
                new_type.as_str(),
                next.stop.as_deref().unwrap_or("-")
            ));
            let mut updated = trade.clone();
            let mut cancelled = true;
            for order_id in [trade.stop_order_id, trade.stop_limit_order_id].into_iter().flatten() {
                self.log(&format!("Cancel {order_id} (stop adjustment)"));
                cancelled &= self.submit_cancel(order_id).is_ok();
            }
            if !cancelled {
                // The old stop is still working: the adjustment is left as it was and tried again.
                self.log(&format!("Stop Adjustment {number} of {label}: the old stop could not be cancelled - the stop stays as it is"));
                continue;
            }
            if new_type == trade_exit::StopOrderType::None {
                // No stop any more: the cancelled orders are forgotten at once.
                updated.stop_order_id = None;
                updated.stop_limit_order_id = None;
                updated.stop_price = None;
                next.replace_pending = false;
            } else {
                next.replace_pending = true;
                updated.stop_price = next.stop.as_deref().and_then(|p| p.parse::<Decimal>().ok());
            }
            updated.adjustments = Some(next);
            self.save_trade(index, updated);
        }
    }

    /// Asks TWS which orders of this client are still working (`reqOpenOrders`); the answer ends
    /// in `reconcile_open_orders`. Sent once per session, as soon as both the trades (after the
    /// executions) and the account (for the safety gate) are known.
    pub(super) fn request_open_orders(&mut self) {
        if self.pending.open_orders_requested || !self.loaded.today_trades_loaded || !self.loaded.account_summary_loaded {
            return;
        }
        let Some(sender) = self.tws_cmd_sender.as_ref() else { return };
        self.pending.open_orders_requested = true;
        if sender.send(TwsCommand::RequestOpenOrders).is_ok() {
            self.open_orders_snapshot = Some(Vec::new());
        } else {
            self.open_orders_snapshot = None;
            self.log("WARNING: the working orders could not be requested from TWS - the trades' stops are not checked");
        }
    }

    /// The reconciliation after a (re)connect (`coding_guideline.md` section 8): compares the exit
    /// orders of every open trade with the orders TWS reports as still `working`.
    ///
    /// - A stop adjustment that was waiting for TWS to confirm the cancel of the old stop
    ///   (`replace_pending`, e.g. `TradePilot` was closed in between) goes on: if the old stop is no
    ///   longer working and did not fill, the adjusted stop is placed now; if it is still working,
    ///   its cancel is sent again; if it filled, nothing is placed and the log says so.
    /// - Any other exit order that is no longer working is reported, with whether executions show
    ///   it filled. Nothing is removed from the trade: an order can also be missing because it was
    ///   placed under another client id, and TWS alone cannot tell which.
    pub(super) fn reconcile_open_orders(&mut self, working: &[i32]) {
        let orders_allowed = self.trading_allowed();
        // Evidence that TWS reports the orders of the client id that placed them: an order of this app is among those working.
        let own_orders_working =
            working.iter().any(|id| self.trades_stored.iter().any(|t| t.order_id == *id || t.close_order_id == Some(*id) || t.exit_order_ids().contains(id)));
        let mut problems = 0_usize;
        for index in 0..self.trades_stored.len() {
            let Some(trade) = self.trades_stored.get(index).cloned() else { continue };
            let label = trade.label();
            // What to do is decided by `reconcile::reconcile_trade`; this carries it out and says so.
            let outcome = reconcile_trade(&trade, working, &self.exec_history, orders_allowed);
            self.debug(&format!("reconcile {label}: exit orders {:?} against the working orders {working:?} -> {outcome:?}", trade.exit_order_ids()));
            match outcome {
                Reconciliation::Working => {}
                Reconciliation::AdjustmentStopFilled { order_id } => {
                    problems += 1;
                    self.log(&format!(
                        "WARNING: {label}: the old stop {order_id} filled while its adjustment waited - no adjusted stop placed; use Mark closed"
                    ));
                }
                Reconciliation::AdjustmentBlocked => {
                    // Placing it now would fail and forget it: it keeps waiting for the next session.
                    problems += 1;
                    self.log(&format!(
                        "WARNING: {label}: the old stop is gone, but the safety gate is shut - the adjusted stop is NOT placed yet, the trade has no stop"
                    ));
                }
                Reconciliation::PlaceAdjustedStop => {
                    self.log(&format!("{label}: the old stop is gone in TWS - placing the adjusted stop that was waiting"));
                    self.place_replacement_stop(index);
                }
                Reconciliation::CancelAgain { order_ids } => {
                    self.log(&format!("{label}: the old stop {order_ids:?} is still working - sending its cancel again for the adjustment"));
                    for id in order_ids {
                        let _logged = self.submit_cancel(id);
                    }
                }
                Reconciliation::Missing { filled, missing, stop_gone } => {
                    problems += 1;
                    if stop_gone {
                        self.notify_missing_stop(&[format!("{label}: the stop is no longer working in TWS - the trade may have NO stop")]);
                    }
                    if !filled.is_empty() {
                        self.log(&format!(
                            "WARNING: {label}: order(s) {filled:?} filled while TradePilot was not running - the trade is closed (at least in part): use Mark closed"
                        ));
                    }
                    if !missing.is_empty() {
                        self.log(&format!(
                            "WARNING: {label}: order(s) {missing:?} are no longer working in TWS (cancelled or expired, or placed under another client id){} - check the trade in TWS",
                            if stop_gone { ": the trade may have NO stop" } else { "" }
                        ));
                        // TWS lists other orders of this app as working, so it reports this client's orders: the missing
                        // ones are gone, and the trade stops showing them ("Stop / target" can attach new ones).
                        if own_orders_working {
                            for id in &missing {
                                self.forget_ended_exit_order(index, *id);
                            }
                        }
                    }
                }
            }
        }
        let summary = if problems == 0 {
            "Reconciliation with TWS: the exit orders of all open trades are working".to_string()
        } else {
            format!("Reconciliation with TWS: {problems} open trade(s) need attention - see the log")
        };
        self.log(&summary);
        if problems > 0 {
            self.instant.template_status = Some(summary);
        }
    }
    /// The old stop of an adjusted trade is cancelled: places the new one (`replacement_stop_orders`)
    /// in the OCA group of the trade's profit target, or in a new group when it has none, and
    /// remembers its order ids. Goes through the same checks as every order.
    pub(super) fn place_replacement_stop(&mut self, index: usize) {
        let Some(trade) = self.trades_stored.get(index).cloned() else {
            return;
        };
        let Some(mut adjustments) = trade.adjustments.clone() else {
            return;
        };
        let label = trade.label();
        adjustments.replace_pending = false;
        let mut updated = trade.clone();
        updated.stop_order_id = None;
        updated.stop_limit_order_id = None;
        let (true, Some(first_id), true) = (self.tws_cmd_sender.is_some(), self.next_order_id, self.trading_allowed()) else {
            self.log(&format!("WARNING: the adjusted stop of {label} was NOT placed (not connected, or the safety gate is shut) - the trade has no stop"));
            updated.adjustments = Some(adjustments);
            self.save_trade(index, updated);
            return;
        };
        let (stop_id, limit_id) = (first_id, first_id.saturating_add(1));
        let group = if trade.profit_order_id.is_some() {
            trade_exit::trade_oca_group(&trade).unwrap_or_else(|| trade_exit::oca_group(stop_id))
        } else {
            trade_exit::oca_group(stop_id)
        };
        let Some(orders) = trade_exit::replacement_stop_orders(&trade, &adjustments, &self.clock.new_york_today(), &group, stop_id, limit_id) else {
            self.log(&format!("WARNING: the adjusted stop of {label} cannot be built - NO stop placed"));
            updated.adjustments = Some(adjustments);
            self.save_trade(index, updated);
            return;
        };
        self.next_order_id = Some(first_id.saturating_add(2));
        updated.oca_group = Some(group);
        if let Some(order) = orders.stop {
            self.log(&format!("Place {} ClientOrderID: {stop_id} Price: {} Qty: {} (adjusted stop)", order.order_type, order.aux_price, order.total_quantity));
            if self.submit_order(stop_id, orders.contract.clone(), order).is_ok() {
                updated.stop_order_id = Some(stop_id);
            }
        }
        if let Some(order) = orders.stop_limit {
            self.log(&format!(
                "Place STP LMT ClientOrderID: {limit_id} Stop Price: {} Limit Price: {} Qty: {} (adjusted stop)",
                order.aux_price, order.lmt_price, order.total_quantity
            ));
            if self.submit_order(limit_id, orders.contract, order).is_ok() {
                updated.stop_limit_order_id = Some(limit_id);
            }
        }
        if updated.stop_order_id.is_some() || updated.stop_limit_order_id.is_some() {
            updated.stop_price = adjustments.stop.as_deref().and_then(|p| p.parse::<Decimal>().ok());
        } else {
            updated.stop_price = None;
            self.log(&format!("WARNING: the adjusted stop of {label} was NOT placed - the trade has no stop"));
        }
        updated.adjustments = Some(adjustments);
        self.save_trade(index, updated);
    }

    /// TWS refused the stop-limit order of a trade whose stop is a plain `Limit stop` (error 201, usually because the limit lies too far
    /// from the market): the stop is placed again with a limit offset 0.05 smaller, until TWS takes it or the offset is 0. A
    /// `Limit stop with OCO` keeps its market backstop and is not touched here.
    pub(super) fn retry_rejected_stop_limit(&mut self, order_id: i32, code: i32) {
        if code != 201 {
            return;
        }
        let Some(index) = self.trades_stored.iter().position(|t| t.is_open() && t.stop_limit_order_id == Some(order_id) && t.stop_order_id.is_none()) else {
            return;
        };
        let Some(trade) = self.trades_stored.get(index).cloned() else { return };
        let label = trade.label();
        let template = self.template_of(&trade).cloned();
        let step = 0.05_f64;
        let current = trade.adjustments.as_ref().and_then(|a| a.limit_offset).or_else(|| template.as_ref().and_then(|t| t.stop_limit_offset)).unwrap_or(0.10);
        if current <= 0.0 {
            self.log(&format!("WARNING: TWS refused the stop-limit order {order_id} of {label} even with a limit offset of 0 - the trade has no stop"));
            return;
        }
        let lowered = (current - step).max(0.0);
        let mut adjustments = trade.adjustments.clone().unwrap_or_else(|| StopAdjustments {
            order_type: "Limit stop".to_string(),
            stop: trade.stop_price.map(|price| price.normalize().to_string()),
            limit_offset: Some(current),
            legs: trade_exit::exit_legs(&trade, &template.clone().unwrap_or_default(), &|_| true).iter().map(ComboLegSpec::stored).collect(),
            trigger_method: template.as_ref().map_or(0, |t| t.stop_trigger),
            outside_rth: template.as_ref().is_some_and(|t| t.order_flags.extended_hour_stop),
            ..StopAdjustments::default()
        });
        adjustments.limit_offset = Some(lowered);
        self.log(&format!("TWS refused the stop-limit order {order_id} of {label}: placing it again with a limit offset of {lowered:.2} (was {current:.2})"));
        let mut updated = trade;
        updated.adjustments = Some(adjustments);
        self.save_trade(index, updated);
        self.place_replacement_stop(index);
    }

    /// Places the profit-target order of `level` (1-3) of the trade at `index` once the level
    /// before was hit: the contracts and the price are those worked out at the fill, the order
    /// joins the OCA group of the trade's stop (so a fill reduces it), or starts a group of its
    /// own when the trade has no stop. Goes through the same checks as every order.
    pub(super) fn place_next_profit_level(&mut self, index: usize, level: usize) {
        let Some(trade) = self.trades_stored.get(index).cloned() else {
            return;
        };
        let Some(mut levels) = trade.exit_levels.clone() else {
            return;
        };
        let label = trade.label();
        let number = level + 1;
        if !self.trading_allowed() {
            self.log(&format!("Profit target {number} of {label}: paper-account safety gate not satisfied - NOT placed"));
            return;
        }
        let (true, Some(order_id)) = (self.tws_cmd_sender.is_some(), self.next_order_id) else {
            self.log(&format!("Profit target {number} of {label}: not connected to TWS - NOT placed"));
            return;
        };
        let price = levels.prices.get(level).and_then(|p| p.as_deref()).and_then(|p| p.parse::<Decimal>().ok());
        let qty = levels.qty.get(level).copied().unwrap_or(0);
        let (Some(price), true) = (price, qty > 0) else {
            self.log(&format!("Profit target {number} of {label}: no price or no contracts for this level - NOT placed"));
            return;
        };
        let legs: Vec<ComboLegSpec> = levels.legs.iter().map(ComboLegSpec::from_stored).collect();
        // A group whose orders are all done cannot take a new one: without a stop that is still
        // working, this level starts its own group.
        let group = if trade.stop_order_id.or(trade.stop_limit_order_id).is_some() {
            trade_exit::trade_oca_group(&trade).unwrap_or_else(|| trade_exit::oca_group(order_id))
        } else {
            trade_exit::oca_group(order_id)
        };
        let Some((contract, order)) = trade_exit::level_order(&trade, &legs, &self.clock.new_york_today(), price, qty, order_id, &group, levels.outside_rth)
        else {
            self.log(&format!("Profit target {number} of {label}: the order cannot be built - NOT placed"));
            return;
        };
        self.next_order_id = Some(order_id.saturating_add(1));
        self.log(&format!("Place Profit Limit {number} ClientOrderID: {order_id} Profit Limit Price: {} Qty: {}", order.lmt_price, order.total_quantity));
        if self.submit_order(order_id, contract, order).is_err() {
            self.log(&format!("Profit target {number} of {label}: NOT placed - the open contracts have no profit target"));
            return;
        }
        if let Some(slot) = levels.order_ids.get_mut(level) {
            *slot = Some(order_id);
        }
        let mut updated = trade;
        updated.exit_levels = Some(levels);
        updated.oca_group = Some(group);
        self.save_trade(index, updated);
    }

    /// Replaces the stored trade at `index` and writes it to `trades.json`.
    pub(super) fn save_trade(&mut self, index: usize, trade: StoredTrade) {
        self.pending.trade_log_dirty = true;
        if !cfg!(test) {
            let path = trades_path(self.link.trading_live);
            if let Err(error) = path.as_ref().map_err(std::string::ToString::to_string).and_then(|p| update_trade(p, &trade).map_err(|e| e.to_string())) {
                self.log(&format!("Could not save the trade: {error}"));
            }
        }
        if let Some(slot) = self.trades_stored.get_mut(index) {
            *slot = trade;
        }
    }

    /// TWS's `orderStatus` for a stop, profit-target or close order of an open trade: the fill
    /// ends the trade; an order that dies unfilled
    /// leaves the trade without it, which is logged and shown.
    pub(super) fn on_exit_order_status(&mut self, order_id: i32, status: &str, remaining: f64, avg_fill_price: f64) {
        let Some(open_trade) = self.trades_stored.iter().find(|t| t.is_open() && t.exit_order_ids().contains(&order_id)).map(|t| t.order_id) else {
            // An exit order of a trade that is closed already: filling now is an over fill.
            self.check_over_fill(order_id, status, remaining);
            return;
        };
        self.apply_exit_order_status(order_id, status, remaining, avg_fill_price);
        // The order that ended the trade is remembered, to tell a second fill from the first.
        let closed = self.trades_stored.iter().any(|t| t.order_id == open_trade && !t.is_open());
        if closed && status == "Filled" && remaining == 0.0 {
            self.watch.closed_by.insert(open_trade, order_id);
        }
    }

    /// What an exit order's status means for its open trade (see `on_exit_order_status`).
    fn apply_exit_order_status(&mut self, order_id: i32, status: &str, remaining: f64, avg_fill_price: f64) {
        let Some(index) = self.trades_stored.iter().position(|t| t.is_open() && t.exit_order_ids().contains(&order_id)) else {
            return;
        };
        let Some(trade) = self.trades_stored.get(index).cloned() else {
            return;
        };
        let label = trade.label();
        // The profit-target level this order is (0-3), if it is one.
        let level = trade.exit_levels.as_ref().and_then(|levels| levels.order_ids.iter().position(|id| *id == Some(order_id)));
        let kind = if trade.stop_order_id == Some(order_id) || trade.stop_limit_order_id == Some(order_id) {
            "stop"
        } else if trade.profit_order_id == Some(order_id) || level.is_some() {
            "profit target"
        } else {
            "close order"
        };
        if status == "Filled" && remaining == 0.0 {
            let Some(paid) = dec(avg_fill_price) else {
                self.log(&format!("The {kind} {order_id} of {label} filled, but the price {avg_fill_price} is unusable - trade left open"));
                return;
            };
            // What the fill paid per contract to close (negative = paid): buying closes a credit
            // trade at a negative price; a debit combo is bought at a negative limit, so a fill at
            // -2.25 brought 2.25; a single long option is sold, so its price is what it brought.
            let price = if trade_exit::closes_by_selling(&trade) { paid } else { -paid };
            let how = format!("{kind} {order_id}");
            if kind == "profit target" {
                self.on_profit_level_filled(index, level.unwrap_or(0), price, &how);
                return;
            }
            // A stop or close order ends what is left; the levels hit before count in the average.
            let Some(average) = self.average_close_price(&trade, price) else {
                self.log(&format!("The {kind} {order_id} of {label} filled, but the closing price cannot be averaged - trade left open"));
                return;
            };
            self.finish_trade(index, if kind == "stop" { TradeStatus::Stopped } else { TradeStatus::Closed }, average, &how);
            if kind == "stop" {
                self.sell_leftover_longs(&trade);
            }
        } else if order_is_final(status) {
            // An order the person deleted ends as asked - also a stop an adjustment was about to replace: it is not placed again.
            if self.watch.user_cancelled.remove(&order_id) {
                self.log(&format!("The {kind} {order_id} of {label} is {status}, as asked"));
                self.forget_ended_exit_order(index, order_id);
                return;
            }
            if kind == "stop" && trade.adjustments.as_ref().is_some_and(|a| a.replace_pending) {
                self.log(&format!("The old stop {order_id} of {label} is {status}: placing the adjusted stop"));
                self.place_replacement_stop(index);
                return;
            }
            let consequence = if kind == "close order" {
                "the trade is still open (its stop and profit target, if any, are still working)"
            } else {
                "the trade no longer has it"
            };
            self.log(&format!("WARNING: the {kind} {order_id} of {label} is {status} - {consequence}"));
            if kind == "stop" {
                self.notify_missing_stop(&[format!("{label}: the stop {order_id} is {status} - the trade may have NO stop")]);
            }
            self.instant.template_status = Some(format!("{label}: {kind} {order_id} {status}"));
            self.forget_ended_exit_order(index, order_id);
        }
    }

    /// An exit order of an open trade ended without a fill (cancelled in TWS, say): the trade no longer has it, so the
    /// Dashboard stops showing its price and "Stop / target" can attach a new one.
    fn forget_ended_exit_order(&mut self, index: usize, order_id: i32) {
        let Some(trade) = self.trades_stored.get(index) else { return };
        let mut updated = trade.clone();
        if trade.stop_order_id == Some(order_id) {
            updated.stop_order_id = None;
        } else if trade.stop_limit_order_id == Some(order_id) {
            updated.stop_limit_order_id = None;
        } else if let Some(slot) = trade.exit_levels.as_ref().and_then(|levels| levels.order_ids.iter().position(|id| *id == Some(order_id))) {
            // One level of a split target: the levels that were hit stay on record, the others keep working.
            if let Some(entry) = updated.exit_levels.as_mut().and_then(|levels| levels.order_ids.get_mut(slot)) {
                *entry = None;
            }
            if trade.profit_order_id == Some(order_id) {
                updated.profit_order_id = None;
            }
            if updated.exit_levels.as_ref().is_some_and(|levels| levels.order_ids.iter().all(Option::is_none)) {
                updated.profit_price = None;
            }
        } else if trade.profit_order_id == Some(order_id) {
            updated.profit_order_id = None;
            updated.profit_price = None;
        } else if trade.close_order_id == Some(order_id) {
            updated.close_order_id = None;
        }
        if updated.stop_order_id.is_none() && updated.stop_limit_order_id.is_none() && (trade.stop_order_id.is_some() || trade.stop_limit_order_id.is_some()) {
            // Without a stop there is nothing the adjustments could move.
            updated.stop_price = None;
            updated.adjustments = None;
        }
        if updated != *trade {
            self.log(&format!("{} {} {}: order {order_id} is no longer part of the trade", trade.symbol, trade.trade_type, trade.strikes));
            self.save_trade(index, updated);
        }
    }

    /// The price per contract a trade averaged over all its closes: the profit-target levels hit so
    /// far plus what is still open at `price`.
    #[allow(clippy::unused_self)] // a method like its neighbours, called as `self.average_close_price(..)`
    pub(super) fn average_close_price(&self, trade: &StoredTrade, price: Decimal) -> Option<Decimal> {
        let closed = trade.exit_levels.as_ref().map(|levels| levels.closed.as_slice()).unwrap_or_default();
        trade_exit::average_close(closed, trade.remaining_qty(), price, trade.qty)
    }

    /// A profit-target level was hit completely at `price` per contract: it is recorded, and the
    /// next level that has contracts is placed; after the last one the trade is "Closed" at the
    /// average price of all its levels. A trade with a single target ends at once.
    pub(super) fn on_profit_level_filled(&mut self, index: usize, level: usize, price: Decimal, how: &str) {
        let Some(trade) = self.trades_stored.get(index).cloned() else {
            return;
        };
        let Some(mut levels) = trade.exit_levels.clone() else {
            self.finish_trade(index, TradeStatus::Closed, price, how);
            return;
        };
        let label = trade.label();
        let qty = levels.qty.get(level).copied().unwrap_or(0);
        levels.closed.push((qty, price.normalize().to_string()));
        levels.level = u8::try_from(level.saturating_add(1)).unwrap_or(4);
        self.log(&format!("Profit target {} of {label} hit: {qty} contract(s) at {}", level + 1, price.normalize()));
        let next = (level + 1..4).find(|next| levels.qty.get(*next).copied().unwrap_or(0) > 0);
        let mut updated = trade.clone();
        updated.exit_levels = Some(levels);
        let remaining = updated.remaining_qty();
        self.save_trade(index, updated.clone());
        match next {
            Some(next) if remaining > 0 => self.place_next_profit_level(index, next),
            _ => match self.average_close_price(&updated, Decimal::ZERO) {
                Some(average) => self.finish_trade(index, TradeStatus::Closed, average, how),
                None => self.log(&format!("The profit targets of {label} are all hit, but the closing price cannot be averaged - trade left open")),
            },
        }
    }

    /// Ends the open trade at `index` at `price_close` per contract (negative = paid to close):
    /// `Stopped` (its stop filled), `Closed` (profit target or close order) or `ClosedOther`
    /// (marked closed by hand).
    pub(super) fn finish_trade(&mut self, index: usize, status: TradeStatus, price_close: Decimal, how: &str) {
        let Some(trade) = self.trades_stored.get(index).cloned() else {
            return;
        };
        let mut finished = trade.clone();
        finished.status = status.as_str().to_string();
        finished.price_close = Some(price_close.normalize());
        let profit = profit_loss(trade.price_open, price_close, trade.qty, trade.multiplier);
        let profit_text = profit.map_or_else(|| "-".to_string(), display_currency);
        let message = format!(
            "Trade {}: {} {} {} x{} closed at {} by {how} -> P/L {profit_text}",
            status.as_str().to_lowercase(),
            trade.symbol,
            trade.trade_type,
            trade.strikes,
            trade.qty,
            price_close.normalize()
        );
        self.log(&message);
        self.save_trade(index, finished);
        self.notify_trade_finished(index, status);
        self.schedule_reentries(&trade, status, how);
        self.refresh_trades(self.clock.instant(), true);
    }

    /// "Leftover Long Behavior: Sell" (the template's `excess_long_behavior` is -1): after a stop that bought back only the shorts (stop
    /// type Short), the longs it left behind are sold at market, each one that has a bid. A long without a bid stays, nothing can be
    /// sold into no market. Whether the stop closed a long is told by the stop type, as at the placement of the stop.
    pub(super) fn sell_leftover_longs(&mut self, trade: &StoredTrade) {
        let Some(template) = self.template_of(trade).cloned() else {
            return;
        };
        if template.excess_long_behavior != -1 || trade.custom_legs.is_some() {
            return;
        }
        let label = trade.label();
        let closed: Vec<i32> = trade_exit::exit_legs(trade, &template, &|_| true).iter().map(|leg| leg.con_id).collect();
        let put_ratio = trade.put_ratio.max(1);
        let call_ratio = trade.call_ratio.max(1);
        let leftovers: Vec<(i32, String, u32)> = [(&trade.long_put, "P", put_ratio), (&trade.long_call, "C", call_ratio)]
            .into_iter()
            .filter_map(|(leg, right, ratio)| leg.as_ref().filter(|l| !closed.contains(&l.con_id)).map(|l| (l.con_id, format!("{} {right}", l.strike), ratio)))
            .collect();
        for (con_id, name, ratio) in leftovers {
            if chain_quote(&self.chains, con_id).is_none_or(|quote| quote.bid <= Decimal::ZERO) {
                self.log(&format!("Leftover long {name} of {label}: no bid - kept"));
                continue;
            }
            if !self.trading_allowed() {
                self.log(&format!("Leftover long {name} of {label}: paper-account safety gate not satisfied - NOT sold"));
                continue;
            }
            let (true, Some(order_id)) = (self.tws_cmd_sender.is_some(), self.next_order_id) else {
                self.log(&format!("Leftover long {name} of {label}: not connected to TWS - NOT sold"));
                continue;
            };
            let leg = ComboLegSpec { con_id, ratio: 1, action: trade_entry::Action::Sell };
            let Some(contract) = trade_exit::exit_contract(trade, &[leg]) else {
                self.log(&format!("Leftover long {name} of {label}: the order cannot be built - NOT sold"));
                continue;
            };
            let quantity = trade.remaining_qty().saturating_mul(ratio);
            self.next_order_id = Some(order_id.saturating_add(1));
            let order = IbOrder {
                action: trade_entry::Action::Sell.as_str().to_string(),
                total_quantity: f64::from(quantity),
                order_type: "MKT".to_string(),
                tif: "DAY".to_string(),
                account: trade.account.clone(),
                order_ref: crate::trade_rebuild::order_ref(order_id, "Close"),
                handling: super::OrderHandling { transmit: true, ..super::OrderHandling::default() },
                ..IbOrder::default()
            };
            self.log(&format!("Place leftover long sale ClientOrderID: {order_id} SELL {quantity} {name} at market (after the stop of {label})"));
            let _ = self.submit_order(order_id, contract, order);
        }
    }

    /// Cancels the stop, profit-target and close orders of a trade that no longer needs them.
    pub(super) fn cancel_orders_of(&mut self, trade: &StoredTrade) {
        if self.tws_cmd_sender.is_none() {
            if !trade.exit_order_ids().is_empty() {
                self.log("Not connected to TWS: cancel the trade's stop/profit/close orders in TWS yourself");
            }
            return;
        }
        self.debug(&format!("cancelling the exit orders {:?} of {} {} {}", trade.exit_order_ids(), trade.symbol, trade.trade_type, trade.strikes));
        for order_id in trade.exit_order_ids() {
            self.log(&format!("Cancel {order_id}"));
            if self.submit_cancel(order_id).is_err() {
                self.log(&format!("Cancel order {order_id} in TWS yourself"));
            }
        }
    }
}

/// The part of the fill price that belongs to one trade of an entry (a strangle is two trades).
fn share_of_price(price_open: Decimal, share: Decimal) -> Option<Decimal> {
    if share == Decimal::ONE {
        Some(price_open)
    } else {
        price_open.checked_mul(share).map(|p| p.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero))
    }
}

/// The stop adjustments of the template for a trade: what to log about the unsupported ones, and
/// the record that watches the trade (None when no adjustment is switched on).
fn stop_adjustments_of(
    template: &TradeTemplate,
    stop: Option<String>,
    legs: &[ComboLegSpec],
    rel: Option<crate::storage::RelSettings>,
    basis: Decimal,
) -> (Vec<String>, Option<StopAdjustments>) {
    let items = adjustment_items(template);
    let mut notes = Vec::new();
    for (number, item) in items.iter().enumerate() {
        if !matches!(item.order_type.as_str(), "" | "Same") && trade_exit::StopOrderType::parse(&item.order_type).is_none() {
            notes.push(format!(
                "stop adjustment {}: stop order type '{}' is not supported yet - this adjustment and the ones after it are not run",
                number + 1,
                item.order_type
            ));
        }
    }
    if items.is_empty() {
        return (notes, None);
    }
    let adjustments = StopAdjustments {
        items,
        level: 0,
        order_type: template.stop_order_type.clone(),
        stop,
        trailing_offset: Some(template.stop_offset.to_string()),
        limit_offset: template.stop_limit_offset,
        market_offset: template.stop_limit_market_offset,
        legs: legs.iter().map(ComboLegSpec::stored).collect(),
        trigger_method: template.stop_trigger,
        outside_rth: template.order_flags.extended_hour_stop,
        replace_pending: false,
        rel,
        basis: Some(basis.normalize().to_string()),
    };
    (notes, Some(adjustments))
}
/// The profit target split into levels: what closes at each level, the prices and the first order.
fn profit_levels(template: &TradeTemplate, trade: &StoredTrade, legs: &[ComboLegSpec], split: [u32; 4], profit_id: i32) -> ExitLevels {
    let level_prices: [Option<String>; 4] = std::array::from_fn(|level| {
        trade_exit::profit_level_price(template, &trade.symbol, legs.len(), trade.price_open, level).map(|p| p.normalize().to_string())
    });
    ExitLevels {
        qty: split,
        level: 0,
        order_ids: [Some(profit_id), None, None, None],
        closed: Vec::new(),
        prices: level_prices,
        legs: legs.iter().map(ComboLegSpec::stored).collect(),
        outside_rth: template.order_flags.extended_hour_pt,
    }
}

/// The stop adjustments of the template that are switched on.
fn adjustment_items(template: &TradeTemplate) -> Vec<StopAdjustment> {
    [
        (
            &template.adjustment1_type,
            template.adjustment1,
            &template.adjustment1_order_type,
            &template.adjustment1_change_type,
            template.adjustment1_change,
            template.adjustment1_change_offset,
            template.adjustment1_hour,
            template.adjustment1_minute,
        ),
        (
            &template.adjustment2_type,
            template.adjustment2,
            &template.adjustment2_order_type,
            &template.adjustment2_change_type,
            template.adjustment2_change,
            template.adjustment2_change_offset,
            template.adjustment2_hour,
            template.adjustment2_minute,
        ),
        (
            &template.adjustment3_type,
            template.adjustment3,
            &template.adjustment3_order_type,
            &template.adjustment3_change_type,
            template.adjustment3_change,
            template.adjustment3_change_offset,
            template.adjustment3_hour,
            template.adjustment3_minute,
        ),
    ]
    .into_iter()
    .filter(|(kind, ..)| !matches!(kind.as_str(), "" | "None"))
    .map(|(kind, value, order_type, change_type, change, change_offset, hour, minute)| StopAdjustment {
        kind: kind.clone(),
        value,
        order_type: order_type.clone(),
        change_type: change_type.clone(),
        change,
        change_offset,
        hour,
        minute,
    })
    .collect()
}

/// The stored trade of a single option that an Instant Order bought or sold and TWS reported completely filled
/// at `avg` per share: a sold option is a credit trade (`ShortPut` / `ShortCall`), a bought one a debit trade
/// (`LongPut` / `LongCall`). `None` for anything that is not a call or put with a usable quantity.
pub(super) fn instant_option_trade(
    contract: &IbContract,
    order: &IbOrder,
    order_id: i32,
    avg: Decimal,
    con_id: i32,
    now: chrono::DateTime<chrono::Local>,
) -> Option<StoredTrade> {
    if contract.sec_type != "OPT" {
        return None;
    }
    let sold = match order.action.as_str() {
        "SELL" => true,
        "BUY" => false,
        _ => return None,
    };
    let (trade_type, put) = match (sold, contract.right.as_str()) {
        (true, "P") => ("ShortPut", true),
        (true, "C") => ("ShortCall", false),
        (false, "P") => ("LongPut", true),
        (false, "C") => ("LongCall", false),
        _ => return None,
    };
    let qty = dec(order.total_quantity).and_then(|q| q.to_u32()).filter(|q| *q > 0)?;
    let strike = dec(contract.strike)?.normalize();
    let leg = Some(crate::storage::StoredLeg { con_id, strike: strike.to_string() });
    let price_open = if sold { avg } else { Decimal::ZERO.checked_sub(avg)? };
    let mut trade = StoredTrade {
        day: now.format("%Y-%m-%d").to_string(),
        opened: now.format("%d-%m-%Y %H:%M:%S").to_string(),
        account: order.account.clone(),
        symbol: contract.symbol.clone(),
        trade_type: trade_type.to_string(),
        strikes: format!("{strike} {}", contract.right),
        order_id,
        qty,
        multiplier: contract.multiplier.trim().parse().ok().filter(|m| *m > 0).unwrap_or(100),
        price_open: price_open.normalize(),
        delta: 0.0,
        short_put: None,
        long_put: None,
        short_call: None,
        long_call: None,
        put_ratio: 1,
        call_ratio: 1,
        expiry: contract.last_trade_date_or_contract_month.clone(),
        status: TradeStatus::Open.as_str().to_string(),
        price_close: None,
        stop_order_id: None,
        profit_order_id: None,
        stop_limit_order_id: None,
        exit_levels: None,
        adjustments: None,
        oca_group: None,
        close_order_id: None,
        stop_price: None,
        profit_price: None,
        entry_unknown: false,
        strategy: String::new(),
        timed_exit: None,
        profit_cancel: None,
        origin: None,
        custom_legs: None,
    };
    match (sold, put) {
        (true, true) => trade.short_put = leg,
        (true, false) => trade.short_call = leg,
        (false, true) => trade.long_put = leg,
        (false, false) => trade.long_call = leg,
    }
    Some(trade)
}

/// Whether buying `contract` closes an option that a stored, still open trade of the same account is short in
/// (same expiration, right and strike): such an order is a closing order and opens no new trade.
pub(super) fn closes_an_open_short(trades: &[StoredTrade], contract: &IbContract, account: &str) -> bool {
    let Some(strike) = dec(contract.strike).map(|s| s.normalize().to_string()) else {
        return false;
    };
    trades.iter().filter(|t| t.is_open() && t.account == account && t.expiry == contract.last_trade_date_or_contract_month).any(|t| {
        let short = if contract.right == "P" { &t.short_put } else { &t.short_call };
        short.as_ref().is_some_and(|leg| leg.strike == strike)
    })
}

impl App {
    /// TWS's `orderStatus` for the Instant Order of a single option: once it is completely filled it is recorded
    /// as a trade, so it shows on the Dashboard (today's trades, `trades.json`) and is valued like a template
    /// trade. It gets no stop or profit target (it has no template). An order that closes a short option of an
    /// open trade is not recorded (the trade is closed from the Dashboard).
    pub(super) fn on_instant_order_status(&mut self, order_id: i32, status: &str, remaining: f64, avg_fill_price: f64) {
        if status != "Filled" || remaining != 0.0 || self.instant.last_order_id != Some(order_id) {
            return;
        }
        let Some((contract, order)) = self.instant.sent.take() else {
            return;
        };
        let Some(avg) = dec(avg_fill_price).filter(|avg| *avg > Decimal::ZERO) else {
            self.log(&format!("Instant order {order_id} filled, but the fill price {avg_fill_price} is unusable - trade not recorded"));
            return;
        };
        if order.action == "BUY" && closes_an_open_short(&self.trades_stored, &contract, &order.account) {
            self.log(&format!("Instant order {order_id} closes an open short option - not recorded as a new trade"));
            return;
        }
        let con_id = self
            .chains
            .contracts
            .iter()
            .find(|c| {
                c.symbol == contract.symbol
                    && c.expiry == contract.last_trade_date_or_contract_month
                    && c.right == contract.right
                    && (c.strike - contract.strike).abs() < 1e-9
            })
            .map_or(0, |c| c.con_id);
        let Some(trade) = instant_option_trade(&contract, &order, order_id, avg, con_id, self.clock.local()) else {
            return;
        };
        self.log(&format!(
            "Trade opened: {} {} {} x{} @ {} (instant order {order_id})",
            trade.symbol, trade.trade_type, trade.strikes, trade.qty, trade.price_open
        ));
        if !cfg!(test) {
            let saved = trades_path(self.link.trading_live).map_err(|e| e.to_string()).and_then(|p| append_trade(&p, &trade).map_err(|e| e.to_string()));
            if let Err(error) = saved {
                self.log(&format!("Could not save the trade: {error}"));
            }
        }
        self.trades_stored.push(trade);
        self.pending.trade_log_dirty = true;
        self.refresh_trades(self.clock.instant(), true);
        self.notify_trades_opened(&[self.trades_stored.len().saturating_sub(1)]);
    }
}

/// What became of a template order for the Debug log.
fn fill_outcome(complete: bool, status: &str) -> &'static str {
    if complete {
        "complete, the trades are recorded"
    } else if order_is_final(status) {
        "ended without a complete fill"
    } else {
        "still working"
    }
}

/// One line about an order as it is handed to TWS: the contract (a combo with its legs), the action, quantity, type,
/// prices, time in force, account and order reference.
fn describe_order(order_id: i32, contract: &IbContract, order: &IbOrder) -> String {
    let legs = if contract.combo_legs.is_empty() {
        String::new()
    } else {
        let legs: Vec<String> = contract.combo_legs.iter().map(|leg| format!("{} {} x{} conId {}", leg.action, leg.exchange, leg.ratio, leg.con_id)).collect();
        format!(" legs [{}]", legs.join("; "))
    };
    format!(
        "order {order_id}: {} {} {} {} {} {} {} {}{legs} | {} {} lmt {} aux {} tif {} transmit {} account {} ref \"{}\"",
        contract.symbol,
        contract.sec_type,
        contract.last_trade_date_or_contract_month,
        price_or_dash(contract.strike),
        contract.right,
        contract.exchange,
        contract.currency,
        contract.trading_class,
        order.action,
        order.total_quantity,
        price_or_dash(order.lmt_price),
        price_or_dash(order.aux_price),
        order.tif,
        order.handling.transmit,
        order.account,
        order.order_ref,
    )
}

/// A price for the Debug log; TWS's "not set" value (the largest number) is shown as a dash.
fn price_or_dash(price: f64) -> String {
    if price >= f64::MAX / 2.0 {
        "-".to_string()
    } else {
        price.to_string()
    }
}
