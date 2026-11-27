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
    app.open_exits_answered(0);
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
    app.open_exits_answered(0);
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
    app.open_exits_answered(0);
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
    app.open_exits_answered(0);
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
    app.open_exits_answered(0);
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

#[test]
fn a_stop_change_sent_just_before_the_app_was_closed_is_taken_from_tws_after_the_restart() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.open_exits_answered(0);
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
    let mut sent = drain(&commands);
    answer_listing_of_all_clients(&mut app, &mut sent);
    assert!(sent.is_empty(), "nothing is sent: TWS already has it");
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
    app.open_exits_answered(0);
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
    app.open_exits_answered(index);
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

#[test]
fn the_orders_of_other_clients_are_listed_once_after_the_reconciliation_and_change_nothing() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let before = app.trades_stored.clone();
    app.tws_sender.send(TwsEvent::AccountSummaryEnd).unwrap();
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    drain(&commands);
    app.tws_sender.send(working(11, "STP", None, Some(0.9))).unwrap();
    app.tws_sender.send(working(12, "LMT", Some(0.15), None)).unwrap();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    assert_eq!(drain(&commands).iter().filter(|c| matches!(c, TwsCommand::RequestAllOpenOrders)).count(), 1);
    // The answer: this client's orders again, and an order of client 7 that has the id of this app's stop.
    app.tws_sender.send(working(11, "STP", None, Some(0.9))).unwrap();
    let TwsEvent::OpenOrder { details: Some(mut other), .. } = working(11, "STP", None, Some(5.0)) else { unreachable!() };
    other.client_id = 7;
    other.perm_id = 424_242;
    other.order_ref = "my stop".into();
    app.tws_sender.send(TwsEvent::OtherClientOrder(other)).unwrap();
    app.tws_sender.send(TwsEvent::OtherClientOrderStatus { order_id: 11, client_id: 7, perm_id: Some(424_242), status: "Cancelled".into() }).unwrap();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    assert!(
        app.log_lines.iter().any(|l| l.contains("TWS order of another client (client 7, perm id 424242, order 11)") && l.contains("ref 'my stop'")),
        "{:?}",
        app.log_lines
    );
    assert!(app.log_lines.iter().any(|l| l.contains("TWS working orders of all API clients listed: 1 of other clients")), "{:?}", app.log_lines);
    assert_eq!(app.trades_stored, before, "the stop 11 of this app keeps its price and stays working");
    assert!(drain(&commands).is_empty(), "nothing is sent and nothing is reconciled again");
    // Later answers of the same session do not ask again.
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    assert!(drain(&commands).is_empty());
}

/// A stop entered by hand in TWS on contract `con_id` of `account`: as TWS reports such an order, client 0, order id 0, known by its
/// perm id.
fn stop_entered_in_tws(con_id: i32, account: &str) -> broker::OpenOrderInfo {
    let TwsEvent::OpenOrder { details: Some(mut order), .. } = working(0, "STP", None, Some(5.0)) else { unreachable!() };
    order.client_id = 0;
    order.perm_id = 622_296_617;
    order.sec_type = "OPT".into();
    order.con_id = con_id;
    order.account = account.to_string();
    *order
}

