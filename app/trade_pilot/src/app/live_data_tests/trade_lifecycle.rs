//! Scenarios over the whole life of one trade, also across a restart of the app: what the list, the stored trade and the orders sent
//! to TWS show after each step. The trade is the filled put spread #10 (short 6690 P contract 100, long 6680 P contract 101, a 0.30
//! credit) with its stop #11 at 0.90 and its profit target #12 at 0.15.

use super::*;

/// The app is closed and started again: what it only kept in memory is gone, the trades are those it had stored.
fn restart(app: &mut App) {
    app.watch = crate::watch::WatchState::default();
    app.exits_pending = None;
    app.close_pending = None;
    app.tws_sender.send(TwsEvent::Connected { server_version: "200".into() }).unwrap();
    app.poll_tws();
}

/// The fills TWS reports for a closing order `order_id` of the spread: the short 6690 P bought back, the long 6680 P sold.
fn closing_fills(app: &App, order_id: i32, short_price: f64, long_price: f64) {
    let time = "20261116 14:00:00 US/Eastern";
    let reference = format!("TraPi-{order_id}");
    for (id, con_id, strike, side, price) in [("c-1", 100, 6690.0, "BOT", short_price), ("c-2", 101, 6680.0, "SLD", long_price)] {
        app.tws_sender.send(TwsEvent::Execution(execution(&format!("{id}-{order_id}"), order_id, &reference, con_id, strike, side, price, time))).unwrap();
    }
}

/// TWS reports the positions of the account: the contracts and their quantities.
fn positions(app: &mut App, held: &[(i32, f64)]) {
    for (con_id, quantity) in held {
        let position = broker::Position {
            account: "DU1234567".into(),
            contract_id: con_id.to_string(),
            symbol: "SPX".into(),
            security_type: "OPT".into(),
            expiry: String::new(),
            strike: 0.0,
            right: "P".into(),
            multiplier: Some(100.0),
            quantity: *quantity,
            average_cost: None,
        };
        app.tws_sender.send(TwsEvent::Position(position)).unwrap();
    }
    app.tws_sender.send(TwsEvent::PositionsEnd).unwrap();
    app.poll_tws();
}

fn at_second(app: &mut App, seconds: u64) {
    let clock = crate::clock::TestClock::at_new_york(test_today().and_hms_opt(11, 0, 0).unwrap());
    clock.advance(Duration::from_secs(seconds));
    app.clock = Box::new(clock);
}

/// The stop is changed in the window to `price` and TWS confirms it.
fn change_stop(app: &mut App, commands: &mpsc::Receiver<TwsCommand>, price: &str) {
    app.request_exits(0);
    let pending = app.exits_pending.as_mut().expect("the stop can be changed");
    pending.stop = price.into();
    pending.stop_unit = Some(attach::ExitUnit::Price);
    app.confirm_exits();
    assert_eq!(placed_orders(&drain(commands)).iter().map(|(id, _)| *id).collect::<Vec<_>>(), [11], "the stop is re-sent under its id");
    app.tws_sender.send(order_status(11, "Submitted", 0.0, 1.0)).unwrap();
    app.poll_tws();
}

fn warnings(app: &App) -> Vec<&String> {
    app.log_lines.iter().filter(|l| l.contains("WARNING")).collect()
}

#[test]
fn changed_stop_then_restart_then_the_target_filled_meanwhile_closes_the_trade_everywhere() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    change_stop(&mut app, &commands, "1.20");
    assert_eq!((app.trade_row_views()[0].stop.as_str(), app.trade_row_views()[0].target.as_str()), ("1.20", "0.15"));

    // TradePilot is closed; the target fills at 0.15 (0.20 for the short, 0.05 for the long); TWS cancels the stop of the OCA group.
    restart(&mut app);
    closing_fills(&app, 12, 0.20, 0.05);
    let sent = reconnect_with_working_orders(&mut app, &commands, &[]);

    assert!(placed_orders(&sent).is_empty() && cancelled_orders(&sent).is_empty(), "nothing is sent for a trade that is over");
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close), ("Closed", Some(Decimal::new(-15, 2))));
    assert!(warnings(&app).is_empty(), "the stop cancelled by its OCA group is no problem: {:?}", warnings(&app));
    // The trade offers no stop / target any more.
    app.request_exits(0);
    assert!(app.exits_pending.is_none());
    app.request_close(0, CloseKind::Market);
    assert!(app.close_pending.is_none());
}

