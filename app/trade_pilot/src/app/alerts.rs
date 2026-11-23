//! The watchers of `App` behind the warning mails: the daily profit target, a stop that the market has passed, open
//! trades without a quote, short ETF/stock options that expire today, an exit order that fills after the trade was
//! closed (over fill), and the orders TWS rejects for margin or other reasons. The conditions are decided in `watch`;
//! this looks at the state of the app, logs, and sends the mails of `trade_mail`.

use super::{live_legs, mask_account, profit_summary, App, Decimal, Duration, Instant, RunStatus, StoredTrade};
use crate::trade_mail;
use crate::watch::{beyond_stop, beyond_stop_percent, far_beyond_stop, profit_target_total};

/// How often the watchers look.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);
/// A stop must be passed this long before it is reported, and the report is repeated after the pause.
const BEYOND_STOP_HOLD: Duration = Duration::from_secs(60);
const REPEAT_PAUSE: Duration = Duration::from_mins(15);
/// The TWS error code of an order it rejected.
const ORDER_REJECTED: i32 = 201;
/// A trade closed beyond its stop is not closed again before this pause (its close order may still be on its way).
const CLOSE_REPEAT_PAUSE: Duration = Duration::from_mins(5);
/// Quotes must be missing this long, and the session must be this old, before it is reported.
const QUOTES_HOLD: Duration = Duration::from_secs(120);
/// Underlyings that settle in cash: their short options carry no assignment risk.
const CASH_SETTLED: [&str; 4] = ["SPX", "XSP", "RUT", "NDX"];

impl App {
    /// One look of every watcher, at most once a second and only while connected to TWS.
    pub(super) fn run_watchers(&mut self) {
        let now = self.clock.instant();
        if self.tws_cmd_sender.is_none() || !crate::clock::due(&mut self.watch.checked, now, WATCH_INTERVAL) {
            return;
        }
        self.check_daily_profit_target(now);
        if self.market_is_open() {
            self.check_beyond_stop(now);
            self.check_missing_quotes(now);
            self.check_open_short_positions();
            self.check_positions_of_open_trades(now);
        }
    }

    /// "Trade Position Missing": an open trade one of whose options TWS no longer holds on its account (closed in TWS by hand, say). Its
    /// stop and target still work there and would open a new position, so it is reported once that lasts a minute (a fill of its own exit
    /// order changes the positions a moment before TWS reports the order filled). Nothing is cancelled or booked on this alone.
    fn check_positions_of_open_trades(&mut self, now: Instant) {
        if !self.loaded.positions_loaded {
            return;
        }
        let trades: Vec<StoredTrade> = self.trades_stored.iter().filter(|t| t.is_open() && !t.entry_unknown).cloned().collect();
        let mut problems = Vec::new();
        for trade in &trades {
            let held = |con_id: i32| {
                self.positions.iter().any(|p| p.account == trade.account && p.contract_id.trim().parse::<i32>().ok() == Some(con_id) && p.quantity != 0.0)
            };
            let missing: Vec<i32> = super::trade_exit::close_legs(trade).iter().map(|leg| leg.con_id).filter(|con_id| !held(*con_id)).collect();
            if self.watch.sustained.fire(&format!("position:{}", trade.key()), !missing.is_empty(), now, BEYOND_STOP_HOLD, REPEAT_PAUSE) {
                problems.push(format!(
                    "{} ({}): TWS holds no position in contract(s) {missing:?} - its stop and target still work",
                    trade.label(),
                    mask_account(&trade.account)
                ));
            }
        }
        if !problems.is_empty() {
            self.log(&format!("WARNING: Trade Position Missing: {}", problems.join("; ")));
            self.notify_trade_mail(&trade_mail::position_gone(&problems), true, true);
        }
    }

    /// Whether the daily profit target was reached today: the entries that were waiting are then over for the day.
    pub(super) fn daily_target_reached_today(&self) -> bool {
        self.watch.profit_target.is_hit(&self.clock.new_york_today())
    }