#[test]
fn no_stop_is_sent_while_an_order_entered_in_tws_works_on_a_leg_of_the_trade() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let account = app.trades_stored[0].account.clone();
    // Opening the window asks TWS what other clients have working; until the answer is complete nothing is sent.
    app.request_exits(0);
    assert_eq!(drain(&commands).iter().filter(|c| matches!(c, TwsCommand::RequestAllOpenOrders)).count(), 1);
    let pending = app.exits_pending.as_mut().unwrap();
    pending.stop = "1.20".into();
    pending.stop_unit = Some(attach::ExitUnit::Price);
    app.confirm_exits();
    assert!(placed_orders(&drain(&commands)).is_empty());
    assert!(app.log_lines.iter().any(|l| l.contains("Blocked stop/target: Waiting for TWS to list the orders of other programs")), "{:?}", app.log_lines);
    // The answer: a stop entered by hand on the short put (contract 100).
    app.tws_sender.send(TwsEvent::OtherClientOrder(Box::new(stop_entered_in_tws(100, &account)))).unwrap();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    app.confirm_exits();
    assert!(placed_orders(&drain(&commands)).is_empty(), "a second stop would not share the OCA group of the one in TWS");
    assert!(
        app.log_lines.iter().any(|l| l.contains("Orders that are not TradePilot's work on contracts of this trade in TWS: BUY 1 STP (perm id 622296617)")),
        "{:?}",
        app.log_lines
    );
    assert!(app.exits_pending.is_some(), "the window stays open with the prices typed");
    // Cancelled in TWS: now the change goes out.
    app.tws_sender.send(TwsEvent::OtherClientOrderStatus { order_id: 0, client_id: 0, perm_id: Some(622_296_617), status: "Cancelled".into() }).unwrap();
    app.poll_tws();
    app.confirm_exits();
    assert_eq!(placed_orders(&drain(&commands)).iter().map(|(id, _)| *id).collect::<Vec<_>>(), [11], "the stop is re-sent under its id");
}

#[test]
fn an_order_entered_in_tws_on_other_contracts_does_not_hold_back_the_stop() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let account = app.trades_stored[0].account.clone();
    app.request_exits(0);
    app.tws_sender.send(TwsEvent::OtherClientOrder(Box::new(stop_entered_in_tws(999, &account)))).unwrap();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    drain(&commands);
    let pending = app.exits_pending.as_mut().unwrap();
    pending.stop = "1.20".into();
    pending.stop_unit = Some(attach::ExitUnit::Price);
    app.confirm_exits();
    assert_eq!(placed_orders(&drain(&commands)).iter().map(|(id, _)| *id).collect::<Vec<_>>(), [11]);
}

/// A put held on account DU1234567 that expires 18-12-2026, as TWS reports a position.
fn put_held(con_id: i32, strike: f64, quantity: f64, average_cost: f64) -> broker::Position {
    broker::Position {
        account: "DU1234567".into(),
        contract_id: con_id.to_string(),
        symbol: "SPX".into(),
        security_type: "OPT".into(),
        expiry: "20261218".into(),
        strike,
        right: "P".into(),
        multiplier: Some(100.0),
        quantity,
        average_cost: Some(average_cost),
    }
}

/// A put spread opened by hand in TWS on an earlier day: short 6650 at 1.20, long 6640 at 0.50, two contracts; both legs ticked.
fn spread_opened_in_tws(app: &mut App) {
    app.tws_sender.send(TwsEvent::Position(put_held(701, 6650.0, -2.0, 120.0))).unwrap();
    app.tws_sender.send(TwsEvent::Position(put_held(702, 6640.0, 2.0, 50.0))).unwrap();
    app.tws_sender.send(TwsEvent::PositionsEnd).unwrap();
    app.poll_tws();
    app.import.selection = vec![("DU1234567".to_string(), 701), ("DU1234567".to_string(), 702)];
}

