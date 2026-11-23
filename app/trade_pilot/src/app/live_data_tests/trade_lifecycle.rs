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
    assert!(app.log_lines.iter().any(|l| l.contains("the stop 11 filled while TradePilot was not running")), "{:?}", app.log_lines);
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
    assert!(drain(&commands).is_empty(), "nothing is cancelled or sent on this alone");
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
    let info = broker::OpenOrderInfo {
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