    /// Settings > Trading > Daily Profit Target: when the profit of today (open trades at today's prices plus closed
    /// ones) stays above the target for the delay, the entries that are still waiting are cancelled and the target is
    /// reported. The open trades are not closed by this: they keep their stops and targets.
    fn check_daily_profit_target(&mut self, now: Instant) {
        let target: Decimal = crate::pricing::parse_decimal(&self.settings.daily_profit_target).unwrap_or(Decimal::ZERO);
        if target <= Decimal::ZERO {
            return;
        }
        let Some(profit) = profit_summary(&self.trades_today) else { return };
        let total = profit_target_total(target, profit.open, profit.closed);
        let delay = Duration::from_secs(self.settings.daily_profit_target_delay_seconds.trim().parse().unwrap_or(15));
        let day = self.clock.new_york_today();
        if !self.watch.profit_target.step(&day, now, total.is_some(), delay) {
            return;
        }
        let total = total.unwrap_or(target);
        self.log(&format!(
            "Daily profit target reached: {} (target {}) - the waiting entries are cancelled",
            total.round_dp(2).normalize(),
            target.normalize()
        ));
        for index in 0..self.schedule.runs.len() {
            if self.schedule.runs.get(index).is_some_and(|run| matches!(run.status, RunStatus::Waiting | RunStatus::Paused)) {
                self.end_schedule_run(index, RunStatus::Cancelled, "the daily profit target was reached");
            }
        }
        self.save_schedule_runs();
        self.notify_trade_mail(&trade_mail::daily_profit_target(total, target), true, true);
    }

    /// "Trade Beyond Stop": an open credit trade whose closing bid is above its stop for a minute although the stop has
    /// not filled. One mail for all trades in this state, repeated after 15 minutes while it lasts.
    fn check_beyond_stop(&mut self, now: Instant) {
        let mut problems = Vec::new();
        let trades: Vec<StoredTrade> = self.trades_stored.iter().filter(|t| t.is_open() && t.price_open > Decimal::ZERO).cloned().collect();
        for trade in &trades {
            let bid = live_legs(trade, &self.chains).and_then(|legs| legs.current_bid(&trade.trade_type)).filter(|bid| *bid > Decimal::ZERO);
            let stop = trade.stop_now();
            let passed = stop.zip(bid).is_some_and(|(stop, bid)| beyond_stop(stop, bid));
            if self.watch.sustained.fire(&format!("beyond:{}", trade.key()), passed, now, BEYOND_STOP_HOLD, REPEAT_PAUSE) {
                problems.push(format!(
                    "{} ({}): stop {}, current bid {}",
                    trade.label(),
                    mask_account(&trade.account),
                    stop.map_or_else(|| "-".to_string(), |s| s.normalize().to_string()),
                    bid.map_or_else(|| "-".to_string(), |b| b.normalize().to_string())
                ));
            }
        }
        if !problems.is_empty() {
            self.log(&format!("WARNING: Trade Beyond Stop: {}", problems.join("; ")));
            self.notify_trade_mail(&trade_mail::beyond_stop(&problems), true, true);
        }
        self.close_beyond_stop(&trades, now);
    }

    /// Settings > Safety > "Close trade beyond stop price": an open credit trade whose cost to close has stayed at least the percent
    /// above its stop for the wait is closed with a market order, or a limit order at the current cost to close. A trade that has a
    /// close order working already is left alone; the close goes through the same checks as every order.
    fn close_beyond_stop(&mut self, trades: &[StoredTrade], now: Instant) {
        if !self.settings.stop_flags.close_beyond_stop_enabled {
            return;
        }
        let Some(percent) = beyond_stop_percent(&self.settings.close_beyond_stop_pct) else { return };
        let wait = Duration::from_secs(self.settings.close_beyond_stop_wait_seconds.trim().parse().unwrap_or(60));
        let market = self.settings.stop_flags.close_beyond_stop_market_order;
        for trade in trades {
            let cost = live_legs(trade, &self.chains).and_then(|legs| legs.current_bid(&trade.trade_type)).filter(|bid| *bid > Decimal::ZERO);
            let far = trade.stop_now().zip(cost).is_some_and(|(stop, cost)| far_beyond_stop(stop, cost, percent));
            let key = format!("beyond-close:{}", trade.key());
            if !self.watch.sustained.fire(&key, far && trade.close_order_id.is_none(), now, wait, CLOSE_REPEAT_PAUSE) {
                continue;
            }
            let Some(index) = self.trades_stored.iter().position(|t| t.is_open() && t.key() == trade.key()) else {
                continue;
            };
            let limit = if market { None } else { self.closing_price(trade) };
            if !market && limit.is_none() {
                self.log(&format!("WARNING: {}: beyond its stop, but there is no price to close it at - NOT closed", trade.label()));
                continue;
            }
            self.log(&format!(
                "{}: the cost to close {} has been {percent} % or more above the stop {} for {} s - closing the trade with a {} order",
                trade.label(),
                cost.map_or_else(|| "-".to_string(), |c| c.normalize().to_string()),
                trade.stop_now().map_or_else(|| "-".to_string(), |s| s.normalize().to_string()),
                wait.as_secs(),
                if market { "market" } else { "limit" }
            ));
            if let Err(reason) = self.place_close_order(index, limit) {
                self.log(&format!("WARNING: {}: the close beyond the stop was NOT sent: {reason}", trade.label()));
            }
        }
    }