#[test]
fn the_stop_filled_while_the_app_was_closed_stops_the_trade_at_its_fill() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    restart(&mut app);
    // Bought back at 1.25 for the short, the long sold at 0.20: 1.05 paid.
    closing_fills(&app, 11, 1.25, 0.20);
    reconnect_with_working_orders(&mut app, &commands, &[]);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close), ("Stopped", Some(Decimal::new(-105, 2))));
    assert!(app.log_lines.iter().any(|l| l.contains("the stop 11 filled while TradePilot was not connected to TWS")), "{:?}", app.log_lines);
}

#[test]
fn the_stop_deleted_in_tws_while_the_app_was_closed_is_reported_and_a_new_one_can_be_attached() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    restart(&mut app);
    reconnect_with_working_orders(&mut app, &commands, &[12]);

    let trade = app.trades_stored.first().unwrap().clone();
    assert!(trade.is_open());
    assert_eq!((trade.stop_order_id, trade.stop_now(), trade.adjustments.is_none()), (None, None, true), "no stop on record any more");
    assert!(!app.trade_row_views()[0].stop.contains('.'), "the list shows none: {}", app.trade_row_views()[0].stop);
    assert!(warnings(&app).iter().any(|l| l.contains("NO stop")), "{:?}", warnings(&app));

    // A new stop joins the group of the target that still works.
    app.request_exits(0);
    let pending = app.exits_pending.as_mut().expect("a stop can be attached");
    pending.stop = "300%".into();
    app.confirm_exits();
    let placed = placed_orders(&drain(&commands));
    let [(id, stop)] = placed.as_slice() else { panic!("one stop expected, got {}", placed.len()) };
    // 300 % of the 0.30 credit: stopped at 1.20.
    assert_eq!((stop.order_type.as_str(), stop.aux_price, stop.oca_group.as_str()), ("STP", 1.2, trade.oca_group.as_deref().unwrap_or_default()));
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_now()), (Some(*id), Some(Decimal::new(12, 1))));
    assert_eq!(app.trade_row_views()[0].stop, "1.20");
}

#[test]
fn a_stop_change_that_tws_confirms_only_after_a_dropped_connection_still_reaches_the_trade() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.request_exits(0);
    let pending = app.exits_pending.as_mut().unwrap();
    pending.stop = "1.10".into();
    pending.stop_unit = Some(attach::ExitUnit::Price);
    app.confirm_exits();
    drain(&commands);
    // The connection drops before TWS answers; after the reconnect TWS reports the stop as working.
    app.tws_sender.send(TwsEvent::Connected { server_version: "200".into() }).unwrap();
    app.poll_tws();
    reconnect_with_working_orders(&mut app, &commands, &[11, 12]);
    app.tws_sender.send(order_status(11, "Submitted", 0.0, 1.0)).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.first().unwrap().stop_now(), Some(Decimal::new(11, 1)));
    assert_eq!(app.trade_row_views()[0].stop, "1.10");
}