#[test]
fn positions_opened_in_tws_are_taken_over_as_an_open_trade_once_tws_has_listed_the_orders_of_other_programs() {
    let (mut app, commands) = connected_app();
    spread_opened_in_tws(&mut app);
    app.open_import();
    assert_eq!(app.import.pending.as_ref().map(|p| p.price.as_str()), Some("0.7"), "1.20 received, 0.50 paid");
    assert_eq!(drain(&commands).iter().filter(|c| matches!(c, TwsCommand::RequestAllOpenOrders)).count(), 1);
    app.confirm_import();
    assert_eq!(app.trades_stored.len(), 0, "not before TWS has listed the orders of other programs");
    assert!(app.log_lines.iter().any(|l| l.contains("Import from TWS positions refused: Waiting for TWS to list")), "{:?}", app.log_lines);
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    app.confirm_import();
    let trade = app.trades_stored.first().expect("taken over").clone();
    assert_eq!((trade.order_id, trade.label().as_str(), trade.qty, trade.status.as_str()), (-1, "SPX PutSpread 6650/6640 P", 2, "Open"));
    assert_eq!((trade.price_open, trade.multiplier, trade.expiry.as_str()), (Decimal::new(7, 1), 100, "20261218"));
    assert!(
        app.log_lines.iter().any(|l| l.contains("Trade imported from TWS positions: TraPi-Imp-1 SPX PutSpread 6650/6640 P x2")
            && l.contains("entry 0.7 (the average cost TWS reports)")),
        "{:?}",
        app.log_lines
    );
    assert_eq!(crate::unmanaged::unmanaged_positions(&app.positions, &app.trades_stored), Some(Vec::new()), "the positions belong to it now");
    assert!(app.import.selection.is_empty() && app.import.pending.is_none());
    assert!(placed_orders(&drain(&commands)).is_empty(), "nothing is sent to TWS");
}

#[test]
fn a_typed_entry_price_is_the_trades_and_an_order_of_another_program_on_a_leg_holds_the_import_back() {
    let (mut app, commands) = connected_app();
    spread_opened_in_tws(&mut app);
    app.open_import();
    // A stop entered by hand in TWS on the short put.
    let TwsEvent::OpenOrder { details: Some(mut stop), .. } = working(0, "STP", None, Some(5.0)) else { unreachable!() };
    (stop.client_id, stop.perm_id, stop.sec_type, stop.con_id) = (0, 622_296_617, "OPT".into(), 701);
    app.tws_sender.send(TwsEvent::OtherClientOrder(stop)).unwrap();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    app.import.pending.as_mut().unwrap().price = "0,65".into();
    app.confirm_import();
    assert_eq!(app.trades_stored.len(), 0);
    assert!(app.log_lines.iter().any(|l| l.contains("Import from TWS positions refused: Orders that are not TradePilot's")), "{:?}", app.log_lines);
    // Cancelled in TWS: taken over at the typed price.
    app.tws_sender.send(TwsEvent::OtherClientOrderStatus { order_id: 0, client_id: 0, perm_id: Some(622_296_617), status: "Cancelled".into() }).unwrap();
    app.poll_tws();
    app.confirm_import();
    assert_eq!(app.trades_stored.first().map(|t| t.price_open), Some(Decimal::new(65, 2)));
    assert!(app.log_lines.iter().any(|l| l.contains("entry 0.65 (typed; 0.7 proposed)")), "{:?}", app.log_lines);
    assert!(placed_orders(&drain(&commands)).is_empty());
}

#[test]
fn legs_that_do_not_make_one_trade_or_an_empty_price_are_not_taken_over() {
    let (mut app, _commands) = connected_app();
    spread_opened_in_tws(&mut app);
    app.open_import();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    app.import.pending.as_mut().unwrap().price = String::new();
    app.confirm_import();
    assert_eq!(app.trades_stored.len(), 0);
    assert!(app.log_lines.iter().any(|l| l.contains("refused: enter the entry price")), "{:?}", app.log_lines);
    // A leg of another underlying.
    app.tws_sender.send(TwsEvent::Position(broker::Position { symbol: "RUT".into(), ..put_held(703, 2400.0, -1.0, 300.0) })).unwrap();
    app.poll_tws();
    app.import.selection.push(("DU1234567".to_string(), 703));
    app.import.pending.as_mut().unwrap().legs = app.import.selection.clone();
    app.import.pending.as_mut().unwrap().price = "0.7".into();
    app.confirm_import();
    assert_eq!(app.trades_stored.len(), 0);
    assert!(app.log_lines.iter().any(|l| l.contains("refused: the positions are on different underlyings")), "{:?}", app.log_lines);
}

