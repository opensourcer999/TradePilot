use super::*;
use crate::schedule::{Days, QuantityRule};
use crate::templates::{ListSwitches, StopSwitches};
use broker::ContractData;

mod custom_trades;
mod dashboard_and_logs;
mod dashboard_rows_and_instant_order;
mod exit_changes_and_reentries;
mod market_data_and_connection;
mod orders_and_exits;
mod reports_mails_and_closing;
mod schedules;
mod stop_management;
mod template_entry_and_valuation;
mod timed_exits_and_conditions;
mod triggers;

/// The day the clock of the test app stands on (see `connected_app`): the tests build their expirations from it.
fn test_today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap()
}

/// An `App` wired to a fake TWS command channel (no socket, no real TWS) that never writes to
/// the real log file.
fn connected_app() -> (App, mpsc::Receiver<TwsCommand>) {
    let mut app = App::new(Some("test".to_string()), None);
    // A fixed moment (Monday 16-11-2026, the session is open): the tests must not depend on the day or the hour they run on.
    app.clock = Box::new(crate::clock::TestClock::at_new_york(chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(11, 0, 0).unwrap()));
    app.log_path = None;
    // Independent of the mode the user last chose (`mode.json`).
    app.link.trading_live = false;
    let (sender, receiver) = mpsc::channel();
    app.tws_cmd_sender = Some(sender);
    app.session.server_version = Some("200".to_string());
    (app, receiver)
}

fn drain(receiver: &mpsc::Receiver<TwsCommand>) -> Vec<TwsCommand> {
    receiver.try_iter().collect()
}

/// The nearest expiry from today on and its DTE: 0 DTE on a weekday, the next trading day on
/// a weekend - so the tests do not depend on the day they run.
fn next_expiry() -> (i32, String) {
    let today = test_today();
    (0..=7).find_map(|dte| expiry_for_dte(today, dte).map(|expiry| (dte, expiry))).expect("an expiry within a week")
}

/// An app on a paper account with a two-leg put chain and a `PutSpread` template (3 fill
/// attempts, 7 s apart, 0.05 per step), the template trade entered and confirmed: order #10
/// went to TWS (`transmit`: right away or held for review).
fn app_with_a_sent_template_order(transmit: bool) -> (App, mpsc::Receiver<TwsCommand>) {
    app_with_a_sent_template_order_of(transmit, "1")
}

/// `app_with_a_sent_template_order` for `qty` contracts.
fn app_with_a_sent_template_order_of(transmit: bool, qty: &str) -> (App, mpsc::Receiver<TwsCommand>) {
    let (dte, expiry) = next_expiry();
    let (mut app, commands) = app_with_a_chain_and_a_template(&expiry, dte);
    app.instant.template_id = Some(4);
    app.instant.template_qty = qty.into();
    app.template_order_flags.instant_template_transmit = transmit;
    app.instant.account = "DU1234567".into();
    app.enter_template_trade();
    app.confirm_template_order();
    (app, commands)
}

/// An app on a paper account with a two-leg put chain expiring on `expiry` and the `PutSpread`
/// template #4 (`dte` days to it), connected to a fake TWS; nothing entered yet.
fn app_with_a_chain_and_a_template(expiry: &str, dte: i32) -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = connected_app();
    app.chains.symbols = ChainSymbol::from_settings(&[]);
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6700.0), price_at: Some(Instant::now()), chain_requested: true });
    for (con_id, strike, bid, ask, delta) in [(100, 6690.0, 1.85, 1.95, -0.195), (101, 6680.0, 1.55, 1.65, -0.165)] {
        app.chains.contracts.push(crate::option_chain::OptionContract {
            con_id,
            symbol: "SPX".into(),
            right: "P".into(),
            strike,
            expiry: expiry.to_string(),
            local_symbol: String::new(),
            req_id: Some(500 + con_id),
            bid: Some(bid),
            ask: Some(ask),
            delta: Some(delta),
            updated: None,
            updated_at: Some(Instant::now()),
        });
    }
    app.templates = vec![TradeTemplate {
        id: 4,
        name: "PCS".into(),
        trade_type: "PutSpread".into(),
        underlying_symbol: "SPX".into(),
        target_type: "Delta".into(),
        target_min: 10.0,
        target_max: 20.0,
        preference: "Highest premium".into(),
        long_type: "Width".into(),
        long_width: "10".into(),
        short_call_dte: dte,
        fill_attempts: 3,
        fill_wait: 7,
        fill_adjustment: 0.05,
        ..TradeTemplate::default()
    }];
    app.next_order_id = Some(10);
    app.session.account_is_paper = Some(true);
    (app, commands)
}