#[test]
fn a_trade_closed_by_hand_in_tws_is_reported_after_a_minute_and_its_orders_are_left_alone() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    at_second(&mut app, 0);
    positions(&mut app, &[(100, -1.0), (101, 1.0)]);
    app.run_watchers();
    assert!(!app.log_lines.iter().any(|l| l.contains("Position Missing")), "the legs are held");

    // Closed in TWS by hand: both positions are gone, the stop and the target still work there.
    positions(&mut app, &[(100, 0.0), (101, 0.0)]);
    for second in [1, 30] {
        at_second(&mut app, second);
        app.run_watchers();
    }
    assert!(!app.log_lines.iter().any(|l| l.contains("Position Missing")), "not before a minute (an own fill moves the positions first)");
    at_second(&mut app, 62);
    app.run_watchers();
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING: Trade Position Missing") && l.contains("100") && l.contains("101")), "{:?}", app.log_lines);
    app.poll_tws();
    assert!(mail_lines(&app).iter().any(|l| l.contains("Open trade not in the TWS positions")), "{:?}", mail_lines(&app));
    let sent = drain(&commands);
    assert!(placed_orders(&sent).is_empty() && cancelled_orders(&sent).is_empty(), "no order is cancelled or sent on this alone");
    assert!(app.trades_stored.first().unwrap().is_open(), "booked by Mark closed, with the price the user knows");
}

#[test]
fn a_stop_adjusted_and_then_changed_by_hand_is_the_stop_everywhere_until_the_trade_ends() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    // The adjustment moves the stop to break even (0.30) once the trade is at 50 % profit.
    set_spread_value(&mut app, 0.10);
    app.check_stop_adjustments(new_york_noon(&app.trades_stored.first().unwrap().expiry.clone()));
    drain(&commands);
    app.tws_sender.send(order_status(11, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
    let sent = placed_orders(&drain(&commands));
    let [(new_stop, order)] = sent.as_slice() else { panic!("the adjusted stop expected, got {}", sent.len()) };
    assert_eq!(order.aux_price, 0.3);
    assert_eq!(app.trade_row_views()[0].stop, "0.30");

    // The user moves it to 0.50 by hand; TWS confirms.
    app.request_exits(0);
    let pending = app.exits_pending.as_mut().unwrap();
    assert_eq!(pending.stop, "0.3", "the window starts from the stop that works");
    pending.stop = "0.50".into();
    app.confirm_exits();
    drain(&commands);
    app.tws_sender.send(order_status(*new_stop, "Submitted", 0.0, 1.0)).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_now(), trade.stop_price), (Some(Decimal::new(5, 1)), Some(Decimal::new(5, 1))));
    assert_eq!(trade.adjustments.as_ref().and_then(|a| a.stop.as_deref()), Some("0.5"));
    assert_eq!(app.trade_row_views()[0].stop, "0.50");

    // The stop fills: the trade is stopped at the fill.
    fill(&mut app, *new_stop, 1.0, 0.55);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close), ("Stopped", Some(Decimal::new(-55, 2))));
}

/// A working order as TWS reports it in its `openOrder` message.
fn working(order_id: i32, order_type: &str, limit_price: Option<f64>, aux_price: Option<f64>) -> TwsEvent {
    working_with(order_id, order_type, limit_price, aux_price, |_| {})
}

/// `working` with a field of the message changed (`change`).
fn working_with(
    order_id: i32,
    order_type: &str,
    limit_price: Option<f64>,
    aux_price: Option<f64>,
    change: impl FnOnce(&mut broker::OpenOrderInfo),
) -> TwsEvent {
    let mut info = broker::OpenOrderInfo {
        order_id,
        con_id: 0,
        symbol: "SPX".into(),
        sec_type: "BAG".into(),
        expiry: String::new(),
        strike: 0.0,
        right: String::new(),
        action: "BUY".into(),
        total_quantity: 1.0,
        order_type: order_type.into(),
        limit_price,
        aux_price,
        tif: "DAY".into(),
        oca_group: "TradePilot-11".into(),
        account: "DU1234567".into(),
        order_ref: String::new(),
    };
    change(&mut info);
    TwsEvent::OpenOrder { order_id, details: Some(Box::new(info)) }
}

