//! Commissions, mails, the daily profit target, reports, rebuilt trades and closing by hand.

use super::*;

#[test]
fn the_spx_is_bullish_above_its_prior_close_and_bearish_below_it() {
    let (mut app, _commands) = connected_app();
    app.request_live_data_once();
    let row = app.quotes.iter_mut().find(|q| q.label == "SPX").expect("the SPX row");
    assert_eq!(spx_direction(row), None, "no price yet");
    row.last = Some(7400.0);
    assert_eq!(spx_direction(row), None, "no reference");
    row.close = Some(7390.0);
    assert_eq!(spx_direction(row), Some(true));
    row.last = Some(7380.0);
    assert_eq!(spx_direction(row), Some(false));
    row.last = Some(7390.0);
    assert_eq!(spx_direction(row), None, "unchanged");
    // While the price moves, its last step decides: a rise is green even below the prior close.
    row.close = Some(7500.0);
    row.last = Some(7400.0);
    row.last_move = None;
    assert_eq!(spx_direction(row), Some(false), "below the prior close");
    row.set_last(7405.0);
    assert_eq!(spx_direction(row), Some(true), "rising, although still below the prior close");
    row.set_last(7405.0);
    assert_eq!(spx_direction(row), Some(true), "an unchanged price keeps the colour");
    row.set_last(7399.0);
    assert_eq!(spx_direction(row), Some(false));
    row.close = None;
    row.last_move = None;
    row.last = Some(7390.0);
    // Without a prior close the first point of the day's curve is the reference.
    row.close = None;
    row.curve = vec![("t1".to_string(), 7395.0), ("t2".to_string(), 7391.0)];
    assert_eq!(spx_direction(row), Some(false));
}

#[test]
fn the_commissions_tws_reports_are_booked_on_the_trade_it_belongs_to() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let trade = app.trades_stored.first().unwrap().clone();
    let legs: Vec<i32> = [&trade.short_put, &trade.long_put, &trade.short_call, &trade.long_call].into_iter().flatten().map(|l| l.con_id).collect();
    assert_ne!(legs.len(), 0, "the trade holds legs");
    let fill = |exec_id: &str, order_id: i32, con_id: i32| StoredExecution {
        exec_id: exec_id.to_string(),
        time: "20261116 11:00:00 America/New_York".to_string(),
        account: trade.account.clone(),
        symbol: "SPX".to_string(),
        con_id,
        order_id,
        order_ref: String::new(),
        sec_type: "OPT".to_string(),
        expiry: trade.expiry.clone(),
        right: "P".to_string(),
        strike: 6690.0,
        side: "SLD".to_string(),
        quantity: 1.0,
        price: 0.3,
    };
    app.exec_history.push(fill("open-1", trade.order_id, legs[0]));
    // Without a commission report the fee is unknown, not 0.
    assert_eq!(app.record_of(&trade).unwrap().commission, None);
    app.tws_sender.send(TwsEvent::Commission { exec_id: "open-1".into(), commission: 1.3, currency: "USD".into() }).unwrap();
    app.poll_tws();
    assert_eq!(app.record_of(&trade).unwrap().commission, Some(Decimal::new(13, 1)));
    // A commission in another currency is not booked; an unknown execution id changes nothing.
    app.tws_sender.send(TwsEvent::Commission { exec_id: "open-2".into(), commission: 9.0, currency: "EUR".into() }).unwrap();
    app.poll_tws();
    assert!(!app.commissions.contains_key("open-2"));
    // The stop fills: the closing fee joins, and the finished trade's profit is what is left after both.
    app.exec_history.push(fill("stop-1", 11, legs[0]));
    app.tws_sender.send(TwsEvent::Commission { exec_id: "stop-1".into(), commission: 1.4, currency: "USD".into() }).unwrap();
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    app.poll_tws();
    let finished = app.trades_stored.first().unwrap().clone();
    let record = app.record_of(&finished).unwrap();
    assert_eq!((record.commission, record.commission_close), (Some(Decimal::new(13, 1)), Some(Decimal::new(14, 1))));
    // (0.30 - 0.90) * 100 = -60 before fees, -62.70 after.
    assert_eq!(record.profit_loss_net, Some(Decimal::new(-6270, 2)));
    assert_eq!(record.current_profit, Decimal::new(-6270, 2));
}