#[test]
fn stop_and_target_of_a_debit_trade_are_the_value_it_is_sold_at_and_percentages_of_the_debit() {
    use super::attach::{typed_stop, typed_target, ExitUnit};
    let debit = Decimal::new(-200, 2);
    assert_eq!(typed_stop("50%", ExitUnit::Percent, debit), Some(Decimal::ONE), "50% of the 2.00 paid lost: sold at 1.00");
    assert_eq!(typed_stop("100%", ExitUnit::Percent, debit), None, "a stop at 0 never triggers");
    assert_eq!(typed_target("50%", ExitUnit::Percent, debit), Some(Decimal::new(300, 2)), "50% gained: sold at 3.00");
    assert_eq!(typed_target("150%", ExitUnit::Percent, debit), Some(Decimal::new(500, 2)), "a debit can gain more than 100%");
    assert_eq!(typed_stop("1.20", ExitUnit::Price, debit), Some(Decimal::new(120, 2)), "a price stays a price");
}

/// The price that counts for an exit order: the trigger of a stop, the limit of a limit order.
fn price_of(order: &IbOrder) -> f64 {
    if order.order_type == "STP" {
        order.aux_price
    } else {
        order.lmt_price
    }
}

/// Takes the ticked positions over as a trade (TWS lists no orders of other programs) and returns its index.
fn take_over(app: &mut App, legs: &[(i32, f64, f64, f64)]) -> usize {
    for (con_id, strike, quantity, cost) in legs {
        app.tws_sender.send(TwsEvent::Position(put_held(*con_id, *strike, *quantity, *cost))).unwrap();
    }
    app.tws_sender.send(TwsEvent::PositionsEnd).unwrap();
    app.poll_tws();
    app.import.selection = legs.iter().map(|(con_id, ..)| ("DU1234567".to_string(), *con_id)).collect();
    app.open_import();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    app.confirm_import();
    app.trades_stored.iter().position(|t| t.order_id < 0).expect("taken over")
}

#[test]
fn a_long_put_taken_over_gets_a_stop_below_and_a_target_above_what_selling_it_brings() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    // The put 6680 of the chain (bid 1.55), bought for 2.00.
    let index = take_over(&mut app, &[(101, 6680.0, 1.0, 200.0)]);
    assert_eq!((app.trades_stored[index].trade_type.as_str(), app.trades_stored[index].price_open), ("LongPut", Decimal::new(-2, 0)));
    drain(&commands);
    app.open_exits_answered(index);
    let pending = app.exits_pending.as_mut().unwrap();
    (pending.stop, pending.stop_unit) = ("1.60".into(), Some(attach::ExitUnit::Price));
    app.confirm_exits();
    assert!(placed_orders(&drain(&commands)).is_empty());
    assert!(app.log_lines.iter().any(|l| l.contains("the stop is passed already (the trade brings 1.55 to close now)")), "{:?}", app.log_lines);
    app.open_exits_answered(index);
    let pending = app.exits_pending.as_mut().unwrap();
    (pending.stop, pending.stop_unit) = ("50%".into(), Some(attach::ExitUnit::Percent));
    (pending.target, pending.target_unit) = ("50%".into(), Some(attach::ExitUnit::Percent));
    app.confirm_exits();
    let placed = placed_orders(&drain(&commands));
    let orders: Vec<(&str, &str, f64, f64)> = placed.iter().map(|(_, o)| (o.order_type.as_str(), o.action.as_str(), price_of(o), o.total_quantity)).collect();
    assert_eq!(orders, [("STP", "SELL", 1.0, 1.0), ("LMT", "SELL", 3.0, 1.0)], "the put is sold: stopped at 1.00, the target at 3.00");
    let trade = &app.trades_stored[index];
    assert_eq!((trade.stop_now(), trade.target_now()), (Some(Decimal::ONE), Some(Decimal::new(3, 0))));
}