#[test]
fn a_stop_change_sent_just_before_the_app_was_closed_is_taken_from_tws_after_the_restart() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.request_exits(0);
    let pending = app.exits_pending.as_mut().unwrap();
    pending.stop = "1.10".into();
    pending.stop_unit = Some(attach::ExitUnit::Price);
    app.confirm_exits();
    drain(&commands);
    // Closed before TWS confirmed the change; TWS made it all the same.
    restart(&mut app);
    assert_eq!(app.trades_stored.first().unwrap().stop_now(), Some(Decimal::new(9, 1)), "the change was never confirmed to the app");
    app.tws_sender.send(TwsEvent::AccountSummaryEnd).unwrap();
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    drain(&commands);
    app.tws_sender.send(working(11, "STP", None, Some(1.1))).unwrap();
    app.tws_sender.send(working(12, "LMT", Some(0.15), None)).unwrap();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_now(), trade.stop_price), (Some(Decimal::new(11, 1)), Some(Decimal::new(11, 1))));
    assert_eq!(app.trade_row_views()[0].stop, "1.10");
    assert!(app.log_lines.iter().any(|l| l.contains("The stop 11 of") && l.contains("is at 1.1 in TWS (0.9 on record) - taken over")), "{:?}", app.log_lines);
    assert!(warnings(&app).is_empty(), "{:?}", warnings(&app));
    assert!(drain(&commands).is_empty(), "nothing is sent: TWS already has it");
}

#[test]
fn a_target_moved_in_tws_by_hand_is_the_target_of_the_trade_and_offsets_are_never_taken_for_prices() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(working(12, "LMT", Some(0.1), None)).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.first().unwrap().target_now(), Some(Decimal::new(1, 1)));
    assert_eq!(app.trade_row_views()[0].target, "0.10");
    // The same price again changes nothing and says nothing.
    let lines = app.log_lines.len();
    app.tws_sender.send(working(12, "LMT", Some(0.1), None)).unwrap();
    app.poll_tws();
    assert_eq!(app.log_lines.iter().skip(lines).filter(|l| l.contains("taken over")).count(), 0);
    // A trailing or relative order carries an offset in its stop price: the stop on record stays.
    app.tws_sender.send(working(11, "TRAIL", None, Some(0.25))).unwrap();
    app.tws_sender.send(working(11, "REL", Some(8.0), Some(0.5))).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.first().unwrap().stop_now(), Some(Decimal::new(9, 1)));
    // An order of no open trade is ignored.
    app.tws_sender.send(working(77, "STP", None, Some(3.0))).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.first().unwrap().stop_now(), Some(Decimal::new(9, 1)));
}

#[test]
fn a_lost_link_between_tws_and_ib_keeps_the_session_and_compares_the_orders_once_it_is_back() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.request_live_data_once();
    drain(&commands);
    // 1100: TWS has no link to IB. The session with TWS stays, nothing is sent.
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: false, data_lost: false }).unwrap();
    app.poll_tws();
    assert!(app.session.ib_link_lost && app.session.server_version.is_some(), "still connected to TWS");
    assert!(warnings(&app).iter().any(|l| l.contains("TWS has lost its connection to IB")), "{:?}", app.log_lines);
    // The socket is checked by asking TWS for its time; the answer is logged.
    let sent = drain(&commands);
    assert_eq!(sent.iter().filter(|c| matches!(c, TwsCommand::RequestCurrentTime)).count(), 1, "the socket is checked");
    assert!(placed_orders(&sent).is_empty());
    app.tws_sender.send(TwsEvent::CurrentTime(1_795_635_718)).unwrap();
    app.poll_tws();
    assert!(app.log_lines.iter().any(|l| l.contains("TWS answered on the API connection") && l.contains("the socket is open")), "{:?}", app.log_lines);
    // While TWS has no link to IB no order goes out, and the person sees why: in the window and on the screen.
    assert!(app.order_block_note().is_some_and(|note| note.contains("1100")), "the windows that send orders show it");
    app.request_exits(0);
    let pending = app.exits_pending.as_mut().unwrap();
    pending.stop = "1.20".into();
    pending.stop_unit = Some(attach::ExitUnit::Price);
    app.confirm_exits();
    assert!(placed_orders(&drain(&commands)).is_empty(), "no order without the link to IB");
    assert!(app.log_lines.iter().any(|l| l.contains("NOT sent: TWS has no connection to IB (1100)")), "{:?}", app.log_lines);
    assert!(app.instant.template_status.as_deref().is_some_and(|s| s.contains("1100")), "said on the screen: {:?}", app.instant.template_status);
    // 1101: back, the market data is gone: every quote is asked for again, and the executions of the day first.
    app.loaded.account_summary_loaded = true;
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: true, data_lost: true }).unwrap();
    app.poll_tws();
    assert!(!app.session.ib_link_lost);
    let sent = drain(&commands);
    assert!(sent.iter().any(|c| matches!(c, TwsCommand::RequestMarketData { .. })), "quotes requested again");
    assert_eq!(sent.iter().filter(|c| matches!(c, TwsCommand::RequestExecutions)).count(), 1, "the executions are asked for again");
    // The target filled while TWS was cut off: its fills come with the executions, then the working orders are compared.
    closing_fills(&app, 12, 0.20, 0.05);
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    assert_eq!(drain(&commands).iter().filter(|c| matches!(c, TwsCommand::RequestOpenOrders)).count(), 1, "then the orders are compared");
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.first().unwrap().status, "Closed", "{:?}", app.log_lines);
    // 1102: back with everything kept: no quotes again, only the executions and the comparison.
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: false, data_lost: false }).unwrap();
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: true, data_lost: false }).unwrap();
    app.poll_tws();
    let sent = drain(&commands);
    assert!(!sent.iter().any(|c| matches!(c, TwsCommand::RequestMarketData { .. })));
    assert_eq!(sent.iter().filter(|c| matches!(c, TwsCommand::RequestExecutions)).count(), 1);
}