#[test]
fn a_margin_rejection_of_an_order_is_mailed() {
    let (mut app, _commands) = app_with_a_sent_template_order(true);
    let order_id = app.template.orders.first().expect("the sent order").order_id;
    app.tws_sender.send(TwsEvent::RequestError { req_id: order_id, code: 201, message: "Order rejected - reason: Insufficient margin".into() }).unwrap();
    app.poll_tws();
    assert!(mail_lines(&app).iter().any(|l| l.contains("\"TradePilot: Unexpected Order Error - Margin Issue\"")), "{:?}", mail_lines(&app));
    // An error about something that is not an order of this app is not mailed.
    let before = mail_lines(&app).len();
    app.tws_sender.send(TwsEvent::RequestError { req_id: 987_654, code: 201, message: "margin".into() }).unwrap();
    app.poll_tws();
    assert_eq!(mail_lines(&app).len(), before);
}

#[test]
fn an_exit_order_that_fills_after_the_trade_was_closed_is_an_over_fill() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    app.poll_tws();
    assert!(!mail_lines(&app).iter().any(|l| l.contains("Over Fill")), "the stop closed the trade");
    // The profit target fills as well: the contracts were closed twice.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 12, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.15 }).unwrap();
    app.poll_tws();
    assert!(mail_lines(&app).iter().any(|l| l.contains("\"TradePilot: ALERT: Exit order over-filled\"")), "{:?}", mail_lines(&app));
    // Said once, however often TWS repeats the status.
    let count = mail_lines(&app).iter().filter(|l| l.contains("Over Fill")).count();
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 12, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.15 }).unwrap();
    app.poll_tws();
    assert_eq!(mail_lines(&app).iter().filter(|l| l.contains("Over Fill")).count(), count);
}

#[test]
fn the_daily_profit_target_cancels_the_waiting_entries_and_is_reported_once() {
    let (mut app, _commands) = connected_app();
    app.settings.daily_profit_target = "100".to_string();
    app.settings.daily_profit_target_delay_seconds = "0".to_string();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
    let window = app.clock.utc();
    app.schedule.runs.push(ScheduleRun {
        id: 1,
        schedule_id: 1,
        day: today,
        window_start: window,
        window_end: window + chrono::Duration::hours(1),
        status: RunStatus::Waiting,
        attempts: 0,
        started: None,
        finished: None,
        notes: String::new(),
        reentry: None,
        trigger: None,
    });
    // Below the target nothing happens.
    let open = |profit: i64| TradeRecord::opened("SPX", Decimal::ONE, 1, 100, 0.0, Some(Decimal::from(profit))).unwrap();
    app.trades_today = vec![open(50)];
    app.run_watchers();
    assert!(!app.daily_target_reached_today() && app.schedule.runs.first().is_some_and(|r| r.status == RunStatus::Waiting));
    // Above it: the waiting run is cancelled, the target is reported, and no entry is made any more today.
    app.trades_today = vec![open(500)];
    app.watch.checked = None;
    app.run_watchers();
    app.poll_tws();
    assert!(app.daily_target_reached_today());
    assert!(app.schedule.runs.first().is_some_and(|r| r.status == RunStatus::Cancelled), "{:?}", app.schedule.runs.first().map(|r| r.status));
    assert!(mail_lines(&app).iter().any(|l| l.contains("\"TradePilot: Daily profit target reached\"")), "{:?}", mail_lines(&app));
    let count = mail_lines(&app).len();
    app.watch.checked = None;
    app.run_watchers();
    app.poll_tws();
    assert_eq!(mail_lines(&app).len(), count, "reported once");
}