fn placed_limits(commands: &[TwsCommand]) -> Vec<f64> {
    commands.iter().filter_map(|c| if let TwsCommand::PlaceOrder { order, .. } = c { Some(order.lmt_price) } else { None }).collect()
}

fn cancelled_orders(commands: &[TwsCommand]) -> Vec<i32> {
    commands.iter().filter_map(|c| if let TwsCommand::CancelOrder { order_id } = c { Some(*order_id) } else { None }).collect()
}

fn order_status(order_id: i32, status: &str, filled: f64, remaining: f64) -> TwsEvent {
    TwsEvent::OrderStatus { order_id, status: status.into(), filled, remaining, avg_fill_price: 0.0 }
}

/// The sent template order #10 filled at a 0.30 credit, after the template got a 3x stop and a
/// 50 % profit target; returns what the app then sent to TWS.
fn filled_trade_with_exits(app: &mut App, commands: &mpsc::Receiver<TwsCommand>) -> Vec<TwsCommand> {
    let template = &mut app.template.orders.first_mut().unwrap().template;
    template.stop_type = "Short".into();
    template.stop_multiple = 2.0;
    template.stop_order_type = "Market stop".into();
    template.profit_target_type = "Percentage".into();
    template.profit_target = Some(50.0);
    drain(commands);
    // TWS reports the average price of a credit combo with the combo's sign: -0.30.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    drain(commands)
}

