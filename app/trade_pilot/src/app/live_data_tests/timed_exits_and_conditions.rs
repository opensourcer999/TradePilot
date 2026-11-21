//! Timed exits, the cancel time of the profit target, early exits and the bars the conditions ask for.

use super::*;

#[test]
fn a_waiting_run_shows_the_time_to_its_start_as_hours_minutes_seconds() {
    let (app, _commands) = connected_app();
    let now = app.clock.utc();
    let mut run = ScheduleRun {
        id: 1,
        schedule_id: 1,
        day: app.clock.new_york().date(),
        window_start: now + chrono::Duration::seconds(3 * 3600 + 25 * 60 + 7),
        window_end: now + chrono::Duration::hours(5),
        status: RunStatus::Waiting,
        attempts: 0,
        started: None,
        finished: None,
        notes: String::new(),
        reentry: None,
        trigger: None,
    };
    assert_eq!(App::run_countdown(&run, now).as_deref(), Some("03:25:07"));
    run.window_start = now - chrono::Duration::seconds(1);
    assert_eq!(App::run_countdown(&run, now), None, "it has started");
    run.window_start = now + chrono::Duration::seconds(60);
    run.status = RunStatus::Paused;
    assert_eq!(App::run_countdown(&run, now), None, "only a waiting run counts down");
}

#[test]
fn the_profit_target_is_cancelled_at_the_time_of_its_template_and_the_stop_stays() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    {
        let template = &mut app.template.orders.first_mut().unwrap().template;
        template.profit_target_expiration_hour = 11;
        template.profit_target_expiration_minute = 45;
    }
    let sent = filled_trade_with_exits(&mut app, &commands);
    let profit_id = sent
        .iter()
        .filter_map(|c| if let TwsCommand::PlaceOrder { order_id, order, .. } = c { Some((*order_id, order.order_type.clone())) } else { None })
        .find(|(_, kind)| kind == "LMT")
        .unwrap()
        .0;
    let stored = app.trades_stored.first().unwrap().profit_cancel.clone().expect("the cancel time is stored");
    assert!(stored.ends_with(" 11:45"), "{stored}");
    // The stored day is the expiry day; the test clock stands on the day of the fill.
    app.trades_stored.first_mut().unwrap().profit_cancel = Some("2026-11-16 11:45".to_string());
    app.check_profit_target_cancels(app.clock.new_york());
    assert!(drain(&commands).is_empty(), "11:00: not yet");
    at_new_york(&mut app, 11, 45, 0);
    app.check_profit_target_cancels(app.clock.new_york());
    let cancelled: Vec<i32> =
        drain(&commands).iter().filter_map(|c| if let TwsCommand::CancelOrder { order_id } = c { Some(*order_id) } else { None }).collect();
    assert_eq!(cancelled, [profit_id], "only the profit target");
    assert_eq!(app.trades_stored.first().unwrap().profit_cancel, None);
}

#[test]
fn a_true_early_exit_condition_closes_the_trade_and_a_false_one_does_not() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.templates.first_mut().unwrap().exit_condition_id = 1;
    filled_trade_with_exits(&mut app, &commands);
    // The condition is false: closing costs about 0.30 (the chain's asks), not more than 100.
    app.conditions.list = vec![early_exit_condition(100.0)];
    app.check_early_exits(app.clock.new_york());
    assert_eq!(app.trades_stored.first().unwrap().timed_exit, None);
    // True: the exit starts at once, as a market order (the template has no limit switch).
    app.conditions.list = vec![early_exit_condition(0.0)];
    app.watch.exit_checked = None;
    app.check_early_exits(app.clock.new_york());
    let exit = app.trades_stored.first().unwrap().timed_exit.clone().expect("the exit is due");
    assert_eq!((exit.at.as_str(), exit.limit), ("2026-11-16 11:00", false));
    assert!(app.log_lines.iter().any(|l| l.contains("Early exit of SPX PutSpread") && l.contains("Cost to close")), "{:?}", app.log_lines);
    app.check_timed_exits(app.clock.new_york());
    let sent = placed_orders(&drain(&commands));
    assert_eq!(sent.iter().map(|(_, order)| order.order_type.as_str()).collect::<Vec<_>>(), ["MKT"]);
}