#[test]
fn a_debit_spread_taken_over_is_bought_back_at_negative_prices_like_the_exits_of_a_template() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    // Long 6690 (bid 1.85) for 3.00, short 6680 (ask 1.65) for 1.00: a debit of 2.00, worth 0.20 to close now.
    let index = take_over(&mut app, &[(100, 6690.0, 1.0, 300.0), (101, 6680.0, -1.0, 100.0)]);
    assert_eq!((app.trades_stored[index].trade_type.as_str(), app.trades_stored[index].price_open), ("PutDebitSpread", Decimal::new(-2, 0)));
    drain(&commands);
    app.open_exits_answered(index);
    let pending = app.exits_pending.as_mut().unwrap();
    (pending.stop, pending.stop_unit) = ("0.10".into(), Some(attach::ExitUnit::Price));
    (pending.target, pending.target_unit) = ("3".into(), Some(attach::ExitUnit::Price));
    app.confirm_exits();
    let placed = placed_orders(&drain(&commands));
    let orders: Vec<(&str, &str, f64)> = placed.iter().map(|(_, o)| (o.order_type.as_str(), o.action.as_str(), price_of(o))).collect();
    assert_eq!(orders, [("STP", "BUY", -0.1), ("LMT", "BUY", -3.0)], "{:?}", app.log_lines);
    // The window shows the prices on record as the value again.
    app.open_exits_answered(index);
    let pending = app.exits_pending.as_ref().unwrap();
    assert_eq!((pending.stop.as_str(), pending.target.as_str()), ("0.1", "3"));
}

#[test]
fn the_exits_of_a_trade_taken_over_carry_its_name_and_the_selection_ends_once_they_are_sent() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let index = take_over(&mut app, &[(101, 6680.0, 1.0, 200.0)]);
    drain(&commands);
    app.ui_state.selected_trade = Some(app.trades_stored[index].key());
    app.open_exits_answered(index);
    let pending = app.exits_pending.as_mut().unwrap();
    (pending.stop, pending.stop_unit) = ("1.00".into(), Some(attach::ExitUnit::Price));
    (pending.target, pending.target_unit) = ("3.00".into(), Some(attach::ExitUnit::Price));
    app.confirm_exits();
    let placed = placed_orders(&drain(&commands));
    let refs: Vec<&str> = placed.iter().map(|(_, o)| o.order_ref.as_str()).collect();
    assert_eq!(refs, ["TraPi-Imp-1 (SL)", "TraPi-Imp-1 (TP)"], "TWS shows which trade the orders belong to");
    assert_eq!(app.ui_state.selected_trade, None, "the edit is done: nothing stays selected");
    // The stop moved in TWS by hand keeps the reference: TWS's price is taken over, the reference is known as the trade's.
    let stop_id = placed[0].0;
    let TwsEvent::OpenOrder { details: Some(mut moved), .. } = working(stop_id, "STP", None, Some(0.9)) else { unreachable!() };
    (moved.order_ref, moved.sec_type, moved.con_id, moved.action) = ("TraPi-Imp-1 (SL)".into(), "OPT".into(), 101, "SELL".into());
    app.tws_sender.send(TwsEvent::OpenOrder { order_id: stop_id, details: Some(moved) }).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored[index].stop_now(), Some(Decimal::new(9, 1)), "{:?}", app.log_lines);
    assert!(!app.log_lines.iter().any(|l| l.contains("does not match")), "{:?}", app.log_lines);
}

#[test]
fn a_selected_trade_is_no_longer_selected_once_a_close_order_for_it_went_out_but_stays_selected_after_a_refusal() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let key = app.trades_stored[0].key();
    app.ui_state.selected_trade = Some(key.clone());
    app.request_close(0, CloseKind::Limit);
    app.close_pending.as_mut().unwrap().price = String::new();
    app.confirm_close();
    assert_eq!(app.ui_state.selected_trade.as_deref(), Some(key.as_str()), "nothing was done: still selected");
    app.request_close(0, CloseKind::Limit);
    app.close_pending.as_mut().unwrap().price = "0.20".into();
    app.confirm_close();
    assert!(placed_orders(&drain(&commands)).iter().any(|(_, o)| o.order_ref.ends_with("(Close)")));
    assert_eq!(app.ui_state.selected_trade, None);
}