/// Order #10 for 4 contracts filled at a 0.30 credit, with a 3x stop and a profit target in two
/// levels: half of the contracts at 50 %, the other half at 75 %. Returns what was sent.
fn filled_trade_with_two_profit_levels(app: &mut App, commands: &mpsc::Receiver<TwsCommand>) -> Vec<TwsCommand> {
    let template = &mut app.template.orders.first_mut().unwrap().template;
    template.stop_type = "Short".into();
    template.stop_multiple = 2.0;
    template.stop_order_type = "Market stop".into();
    template.profit_target_type = "Percentage".into();
    template.profit_target = Some(50.0);
    template.profit_target_trade_percent = Some(50.0);
    template.profit_target2 = Some(75.0);
    template.profit_target2_trade_percent = Some(50.0);
    drain(commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 4.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    drain(commands)
}

fn fill(app: &mut App, order_id: i32, filled: f64, price: f64) {
    app.tws_sender.send(TwsEvent::OrderStatus { order_id, status: "Filled".into(), filled, remaining: 0.0, avg_fill_price: price }).unwrap();
    app.poll_tws();
}

/// Order #10 filled at a 0.30 credit with a 3x stop (0.90), a 50 % target and a stop adjustment:
/// at a 50 % profit level the stop moves to the entry price (break even), from `hour` on.
fn filled_trade_with_an_adjustment(app: &mut App, commands: &mpsc::Receiver<TwsCommand>, hour: i32) {
    let template = &mut app.template.orders.first_mut().unwrap().template;
    template.stop_type = "Short".into();
    template.stop_multiple = 2.0;
    template.stop_order_type = "Market stop".into();
    template.profit_target_type = "Percentage".into();
    template.profit_target = Some(50.0);
    template.adjustment1_type = "Profit percent".into();
    template.adjustment1 = Some(50.0);
    template.adjustment1_order_type = "Same".into();
    template.adjustment1_change_type = "Stop factor".into();
    template.adjustment1_change = Some(0.0);
    template.adjustment1_hour = hour;
    drain(commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    drain(commands);
}

/// The spread's value now (short bid - long ask) is `value`: 1.65 is the long put's ask.
fn set_spread_value(app: &mut App, value: f64) {
    for contract in &mut app.chains.contracts {
        if contract.con_id == 100 {
            contract.bid = Some(1.65 + value);
        }
    }
}

fn new_york_noon(date: &str) -> chrono::NaiveDateTime {
    chrono::NaiveDate::parse_from_str(date, "%Y%m%d").unwrap().and_hms_opt(12, 0, 0).unwrap()
}

/// Order #10 for 4 contracts as in `filled_trade_with_two_profit_levels` (stop #11 for 4, level 1
/// #12 for 2, group "TradePilot-11"), plus the stop adjustments `adjust` sets on the template.
fn filled_trade_with_levels_and(app: &mut App, commands: &mpsc::Receiver<TwsCommand>, adjust: impl FnOnce(&mut TradeTemplate)) {
    adjust(&mut app.template.orders.first_mut().unwrap().template);
    filled_trade_with_two_profit_levels(app, commands);
}

/// A stop adjustment to the entry price (break even): stop multiple 0, same order type.
fn break_even(kind: &str, value: Option<f64>) -> impl FnOnce(&mut TradeTemplate) {
    let kind = kind.to_string();
    move |template: &mut TradeTemplate| {
        template.adjustment1_type = kind;
        template.adjustment1 = value;
        template.adjustment1_order_type = "Same".into();
        template.adjustment1_change_type = "Stop factor".into();
        template.adjustment1_change = Some(0.0);
    }
}

/// A (re)connect: the account and the executions arrive, the app asks for the working orders,
/// and TWS answers with `working`. Returns what the app sent after the answer.
fn reconnect_with_working_orders(app: &mut App, commands: &mpsc::Receiver<TwsCommand>, working: &[i32]) -> Vec<TwsCommand> {
    app.tws_sender.send(TwsEvent::AccountSummaryEnd).unwrap();
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    let asked = drain(commands);
    assert_eq!(asked.iter().filter(|c| matches!(c, TwsCommand::RequestOpenOrders)).count(), 1, "the working orders are asked for once");
    for order_id in working {
        app.tws_sender.send(TwsEvent::OpenOrder { order_id: *order_id }).unwrap();
    }
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    drain(commands)
}

/// Order #10 filled with stop #11 and target #12; its adjustment was hit and the cancel of #11
/// sent - but TWS's confirmation never arrived (`TradePilot` was closed, the connection dropped).
fn adjustment_waiting_for_its_cancel(app: &mut App, commands: &mpsc::Receiver<TwsCommand>) {
    filled_trade_with_an_adjustment(app, commands, 0);
    set_spread_value(app, 0.10);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    assert_eq!(cancelled_orders(&drain(commands)), [11]);
    assert_eq!(app.trades_stored.first().and_then(|t| t.adjustments.as_ref()).map(|a| a.replace_pending), Some(true));
}

/// Order #10 filled at a 0.30 credit with a 3x Short stop, the profit target and the REL settings
/// `rel` applied to the template; returns what the app then sent to TWS.
fn filled_trade_with_a_rel_stop(app: &mut App, commands: &mpsc::Receiver<TwsCommand>, rel: impl FnOnce(&mut TradeTemplate)) -> Vec<TwsCommand> {
    let template = &mut app.template.orders.first_mut().unwrap().template;
    template.stop_type = "Short".into();
    template.stop_multiple = 2.0;
    template.stop_order_type = "Market stop".into();
    template.stop_flags.stop_rel = true;
    template.profit_target_type = "Percentage".into();
    template.profit_target = Some(50.0);
    rel(template);
    drain(commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    drain(commands)
}

fn placed_orders(sent: &[TwsCommand]) -> Vec<(i32, IbOrder)> {
    sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { order_id, order, .. } = c { Some((*order_id, (**order).clone())) } else { None }).collect()
}

// ---- calendars in the preview ----

/// Two weekdays within the next ten days for the near and the far leg of a calendar.
fn calendar_dtes() -> ((i32, String), (i32, String)) {
    let today = test_today();
    let mut found = (0..=10).filter_map(|dte| expiry_for_dte(today, dte).map(|expiry| (dte, expiry)));
    let near = found.next().expect("a weekday");
    let far = found.find(|(dte, _)| *dte >= near.0 + 2).expect("a later weekday");
    (near, far)
}

/// An app with SPX at 6700, call chains of `expiries` (every option $1.00 dearer per later
/// expiration) and a `CallCalendar` template on `near` / `far` DTE, ready to preview.
fn app_for_a_calendar_preview(expiries: &[&str], near: i32, far: i32) -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = connected_app();
    app.chains.symbols = ChainSymbol::from_settings(&[]);
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6700.0), price_at: Some(Instant::now()), chain_requested: true });
    let mut con_id = 1000;
    for (index, expiry) in expiries.iter().enumerate() {
        for k in 0..8_i32 {
            let ask = 2.0 - 0.1 * f64::from(k) + f64::from(u8::try_from(index).unwrap_or(0));
            app.chains.contracts.push(crate::option_chain::OptionContract {
                con_id,
                symbol: "SPX".into(),
                right: "C".into(),
                strike: 6710.0 + 5.0 * f64::from(k),
                expiry: (*expiry).to_string(),
                local_symbol: String::new(),
                req_id: Some(500 + con_id),
                bid: Some(ask - 0.1),
                ask: Some(ask),
                delta: Some(0.28 - 0.014 * f64::from(k)),
                updated: None,
                updated_at: Some(Instant::now()),
            });
            con_id += 1;
        }
    }
    app.templates = vec![TradeTemplate {
        id: 7,
        name: "CC".into(),
        trade_type: "CallCalendar".into(),
        underlying_symbol: "SPX".into(),
        short_call_target_type: "Delta".into(),
        short_call_target: 20.0,
        long_call_target_type: "Spread width".into(),
        long_call_target: 0.0,
        short_call_dte: near,
        long_call_dte: far,
        ..TradeTemplate::default()
    }];
    app.instant.template_id = Some(7);
    (app, commands)
}