#[test]
fn no_early_exit_is_checked_in_the_first_two_minutes_or_for_a_trade_whose_template_has_no_condition() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.conditions.list = vec![early_exit_condition(0.0)];
    // No condition on the template.
    app.check_early_exits(app.clock.new_york());
    assert_eq!(app.trades_stored.first().unwrap().timed_exit, None);
    // A condition, but 09:31 New York.
    app.templates.first_mut().unwrap().exit_condition_id = 1;
    at_new_york(&mut app, 9, 31, 0);
    app.watch.exit_checked = None;
    app.check_early_exits(app.clock.new_york());
    assert_eq!(app.trades_stored.first().unwrap().timed_exit, None);
}

#[test]
fn a_rule_on_bars_asks_for_them_once_is_unknown_until_they_come_and_then_is_decided() {
    let (mut app, commands) = connected_app();
    let condition = ema_condition(3, "4");
    // The first look asks for the one-minute bars of the underlying and cannot decide.
    let snapshot = app.with_bars(app.condition_snapshot(), &condition, "SPX");
    assert_eq!(crate::condition::evaluate(&condition, &snapshot).verdict, crate::condition::Verdict::Retry);
    let asked = indicator_requests(&drain(&commands));
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!((asked[0].1.as_str(), asked[0].2.as_str(), asked[0].3.as_str()), ("SPX", "1 min", "4 D"));
    // The next look does not ask again.
    app.clock = Box::new(crate::clock::TestClock::at_new_york(chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(11, 0, 5).unwrap()));
    let _ = app.with_bars(app.condition_snapshot(), &condition, "SPX");
    assert_eq!(indicator_requests(&drain(&commands)), []);
    // The bars come: closes 1..=6 and the forming bar.
    for (i, close) in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 100.0].into_iter().enumerate() {
        app.tws_sender.send(TwsEvent::HistoricalBar { req_id: asked[0].0, date: format!("20261116 10:{i:02}:00"), high: close, low: close, close }).unwrap();
    }
    app.poll_tws();
    let snapshot = app.with_bars(app.condition_snapshot(), &condition, "SPX");
    let evaluation = crate::condition::evaluate(&condition, &snapshot);
    assert_eq!(evaluation.verdict, crate::condition::Verdict::Pass, "{:?}", evaluation.lines);
    assert!(evaluation.lines.iter().any(|l| l.contains("EMA(3) 5 > 4")), "{:?}", evaluation.lines);
    // Another underlying has bars of its own.
    let _ = app.with_bars(app.condition_snapshot(), &condition, "XSP");
    assert_eq!(indicator_requests(&drain(&commands)).iter().map(|r| r.1.clone()).collect::<Vec<_>>(), ["XSP"]);
}

#[test]
fn the_vix_rank_asks_for_the_days_its_length_needs_and_the_daily_values_for_the_spx() {
    use crate::condition::{Comparison, Condition, Input, Operator, Rule};
    let (mut app, commands) = connected_app();
    let rule = |input, input_length| Rule {
        group: 1,
        input,
        operator: Operator::Greater,
        comparison: Comparison::Value(rust_decimal::Decimal::ONE),
        input_length,
        comparison_length: 0,
    };
    let condition = Condition { id: 2, name: "Daily".into(), retry_until_expiration: false, rules: vec![rule(Input::VixRank, 1000), rule(Input::Rsi, 0)] };
    let _ = app.with_bars(app.condition_snapshot(), &condition, "SPX");
    let asked = indicator_requests(&drain(&commands));
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(asked.iter().any(|r| r.1 == "VIX" && r.2 == "1 min" && r.3 == "5 D"), "1000 bars = 2 sessions + 3 days: {asked:?}");
    assert!(asked.iter().any(|r| r.1 == "SPX" && r.2 == "1 day" && r.3 == "365 D"), "{asked:?}");
}

#[test]
fn a_stream_that_has_been_silent_for_six_minutes_is_asked_for_again_and_a_new_session_forgets_the_bars() {
    let (mut app, commands) = connected_app();
    let condition = ema_condition(3, "4");
    let _ = app.with_bars(app.condition_snapshot(), &condition, "SPX");
    let first = indicator_requests(&drain(&commands));
    app.indicators.age(Duration::from_secs(400));
    let _ = app.with_bars(app.condition_snapshot(), &condition, "SPX");
    let again = indicator_requests(&drain(&commands));
    assert_eq!(again.len(), 1, "silent for more than six minutes");
    assert_ne!(first[0].0, again[0].0, "a new request id");
    app.reset_session();
    assert_eq!(app.indicators.series_len(), 0);
}