    /// Settings > Safety > "Close trade on stop margin rejection": TWS refused the stop of an open trade for margin, so the trade is
    /// closed with a market order instead of being left without a stop.
    fn close_on_stop_margin_rejection(&mut self, order_id: i32) {
        if !self.settings.stop_flags.close_on_stop_margin_rejection {
            return;
        }
        let Some(index) = self
            .trades_stored
            .iter()
            .position(|t| t.is_open() && (t.stop_order_id == Some(order_id) || t.stop_limit_order_id == Some(order_id)) && t.close_order_id.is_none())
        else {
            return;
        };
        let label = self.trades_stored.get(index).map(StoredTrade::label).unwrap_or_default();
        self.log(&format!("{label}: TWS refused the stop {order_id} for margin - closing the trade at market"));
        if let Err(reason) = self.place_close_order(index, None) {
            self.log(&format!("WARNING: {label}: the close after the margin rejection of the stop was NOT sent: {reason} - the trade may have NO stop"));
        }
    }

    /// "Trade Quotes Missing or Outdated": open trades that cannot be valued for two minutes, once the session is as old.
    fn check_missing_quotes(&mut self, now: Instant) {
        let session_old = self.session.connected_at.is_some_and(|at| now.saturating_duration_since(at) >= QUOTES_HOLD);
        let unpriced: Vec<String> = self
            .trades_stored
            .iter()
            .filter(|t| t.is_open() && !t.entry_unknown)
            .filter(|t| live_legs(t, &self.chains).and_then(|legs| legs.current_bid(&t.trade_type)).is_none())
            .map(|t| format!("{} ({}): no current quote", t.label(), mask_account(&t.account)))
            .collect();
        let active = session_old && !unpriced.is_empty();
        if self.watch.sustained.fire("quotes", active, now, QUOTES_HOLD, REPEAT_PAUSE) {
            self.log(&format!("WARNING: Trade Quotes Missing or Outdated: {}", unpriced.join("; ")));
            self.notify_trade_mail(&trade_mail::quotes_missing(&unpriced), true, true);
        }
    }

    /// "Open Short 0 DTE ETF/Equity Position(s)": a short option on an ETF or stock that expires today (cash-settled
    /// indexes are fine). Every position is reported once a day.
    fn check_open_short_positions(&mut self) {
        let today = self.clock.new_york_today();
        if self.watch.short_day != today {
            self.watch.short_day.clone_from(&today);
            self.watch.short_reported.clear();
        }
        let mut found: Vec<(i32, String)> = Vec::new();
        for position in &self.positions {
            let Ok(con_id) = position.contract_id.trim().parse::<i32>() else { continue };
            if position.security_type != "OPT"
                || position.quantity >= 0.0
                || CASH_SETTLED.contains(&position.symbol.as_str())
                || self.watch.short_reported.contains(&con_id)
            {
                continue;
            }
            let Some(contract) = self.chains.contracts.iter().find(|c| c.con_id == con_id && c.expiry == today) else { continue };
            let in_the_money =
                self.chains
                    .price(&contract.symbol)
                    .is_some_and(|price| if contract.right == "P" { price <= contract.strike } else { price >= contract.strike });
            found.push((
                con_id,
                format!(
                    "Open Short Position{} ({}): {} {} {} {} (Position: {})",
                    if in_the_money { " ITM!" } else { "" },
                    mask_account(&position.account),
                    contract.symbol,
                    contract.strike,
                    contract.right,
                    contract.expiry,
                    position.quantity
                ),
            ));
        }
        if found.is_empty() {
            return;
        }
        let texts: Vec<String> = found.iter().map(|(_, line)| line.clone()).collect();
        self.watch.short_reported.extend(found.into_iter().map(|(id, _)| id));
        self.log(&format!("WARNING: {}", texts.join("; ")));
        self.notify_trade_mail(&trade_mail::open_short(&texts), true, true);
    }