#[test]
fn a_trade_on_a_symbol_no_setting_follows_gets_its_chain_and_quotes_for_its_legs_only() {
    let (mut app, commands) = connected_app();
    // The chains of the settings are running (SPX).
    app.chains.symbols = crate::option_chain::ChainSymbol::from_settings(&[]);
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6700.0), price_at: Some(Instant::now()), chain_requested: true });
    // An INTC put spread opened by hand in TWS, taken over: short 60 at 2.00, long 55 at 0.50.
    let intc = |con_id: i32, strike: f64, quantity: f64, cost: f64| broker::Position { symbol: "INTC".into(), ..put_held(con_id, strike, quantity, cost) };
    app.tws_sender.send(TwsEvent::Position(intc(860, 60.0, -1.0, 200.0))).unwrap();
    app.tws_sender.send(TwsEvent::Position(intc(855, 55.0, 1.0, 50.0))).unwrap();
    app.tws_sender.send(TwsEvent::PositionsEnd).unwrap();
    app.poll_tws();
    app.import.selection = vec![("DU1234567".to_string(), 860), ("DU1234567".to_string(), 855)];
    app.open_import();
    app.tws_sender.send(TwsEvent::OpenOrderEnd).unwrap();
    app.poll_tws();
    app.confirm_import();
    assert_eq!(app.trades_stored.first().map(StoredTrade::label), Some("INTC PutSpread 60/55 P".to_string()));
    drain(&commands);

    app.manage_chain_subscriptions(Instant::now(), true);
    let sent = drain(&commands);
    let underlying = sent.iter().find_map(|c| match c {
        TwsCommand::RequestMarketData { req_id, contract } if contract.symbol == "INTC" && contract.sec_type == "STK" => Some(*req_id),
        _ => None,
    });
    let lookup = sent.iter().find_map(|c| match c {
        TwsCommand::RequestContractDetails { req_id, contract } if contract.symbol == "INTC" && contract.last_trade_date_or_contract_month == "20261218" => {
            Some(*req_id)
        }
        _ => None,
    });
    let (Some(underlying), Some(lookup)) = (underlying, lookup) else { panic!("INTC and its 18-12-2026 chain are asked for: {:?}", app.log_lines) };
    app.manage_chain_subscriptions(Instant::now(), true);
    assert!(!drain(&commands).iter().any(|c| matches!(c, TwsCommand::RequestContractDetails { .. })), "each chain once");

    // TWS answers: three puts of the chain and a price of INTC.
    for (con_id, strike) in [(855, 55.0), (857, 57.5), (860, 60.0)] {
        app.tws_sender
            .send(TwsEvent::ContractDetails(broker::ContractData {
                req_id: lookup,
                con_id,
                symbol: "INTC".into(),
                sec_type: "OPT".into(),
                expiry: "20261218".into(),
                strike,
                right: "P".into(),
                exchange: "SMART".into(),
                currency: "USD".into(),
                local_symbol: String::new(),
                trading_class: "INTC".into(),
            }))
            .unwrap();
    }
    app.tws_sender.send(TwsEvent::ContractDetailsEnd { req_id: lookup }).unwrap();
    app.tws_sender.send(TwsEvent::TickPrice { ticker_id: underlying, tick_type: broker::TICK_LAST, price: 57.5 }).unwrap();
    app.poll_tws();
    app.manage_chain_subscriptions(Instant::now(), true);
    let mut quoted: Vec<i32> = drain(&commands)
        .iter()
        .filter_map(|c| match c {
            TwsCommand::RequestMarketData { contract, .. } if contract.sec_type == "OPT" => Some(contract.con_id),
            _ => None,
        })
        .collect();
    quoted.sort_unstable();
    assert_eq!(quoted, [855, 860], "the legs of the trade, not the 57.5 at the price");
}