#[test]
fn the_report_can_be_limited_to_one_strategy() {
    let (mut app, _commands) = connected_app();
    let trade = |order_id: i32, strategy: &str| {
        let mut trade: StoredTrade = serde_json::from_str(&format!(
            r#"{{"day":"2026-11-16","opened":"16-11-2026 10:00:00","account":"DU1","symbol":"SPX","trade_type":"PutSpread","strikes":"6650/6640 P","order_id":{order_id},"qty":1,"multiplier":100,"price_open":"1","delta":0.0}}"#
        ))
        .unwrap();
        trade.strategy = strategy.to_string();
        trade
    };
    app.trade_log = vec![trade(1, "0DTE"), trade(2, "Weekly"), trade(3, "")];
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
    (app.reports.start, app.reports.end) = (day, day);
    assert_eq!(app.report_trades().len(), 3, "all strategies");
    app.reports.strategy = Some("0DTE".to_string());
    assert_eq!(app.report_trades().iter().map(|t| t.order_id).collect::<Vec<_>>(), [1]);
    app.reports.strategy = Some(String::new());
    assert_eq!(app.report_trades().iter().map(|t| t.order_id).collect::<Vec<_>>(), [3], "trades without a strategy");
}

#[test]
fn the_debug_level_adds_what_tws_answered_and_how_it_was_read_while_the_info_level_stays_quiet() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    app.poll_tws();
    assert!(debug_lines(&app).is_empty(), "{:?}", debug_lines(&app));

    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.settings.view_flags.settings_logging_debug = true;
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    app.poll_tws();
    let lines = debug_lines(&app);
    for expected in ["handed to TWS: order 11:", "exit prices of SPX", "template order", "TWS orderStatus 11: Filled, filled 1, remaining 0", "stop of SPX"] {
        assert!(lines.iter().any(|l| l.contains(expected)), "missing \"{expected}\" in {lines:#?}");
    }
}

#[test]
fn a_trade_that_opens_and_is_stopped_is_reported_by_email_with_its_account_masked() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.settings.notify_flags.notify_trade_opened_email = true;
    app.settings.notify_flags.notify_trade_stopped_email = true;
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    app.poll_tws();
    let lines = mail_lines(&app);
    assert!(lines.iter().any(|l| l.contains("\"TradePilot: Entry filled (")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("\"TradePilot: Stop filled (")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("DU1234567")), "{lines:?}");
}

#[test]
fn a_trade_closed_at_its_profit_target_is_reported_and_nothing_is_sent_while_the_switches_are_off() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 12, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.15 }).unwrap();
    app.poll_tws();
    assert!(mail_lines(&app).is_empty(), "the switches are off");
    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.settings.notify_flags.notify_trade_stopped_email = true;
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 12, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.15 }).unwrap();
    app.poll_tws();
    assert!(mail_lines(&app).iter().any(|l| l.contains("\"TradePilot: Trade closed (")), "{:?}", mail_lines(&app));
}

#[test]
fn a_filled_profit_target_closes_the_trade_with_a_gain() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 12, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.15 }).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close.map(|p| p.normalize().to_string()).as_deref()), ("Closed", Some("-0.15")));
    // (0.30 - 0.15) * 100 = 15.
    assert_eq!(app.trades_today.first().map(|r| (r.status, r.current_profit)), Some((TradeStatus::Closed, Decimal::from(15))));
}

#[test]
fn a_stop_that_dies_unfilled_is_reported_and_the_trade_stays_open() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(order_status(11, "Inactive", 0.0, 1.0)).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.first().map(|t| t.status.as_str()), Some("Open"));
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING") && l.contains("stop 11")));
}