#[test]
fn a_stop_still_working_for_a_trade_that_is_closed_is_cancelled_at_the_comparison() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    // The trade is marked closed by hand, but TWS still reports its stop 11 and target 12 as working.
    let mut closed = app.trades_stored.first().unwrap().clone();
    closed.status = "ClosedOther".into();
    app.trades_stored[0] = closed;
    restart(&mut app);
    let sent = reconnect_with_working_orders(&mut app, &commands, &[11, 12]);
    let mut cancelled = cancelled_orders(&sent);
    cancelled.sort_unstable();
    assert_eq!(cancelled, [11, 12], "a stop of a closed trade would open a new position");
    assert!(warnings(&app).iter().any(|l| l.contains("is no longer open, but its order 11 is still working in TWS")), "{:?}", app.log_lines);
    // Orders of open trades and orders of nobody are left alone.
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    restart(&mut app);
    assert_eq!(cancelled_orders(&reconnect_with_working_orders(&mut app, &commands, &[11, 12, 99])).len(), 0);
}

#[test]
fn a_scheduled_entry_waits_while_tws_has_no_connection_to_ib() {
    let (mut app, commands) = app_with_a_schedule(0);
    app.session.ib_link_lost = true;
    run_the_schedules(&mut app, 0);
    assert!(placed_orders(&drain(&commands)).is_empty(), "no entry without the link to IB");
    let run = app.schedule.runs.first().expect("the run of the schedule");
    assert_eq!(run.status, crate::schedule_run::RunStatus::Waiting, "it waits, it is not cancelled");
    assert!(run.notes.contains("TWS has no connection to IB"), "{}", run.notes);
}

/// The fills of a put spread that another `TradePilot` (another computer) opened with order 2 and closed with its profit target 8.
fn trade_of_another_instance(app: &App, closed: bool) {
    let (open, close) = ("20261116 15:00:00 US/Eastern", "20261116 15:20:00 US/Eastern");
    let mut fills = vec![execution("o-1", 2, "TraPi-2", 300, 7110.0, "SLD", 1.10, open), execution("o-2", 2, "TraPi-2", 301, 7105.0, "BOT", 0.35, open)];
    if closed {
        fills.push(execution("c-1", 8, "TraPi-8 (TP)", 300, 7110.0, "BOT", 0.30, close));
        fills.push(execution("c-2", 8, "TraPi-8 (TP)", 301, 7105.0, "SLD", 0.05, close));
    }
    for fill in fills {
        app.tws_sender.send(TwsEvent::Execution(fill)).unwrap();
    }
}

