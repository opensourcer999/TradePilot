//! Deleting and attaching exits, re-entries, limit exits and changes TWS refuses.

use super::*;

#[test]
fn a_stop_deleted_by_the_person_is_cancelled_and_forgotten_without_a_warning_or_a_mail() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.settings.alert_flags.missing_stop_notifications_enabled = true;
    filled_trade_with_exits(&mut app, &commands);
    app.delete_exit_orders(0, true);
    assert_eq!(cancelled_orders(&drain(&commands)), [11], "only the stop");
    app.tws_sender.send(order_status(11, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_price, trade.profit_order_id.is_some()), (None, None, true), "the target stays");
    assert!(app.log_lines.iter().any(|l| l.contains("The stop 11") && l.contains("as asked")), "{:?}", app.log_lines);
    assert!(!app.log_lines.iter().any(|l| l.contains("WARNING")), "{:?}", app.log_lines);
    assert!(mail_lines(&app).is_empty(), "{:?}", mail_lines(&app));
    // The same cancel from TWS without having asked for it is still a warning.
    let (mut other, other_commands) = app_with_a_sent_template_order(true);
    other.settings.alert_flags.missing_stop_notifications_enabled = true;
    filled_trade_with_exits(&mut other, &other_commands);
    other.tws_sender.send(order_status(11, "Cancelled", 0.0, 1.0)).unwrap();
    other.poll_tws();
    assert!(other.log_lines.iter().any(|l| l.contains("WARNING: the stop 11")), "{:?}", other.log_lines);
}

#[test]
fn a_profit_target_deleted_by_the_person_leaves_the_stop_and_asking_for_a_missing_one_does_nothing() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.delete_exit_orders(0, false);
    assert_eq!(cancelled_orders(&drain(&commands)), [12]);
    app.tws_sender.send(order_status(12, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.profit_order_id, trade.profit_price, trade.stop_order_id), (None, None, Some(11)));
    assert!(!app.log_lines.iter().any(|l| l.contains("WARNING")), "{:?}", app.log_lines);
    // There is no target any more: nothing is sent.
    app.delete_exit_orders(0, false);
    assert_eq!(cancelled_orders(&drain(&commands)), [] as [i32; 0]);
    assert!(app.log_lines.iter().any(|l| l.contains("Delete profit target") && l.contains("the trade has none")));
}

#[test]
fn a_trade_keeps_the_ratio_of_its_entry_and_the_conditions_see_the_deltas_of_the_open_trades() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let origin = app.trades_stored.first().unwrap().origin.clone().expect("origin");
    // Short 6690 ask 1.95 over long 6680 ask 1.65.
    let ratio: Decimal = origin.ratio_open.as_deref().expect("the ratio of the entry is kept").parse().unwrap();
    assert_eq!(ratio.round_dp(4), Decimal::new(11818, 4));
    // The snapshot of the open trade carries its account and the ratio change.
    let trade = app.trades_stored.first().cloned().unwrap();
    let snapshot = app.trade_snapshot(&trade);
    assert_eq!(snapshot.account, "DU1234567");
    assert_eq!(snapshot.value_with(crate::condition::Input::TradeShortLongRatioChangePct, 0).map(|v| v.round_dp(0)), Some(Decimal::ZERO), "unchanged quotes");
    // A schedule condition on the total delta is checked with the account of the schedule.
    let condition = crate::condition::Condition {
        id: 5,
        name: "Flat".into(),
        retry_until_expiration: false,
        rules: vec![crate::condition::Rule {
            group: 1,
            input: crate::condition::Input::DeltaAccount,
            operator: crate::condition::Operator::Greater,
            comparison: crate::condition::Comparison::Value(Decimal::from(-1000)),
            input_length: 0,
            comparison_length: 0,
        }],
    };
    // Only a condition that uses a delta total values every open trade for it.
    let with_totals = app.with_bars(app.trade_snapshot(&trade), &condition, "SPX");
    assert!(with_totals.open_deltas.iter().any(|open| open.symbol == "SPX" && open.account == "DU1234567"), "{:?}", with_totals.open_deltas);
    let on_vix =
        crate::condition::Condition { rules: vec![crate::condition::Rule { input: crate::condition::Input::Vix, ..condition.rules[0] }], ..condition.clone() };
    assert_eq!(app.with_bars(app.trade_snapshot(&trade), &on_vix, "SPX").open_deltas.len(), 0);
    app.conditions.list = vec![condition];
    assert_eq!(app.check_schedule_condition(5, 1, "SPX", "DU1234567"), Verdict::Pass, "{:?}", app.log_lines);
}