fn opening_execution(con_id: i32, side: &str, price: f64) -> StoredExecution {
    StoredExecution {
        exec_id: format!("x{con_id}"),
        time: "20261116 10:00:00 America/New_York".into(),
        account: "DU1234567".into(),
        symbol: "SPX".into(),
        con_id,
        order_id: 10,
        order_ref: "TraPi-10".into(),
        sec_type: "OPT".into(),
        expiry: "20261116".into(),
        right: "P".into(),
        strike: 6690.0,
        side: side.into(),
        quantity: 1.0,
        price,
    }
}

/// Order #10 filled at a 0.30 credit (short put 1.95 sold, long put 1.65 bought) with a 3x Short
/// stop worked out from `basis`; returns what the app then sent to TWS.
fn filled_trade_with_a_stop_basis(app: &mut App, commands: &mpsc::Receiver<TwsCommand>, basis: &str, executions: bool) -> Vec<TwsCommand> {
    let template = &mut app.template.orders.first_mut().unwrap().template;
    template.stop_type = "Short".into();
    template.stop_multiple = 2.0;
    template.stop_order_type = "Market stop".into();
    template.stop_basis = basis.into();
    template.adjustment1_type = "Profit percent".into();
    template.adjustment1 = Some(50.0);
    template.adjustment1_order_type = "Same".into();
    template.adjustment1_change_type = "Stop factor".into();
    template.adjustment1_change = Some(0.0);
    if executions {
        app.exec_history = vec![opening_execution(100, "SLD", 1.95), opening_execution(101, "BOT", 1.65)];
    }
    drain(commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    drain(commands)
}

/// A put spread filled with a Short stop (see `filled_trade_with_exits`), the template's leftover long behavior set, and its stop filled.
fn stopped_short_with_leftover_long(behavior: i32, long_bid: Option<f64>) -> (App, Vec<TwsCommand>) {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    for template in &mut app.templates {
        template.excess_long_behavior = behavior;
        template.stop_type = "Short".into();
        template.stop_order_type = "Market stop".into();
    }
    filled_trade_with_exits(&mut app, &commands);
    for contract in app.chains.contracts.iter_mut().filter(|c| c.con_id == 101) {
        contract.bid = long_bid;
    }
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    app.poll_tws();
    let sent = drain(&commands);
    (app, sent)
}

/// A filled template trade whose template exits at 11:30 New York time (the test clock stands at 11:00), with the sent
/// commands drained.
fn filled_trade_with_a_timed_exit(limit: bool) -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    {
        let template = &mut app.template.orders.first_mut().unwrap().template;
        template.exit_hour = 11;
        template.exit_minute = 30;
        template.exit_flags.exit_order_limit = limit;
    }
    filled_trade_with_exits(&mut app, &commands);
    (app, commands)
}