#[test]
fn a_trade_opened_and_closed_by_another_instance_is_shown_closed_after_the_connect() {
    let (mut app, commands) = connected_app();
    trade_of_another_instance(&app, true);
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    drain(&commands);
    let trade = app.trades_stored.iter().find(|t| t.strikes == "7110/7105 P").expect("recovered from the executions");
    assert_eq!((trade.status.as_str(), trade.price_close), ("Closed", Some(Decimal::new(-25, 2))), "{:?}", app.log_lines);
    // 0.75 taken in, 0.25 paid back: 50 made.
    assert_eq!(trade.price_open, Decimal::new(75, 2));
    // Nothing can be done with a closed trade: no stop, target or close is offered.
    let index = app.trades_stored.iter().position(|t| t.strikes == "7110/7105 P").unwrap();
    app.request_exits(index);
    assert!(app.exits_pending.is_none());
}

#[test]
fn a_trade_recovered_as_open_is_closed_once_the_order_of_the_other_instance_that_closed_it_shows() {
    let (mut app, commands) = connected_app();
    // First connect: only the opening was filled so far, the trade is recovered as open (without stop or target of this app).
    trade_of_another_instance(&app, false);
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    assert!(app.trades_stored.iter().any(|t| t.strikes == "7110/7105 P" && t.is_open()), "{:?}", app.log_lines);
    // The other instance's profit target fills; the next connect brings the executions of the day again.
    app.tws_sender.send(TwsEvent::Connected { server_version: "200".into() }).unwrap();
    app.poll_tws();
    trade_of_another_instance(&app, true);
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    drain(&commands);
    let trade = app.trades_stored.iter().find(|t| t.strikes == "7110/7105 P").unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close), ("Closed", Some(Decimal::new(-25, 2))), "{:?}", app.log_lines);
    assert!(app.log_lines.iter().any(|l| l.contains("closed by an order outside this session")), "{:?}", app.log_lines);
}

#[test]
fn tws_without_its_connection_to_ib_for_two_minutes_is_reported_by_mail_once() {
    let (mut app, _commands) = app_with_a_sent_template_order(true);
    at_second(&mut app, 0);
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: false, data_lost: false }).unwrap();
    app.poll_tws();
    for second in [1, 60] {
        at_second(&mut app, second);
        app.run_watchers();
    }
    app.poll_tws();
    assert!(!mail_lines(&app).iter().any(|l| l.contains("TWS without connection to IB")), "a short gap (the nightly reset) says nothing");
    at_second(&mut app, 121);
    app.run_watchers();
    app.poll_tws();
    assert_eq!(mail_lines(&app).iter().filter(|l| l.contains("TWS without connection to IB")).count(), 1, "{:?}", mail_lines(&app));
    // Back again: nothing more is mailed.
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: true, data_lost: false }).unwrap();
    app.poll_tws();
    at_second(&mut app, 2000);
    app.run_watchers();
    app.poll_tws();
    assert_eq!(mail_lines(&app).iter().filter(|l| l.contains("TWS without connection to IB")).count(), 1);
}

#[test]
fn a_session_that_ends_during_1100_is_logged_with_the_time_since_1100() {
    let (mut app, _commands) = app_with_a_sent_template_order(true);
    at_second(&mut app, 0);
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: false, data_lost: false }).unwrap();
    app.poll_tws();
    at_second(&mut app, 45);
    app.tws_sender.send(TwsEvent::Failed("TWS connection closed: TWS closed the connection".into())).unwrap();
    app.poll_tws();
    assert!(app.log_lines.iter().any(|l| l.contains("The TWS session ended 45 s after 1100")), "{:?}", app.log_lines);
    assert!(!app.session.ib_link_lost && app.order_block_note().is_none(), "a new session starts without the state of the old one");
}