#[test]
fn long_reuse_only_books_a_long_trade_closed_at_zero_and_leaves_other_trades_alone() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let mut template = app.templates.first().cloned().unwrap();
    // Not switched on: nothing changes.
    assert_eq!(app.mark_long_reuse_trades(vec![0], &template), [0]);
    // Switched on, but a spread is no long trade.
    template.long_flags.long_reuse_only = true;
    assert_eq!(app.mark_long_reuse_trades(vec![0], &template), [0]);
    assert!(app.trades_stored.first().unwrap().is_open());
    // A long trade is booked closed at 0 and gets no exits.
    app.trades_stored.first_mut().unwrap().trade_type = "LongPut".into();
    assert_eq!(app.mark_long_reuse_trades(vec![0], &template), [] as [usize; 0]);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close), ("ClosedOther", Some(Decimal::ZERO)));
    assert!(app.log_lines.iter().any(|l| l.contains("Long Reuse Only - marked trade closed")), "{:?}", app.log_lines);
}

#[test]
fn all_or_none_of_the_template_is_set_on_the_opening_order_and_kept_by_the_fill_attempts() {
    let (mut app, commands) = app_with_a_chain_and_a_template(&next_expiry().1, next_expiry().0);
    app.templates.first_mut().unwrap().order_flags.all_or_none = true;
    app.instant.template_id = Some(4);
    app.instant.template_qty = "1".into();
    app.instant.account = "DU1234567".into();
    app.template_order_flags.instant_template_transmit = true;
    app.enter_template_trade();
    app.confirm_template_order();
    let sent = drain(&commands);
    let orders: Vec<&IbOrder> = sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { order, .. } = c { Some(&**order) } else { None }).collect();
    assert_eq!(orders.len(), 1);
    assert!(orders[0].fill.all_or_none);
    // Without the switch the order is a plain one.
    let (_plain, plain_commands) = app_with_a_sent_template_order(true);
    let sent = drain(&plain_commands);
    assert!(sent.iter().any(|c| matches!(c, TwsCommand::PlaceOrder { order, .. } if !order.fill.all_or_none)));
}

#[test]
fn a_stopped_trade_of_a_template_with_re_entry_schedules_one_after_the_delay() {
    let (mut app, _commands) = filled_trade_that_re_enters();
    let origin = app.trades_stored.first().unwrap().origin.clone().expect("the origin is stored");
    assert_eq!((origin.template_id, origin.root, origin.entries), (4, 10, 0));
    stop_fills(&mut app);
    let runs: Vec<&ScheduleRun> = app.schedule.runs.iter().filter(|run| run.reentry.is_some()).collect();
    assert_eq!(runs.len(), 1, "{:?}", app.log_lines);
    let run = runs[0];
    let reentry = run.reentry.as_ref().unwrap();
    assert_eq!((reentry.schedule.template_id, reentry.root, reentry.entries, reentry.parent_order), (4, 10, 1, 10));
    assert_eq!(run.window_start - app.clock.utc(), chrono::Duration::minutes(3), "after the delay");
    assert_eq!(run.status, RunStatus::Waiting);
    assert!(app.log_lines.iter().any(|l| l.contains("re-entry 1 after the stop")), "{:?}", app.log_lines);
    // A second look at the same entry order schedules nothing more.
    let trade = app.trades_stored.first().cloned().unwrap();
    app.schedule_reentries(&trade, TradeStatus::Stopped, "stop 11");
    assert_eq!(app.schedule.runs.iter().filter(|run| run.reentry.is_some()).count(), 1);
}