fn at_new_york(app: &mut App, hour: u32, minute: u32, second: u32) {
    app.clock =
        Box::new(crate::clock::TestClock::at_new_york(chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(hour, minute, second).unwrap()));
}

/// The filled template trade (#10, stop 11, target 12) of a template that re-enters after a stop (delay 3 minutes,
/// at most two re-entries).
fn filled_trade_that_re_enters() -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    for template in [&mut app.template.orders.first_mut().unwrap().template, app.templates.first_mut().unwrap()] {
        template.reenter.re_enter_stop = true;
        template.reentry_delay = 3;
        template.reentry_max_entries = 2;
    }
    filled_trade_with_exits(&mut app, &commands);
    (app, commands)
}

fn stop_fills(app: &mut App) {
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    app.poll_tws();
}

fn early_exit_condition(limit: f64) -> crate::condition::Condition {
    use crate::condition::{Comparison, Condition, Input, Operator, Rule};
    Condition {
        id: 1,
        name: "Cost to close".into(),
        retry_until_expiration: false,
        rules: vec![Rule {
            group: 1,
            input: Input::TradeAskPrice,
            operator: Operator::Greater,
            comparison: Comparison::Value(Decimal::try_from(limit).unwrap()),
            input_length: 0,
            comparison_length: 0,
        }],
    }
}

fn indicator_requests(commands: &[TwsCommand]) -> Vec<(i32, String, String, String)> {
    commands
        .iter()
        .filter_map(|c| {
            if let TwsCommand::RequestIndicatorBars { req_id, contract, bar_size, duration } = c {
                Some((*req_id, contract.symbol.clone(), bar_size.clone(), duration.clone()))
            } else {
                None
            }
        })
        .collect()
}

fn ema_condition(length: u32, limit: &str) -> crate::condition::Condition {
    use crate::condition::{Comparison, Condition, Input, Operator, Rule};
    Condition {
        id: 1,
        name: "Above the average".into(),
        retry_until_expiration: false,
        rules: vec![Rule {
            group: 1,
            input: Input::Ema,
            operator: Operator::Greater,
            comparison: Comparison::Value(limit.parse().unwrap()),
            input_length: length,
            comparison_length: 0,
        }],
    }
}

fn a_trigger() -> crate::trigger::Trigger {
    use crate::schedule::{Days, QuantityRule, Schedule};
    crate::trigger::Trigger {
        base: Schedule {
            id: 1,
            name: "Hook".into(),
            template_id: 4,
            account: "DU1234567".into(),
            quantity: QuantityRule::Fixed { count: 1 },
            active: true,
            hour: 9,
            minute: 30,
            expiration_minutes: 5,
            days: Days::WEEKDAYS,
            ..Schedule::default()
        },
        end_hour: 15,
        end_minute: 45,
        max_executions: 1,
        strategy: "From a hook".into(),
        ..crate::trigger::Trigger::default()
    }
}