    /// An exit order of a trade that is already closed filled: the contracts were closed twice ("over fill"). Only when
    /// the order that closed the trade is known (this session) and is another one.
    pub(super) fn check_over_fill(&mut self, order_id: i32, status: &str, remaining: f64) {
        if status != "Filled" || remaining != 0.0 {
            return;
        }
        let Some(trade) = self.trades_stored.iter().find(|t| !t.is_open() && t.exit_order_ids().contains(&order_id)).cloned() else { return };
        let Some(closer) = self.watch.closed_by.get(&trade.key()).copied().filter(|closer| *closer != order_id) else { return };
        if !self.watch.over_fills.insert(order_id) {
            return;
        }
        let line = format!(
            "{} ({}), opened {}: order {order_id} filled after order {closer} had closed the trade (qty {})",
            trade.label(),
            mask_account(&trade.account),
            trade.opened,
            trade.qty
        );
        self.log(&format!("WARNING: Over Fill: {line}"));
        self.notify_trade_mail(&trade_mail::over_fill(&[line]), true, true);
    }

    /// An error TWS reports for an order of this app: a margin rejection of an entry (scheduled or not) is mailed, and so
    /// is any other rejection of an exit order of a trade (it may be left without a stop).
    pub(super) fn report_order_error(&mut self, order_id: i32, code: i32, message: &str) {
        let sent = self.template.orders.iter().find(|o| o.order_id == order_id).map(|o| (o.account.clone(), o.run_id));
        let exit = self
            .trades_stored
            .iter()
            .find(|t| t.exit_order_ids().contains(&order_id) || t.close_order_id == Some(order_id))
            .map(|t| (t.account.clone(), t.label()));
        let Some(account) = sent.as_ref().map(|(account, _)| account.clone()).or_else(|| exit.as_ref().map(|(account, _)| account.clone())) else {
            return;
        };
        let text = format!("TWS {code}: {message}");
        if is_margin_message(message) {
            let entry = sent.and_then(|(_, run)| run).map(|run| format!("Schedule run {run}"));
            self.log(&format!("WARNING: margin rejection of order {order_id}: {text}"));
            self.notify_trade_mail(&trade_mail::margin_issue(entry.as_deref(), &mask_account(&account), &text), true, true);
            // Only a rejection ends the order; a margin warning leaves it working, and acting on it would send a second order.
            if code == ORDER_REJECTED {
                self.close_on_stop_margin_rejection(order_id);
                self.plan_margin_retry(order_id);
            }
        } else if let Some((_, label)) = exit {
            if is_order_failure(code) {
                let line = format!("{label}: order {order_id} - {text}");
                self.log(&format!("WARNING: Order Processing Error: {line}"));
                self.notify_trade_mail(&trade_mail::processing_error(&[line]), true, true);
            }
        }
    }
}

/// Whether TWS speaks of margin in the message of an order error: a rejection (code 201) or a warning (2xxx) for lack of margin.
pub(super) fn is_margin_message(message: &str) -> bool {
    message.to_lowercase().contains("margin")
}

/// Whether a TWS error code about an order means it failed. Not failures: the cancel confirmations (202, 161) and the
/// order warnings (399, 2xxx).
pub(super) fn is_order_failure(code: i32) -> bool {
    !matches!(code, 202 | 161 | 399) && code < 2000
}