#[test]
fn no_re_entry_follows_an_ending_the_template_does_not_ask_for_a_max_chain_a_late_deadline_or_the_daily_target() {
    // The profit target and a close are not switched on.
    let (mut app, _commands) = filled_trade_that_re_enters();
    let trade = app.trades_stored.first().cloned().unwrap();
    app.schedule_reentries(&trade, TradeStatus::Closed, "profit target 12");
    app.schedule_reentries(&trade, TradeStatus::Closed, "close order 13");
    app.schedule_reentries(&trade, TradeStatus::ClosedOther, "Mark Closed");
    assert_eq!(app.schedule.runs, []);
    // The chain is at its maximum.
    app.trades_stored.first_mut().unwrap().origin.as_mut().unwrap().entries = 2;
    let trade = app.trades_stored.first().cloned().unwrap();
    app.schedule_reentries(&trade, TradeStatus::Stopped, "stop 11");
    assert_eq!(app.schedule.runs, []);
    assert!(app.log_lines.iter().any(|l| l.contains("maximum of 2 re-entries")), "{:?}", app.log_lines);
    // The deadline (11:02) is before the re-entry (11:03).
    app.trades_stored.first_mut().unwrap().origin.as_mut().unwrap().entries = 0;
    app.templates.first_mut().unwrap().reentry_expiration_hour = 11;
    app.templates.first_mut().unwrap().reentry_expiration_minute = 2;
    let trade = app.trades_stored.first().cloned().unwrap();
    app.schedule_reentries(&trade, TradeStatus::Stopped, "stop 11");
    assert_eq!(app.schedule.runs, []);
    assert!(app.log_lines.iter().any(|l| l.contains("after the deadline 11:02")), "{:?}", app.log_lines);
}

#[test]
fn the_templates_of_a_re_entry_are_the_ones_chosen_for_the_ending_and_the_second_one_enters_too() {
    let (mut app, _commands) = filled_trade_that_re_enters();
    {
        let template = app.templates.first_mut().unwrap();
        template.reentry_stop_template_id = 4;
        template.reentry_stop_template_id2 = 8;
    }
    stop_fills(&mut app);
    let ids: Vec<i32> = app.schedule.runs.iter().filter_map(|run| run.reentry.as_ref().map(|r| r.schedule.template_id)).collect();
    assert_eq!(ids, [4, 8]);
}

#[test]
fn a_re_entry_run_enters_its_trade_and_the_new_trade_knows_its_place_in_the_chain() {
    let (mut app, commands) = filled_trade_that_re_enters();
    app.templates.first_mut().unwrap().reentry_delay = 0;
    stop_fills(&mut app);
    drain(&commands);
    app.schedule.runner_checked_at = None;
    app.run_schedules();
    let sent = drain(&commands);
    assert_eq!(placed_orders(&sent).len(), 1, "{:?} {:?}", app.log_lines, app.schedule.runs);
    let run_id = app.schedule.runs.iter().find(|run| run.reentry.is_some()).map(|run| run.id).unwrap();
    let order_id = app.template.orders.iter().find(|o| o.run_id == Some(run_id)).map(|o| o.order_id).unwrap();
    app.tws_sender.send(TwsEvent::OrderStatus { order_id, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    let again = app.trades_stored.iter().find(|t| t.order_id == order_id).expect("the re-entry is a trade");
    let origin = again.origin.clone().unwrap();
    assert_eq!((origin.root, origin.entries), (10, 1), "same chain, one re-entry made");
    assert_eq!(app.schedule.runs.iter().find(|run| run.id == run_id).map(|run| run.status), Some(RunStatus::Complete));
}

#[test]
fn a_trade_is_closed_at_market_at_the_exit_time_of_its_template() {
    let (mut app, commands) = filled_trade_with_a_timed_exit(false);
    assert_eq!(app.trades_stored.first().unwrap().timed_exit.as_ref().map(|e| e.at.as_str()), Some("2026-11-16 11:30"));
    app.check_timed_exits(app.clock.new_york());
    assert!(drain(&commands).is_empty(), "11:00: not yet");
    at_new_york(&mut app, 11, 30, 5);
    app.check_timed_exits(app.clock.new_york());
    let sent = drain(&commands);
    let closes: Vec<&IbOrder> = sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { order, .. } = c { Some(&**order) } else { None }).collect();
    assert_eq!(closes.len(), 1, "one close order");
    assert_eq!((closes[0].order_type.as_str(), closes[0].action.as_str(), closes[0].total_quantity), ("MKT", "BUY", 1.0));
    assert_eq!(closes[0].oca_type, 3, "it joins the group of the exit orders");
    assert!(app.trades_stored.first().unwrap().close_order_id.is_some());
    // Said once: the close is on its way.
    app.watch.timed_tried.clear();
    app.check_timed_exits(app.clock.new_york());
    assert!(drain(&commands).is_empty());
}

#[test]
fn a_limit_exit_tries_a_limit_order_first_and_a_market_order_when_it_does_not_fill() {
    let (mut app, commands) = filled_trade_with_a_timed_exit(true);
    at_new_york(&mut app, 11, 31, 0);
    app.check_timed_exits(app.clock.new_york());
    let sent = orders_sent_now(&commands);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1.order_type, "LMT", "the chain is quoted: a limit order at the ask");
    assert!(sent[0].1.lmt_price > 0.0);
    let limit_id = sent[0].0;
    // It does not fill within two minutes: it is cancelled ...
    let long_ago = app.clock.instant().checked_sub(Duration::from_secs(150)).unwrap();
    app.watch.timed_limit_sent.insert(limit_id, long_ago);
    app.check_timed_exits(app.clock.new_york());
    assert!(drain(&commands).iter().any(|c| matches!(c, TwsCommand::CancelOrder { order_id } if *order_id == limit_id)), "the limit order is cancelled");
    // ... and once TWS confirms, a market order goes out.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: limit_id, status: "Cancelled".into(), filled: 0.0, remaining: 1.0, avg_fill_price: 0.0 }).unwrap();
    app.poll_tws();
    app.watch.timed_tried.clear();
    app.check_timed_exits(app.clock.new_york());
    let sent = orders_sent_now(&commands);
    assert_eq!(sent.iter().map(|(_, order)| order.order_type.as_str()).collect::<Vec<_>>(), ["MKT"]);
}