fn app_with_a_trigger() -> (App, mpsc::Receiver<TwsCommand>) {
    let (dte, expiry) = next_expiry();
    let (mut app, commands) = app_with_a_chain_and_a_template(&expiry, dte);
    app.triggers.list = vec![a_trigger()];
    app.instant.account = "DU1234567".into();
    (app, commands)
}

/// One-minute bars `from` the minute with the number `first` (09:30 is 0): bar `n` closes at `closes[n]`.
fn minute_bars(closes: &[f64], first: usize) -> Vec<crate::bars::Bar> {
    closes
        .iter()
        .enumerate()
        .map(|(n, close)| {
            let minutes = 9 * 60 + 30 + first + n;
            let price = Decimal::try_from(*close).unwrap();
            crate::bars::Bar { date: format!("20261116 {:02}:{:02}:00", minutes / 60, minutes % 60), high: price, low: price, close: price }
        })
        .collect()
}

/// An app whose template #4 is a custom put spread: sell the put near 20 delta, buy the one 10 points below; the chain has the
/// 6690 and 6680 puts. The entry is entered and confirmed (order #10 went to TWS).
fn app_with_a_sent_custom_order() -> (App, mpsc::Receiver<TwsCommand>) {
    app_with_a_sent_custom_order_with(|_| {})
}

fn app_with_a_sent_custom_order_with(adjust: impl FnOnce(&mut TradeTemplate)) -> (App, mpsc::Receiver<TwsCommand>) {
    app_with_a_sent_custom_order_of("1", adjust)
}

fn app_with_a_sent_custom_order_of(qty: &str, adjust: impl FnOnce(&mut TradeTemplate)) -> (App, mpsc::Receiver<TwsCommand>) {
    let (dte, expiry) = next_expiry();
    let (mut app, commands) = app_with_a_chain_and_a_template(&expiry, dte);
    {
        let t = app.templates.first_mut().unwrap();
        t.trade_type = "Custom".into();
        (t.custom_leg1_action, t.custom_leg1_put_call, t.custom_leg1_ratio, t.custom_leg1_target, t.custom_leg1_target_type, t.custom_leg1_dte) =
            ("Sell".into(), "P".into(), 1, 20.0, "Delta".into(), dte);
        (t.custom_leg2_action, t.custom_leg2_put_call, t.custom_leg2_ratio, t.custom_leg2_target, t.custom_leg2_target_type, t.custom_leg2_dte) =
            ("Buy".into(), "P".into(), 1, -10.0, "Offset from leg 1".into(), dte);
        t.custom_leg3_action = "None".into();
        t.custom_leg4_action = "None".into();
        adjust(t);
    }
    app.instant.template_id = Some(4);
    app.instant.template_qty = qty.into();
    app.template_order_flags.instant_template_transmit = true;
    app.instant.account = "DU1234567".into();
    app.enter_template_trade();
    app.confirm_template_order();
    (app, commands)
}

fn orders_sent_now(commands: &mpsc::Receiver<TwsCommand>) -> Vec<(i32, IbOrder)> {
    drain(commands)
        .iter()
        .filter_map(|c| if let TwsCommand::PlaceOrder { order_id, order, .. } = c { Some((*order_id, (**order).clone())) } else { None })
        .collect()
}

/// A trade with a stop at 0.9 and a target at 0.15, attached from the window, and the id of its stop.
fn trade_with_attached_exits() -> (App, std::sync::mpsc::Receiver<TwsCommand>, i32) {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let mut bare = app.trades_stored.first().unwrap().clone();
    (bare.stop_order_id, bare.profit_order_id, bare.stop_price, bare.profit_price, bare.oca_group) = (None, None, None, None, None);
    app.trades_stored[0] = bare;
    app.request_exits(0);
    app.exits_pending.as_mut().unwrap().stop = "0.9".to_string();
    app.exits_pending.as_mut().unwrap().target = "0.15".to_string();
    let stop_id = app.next_order_id.unwrap();
    app.confirm_exits();
    drain(&commands);
    (app, commands, stop_id)
}