#[test]
fn an_open_order_message_that_does_not_match_what_the_app_knows_is_not_used_and_reported_once() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    for change in [
        Box::new(|i: &mut broker::OpenOrderInfo| i.account = "U9999999".into()) as Box<dyn Fn(&mut broker::OpenOrderInfo)>,
        Box::new(|i: &mut broker::OpenOrderInfo| i.symbol = "QQQ".into()),
        Box::new(|i: &mut broker::OpenOrderInfo| i.order_ref = "TraPi-99 (SL)".into()),
    ] {
        app.tws_sender.send(working_with(11, "STP", None, Some(1.5), |i| change(i))).unwrap();
        app.poll_tws();
        assert_eq!(app.trades_stored.first().unwrap().stop_now(), Some(Decimal::new(9, 1)), "the stop on record stays");
    }
    let reported = app.log_lines.iter().filter(|l| l.contains("openOrder message of order 11 does not match")).count();
    assert_eq!(reported, 1, "reported once: {:?}", app.log_lines);
    // A message that matches (the reference of this order) is used.
    app.tws_sender.send(working_with(11, "STP", None, Some(1.5), |i| i.order_ref = "TraPi-11 (SL)".into())).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.first().unwrap().stop_now(), Some(Decimal::new(15, 1)));
}

#[test]
fn the_api_socket_is_checked_again_every_minute_while_tws_has_no_connection_to_ib() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    at_second(&mut app, 0);
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: false, data_lost: false }).unwrap();
    app.poll_tws();
    let probes = |commands: &mpsc::Receiver<TwsCommand>| drain(commands).iter().filter(|c| matches!(c, TwsCommand::RequestCurrentTime)).count();
    assert_eq!(probes(&commands), 1, "right away");
    at_second(&mut app, 30);
    app.run_watchers();
    assert_eq!(probes(&commands), 0, "not before a minute");
    at_second(&mut app, 61);
    app.run_watchers();
    assert_eq!(probes(&commands), 1, "again after a minute");
    // Back: no more checks.
    app.tws_sender.send(TwsEvent::IbConnectivity { connected: true, data_lost: false }).unwrap();
    app.poll_tws();
    drain(&commands);
    at_second(&mut app, 200);
    app.run_watchers();
    assert_eq!(probes(&commands), 0);
}

#[test]
fn while_tws_has_no_connection_to_ib_no_trade_and_no_close_is_sent_but_marking_closed_still_works() {
    let (dte, expiry) = next_expiry();
    let (mut app, commands) = app_with_a_chain_and_a_template(&expiry, dte);
    app.session.ib_link_lost = true;
    app.instant.template_id = Some(4);
    app.instant.template_qty = "1".into();
    app.template_order_flags.instant_template_transmit = true;
    app.instant.account = "DU1234567".into();
    app.enter_template_trade();
    app.confirm_template_order();
    assert!(placed_orders(&drain(&commands)).is_empty(), "no opening order");
    assert!(app.instant.template_status.as_deref().is_some_and(|s| s.contains("1100")), "{:?}", app.instant.template_status);

    // A filled trade: closing with a limit order is refused, marking it closed (which only cancels) is not.
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.session.ib_link_lost = true;
    close_with(&mut app, CloseKind::Limit, "0.5");
    assert!(placed_orders(&drain(&commands)).is_empty(), "no close order");
    assert!(app.trades_stored.first().unwrap().is_open());
    close_with(&mut app, CloseKind::Mark, "0.5");
    let sent = drain(&commands);
    assert!(placed_orders(&sent).is_empty());
    assert!(!cancelled_orders(&sent).is_empty(), "its stop and target are cancelled");
    assert!(!app.trades_stored.first().unwrap().is_open(), "marked closed");
}

#[test]
fn the_executions_of_the_day_sent_again_are_listed_once() {
    let (mut app, _commands) = connected_app();
    for _ in 0..2 {
        trade_of_another_instance(&app, true);
    }
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    assert_eq!(app.today_trades.len(), 4, "four fills, each once");
    assert_eq!(app.exec_history.len(), 4);
}