#[test]
fn a_limit_exit_starts_in_the_middle_and_moves_towards_the_ask_every_three_seconds() {
    let (mut app, commands) = filled_trade_with_a_timed_exit(true);
    at_new_york(&mut app, 11, 31, 0);
    app.check_timed_exits(app.clock.new_york());
    let sent = orders_sent_now(&commands);
    assert_eq!(sent.len(), 1);
    // The spread costs 0.40 to close (ask) and brings 0.20 (bid): the middle is 0.30.
    assert_eq!((sent[0].1.order_type.as_str(), sent[0].1.lmt_price), ("LMT", 0.3));
    let id = sent[0].0;
    let trade_id = app.trades_stored.first().unwrap().order_id;
    // Not three seconds yet: nothing moves.
    app.check_timed_exits(app.clock.new_york());
    assert!(orders_sent_now(&commands).is_empty());
    // Three seconds later the same order is sent again, 0.05 higher.
    let back = |app: &mut App| {
        let moved = app.watch.progressive_close.get_mut(&trade_id).unwrap();
        moved.moved = moved.moved.checked_sub(Duration::from_secs(4)).unwrap();
    };
    back(&mut app);
    app.check_timed_exits(app.clock.new_york());
    let moved = orders_sent_now(&commands);
    assert_eq!(moved.iter().map(|(order_id, order)| (*order_id, order.lmt_price)).collect::<Vec<_>>(), [(id, 0.35)]);
    // The next step reaches the ask (0.40), the one after that has nowhere to go: no order.
    back(&mut app);
    app.check_timed_exits(app.clock.new_york());
    assert_eq!(orders_sent_now(&commands).iter().map(|(_, order)| order.lmt_price).collect::<Vec<_>>(), [0.4]);
    back(&mut app);
    app.check_timed_exits(app.clock.new_york());
    assert!(orders_sent_now(&commands).is_empty(), "at the ask already");
}