fn debug_lines(app: &App) -> Vec<&str> {
    app.log_lines.iter().map(String::as_str).filter(|line| line.contains(" DEBUG ")).collect()
}

fn mail_lines(app: &App) -> Vec<&str> {
    app.log_lines.iter().map(String::as_str).filter(|line| line.contains("otification email")).collect()
}

#[allow(clippy::too_many_arguments)]
fn execution(id: &str, order: i32, reference: &str, con: i32, strike: f64, side: &str, price: f64, time: &str) -> broker::ExecutionRow {
    broker::ExecutionRow {
        account: "DU1234567".into(),
        symbol: "SPX".into(),
        con_id: con,
        time: time.into(),
        exec_id: id.into(),
        order_id: order,
        order_ref: reference.into(),
        expiry: "20261113".into(),
        sec_type: "OPT".into(),
        right: "P".into(),
        strike,
        side: side.into(),
        quantity: 1.0,
        price,
    }
}

fn close_with(app: &mut App, kind: CloseKind, price: &str) {
    app.request_close(0, kind);
    app.close_pending.as_mut().unwrap().price = price.to_string();
    app.confirm_close();
}

fn requested_symbols(commands: &[TwsCommand]) -> Vec<(i32, String)> {
    commands
        .iter()
        .filter_map(|c| match c {
            TwsCommand::RequestMarketData { req_id, contract } => Some((*req_id, contract.symbol.clone())),
            _ => None,
        })
        .collect()
}

fn spx_option(req_id: i32, con_id: i32, right: &str, strike: f64, expiry: &str) -> ContractData {
    ContractData {
        req_id,
        con_id,
        symbol: "SPX".into(),
        sec_type: "OPT".into(),
        expiry: expiry.into(),
        strike,
        right: right.into(),
        exchange: "SMART".into(),
        currency: "USD".into(),
        local_symbol: String::new(),
        trading_class: "SPXW".into(),
    }
}

/// An app that has sent an Instant Order for one SPX option (`action`, strike 7400, `right`, expiry 20261113).
fn app_with_a_sent_instant_option(action: &str, right: &str) -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = connected_app();
    app.session.account_is_paper = Some(true);
    app.next_order_id = Some(30);
    app.instant.account = "DU1234567".into();
    app.instant.sec_type = "OPT".into();
    app.instant.expiry = "20261113".into();
    app.instant.strike = "7400".into();
    app.instant.right = right.into();
    app.instant.action = action.into();
    app.instant.qty = "2".into();
    app.instant.order_type = "LMT".into();
    app.instant.limit_price = "1.50".into();
    app.submit_instant_order();
    app.confirm_instant_order();
    drain(&commands);
    assert_eq!(app.instant.last_order_id, Some(30));
    (app, commands)
}

fn quote<'a>(app: &'a App, label: &str) -> &'a QuoteRow {
    app.quotes.iter().find(|q| q.label == label).unwrap_or_else(|| panic!("no {label} row"))
}

// ---- schedules: a run enters its trade through the one template path ----

/// The expiry of the test chain (0 DTE on the Monday the clock of the tests stands on).
const SCHEDULE_EXPIRY: &str = "20261116";

/// An app with the test chain and the `PutSpread` template #4 on the Monday of the tests, the
/// clock at 09:35:`second` New York time, and one schedule (09:35:00, five minutes of window,
/// one contract, Monday to Friday) whose run is made.
fn app_with_a_schedule(second: u32) -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = app_with_a_chain_and_a_template(SCHEDULE_EXPIRY, 0);
    app.schedule.list = vec![a_schedule(1)];
    set_schedule_clock(&mut app, second);
    app.update_schedule_runs();
    (app, commands)
}

fn a_schedule(id: u32) -> Schedule {
    Schedule {
        id,
        template_id: 4,
        account: "DU1234567".into(),
        hour: 9,
        minute: 35,
        expiration_minutes: 5,
        quantity: QuantityRule::Fixed { count: 1 },
        active: true,
        days: Days::WEEKDAYS,
        ..Schedule::default()
    }
}

