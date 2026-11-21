//! Triggers: webhook signals, EMA crossovers and closing signals.

use super::*;

#[test]
fn a_signal_starts_the_entry_of_its_trigger_and_the_trade_carries_the_trigger_and_its_strategy() {
    let (mut app, commands) = app_with_a_trigger();
    app.process_signal(&[1], &[]);
    let run = app.schedule.runs.iter().find(|run| run.trigger.is_some()).cloned().expect("a run is created");
    assert_eq!((run.status, run.schedule_id), (RunStatus::Waiting, 0));
    assert!(app.log_lines.iter().any(|l| l.contains("Trigger 1: processed")), "{:?}", app.log_lines);
    // The run enters like a schedule's run does.
    drain(&commands);
    app.schedule.runner_checked_at = None;
    app.run_schedules();
    assert_eq!(placed_orders(&drain(&commands)).len(), 1, "{:?}", app.log_lines);
    let order_id = app.template.orders.iter().find(|o| o.run_id == Some(run.id)).map(|o| o.order_id).unwrap();
    app.tws_sender.send(TwsEvent::OrderStatus { order_id, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.iter().find(|t| t.order_id == order_id).expect("the trade exists");
    assert_eq!((trade.strategy.as_str(), trade.origin.as_ref().map(|o| o.trigger)), ("From a hook", Some(1)));
    // A second signal the same day: the trigger may start one entry a day.
    app.process_signal(&[1], &[]);
    assert!(app.log_lines.iter().any(|l| l.contains("Trigger 1: skipped") && l.contains("maximum of 1 entries")), "{:?}", app.log_lines);
    assert_eq!(app.schedule.runs.iter().filter(|r| r.trigger.is_some()).count(), 1);
}

#[test]
fn an_ema_trigger_starts_its_entry_on_a_new_bar_that_crosses_but_not_on_the_history() {
    let (mut app, _commands) = app_with_a_trigger();
    app.triggers.list[0].entry = "EMA5 Cross Over EMA20".into();
    // 60 bars that fall: the fast average is below the slow one. The history arrives in one batch.
    let mut closes: Vec<f64> = (0..60).map(|n| 7300.0 - f64::from(n)).collect();
    app.feed_test_bars("min:SPX", minute_bars(&closes, 0));
    app.check_ema_triggers();
    // The bars that follow come in one at a time: a rise lifts the fast average over the slow one.
    let mut fired = false;
    for step in 0..12 {
        closes.push(7250.0 + f64::from(step) * 8.0);
        app.feed_test_bars("min:SPX", minute_bars(&closes, 0));
        app.check_ema_triggers();
        if app.schedule.runs.iter().any(|run| run.trigger.is_some()) {
            fired = true;
            break;
        }
    }
    assert!(fired, "the crossover starts the entry: {:?}", app.log_lines);
    assert_eq!(app.schedule.runs.iter().filter(|run| run.trigger.is_some()).count(), 1);
    assert!(app.log_lines.iter().any(|l| l.contains("Trigger 1: EMA5 Cross Over EMA20 at the bar of")), "{:?}", app.log_lines);
    // The same bar is not looked at twice.
    app.check_ema_triggers();
    assert_eq!(app.schedule.runs.iter().filter(|run| run.trigger.is_some()).count(), 1);
}

#[test]
fn an_ema_trigger_ignores_a_crossover_in_the_history_that_streams_in_and_a_webhook_trigger_is_not_watched() {
    let (mut app, _commands) = app_with_a_trigger();
    app.triggers.list[0].entry = "EMA5 Cross Over EMA20".into();
    let mut closes: Vec<f64> = (0..60).map(|n| 7300.0 - f64::from(n)).collect();
    app.feed_test_bars("min:SPX", minute_bars(&closes, 0));
    app.check_ema_triggers();
    // A batch of many bars, with the crossover somewhere in it, arrives at once: that is history.
    closes.extend((0..30).map(|n| 7240.0 + f64::from(n) * 10.0));
    app.feed_test_bars("min:SPX", minute_bars(&closes, 0));
    app.check_ema_triggers();
    assert_eq!(app.schedule.runs, [], "nothing starts: {:?}", app.log_lines);
    // A webhook trigger does not look at bars at all.
    app.triggers.list[0].entry = "Webhook".into();
    app.triggers.ema_seen.clear();
    app.check_ema_triggers();
    assert!(app.triggers.ema_seen.is_empty());
}

#[test]
fn a_signal_is_ignored_outside_the_window_for_an_unknown_or_switched_off_trigger_and_when_the_trades_are_open() {
    let (mut app, _commands) = app_with_a_trigger();
    app.process_signal(&[9], &[]);
    assert!(app.log_lines.iter().any(|l| l.contains("Trigger 9: skipped - there is no such trigger")));
    app.triggers.list[0].base.active = false;
    app.process_signal(&[1], &[]);
    assert!(app.log_lines.iter().any(|l| l.contains("Trigger 1: skipped - the trigger is switched off")));
    app.triggers.list[0].base.active = true;
    app.triggers.list[0].end_hour = 10;
    app.triggers.list[0].end_minute = 30;
    app.process_signal(&[1], &[]);
    assert!(app.log_lines.iter().any(|l| l.contains("not within the time window")), "11:00 is after the window end");
    assert_eq!(app.schedule.runs, []);
    // An open trade of the trigger holds the next signal back.
    app.triggers.list[0].end_hour = 15;
    app.triggers.list[0].max_open = 1;
    app.triggers.list[0].max_executions = 0;
    app.trades_stored.push(StoredTrade {
        account: "DU1234567".into(),
        origin: Some(crate::storage::TradeOrigin { template_id: 4, root: 1, entries: 0, window_minutes: 5, trigger: 1, ratio_open: None }),
        ..stored_put_spread()
    });
    app.process_signal(&[1], &[]);
    assert!(app.log_lines.iter().any(|l| l.contains("1 trades of the trigger are open already")), "{:?}", app.log_lines);
    assert_eq!(app.schedule.runs, []);
    // The daily profit target stops every signal.
    app.triggers.list[0].max_open = 0;
    app.settings.daily_profit_target = "100".to_string();
    app.settings.daily_profit_target_delay_seconds = "0".to_string();
    app.trades_today = vec![TradeRecord::opened("SPX", Decimal::ONE, 1, 100, 0.0, Some(Decimal::from(500))).unwrap()];
    app.watch.checked = None;
    app.run_watchers();
    app.process_signal(&[1], &[]);
    assert_eq!(app.schedule.runs, []);
}

#[test]
fn a_close_signal_closes_the_open_trades_of_its_trigger_only() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.process_signal(&[], &[1]);
    assert_eq!(app.trades_stored.first().unwrap().timed_exit, None, "the trade is not from trigger 1");
    assert!(app.log_lines.iter().any(|l| l.contains("no open trade of these triggers")));
    app.trades_stored.first_mut().unwrap().origin =
        Some(crate::storage::TradeOrigin { template_id: 4, root: 10, entries: 0, window_minutes: 5, trigger: 1, ratio_open: None });
    app.process_signal(&[], &[1]);
    let exit = app.trades_stored.first().unwrap().timed_exit.clone().expect("the exit is due");
    assert_eq!(exit.at, "2026-11-16 11:00");
    app.check_timed_exits(app.clock.new_york());
    assert_eq!(placed_orders(&drain(&commands)).iter().map(|(_, order)| order.order_type.clone()).collect::<Vec<_>>(), ["MKT"]);
}

#[test]
fn the_receiver_follows_the_switch_and_a_request_with_the_token_starts_the_trigger() {
    use std::io::{Read, Write};
    let (mut app, _commands) = app_with_a_trigger();
    app.triggers.webhook_now = Some(|| 1_700_000_000);
    app.sync_webhook_server();
    assert!(app.triggers.server.is_none(), "off by default");
    app.settings.mail_flags.webhook_triggers_enabled = true;
    app.sync_webhook_server();
    let (port, config) = app.triggers.server.as_ref().map(|(server, _, config)| (server.port(), config.clone())).expect("the receiver runs");
    let body = format!(
        "{{\"Token\": \"{}\", \"Timestamp\": \"{}\", \"Instance\": \"{}\", \"TriggerID\": [1]}}",
        config.token,
        chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap().format("%Y-%m-%dT%H:%M:%SZ"),
        config.instance
    );
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "POST /signal HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    // The signal is taken up in the next round of the app.
    let mut taken = false;
    for _ in 0..50 {
        app.poll_webhook();
        taken = app.schedule.runs.iter().any(|run| run.trigger.is_some());
        if taken {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(taken, "{:?}", app.log_lines);
    assert!(app.log_lines.iter().any(|l| l.contains("Webhook signal received: TriggerID [1]")));
    // A new token replaces the old one; switched off, the receiver stops.
    app.regenerate_webhook_token();
    let renewed = app.triggers.server.as_ref().map(|(_, _, c)| c.token.clone()).unwrap();
    assert_ne!(renewed, config.token);
    app.settings.mail_flags.webhook_triggers_enabled = false;
    app.sync_webhook_server();
    assert!(app.triggers.server.is_none());
}