#[test]
fn a_limit_exit_without_a_quote_waits_and_then_goes_to_market_and_a_missed_window_is_reported() {
    let (mut app, commands) = filled_trade_with_a_timed_exit(true);
    for contract in &mut app.chains.contracts {
        (contract.bid, contract.ask) = (None, None);
    }
    at_new_york(&mut app, 11, 31, 0);
    app.check_timed_exits(app.clock.new_york());
    assert!(orders_sent_now(&commands).is_empty(), "a limit order waits for a quote");
    at_new_york(&mut app, 11, 33, 30);
    app.watch.timed_tried.clear();
    app.check_timed_exits(app.clock.new_york());
    assert_eq!(orders_sent_now(&commands).iter().map(|(_, order)| order.order_type.clone()).collect::<Vec<_>>(), ["MKT"], "three minutes without a quote");
    // A trade whose window has run out is given up and reported.
    let (mut app, commands) = filled_trade_with_a_timed_exit(false);
    at_new_york(&mut app, 11, 36, 0);
    app.check_timed_exits(app.clock.new_york());
    app.poll_tws();
    assert!(orders_sent_now(&commands).is_empty(), "nothing is sent after the window");
    assert!(mail_lines(&app).iter().any(|l| l.contains("\"ALERT: Timed close not completed\"")), "{:?}", mail_lines(&app));
    assert_eq!(app.trades_stored.first().unwrap().timed_exit, None, "dropped");
}

#[test]
fn a_stop_and_a_target_can_be_attached_to_an_open_trade_that_lacks_them() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    // A trade that has both offers them for a change, prefilled with the prices on record.
    app.request_exits(0);
    assert!(app.exits_pending.is_some(), "an existing stop and target can be changed");
    app.exits_pending = None;
    // The stop and the target are gone (say, cancelled in TWS and the trade was edited by hand).
    let mut bare = app.trades_stored.first().unwrap().clone();
    (bare.stop_order_id, bare.profit_order_id, bare.stop_price, bare.profit_price, bare.oca_group) = (None, None, None, None, None);
    app.trades_stored[0] = bare;
    app.request_exits(0);
    assert!(app.exits_pending.is_some());
    app.exits_pending.as_mut().unwrap().stop = "0,90".to_string();
    app.exits_pending.as_mut().unwrap().target = "0.15".to_string();
    let first_id = app.next_order_id.unwrap();
    app.confirm_exits();
    let sent = drain(&commands);
    let orders: Vec<(i32, IbOrder)> =
        sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { order_id, order, .. } = c { Some((*order_id, (**order).clone())) } else { None }).collect();
    assert_eq!(orders.len(), 2, "a stop and a target out of {} commands", sent.len());
    let ((stop_id, stop), (target_id, target)) = (&orders[0], &orders[1]);
    assert_eq!((*stop_id, *target_id), (first_id, first_id + 1));
    assert_eq!((stop.order_type.as_str(), stop.aux_price, stop.action.as_str()), ("STP", 0.9, "BUY"));
    assert_eq!((target.order_type.as_str(), target.lmt_price), ("LMT", 0.15));
    assert_eq!((stop.oca_group.as_str(), stop.oca_type), (target.oca_group.as_str(), 3));
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.profit_order_id), (Some(first_id), Some(first_id + 1)));
    assert_eq!(trade.oca_group.as_deref(), Some(stop.oca_group.as_str()));
    assert_eq!(trade.stop_price.map(|p| p.normalize().to_string()).as_deref(), Some("0.9"));
    // Changing the stop re-sends the working order under its own id; the unchanged target is left alone.
    app.request_exits(0);
    let pending = app.exits_pending.as_mut().expect("both can be changed");
    assert_eq!((pending.stop.as_str(), pending.target.as_str()), ("0.9", "0.15"));
    pending.stop = "1,10".to_string();
    app.confirm_exits();
    let sent = drain(&commands);
    let orders: Vec<(i32, IbOrder)> =
        sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { order_id, order, .. } = c { Some((*order_id, (**order).clone())) } else { None }).collect();
    assert_eq!(orders.len(), 1, "only the stop is re-sent");
    assert_eq!((orders[0].0, orders[0].1.order_type.as_str(), orders[0].1.aux_price), (first_id, "STP", 1.1));
    assert!(app.log_lines.iter().any(|l| l.contains(&format!("Modify STP ClientOrderID: {first_id} Price: 0.9 -> 1.1"))), "{:?}", app.log_lines);
    let stop_price = |app: &App| app.trades_stored.first().unwrap().stop_price.map(|p| p.normalize().to_string());
    assert_eq!(stop_price(&app).as_deref(), Some("0.9"), "the old price stays until TWS confirms the change");
    app.tws_sender.send(order_status(first_id, "Submitted", 0.0, 1.0)).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.profit_order_id), (Some(first_id), Some(first_id + 1)));
    assert_eq!(stop_price(&app).as_deref(), Some("1.1"));
    assert!(app.watch.exit_changes.is_empty());
}