/// The clock of the app at 09:35:`second` on the Monday of the tests; the checks of the runs are due.
fn set_schedule_clock(app: &mut App, second: u32) {
    let monday = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
    app.clock = Box::new(crate::clock::TestClock::at_new_york(monday.and_hms_opt(9, 35, second).unwrap()));
    app.schedule.runs_checked_at = None;
    app.schedule.runner_checked_at = None;
    // The quotes of the chain are fresh on this clock.
    let now = app.clock.instant();
    for contract in &mut app.chains.contracts {
        contract.updated_at = Some(now);
    }
    for underlying in &mut app.chains.underlyings {
        underlying.price_at = Some(now);
    }
}

fn run_the_schedules(app: &mut App, second: u32) {
    set_schedule_clock(app, second);
    app.update_schedule_runs();
    app.run_schedules();
}

fn placed_quantities(commands: &[TwsCommand]) -> Vec<f64> {
    commands.iter().filter_map(|c| if let TwsCommand::PlaceOrder { order, .. } = c { Some(order.total_quantity) } else { None }).collect()
}

fn filled(order_id: i32) -> TwsEvent {
    TwsEvent::OrderStatus { order_id, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }
}

// ---- conditions ----

/// A condition with one rule: the VIX below `limit`.
fn vix_below(limit: i64) -> Condition {
    Condition {
        id: 1,
        name: "Calm".into(),
        retry_until_expiration: false,
        rules: vec![crate::condition::Rule {
            group: 1,
            input: crate::condition::Input::Vix,
            operator: crate::condition::Operator::Less,
            comparison: crate::condition::Comparison::Value(Decimal::new(limit, 0)),
            input_length: 0,
            comparison_length: 0,
        }],
    }
}

/// The VIX quote of the Live Data page with `last` as its price.
fn push_vix(app: &mut App, last: f64) {
    app.quotes.push(QuoteRow {
        label: "VIX".into(),
        req_id: 1,
        bid: None,
        ask: None,
        last: Some(last),
        last_move: None,
        bid_size: None,
        ask_size: None,
        last_size: None,
        close: None,
        open: None,
        last_update: None,
        acknowledged: true,
        requested_at: Instant::now(),
        attempts: 1,
        history_req_id: None,
        history: None,
        prev_close: None,
        bars_req_id: None,
        curve_req_id: None,
        curve: Vec::new(),
    });
}

/// An `App` with no TWS session at all (as after start or a connection loss).
fn offline_app(mode: &str) -> App {
    let mut app = App::new(Some("test".to_string()), None);
    app.log_path = None;
    app.settings.settings_maintain_connection = mode.to_string();
    app
}

fn accounts(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

fn quoted_option(con_id: i32, right: &str, strike: f64, bid: f64, ask: f64, delta: f64) -> OptionContract {
    OptionContract {
        con_id,
        symbol: "SPX".into(),
        right: right.into(),
        strike,
        expiry: "20261113".into(),
        local_symbol: String::new(),
        req_id: Some(900 + con_id),
        bid: Some(bid),
        ask: Some(ask),
        delta: Some(delta),
        updated: None,
        updated_at: Some(Instant::now()),
    }
}

fn stored_put_spread() -> StoredTrade {
    StoredTrade {
        day: "2026-11-13".into(),
        opened: "13-11-2026 15:30:00".into(),
        account: "DU1234567".into(),
        symbol: "SPX".into(),
        trade_type: "PutSpread".into(),
        strikes: "6650/6640 P".into(),
        order_id: 7,
        qty: 2,
        multiplier: 100,
        price_open: rust_decimal::Decimal::from_str_exact("1.00").unwrap(),
        delta: 4.0,
        short_put: Some(StoredLeg { con_id: 11, strike: "6650".into() }),
        long_put: Some(StoredLeg { con_id: 12, strike: "6640".into() }),
        short_call: None,
        long_call: None,
        put_ratio: 1,
        call_ratio: 1,
        expiry: "20261113".into(),
        status: "Open".into(),
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
    }
}