#[test]
fn executions_are_kept_once_and_trades_are_rebuilt_from_them() {
    let (mut app, _commands) = connected_app();
    let open = "TraPi-7";
    let t1 = "20261113 21:30:05 Europe/Berlin";
    let t2 = "20261114 03:46:40 Europe/Berlin";
    for row in [
        execution("a", 7, open, 11, 7535.0, "SLD", 1.4, t1),
        execution("b", 7, open, 12, 7530.0, "BOT", 0.65, t1),
        execution("b", 7, open, 12, 7530.0, "BOT", 0.65, t1), // the same execution again
        execution("x", 0, "", 11, 7535.0, "BOT", 0.0, t2),
        execution("y", 0, "", 12, 7530.0, "SLD", 0.0, t2),
        // An expiry whose entry TWS no longer holds.
        execution("p", 0, "", 21, 7400.0, "BOT", 0.0, t2),
        execution("q", 0, "", 22, 7390.0, "SLD", 0.0, t2),
    ] {
        app.tws_sender.send(TwsEvent::Execution(row)).unwrap();
    }
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    assert_eq!(app.exec_history.len(), 6, "the repeated execution is stored once");

    let kept = app.trade_log.iter().find(|t| t.order_id == 7).expect("the opening order's trade");
    assert_eq!(
        (
            kept.strikes.as_str(),
            kept.price_open.normalize().to_string().as_str(),
            kept.status.as_str(),
            kept.price_close.map(|p| p.normalize().to_string()).as_deref()
        ),
        ("7535/7530 P", "0.75", "Expired", Some("0"))
    );
    let orphan = app.trade_log.iter().find(|t| t.entry_unknown).expect("the expiry without an entry");
    assert_eq!((orphan.strikes.as_str(), orphan.status.as_str()), ("7400/7390 P", "Expired"));
    assert_eq!(app.trade_log.len(), 2);

    // Only the trade with a known entry is counted; the other is listed, never summed.
    assert_eq!(app.trades_today.len(), 1);
    assert_eq!(app.trades_today.first().map(|r| r.current_profit), Some(Decimal::from(75)));

    // A second run (another connect) adds nothing.
    app.tws_sender.send(TwsEvent::ExecutionsEnd).unwrap();
    app.poll_tws();
    assert_eq!(app.trade_log.len(), 2);
}

#[test]
fn the_reports_count_only_the_trades_of_the_picked_period() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    assert!(app.pending.trade_log_dirty, "a new trade makes the reports reload");
    app.reload_trade_log();
    assert!(!app.pending.trade_log_dirty);

    let today = test_today();
    let days = chrono::Duration::days;
    app.reports.start = today;
    app.reports.end = today;
    assert_eq!(app.report_records().len(), 1, "today's trade");
    app.reports.start = today - days(7);
    app.reports.end = today - days(1);
    assert_eq!(app.report_records().len(), 0, "a period before the trade");

    // An older trade is found as soon as the period reaches back to its day.
    let mut older = app.trade_log.first().cloned().unwrap();
    older.day = (today - days(3)).format("%Y-%m-%d").to_string();
    older.order_id = 99;
    app.trade_log.push(older);
    app.reports.end = today;
    assert_eq!(app.report_records().len(), 2);
    app.reports.end = today - days(1);
    assert_eq!(app.report_records().len(), 1, "only the older trade");
    assert_eq!(app.report_trades().first().map(|t| t.order_id), Some(99));
}

#[test]
fn the_report_cards_and_the_csv_follow_the_picked_period() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    app.reload_trade_log();
    let today = test_today();
    app.reports.start = today;
    app.reports.end = today;

    // One put-spread credit of 0.30 x100 = $30, open and valued live at the chain quotes.
    let stats = reports::report_stats(&app.report_rows()).unwrap();
    assert_eq!((stats.puts.trades, stats.puts.premium_sold, stats.calls.trades), (1, Decimal::from(30), 0));
    assert_eq!((stats.days, stats.winning_days, stats.losing_days), (1, 0, 1), "closing costs 0.40 > 0.30 credit: a loss today");

    let csv = app.report_csv();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 2, "header + the one trade");
    assert!(lines.get(1).is_some_and(|l| l.contains("PutSpread") && l.contains(",0.3,")), "{csv}");
    assert!(!csv.contains("DU1234567"), "the account is masked");

    app.reports.start = today - chrono::Duration::days(9);
    app.reports.end = today - chrono::Duration::days(8);
    assert_eq!(app.report_csv().lines().count(), 1, "an empty period exports only the header");
}