#[test]
fn a_typed_price_that_rounds_to_the_price_on_record_sends_nothing() {
    let (mut app, commands, _) = trade_with_attached_exits();
    app.request_exits(0);
    // 0.92 lies on the 0.05 grid of an SPX spread at 0.90, the stop on record.
    app.exits_pending.as_mut().unwrap().stop = "0,92".to_string();
    app.confirm_exits();
    assert!(drain(&commands).iter().all(|c| !matches!(c, TwsCommand::PlaceOrder { .. })), "no order is sent");
    assert!(app.log_lines.iter().any(|l| l.contains("nothing to change")), "{:?}", app.log_lines);
    assert!(!app.log_lines.iter().any(|l| l.contains("TWS did not take the orders")), "{:?}", app.log_lines);
}

#[test]
fn a_change_that_tws_refuses_or_that_meets_an_ended_order_keeps_the_old_price() {
    // TWS refuses the modify.
    let (mut app, commands, stop_id) = trade_with_attached_exits();
    app.request_exits(0);
    app.exits_pending.as_mut().unwrap().stop = "1.1".to_string();
    app.confirm_exits();
    drain(&commands);
    app.tws_sender.send(TwsEvent::RequestError { req_id: stop_id, code: 104, message: "Cannot modify a filled order".into() }).unwrap();
    app.poll_tws();
    let stop_price = |app: &App| app.trades_stored.first().unwrap().stop_price.map(|p| p.normalize().to_string());
    assert_eq!(stop_price(&app).as_deref(), Some("0.9"));
    assert!(app.watch.exit_changes.is_empty());
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING: TWS refused the change of order")), "{:?}", app.log_lines);
    // A later status of the order no longer applies the dropped change.
    app.tws_sender.send(order_status(stop_id, "Submitted", 0.0, 1.0)).unwrap();
    app.poll_tws();
    assert_eq!(stop_price(&app).as_deref(), Some("0.9"));
    // The order was cancelled in TWS before the change reached it: the trade shows no stop, not one at the new price.
    let (mut app, commands, stop_id) = trade_with_attached_exits();
    app.request_exits(0);
    app.exits_pending.as_mut().unwrap().stop = "1.1".to_string();
    app.confirm_exits();
    drain(&commands);
    app.tws_sender.send(order_status(stop_id, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_price), (None, None), "the stop is gone");
    assert!(app.watch.exit_changes.is_empty());
    assert!(app.log_lines.iter().any(|l| l.contains(&format!("the change of order {stop_id} to 1.1 was not made"))), "{:?}", app.log_lines);
}

#[test]
fn a_stop_cancelled_in_tws_disappears_from_the_trade_and_can_be_attached_again() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Cancelled".into(), filled: 0.0, remaining: 1.0, avg_fill_price: 0.0 }).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_price), (None, None), "the stop is gone");
    assert_eq!((trade.profit_order_id, trade.profit_price.is_some()), (Some(12), true), "the target stays");
    assert!(trade.is_open());
    // The Dashboard offers a new stop, but no new target.
    app.request_exits(0);
    assert!(app.exits_pending.is_some());
    assert_eq!(attach::missing_exits(app.trades_stored.first().unwrap()), (true, false));
}

#[test]
fn a_profit_target_cancelled_in_tws_disappears_from_the_trade_too() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 12, status: "Cancelled".into(), filled: 0.0, remaining: 1.0, avg_fill_price: 0.0 }).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.profit_order_id, trade.profit_price), (None, None), "the target is gone");
    assert_eq!(trade.stop_order_id, Some(11), "the stop stays");
    assert!(trade.is_open());
    assert_eq!(attach::missing_exits(trade), (false, true));
}
