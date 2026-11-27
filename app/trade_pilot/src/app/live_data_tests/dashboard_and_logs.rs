//! The log, the list of trades, the SPX curve and the top bar.

use super::*;

#[test]
fn the_text_of_the_logs_page_is_built_again_only_after_a_new_line() {
    let (mut app, _commands) = connected_app();
    app.ui_state.log_view = Some("old".into());
    app.log("a new line");
    assert_eq!(app.ui_state.log_view, None, "a new line makes it stale");
    app.ui_state.log_view = Some("old".into());
    app.clear_logs();
    assert_eq!(app.ui_state.log_view, None, "so does clearing");
}

#[test]
fn the_log_can_be_cleared() {
    let (mut app, _commands) = connected_app();
    app.log("something happened");
    assert!(!app.log_lines.is_empty());
    app.clear_logs();
    assert!(app.log_lines.is_empty());
}

#[test]
fn the_options_of_open_trades_with_exit_orders_are_marked_with_the_side_of_those_orders() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let sides = app.working_exit_sides("");
    let trade = app.trades_stored.first().unwrap();
    let (short, long) = (trade.short_put.as_ref().unwrap().con_id, trade.long_put.as_ref().unwrap().con_id);
    assert_eq!(sides.get(&short), Some(&trade_entry::WorkingSide::Buy), "the short put is bought back");
    assert_eq!(sides.get(&long), Some(&trade_entry::WorkingSide::Sell), "the long put is sold");
    assert!(app.working_exit_sides("another account").is_empty());
}

#[test]
fn the_spx_day_curve_comes_from_one_minute_bars_and_the_forming_bar_replaces_its_point() {
    let (mut app, commands) = connected_app();
    app.request_live_data_once();
    let asked: Vec<(i32, String)> = drain(&commands)
        .iter()
        .filter_map(|c| if let TwsCommand::RequestLiveBars { req_id, contract } = c { Some((*req_id, contract.symbol.clone())) } else { None })
        .collect();
    assert_eq!(asked.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["SPX"], "only SPX has a curve");
    let id = asked[0].0;
    for (date, close) in [("20261117 15:30:00", 7400.0), ("20261117 15:31:00", 7402.0), ("20261117 15:31:00", 7403.5)] {
        app.tws_sender.send(TwsEvent::HistoricalBar { req_id: id, date: date.into(), high: close, low: close, close }).unwrap();
    }
    app.poll_tws();
    let spx = quote(&app, "SPX");
    assert_eq!(spx.curve, [("20261117 15:30:00".to_string(), 7400.0), ("20261117 15:31:00".to_string(), 7403.5)]);
    assert_eq!(spx.last, None, "a bar is never taken for the price SPX is traded on");
    assert_eq!(percent_change(7425.0, 7500.0).map(|p| (p * 100.0).round()), Some(-100.0));
    assert_eq!(percent_change(7425.0, 0.0), None);
}

#[test]
fn the_trade_list_shows_the_stop_and_the_profit_target_and_the_stop_as_adjusted() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let row = &app.trade_row_views()[0];
    let trade = app.trades_stored.first().unwrap().clone();
    assert_eq!(row.stop, "0.90", "the stop that was placed");
    assert_eq!(row.target, trade.profit_price.map(|p| format!("{p:.2}")).unwrap_or_default(), "the profit target");
    assert_ne!(row.target, "-");
    // Once the adjustments moved the stop, the list shows the moved one.
    let mut moved = trade;
    moved.adjustments = Some(StopAdjustments { stop: Some("1.20".into()), ..StopAdjustments::default() });
    app.trades_stored[0] = moved;
    assert_eq!(app.trade_row_views()[0].stop, "1.20");
}

#[test]
fn a_stop_that_fills_far_beyond_its_price_and_a_missing_stop_are_reported_when_the_switches_are_on() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.settings.alert_flags.high_slippage_notifications_enabled = true;
    app.settings.alert_flags.missing_stop_notifications_enabled = true;
    app.settings.high_slippage_pct = "50".to_string();
    filled_trade_with_exits(&mut app, &commands);
    // The stop is 0.90; it filled at 1.80: 100 % beyond it.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 1.8 }).unwrap();
    app.poll_tws();
    assert!(mail_lines(&app).iter().any(|l| l.contains("\"TradePilot: ALERT: Stop filled with high slippage (")), "{:?}", mail_lines(&app));

    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.settings.alert_flags.high_slippage_notifications_enabled = true;
    app.settings.high_slippage_pct = "150".to_string();
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 1.8 }).unwrap();
    app.poll_tws();
    assert!(!mail_lines(&app).iter().any(|l| l.contains("High Slippage")), "below the limit of 150 %");

    // The stop that was wanted but is gone in TWS after a connect.
    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.settings.alert_flags.missing_stop_notifications_enabled = true;
    filled_trade_with_exits(&mut app, &commands);
    app.reconcile_open_orders(&[12]);
    app.poll_tws();
    assert!(mail_lines(&app).iter().any(|l| l.contains("\"TradePilot: ALERT: Trade without a stop\"")), "{:?}", mail_lines(&app));
}

#[test]
fn the_top_bar_names_when_the_market_closes_only_while_it_is_open() {
    let (mut app, _commands) = connected_app();
    let text = app.market_close_text().expect("Monday 11:00 New York is open");
    assert!(text.starts_with("Closes ") && text.ends_with("(in 5h 00m 00s)"), "{text}");
    app.clock = Box::new(crate::clock::TestClock::at_new_york(chrono::NaiveDate::from_ymd_opt(2026, 11, 27).unwrap().and_hms_opt(12, 30, 0).unwrap()));
    assert!(app.market_close_text().is_some_and(|t| t.contains("(in 30m 00s, early close)")), "{:?}", app.market_close_text());
    app.clock = Box::new(crate::clock::TestClock::at_new_york(chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(18, 0, 0).unwrap()));
    assert_eq!(app.market_close_text(), None);
}