#[test]
fn a_limit_close_joins_the_exit_orders_oca_group_and_a_fill_closes_the_trade() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.request_close(0, CloseKind::Limit);
    assert_eq!(app.close_pending.as_ref().map(|p| p.price.as_str()), Some("0.4"), "the suggestion is the current price to close: 1.95 ask - 1.55 bid");
    close_with(&mut app, CloseKind::Limit, "0.2");
    let sent = drain(&commands);
    assert!(cancelled_orders(&sent).is_empty(), "stop and target keep working until the close fills");
    let Some(TwsCommand::PlaceOrder { order_id, contract, order }) = sent.first() else { panic!("no close order was sent ({} commands)", sent.len()) };
    assert_eq!(*order_id, 14);
    assert_eq!((contract.sec_type.as_str(), contract.combo_legs.len()), ("BAG", 2), "every leg of the spread is closed");
    assert_eq!(contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), [(101, "SELL"), (100, "BUY")]);
    assert_eq!((order.action.as_str(), order.order_type.as_str(), order.lmt_price, order.total_quantity), ("BUY", "LMT", 0.2, 1.0));
    assert_eq!((order.oca_group.as_str(), order.oca_type), ("TradePilot-11", 3), "the stop's group");
    assert_eq!(app.trades_stored.first().map(|t| t.close_order_id), Some(Some(14)));

    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 14, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.2 }).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close.map(|p| p.normalize().to_string()).as_deref()), ("Closed", Some("-0.2")));
}

#[test]
fn a_market_close_sends_a_market_order() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.request_close(0, CloseKind::Market);
    assert_eq!(app.close_pending.as_ref().map(|p| p.price.as_str()), Some(""), "no price for a market order");
    app.confirm_close();
    let sent = drain(&commands);
    let Some(TwsCommand::PlaceOrder { order, .. }) = sent.first() else { panic!("no close order was sent ({} commands)", sent.len()) };
    assert_eq!((order.order_type.as_str(), order.action.as_str()), ("MKT", "BUY"));
}

#[test]
fn no_close_order_goes_out_while_the_safety_gate_is_shut_or_the_price_is_missing() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    close_with(&mut app, CloseKind::Limit, "");
    assert!(drain(&commands).is_empty(), "a limit close needs a price");
    app.kill_switch.trip();
    close_with(&mut app, CloseKind::Market, "");
    assert!(drain(&commands).is_empty(), "kill switch on");
    assert!(app.log_lines.iter().any(|l| l.contains("Blocked close")));
    assert_eq!(app.trades_stored.first().map(|t| t.status.as_str()), Some("Open"));
}

#[test]
fn mark_closed_records_the_trade_and_cancels_its_orders_even_with_the_kill_switch_on() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.kill_switch.trip();
    close_with(&mut app, CloseKind::Mark, "0.4");
    let sent = drain(&commands);
    assert_eq!(cancelled_orders(&sent), [11, 12], "cancelling only reduces risk");
    assert_eq!(placed_limits(&sent), [] as [f64; 0]);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close.map(|p| p.normalize().to_string()).as_deref()), ("ClosedOther", Some("-0.4")));
    // (0.30 - 0.40) * 100 = -10.
    assert_eq!(app.trades_today.first().map(|r| (r.status, r.current_profit)), Some((TradeStatus::ClosedOther, Decimal::from(-10))));

    // Without a price nothing is recorded.
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    close_with(&mut app, CloseKind::Mark, "abc");
    assert_eq!(app.trades_stored.first().map(|t| t.status.as_str()), Some("Open"));
    assert_eq!(cancelled_orders(&drain(&commands)), [] as [i32; 0]);
}

#[test]
fn the_fees_follow_a_new_execution_although_they_are_taken_from_an_index() {
    let (mut app, _commands) = connected_app();
    let trade = StoredTrade { short_put: Some(StoredLeg { con_id: 11, strike: "6650".into() }), ..stored_put_spread() };
    assert_eq!(app.fees_of(&trade).open, None, "no fill yet");
    let mut execution = opening_execution(11, "SLD", 1.0);
    execution.order_id = trade.order_id;
    execution.exec_id = "fee-1".into();
    app.exec_history.push(execution);
    app.commissions.insert("fee-1".into(), Decimal::new(65, 2));
    assert_eq!(app.fees_of(&trade).open, Some(Decimal::new(65, 2)), "the index is built again for the new execution");
}
