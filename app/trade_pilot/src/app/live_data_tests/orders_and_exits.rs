//! The settings file, template orders, their fill attempts, stops, profit-target levels, stop adjustments, REL stops, stop bases, the kill switch and the reconnect.

use super::*;

/// The switches of the settings file are grouped in the program, but the file keeps its flat keys
/// (an older file loads, and an older version can read a newer file).
#[test]
fn the_settings_file_keeps_its_flat_keys_with_grouped_switches() {
    let json = serde_json::to_string(&blank_saved_settings()).unwrap();
    assert!(json.contains("\"convert_0dte_enabled\":false"), "{json}");
    assert!(json.contains("\"retry_on_margin_rejection\":false"), "{json}");
    assert!(!json.contains("time_flags") && !json.contains("alert_flags"), "{json}");
    let back: PersistedSettings = serde_json::from_str(&json).unwrap();
    assert!(back == blank_saved_settings());
}

#[test]
fn an_unfilled_order_is_re_priced_every_fill_wait_and_cancelled_after_the_last_attempt() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    assert_eq!(placed_limits(&drain(&commands)), [-0.3], "the order itself: a 0.30 credit");
    let t0 = Instant::now();

    // The timer starts only once TWS reports the order as active.
    app.process_fill_attempts(t0 + Duration::from_secs(600));
    assert!(drain(&commands).is_empty(), "no status from TWS yet: nothing to re-price");
    app.tws_sender.send(order_status(10, "Submitted", 0.0, 1.0)).unwrap();
    app.poll_tws();
    app.process_fill_attempts(Instant::now() + Duration::from_secs(3));
    assert!(drain(&commands).is_empty(), "before FillWait (7 s) nothing happens");

    // Attempt 2 and 3: 0.05 less credit each time, same order id.
    let t1 = Instant::now() + Duration::from_secs(8);
    app.process_fill_attempts(t1);
    let sent = drain(&commands);
    assert_eq!(placed_limits(&sent), [-0.25]);
    assert!(matches!(sent.first(), Some(TwsCommand::PlaceOrder { order_id: 10, .. })), "the same order is modified");
    assert!(app.instant.template_status.as_deref().is_some_and(|s| s.contains("attempt 2/3")), "{:?}", app.instant.template_status);
    // Not due again right away.
    app.process_fill_attempts(t1 + Duration::from_secs(3));
    assert!(drain(&commands).is_empty());
    let t2 = t1 + Duration::from_secs(8);
    app.process_fill_attempts(t2);
    assert_eq!(placed_limits(&drain(&commands)), [-0.2]);

    // All attempts used: cancelled, once.
    let t3 = t2 + Duration::from_secs(8);
    app.process_fill_attempts(t3);
    let sent = drain(&commands);
    assert_eq!(cancelled_orders(&sent), [10]);
    assert_eq!(placed_limits(&sent), [] as [f64; 0]);
    app.process_fill_attempts(t3 + Duration::from_secs(60));
    assert!(drain(&commands).is_empty(), "finished: nothing more is sent");
    assert!(app.instant.template_status.as_deref().is_some_and(|s| s.contains("cancelled after 3 attempts")));
}

#[test]
fn a_filled_or_cancelled_order_is_left_alone() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    app.tws_sender.send(order_status(10, "Submitted", 0.0, 1.0)).unwrap();
    app.tws_sender.send(order_status(10, "Filled", 1.0, 0.0)).unwrap();
    app.poll_tws();
    app.process_fill_attempts(Instant::now() + Duration::from_secs(60));
    assert!(drain(&commands).is_empty(), "a filled order is not touched");

    // Cancelled in TWS by the user: no re-pricing, no second cancel.
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    app.tws_sender.send(order_status(10, "Submitted", 0.0, 1.0)).unwrap();
    app.tws_sender.send(order_status(10, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
    app.process_fill_attempts(Instant::now() + Duration::from_secs(60));
    assert!(drain(&commands).is_empty());
}

#[test]
fn an_order_held_in_tws_for_review_is_never_changed_or_cancelled_by_the_app() {
    let (mut app, commands) = app_with_a_sent_template_order(false);
    let sent = drain(&commands);
    assert_eq!(placed_limits(&sent), [-0.3]);
    assert!(matches!(sent.first(), Some(TwsCommand::PlaceOrder { order, .. }) if !order.handling.transmit));
    app.tws_sender.send(order_status(10, "PreSubmitted", 0.0, 1.0)).unwrap();
    app.poll_tws();
    for seconds in [8, 30, 120, 3600] {
        app.process_fill_attempts(Instant::now() + Duration::from_secs(seconds));
    }
    assert!(drain(&commands).is_empty(), "the user is reviewing it in TWS");
}

#[test]
fn a_tripped_kill_switch_cancels_instead_of_moving_the_order() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    app.tws_sender.send(order_status(10, "Submitted", 0.0, 1.0)).unwrap();
    app.poll_tws();
    app.kill_switch.trip();
    app.process_fill_attempts(Instant::now() + Duration::from_secs(8));
    let sent = drain(&commands);
    assert_eq!(cancelled_orders(&sent), [10]);
    assert!(placed_limits(&sent).is_empty(), "never a new price once the gate is shut");
}

#[test]
fn a_filled_template_trade_gets_a_stop_and_a_profit_target_in_one_oca_group() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let sent = filled_trade_with_exits(&mut app, &commands);
    let orders: Vec<(i32, &IbContract, &IbOrder)> = sent
        .iter()
        .filter_map(|c| if let TwsCommand::PlaceOrder { order_id, contract, order } = c { Some((*order_id, contract, &**order)) } else { None })
        .collect();
    assert_eq!(orders.len(), 2, "a stop and a profit target");
    let (stop_id, stop_contract, stop) = orders[0];
    let (profit_id, _, profit) = orders[1];
    assert_eq!((stop_id, profit_id), (11, 12), "the ids follow the opening order's");
    // The put spread's short put is bought back: one option, 0.30 * (2 + 1) = 0.90 stop, 50 % target.
    assert_eq!((stop_contract.sec_type.as_str(), stop_contract.strike, stop_contract.right.as_str()), ("OPT", 6690.0, "P"));
    assert_eq!((stop.action.as_str(), stop.order_type.as_str(), stop.aux_price, stop.total_quantity), ("BUY", "STP", 0.9, 1.0));
    assert_eq!((profit.order_type.as_str(), profit.lmt_price), ("LMT", 0.15));
    assert_eq!((stop.oca_group.as_str(), stop.oca_type), (profit.oca_group.as_str(), 3));
    assert_ne!(stop.oca_group, "");
    assert!(stop.handling.transmit && profit.handling.transmit);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.profit_order_id), (Some(11), Some(12)));
    assert_eq!(
        (trade.stop_price.map(|p| p.normalize().to_string()).as_deref(), trade.profit_price.map(|p| p.normalize().to_string()).as_deref()),
        (Some("0.9"), Some("0.15"))
    );
    assert_eq!(app.next_order_id, Some(14));
}

#[test]
fn the_profit_target_levels_are_placed_one_after_the_other_and_the_trade_closes_at_their_average() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    let sent = filled_trade_with_two_profit_levels(&mut app, &commands);
    let orders: Vec<(i32, &IbOrder)> =
        sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { order_id, order, .. } = c { Some((*order_id, &**order)) } else { None }).collect();
    // The stop covers all 4 contracts, the first target only its half.
    assert_eq!(orders.iter().map(|(id, o)| (*id, o.total_quantity)).collect::<Vec<_>>(), [(11, 4.0), (12, 2.0)]);
    let group = orders[0].1.oca_group.clone();
    let trade = app.trades_stored.first().unwrap().clone();
    assert_eq!(trade.exit_levels.as_ref().map(|l| (l.qty, l.level)), Some(([2, 2, 0, 0], 0)));
    assert_eq!(trade.remaining_qty(), 4);

    // Level 1 is hit: its 2 contracts are closed at 0.15, level 2 follows in the same group.
    fill(&mut app, 12, 2.0, 0.15);
    let sent = drain(&commands);
    let [TwsCommand::PlaceOrder { order_id, order, .. }] = sent.as_slice() else { panic!("one new order expected, got {}", sent.len()) };
    assert_eq!((*order_id, order.total_quantity, order.order_type.as_str(), order.oca_group.as_str(), order.oca_type), (14, 2.0, "LMT", group.as_str(), 3));
    assert_eq!(order.order_ref, "TraPi-14 (TP)");
    let trade = app.trades_stored.first().unwrap().clone();
    assert_eq!((trade.status.as_str(), trade.remaining_qty()), ("Open", 2));
    assert_eq!(trade.exit_levels.as_ref().map(|l| (l.closed.clone(), l.order_ids[1])), Some((vec![(2, "-0.15".to_string())], Some(14))));
    // 2 contracts closed at 0.15 against a 0.30 credit: 0.15 * 2 * 100 = 30 made so far.
    assert_eq!(trade.realized_profit(Decimal::new(3, 1)), Some(Decimal::from(30)));
    assert_eq!(app.next_order_id, Some(15));

    // Level 2 is hit at 0.05: the trade ends at the average (2 * 0.15 + 2 * 0.05) / 4 = 0.10.
    fill(&mut app, 14, 2.0, 0.05);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close.map(|p| p.normalize().to_string()).as_deref()), ("Closed", Some("-0.1")));
    assert!(drain(&commands).is_empty(), "nothing more to place");
}

#[test]
fn deleting_the_target_after_a_level_filled_cancels_only_the_working_level() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    filled_trade_with_two_profit_levels(&mut app, &commands);
    // Level 1 (order 12) fills; level 2 (order 14) is placed.
    fill(&mut app, 12, 2.0, 0.15);
    drain(&commands);
    app.delete_exit_orders(0, false);
    assert_eq!(cancelled_orders(&drain(&commands)), [14], "the filled level 12 is not cancelled");
    app.tws_sender.send(order_status(14, "Cancelled", 0.0, 2.0)).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!(attach::missing_exits(trade), (false, true), "the stop is there, the target is gone: Stop / target can attach a new one");
    assert!(app.watch.user_cancelled.is_empty(), "no id is kept");
    assert!(!app.log_lines.iter().any(|l| l.contains("WARNING")), "{:?}", app.log_lines);
}

#[test]
fn a_stop_deleted_while_an_adjustment_replaces_it_is_not_placed_again() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    // The adjustment is hit: the old stop #11 is cancelled, the new one waits for the confirmation.
    set_spread_value(&mut app, 0.10);
    app.check_stop_adjustments(now);
    drain(&commands);
    assert_eq!(app.trades_stored.first().unwrap().adjustments.as_ref().map(|a| a.replace_pending), Some(true));
    // The person deletes the stop before TWS has confirmed the cancel.
    app.delete_exit_orders(0, true);
    drain(&commands);
    app.tws_sender.send(order_status(11, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
    assert_eq!(placed_orders(&drain(&commands)).len(), 0, "no adjusted stop against the delete");
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_price, trade.adjustments.is_none()), (None, None, true));
    assert!(app.watch.user_cancelled.is_empty(), "the id is not kept");
    assert!(app.log_lines.iter().any(|l| l.contains("The stop 11") && l.contains("as asked")), "{:?}", app.log_lines);
}

#[test]
fn a_hit_adjustment_moves_the_stop_after_tws_confirmed_the_cancel_of_the_old_one() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    // 0.20 now: the 50 % level (0.15) is not reached, nothing happens.
    app.check_stop_adjustments(now);
    assert!(drain(&commands).is_empty());
    // 0.10 now: hit. The old stop #11 is cancelled, the new one waits for the confirmation.
    set_spread_value(&mut app, 0.10);
    app.check_stop_adjustments(now);
    let sent = drain(&commands);
    assert_eq!(cancelled_orders(&sent), [11]);
    assert!(placed_limits(&sent).is_empty(), "no new stop before the old one is gone");
    let trade = app.trades_stored.first().unwrap().clone();
    assert_eq!(trade.adjustments.as_ref().map(|a| (a.level, a.replace_pending, a.stop.clone())), Some((1, true, Some("0.3".to_string()))));
    // The adjustment is used up: another check does not move the stop again.
    app.check_stop_adjustments(now);
    assert!(drain(&commands).is_empty());

    // TWS confirms the cancel: the stop at the entry price (0.30) is placed in the group of the target.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Cancelled".into(), filled: 0.0, remaining: 1.0, avg_fill_price: 0.0 }).unwrap();
    app.poll_tws();
    let sent = drain(&commands);
    let [TwsCommand::PlaceOrder { order_id, order, .. }] = sent.as_slice() else { panic!("one new stop expected, got {}", sent.len()) };
    assert_eq!((*order_id, order.order_type.as_str(), order.aux_price, order.action.as_str()), (14, "STP", 0.3, "BUY"));
    assert_eq!(order.oca_group, "TradePilot-11", "the group of the target, which is still working");
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_price.map(|p| p.normalize().to_string()).as_deref(), trade.profit_order_id), (Some(14), Some("0.3"), Some(12)));
    assert_eq!(trade.adjustments.as_ref().map(|a| a.replace_pending), Some(false));
    assert_eq!(app.next_order_id, Some(16));
    assert!(!app.log_lines.iter().any(|l| l.contains("WARNING")), "a cancelled stop for an adjustment is no warning");
}

#[test]
fn a_stop_adjusted_after_a_profit_level_covers_only_the_open_contracts_and_a_close_joins_its_group() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    filled_trade_with_levels_and(&mut app, &commands, break_even("Target 1", None));
    // Level 1 closes 2 of the 4 contracts; level 2 (#14) follows.
    fill(&mut app, 12, 2.0, 0.15);
    drain(&commands);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    assert_eq!(cancelled_orders(&drain(&commands)), [11], "ProfitTarget1 was hit: the stop moves");
    app.tws_sender.send(order_status(11, "Cancelled", 0.0, 4.0)).unwrap();
    app.poll_tws();
    let sent = placed_orders(&drain(&commands));
    let [(order_id, stop)] = sent.as_slice() else { panic!("one new stop expected, got {}", sent.len()) };
    // Only 2 contracts are still open: a stop for 4 would open 2 the other way round.
    assert_eq!((*order_id, stop.order_type.as_str(), stop.aux_price, stop.total_quantity), (15, "STP", 0.3, 2.0));
    assert_eq!((stop.oca_group.as_str(), stop.oca_type), ("TradePilot-11", 3));
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.oca_group.as_deref(), trade.remaining_qty()), (Some(15), Some("TradePilot-11"), 2));

    // Closing by hand now joins the group the stop and level 2 are in - not one named after #15.
    close_with(&mut app, CloseKind::Market, "");
    let sent = placed_orders(&drain(&commands));
    let [(_, close)] = sent.as_slice() else { panic!("one close order expected, got {}", sent.len()) };
    assert_eq!((close.total_quantity, close.oca_group.as_str(), close.oca_type), (2.0, "TradePilot-11", 3));
}

#[test]
fn after_a_replaced_stop_the_next_profit_level_and_the_next_adjustment_stay_in_the_trades_group() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    filled_trade_with_levels_and(&mut app, &commands, |template| {
        break_even("Profit percent", Some(50.0))(template);
        // The second adjustment: at a 60 % profit level, the stop moves to 0.20.
        template.adjustment2_type = "Profit percent".into();
        template.adjustment2 = Some(60.0);
        template.adjustment2_order_type = "Same".into();
        template.adjustment2_change_type = "Fixed price".into();
        template.adjustment2_change = Some(0.2);
    });
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    // 0.10 now: both levels are reached, one adjustment per check. The first one replaces #11 by #14.
    set_spread_value(&mut app, 0.10);
    app.check_stop_adjustments(now);
    assert_eq!(cancelled_orders(&drain(&commands)), [11]);
    app.tws_sender.send(order_status(11, "Cancelled", 0.0, 4.0)).unwrap();
    app.poll_tws();
    let sent = placed_orders(&drain(&commands));
    assert_eq!(sent.iter().map(|(id, o)| (*id, o.total_quantity, o.oca_group.clone())).collect::<Vec<_>>(), [(14, 4.0, "TradePilot-11".to_string())]);

    // Level 1 is hit: level 2 (#16) goes into the group of the stop #14, so its fill reduces it.
    fill(&mut app, 12, 2.0, 0.15);
    let sent = placed_orders(&drain(&commands));
    let [(level_id, level)] = sent.as_slice() else { panic!("one new level expected, got {}", sent.len()) };
    assert_eq!((*level_id, level.total_quantity, level.oca_group.as_str()), (16, 2.0, "TradePilot-11"));

    // The second adjustment replaces #14 by #17 - still linked to the target, and for 2 contracts.
    app.check_stop_adjustments(now);
    assert_eq!(cancelled_orders(&drain(&commands)), [14]);
    app.tws_sender.send(order_status(14, "Cancelled", 0.0, 2.0)).unwrap();
    app.poll_tws();
    let sent = placed_orders(&drain(&commands));
    let [(stop_id, stop)] = sent.as_slice() else { panic!("one new stop expected, got {}", sent.len()) };
    assert_eq!((*stop_id, stop.aux_price, stop.total_quantity, stop.oca_group.as_str()), (17, 0.2, 2.0, "TradePilot-11"));
    assert_eq!(app.trades_stored.first().and_then(|t| t.oca_group.clone()).as_deref(), Some("TradePilot-11"));
}

#[test]
fn exit_orders_go_through_the_risk_check_and_a_refused_one_is_not_recorded() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    // At most 3 contracts per order: the stop for 4 is refused, the first target for 2 is not.
    app.risk_limits = OrderRiskLimits { max_quantity: core_types::Quantity::new(Decimal::from(3)).unwrap() };
    let sent = placed_orders(&filled_trade_with_two_profit_levels(&mut app, &commands));
    assert_eq!(sent.iter().map(|(id, o)| (*id, o.total_quantity)).collect::<Vec<_>>(), [(12, 2.0)]);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!(
        (trade.stop_order_id, trade.stop_price.map(|p| p.normalize().to_string()).as_deref(), trade.profit_order_id),
        (None, None, Some(12)),
        "no stop that was never sent"
    );
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING: order 11 NOT sent: risk check failed")), "{:?}", app.log_lines);
}

#[test]
fn exit_orders_that_cannot_be_handed_to_the_connection_are_not_taken_for_working_ones() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let template = &mut app.template.orders.first_mut().unwrap().template;
    template.stop_type = "Short".into();
    template.stop_multiple = 2.0;
    template.stop_order_type = "Market stop".into();
    template.profit_target_type = "Percentage".into();
    template.profit_target = Some(50.0);
    // The connection thread is gone: nothing receives the commands any more.
    drop(commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!(
        (trade.stop_order_id, trade.profit_order_id, trade.stop_price.map(|p| p.normalize().to_string()).as_deref()),
        (None, None, None),
        "the trade has no stop - and says so"
    );
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING: order 11 NOT sent: the connection to TWS is gone")), "{:?}", app.log_lines);
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING: order 12 NOT sent")));
}

#[test]
fn an_adjustment_whose_cancel_cannot_be_sent_leaves_the_stop_as_it_is() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    set_spread_value(&mut app, 0.10);
    drop(commands);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    // The old stop is still working: no replacement is waited for, the adjustment is not used up.
    let trade = app.trades_stored.first().unwrap();
    assert_eq!(trade.stop_order_id, Some(11));
    assert_eq!(trade.adjustments.as_ref().map(|a| (a.level, a.replace_pending)), Some((0, false)));
    assert!(app.log_lines.iter().any(|l| l.contains("the old stop could not be cancelled")), "{:?}", app.log_lines);
}

#[test]
fn the_kill_switch_cancels_an_unfilled_template_order_and_ends_its_fill_attempts() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    app.trip_kill_switch();
    assert_eq!(cancelled_orders(&drain(&commands)), [10]);
    // The fill attempts are over: no new price later, and no new order at all.
    app.process_fill_attempts(Instant::now() + Duration::from_secs(60));
    assert!(drain(&commands).is_empty());
    assert!(!app.trading_allowed());
    assert!(app.log_lines.iter().any(|l| l.contains("KILL SWITCH on") && l.contains("[10]")), "{:?}", app.log_lines);
}

#[test]
fn the_kill_switch_cancels_the_exit_orders_of_open_trades_and_names_those_left_without_a_stop() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.trip_kill_switch();
    // The stop #11 and the target #12; the filled opening order #10 is not touched.
    assert_eq!(cancelled_orders(&drain(&commands)), [11, 12]);
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING: KILL SWITCH: these open trades lose their stop: SPX PutSpread")), "{:?}", app.log_lines);
    assert!(app.instant.template_status.as_deref().is_some_and(|s| s.contains("2 order(s) cancelled")));
    // Whatever happens next, nothing new is placed - not even an adjusted stop.
    app.check_stop_adjustments(new_york_noon(&app.trades_stored.first().unwrap().expiry.clone()));
    assert!(placed_orders(&drain(&commands)).is_empty());
}

#[test]
fn a_kill_switch_cancel_that_cannot_be_sent_is_reported() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    drop(commands);
    app.trip_kill_switch();
    assert!(app.log_lines.iter().any(|l| l.contains("the cancel of [11, 12] could not be sent") && l.contains("in TWS yourself")), "{:?}", app.log_lines);
    assert!(app.kill_switch.is_tripped(), "new orders are blocked anyway");
}

#[test]
fn templates_are_not_saved_over_a_file_that_could_not_be_read() {
    let (mut app, _commands) = connected_app();
    app.template_ui.templates_save_blocked = true;
    app.save_templates();
    assert!(app.template.status.as_deref().is_some_and(|s| s.contains("NOT saved")), "{:?}", app.template.status);
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING: Trade templates NOT saved")));
}

#[test]
fn after_a_reconnect_a_waiting_adjustment_places_its_stop_once_the_old_one_is_gone() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    adjustment_waiting_for_its_cancel(&mut app, &commands);
    // Only the target is still working: the old stop was cancelled while nobody listened.
    let sent = placed_orders(&reconnect_with_working_orders(&mut app, &commands, &[12]));
    let [(order_id, stop)] = sent.as_slice() else { panic!("one adjusted stop expected, got {}", sent.len()) };
    assert_eq!((*order_id, stop.order_type.as_str(), stop.aux_price, stop.oca_group.as_str()), (14, "STP", 0.3, "TradePilot-11"));
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.adjustments.as_ref().map(|a| a.replace_pending)), (Some(14), Some(false)));
}

#[test]
fn after_a_reconnect_a_waiting_adjustment_cancels_an_old_stop_that_still_works_again() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    adjustment_waiting_for_its_cancel(&mut app, &commands);
    let sent = reconnect_with_working_orders(&mut app, &commands, &[11, 12]);
    assert_eq!(cancelled_orders(&sent), [11], "the cancel goes out again");
    assert!(placed_orders(&sent).is_empty(), "no second stop while the old one works");
    assert_eq!(app.trades_stored.first().and_then(|t| t.adjustments.as_ref()).map(|a| a.replace_pending), Some(true), "still waiting for TWS");
}

#[test]
fn after_a_reconnect_an_old_stop_that_filled_gets_no_adjusted_stop() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    adjustment_waiting_for_its_cancel(&mut app, &commands);
    // The executions show that #11 filled before its cancel arrived.
    app.exec_history.push(StoredExecution {
        exec_id: "e-11".into(),
        time: "20261113 15:00:00 US/Eastern".into(),
        account: "DU1".into(),
        symbol: "SPX".into(),
        con_id: 100,
        order_id: 11,
        order_ref: "TraPi-11".into(),
        sec_type: "OPT".into(),
        expiry: String::new(),
        right: "P".into(),
        strike: 6690.0,
        side: "BOT".into(),
        quantity: 1.0,
        price: 0.9,
    });
    let sent = reconnect_with_working_orders(&mut app, &commands, &[12]);
    assert!(placed_orders(&sent).is_empty(), "a filled stop is not replaced");
    assert!(app.log_lines.iter().any(|l| l.contains("the old stop 11 filled") && l.contains("Mark closed")), "{:?}", app.log_lines);
}

#[test]
fn after_a_reconnect_a_waiting_adjustment_keeps_waiting_while_the_safety_gate_is_shut() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    adjustment_waiting_for_its_cancel(&mut app, &commands);
    app.kill_switch.trip();
    let sent = reconnect_with_working_orders(&mut app, &commands, &[12]);
    assert!(placed_orders(&sent).is_empty());
    assert_eq!(
        app.trades_stored.first().and_then(|t| t.adjustments.as_ref()).map(|a| a.replace_pending),
        Some(true),
        "not forgotten: placed in a later session"
    );
    assert!(app.log_lines.iter().any(|l| l.contains("safety gate is shut") && l.contains("has no stop")));
}

#[test]
fn after_a_reconnect_exit_orders_that_are_gone_are_reported_and_taken_off_the_trade() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    // All working: nothing to report.
    assert!(reconnect_with_working_orders(&mut app, &commands, &[11, 12]).is_empty());
    assert!(app.log_lines.iter().any(|l| l.contains("the exit orders of all open trades are working")));
    // The next session: the stop is gone.
    app.tws_sender.send(TwsEvent::Connected { server_version: "200".into() }).unwrap();
    app.poll_tws();
    let sent = reconnect_with_working_orders(&mut app, &commands, &[12]);
    assert!(sent.is_empty(), "nothing is placed or cancelled on a guess");
    assert!(app.log_lines.iter().any(|l| l.contains("[11] are no longer working") && l.contains("NO stop")), "{:?}", app.log_lines);
    // The target (12) is still working, so TWS reports this client's orders: the stop is gone for good.
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_price, trade.profit_order_id), (None, None, Some(12)));
    assert!(app.instant.template_status.as_deref().is_some_and(|s| s.contains("1 open trade(s) need attention")));
}

#[test]
fn orders_are_kept_when_nothing_of_this_app_is_reported_working() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    // Nothing of this app is listed (say, another client id is in use): no evidence that the orders are gone.
    app.reconcile_open_orders(&[]);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.profit_order_id), (Some(11), Some(12)), "kept: they may work under another client id");
}

#[test]
fn an_adjustment_to_a_stop_type_that_is_not_supported_keeps_the_stop_and_is_reported() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let template = &mut app.template.orders.first_mut().unwrap().template;
    template.stop_type = "Short".into();
    template.stop_multiple = 2.0;
    template.stop_order_type = "Market stop".into();
    template.profit_target_type = "Percentage".into();
    template.profit_target = Some(50.0);
    template.adjustment1_type = "Profit percent".into();
    template.adjustment1 = Some(50.0);
    template.adjustment1_order_type = "TrailingPct".into();
    template.adjustment1_change_type = "Stop factor".into();
    template.adjustment1_change = Some(0.0);
    drain(&commands);
    fill(&mut app, 10, 1.0, -0.3);
    drain(&commands);
    // Said once, when the stop is placed.
    assert!(app.log_lines.iter().any(|l| l.contains("stop adjustment 1: stop order type 'TrailingPct' is not supported yet")), "{:?}", app.log_lines);
    set_spread_value(&mut app, 0.10);
    app.check_stop_adjustments(new_york_noon(&app.trades_stored.first().unwrap().expiry.clone()));
    assert!(drain(&commands).is_empty(), "the old stop is not cancelled for a stop that cannot be placed");
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.adjustments.as_ref().map(|a| (a.level, a.replace_pending))), (Some(11), Some((0, false))));
}

#[test]
fn the_app_reads_its_time_from_its_clock() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 15);
    set_spread_value(&mut app, 0.10);
    let expiry = chrono::NaiveDate::parse_from_str(&app.trades_stored.first().unwrap().expiry, "%Y%m%d").unwrap();
    // 14:59 in New York on the expiration day: the adjustment's time (15:00) has not come yet.
    app.clock = Box::new(crate::clock::TestClock::at_new_york(expiry.and_hms_opt(14, 59, 0).unwrap()));
    app.refresh_trades(app.clock.instant(), true);
    assert!(drain(&commands).is_empty());
    // Two minutes later - on the app's own clock, not the computer's - it applies.
    app.clock = Box::new(crate::clock::TestClock::at_new_york(expiry.and_hms_opt(15, 1, 0).unwrap()));
    app.refresh_trades(app.clock.instant(), true);
    assert_eq!(cancelled_orders(&drain(&commands)), [11]);
    // The log is stamped with the clock's time, in the computer's time zone.
    let stamp = app.clock.local().format("%d-%m-%Y %H:%M").to_string();
    assert!(app.log_lines.iter().any(|l| l.starts_with(&stamp) && l.contains("Cancel 11")), "{stamp} / {:?}", app.log_lines);
}

#[test]
fn an_adjustment_with_a_time_waits_for_the_day_of_the_expiration() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 15);
    set_spread_value(&mut app, 0.10);
    let expiry = app.trades_stored.first().unwrap().expiry.clone();
    let date = chrono::NaiveDate::parse_from_str(&expiry, "%Y%m%d").unwrap();
    // The day before and the morning of the day: not yet.
    app.check_stop_adjustments(date.pred_opt().unwrap().and_hms_opt(23, 0, 0).unwrap());
    app.check_stop_adjustments(date.and_hms_opt(14, 59, 0).unwrap());
    assert!(drain(&commands).is_empty());
    // After 15:00 on the expiration day it applies.
    app.check_stop_adjustments(date.and_hms_opt(15, 1, 0).unwrap());
    assert_eq!(cancelled_orders(&drain(&commands)), [11]);
}

#[test]
fn no_adjustment_is_touched_while_the_safety_gate_is_shut() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    set_spread_value(&mut app, 0.10);
    app.kill_switch.trip();
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    assert!(drain(&commands).is_empty(), "a cancelled stop could not be replaced");
    assert_eq!(app.trades_stored.first().and_then(|t| t.adjustments.as_ref()).map(|a| a.level), Some(0));
}

#[test]
fn an_adjustment_to_none_removes_the_stop_and_keeps_the_target() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    if let Some(adjustments) = app.trades_stored.first_mut().and_then(|t| t.adjustments.as_mut()) {
        if let Some(item) = adjustments.items.first_mut() {
            item.order_type = "None".into();
        }
    }
    set_spread_value(&mut app, 0.10);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    assert_eq!(cancelled_orders(&drain(&commands)), [11]);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.profit_order_id), (None, Some(12)));
    assert_eq!(trade.adjustments.as_ref().map(|a| (a.order_type.as_str(), a.replace_pending)), Some(("None", false)));
}

#[test]
fn a_refused_stop_limit_is_placed_again_with_a_smaller_limit_offset_until_tws_takes_it() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    {
        let template = &mut app.template.orders.first_mut().unwrap().template;
        template.stop_type = "Short".into();
        template.stop_multiple = 2.0;
        template.stop_order_type = "Limit stop".into();
        template.stop_limit_offset = Some(0.20);
    }
    app.templates.first_mut().unwrap().stop_limit_offset = Some(0.20);
    drain(&commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    let first = placed_orders(&drain(&commands));
    let (limit_id, limit) = first.iter().find(|(_, order)| order.order_type == "STP LMT").cloned().expect("the stop-limit is placed");
    let stop = limit.aux_price;
    assert!((limit.lmt_price - (stop + 0.20)).abs() < 1e-9, "{} after {stop}", limit.lmt_price);
    assert_eq!(app.trades_stored.first().unwrap().stop_limit_order_id, Some(limit_id));
    // TWS refuses it: it goes out again 0.05 closer, as a new order.
    app.tws_sender.send(TwsEvent::RequestError { req_id: limit_id, code: 201, message: "Order rejected - limit too far from the market".into() }).unwrap();
    app.poll_tws();
    let again = placed_orders(&drain(&commands));
    let (new_id, retry) = again.iter().find(|(_, order)| order.order_type == "STP LMT").cloned().expect("it is placed again");
    assert_ne!(new_id, limit_id);
    assert!((retry.lmt_price - (stop + 0.15)).abs() < 1e-9, "{} after {stop}", retry.lmt_price);
    assert_eq!(app.trades_stored.first().unwrap().stop_limit_order_id, Some(new_id), "the trade follows the new order");
    // Another refusal of a different order, or another kind of error, does nothing.
    app.tws_sender.send(TwsEvent::RequestError { req_id: 999, code: 201, message: "rejected".into() }).unwrap();
    app.tws_sender.send(TwsEvent::RequestError { req_id: new_id, code: 2104, message: "farm connection is OK".into() }).unwrap();
    app.poll_tws();
    assert!(placed_orders(&drain(&commands)).is_empty());
}

#[test]
fn a_rel_stop_is_placed_with_the_price_condition_of_its_option() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let sent = filled_trade_with_a_rel_stop(&mut app, &commands, |t| {
        t.stop_rel_limit = Some(3.0);
        t.stop_rel_offset = Some(0.25);
    });
    let orders = placed_orders(&sent);
    let (id, stop) = &orders[0];
    assert_eq!((*id, stop.order_type.as_str(), stop.lmt_price, stop.aux_price, stop.action.as_str()), (11, "REL", 3.0, 0.25, "BUY"));
    // The short put (contract 100) at or above the stop price 0.90.
    assert_eq!(
        stop.conditions,
        [broker::IbCondition::Price { and_next: false, is_more: true, price: 0.9, con_id: 100, exchange: "SMART".into(), trigger_method: 0 }]
    );
    assert_eq!(orders[1].1.order_type, "LMT", "the profit target is a plain limit order");
    assert_eq!(orders[1].1.oca_group, stop.oca_group);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_price.map(|p| p.normalize().to_string()).as_deref()), (Some(11), Some("0.9")));
}

#[test]
fn the_itm_protection_of_a_rel_stop_names_the_underlying_and_the_closing_minutes() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let sent = filled_trade_with_a_rel_stop(&mut app, &commands, |t| {
        t.stop_rel_itm = Some(5);
        t.stop_rel_itm_minutes = Some(20);
    });
    let (_, stop) = placed_orders(&sent).remove(0);
    assert_eq!(stop.conditions.len(), 3);
    // The put's strike is 6690: the underlying at or below 6695.
    assert!(
        matches!(&stop.conditions[1], broker::IbCondition::Price { and_next: true, is_more: false, price, con_id: 416_904, exchange, .. } if price.total_cmp(&6695.0).is_eq() && exchange == "CBOE")
    );
    assert!(matches!(&stop.conditions[2], broker::IbCondition::Time { and_next: false, is_more: true, time } if time.ends_with("US/Central")));
}

#[test]
fn a_rel_stop_whose_conditions_cannot_be_built_is_not_placed_and_the_log_says_so() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_a_rel_stop(&mut app, &commands, |_| {});
    // The same trade on an underlying TWS has no contract id for here.
    let mut trade = app.trades_stored.first().unwrap().clone();
    trade.symbol = "XYZ".into();
    trade.stop_order_id = None;
    trade.profit_order_id = None;
    trade.stop_price = None;
    app.trades_stored.push(trade);
    let template = TradeTemplate {
        stop_type: "Short".into(),
        stop_multiple: 2.0,
        stop_order_type: "Market stop".into(),
        stop_flags: StopSwitches { stop_rel: true, ..StopSwitches::default() },
        stop_rel_itm: Some(5),
        profit_target_type: "Percentage".into(),
        profit_target: Some(50.0),
        ..TradeTemplate::default()
    };
    drain(&commands);
    app.place_exit_orders(1, &template);
    let orders = placed_orders(&drain(&commands));
    assert_eq!(orders.iter().map(|(_, o)| o.order_type.as_str()).collect::<Vec<_>>(), ["LMT"], "only the profit target");
    assert!(app.log_lines.iter().any(|l| l.contains("conditions of the REL stop cannot be built")));
    let trade = app.trades_stored.get(1).unwrap();
    assert_eq!((trade.stop_order_id, trade.stop_price.map(|p| p.normalize().to_string()).as_deref()), (None, None), "no stop is shown that is not there");
}

#[test]
fn an_adjusted_rel_stop_is_placed_again_as_a_rel_stop_with_its_conditions() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    if let Some(adjustments) = app.trades_stored.first_mut().and_then(|t| t.adjustments.as_mut()) {
        adjustments.rel = Some(RelSettings { itm: Some(5), ..RelSettings::default() });
    }
    set_spread_value(&mut app, 0.10);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    drain(&commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Cancelled".into(), filled: 0.0, remaining: 1.0, avg_fill_price: 0.0 }).unwrap();
    app.poll_tws();
    let (_, stop) = placed_orders(&drain(&commands)).remove(0);
    assert_eq!((stop.order_type.as_str(), stop.conditions.len()), ("REL", 3));
    assert!(matches!(&stop.conditions[0], broker::IbCondition::Price { price, con_id: 100, .. } if price.total_cmp(&0.3).is_eq()), "the adjusted stop price");
}

#[test]
fn a_later_profit_level_is_not_placed_while_the_safety_gate_is_shut() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    filled_trade_with_two_profit_levels(&mut app, &commands);
    app.kill_switch.trip();
    fill(&mut app, 12, 2.0, 0.15);
    assert!(placed_orders(&drain(&commands)).is_empty(), "no order of any kind with the kill switch on");
    assert!(app.log_lines.iter().any(|l| l.contains("Profit target 2") && l.contains("NOT placed")));
    // Level 1 is recorded all the same, and the trade stays open with what is left.
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.remaining_qty()), ("Open", 2));
    assert_eq!(trade.exit_levels.as_ref().map(|l| (l.level, l.order_ids[1])), Some((1, None)));
}

#[test]
fn without_a_stop_each_later_profit_level_starts_its_own_oca_group() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    let template = &mut app.template.orders.first_mut().unwrap().template;
    template.stop_order_type = "None".into();
    template.profit_target_type = "Percentage".into();
    template.profit_target = Some(50.0);
    template.profit_target_trade_percent = Some(50.0);
    template.profit_target2 = Some(75.0);
    template.profit_target2_trade_percent = Some(50.0);
    drain(&commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 4.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    let first = placed_orders(&drain(&commands));
    assert_eq!(first.iter().map(|(id, o)| (*id, o.total_quantity)).collect::<Vec<_>>(), [(12, 2.0)], "only the first level, no stop");
    fill(&mut app, 12, 2.0, 0.15);
    let second = placed_orders(&drain(&commands));
    let [(id, order)] = second.as_slice() else { panic!("one order expected, got {}", second.len()) };
    // The group of level 1 is done: a group of its own, named after this order.
    assert_eq!((*id, order.oca_group.as_str()), (14, "TradePilot-14"));
    assert_ne!(order.oca_group, first[0].1.oca_group);
}

#[test]
fn a_trade_that_is_being_closed_keeps_its_stop_when_an_adjustment_is_hit() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    set_spread_value(&mut app, 0.10);
    if let Some(trade) = app.trades_stored.first_mut() {
        trade.close_order_id = Some(99);
    }
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    assert!(drain(&commands).is_empty(), "the close order is in charge");
}

#[test]
fn an_adjustment_waits_for_a_quote_of_every_leg() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    set_spread_value(&mut app, 0.10);
    // The long put has no ask any more: the cost to close is not known.
    for contract in &mut app.chains.contracts {
        if contract.con_id == 101 {
            contract.ask = None;
        }
    }
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    assert!(drain(&commands).is_empty());
    assert_eq!(app.trades_stored.first().and_then(|t| t.adjustments.as_ref()).map(|a| a.level), Some(0));
}

#[test]
fn nothing_is_adjusted_without_a_connection_to_tws() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    set_spread_value(&mut app, 0.10);
    app.tws_cmd_sender = None;
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    assert_eq!(app.trades_stored.first().and_then(|t| t.adjustments.as_ref()).map(|a| (a.level, a.replace_pending)), Some((0, false)));
}

#[test]
fn a_replacement_stop_that_cannot_be_placed_is_reported_and_forgotten() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_an_adjustment(&mut app, &commands, 0);
    set_spread_value(&mut app, 0.10);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    drain(&commands);
    // The gate shuts between the cancel and its confirmation.
    app.kill_switch.trip();
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Cancelled".into(), filled: 0.0, remaining: 1.0, avg_fill_price: 0.0 }).unwrap();
    app.poll_tws();
    assert!(placed_orders(&drain(&commands)).is_empty());
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING") && l.contains("adjusted stop") && l.contains("has no stop")));
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.adjustments.as_ref().map(|a| a.replace_pending)), (None, Some(false)), "no order id of a stop that is gone");
}

#[test]
fn a_calendar_preview_picks_the_strikes_from_the_chains_of_both_expirations() {
    let ((near, near_expiry), (far, far_expiry)) = calendar_dtes();
    let (mut app, _commands) = app_for_a_calendar_preview(&[near_expiry.as_str(), far_expiry.as_str()], near, far);
    app.preview_template_trade();
    // Delta 0.20 is the sixth call (6740): sold at the near mid, bought at the far one, a debit of 1.00.
    assert!(app.instant.template_preview.contains("CALL:") && app.instant.template_preview.contains("6740"), "{}", app.instant.template_preview);
    assert!(
        app.instant.template_preview.contains("Einstiegspreis (Limit): -1") || app.instant.template_preview.contains("Entry price (limit): -1"),
        "{}",
        app.instant.template_preview
    );
    let (template, outcome) = app.instant.template_outcome.take().expect("a trade was found");
    assert_eq!(template.trade_type, "CallCalendar");
    let call = outcome.call.expect("the call side");
    assert_eq!((call.short.map(|l| l.expiry), call.long.map(|l| l.expiry)), (Some(near_expiry), Some(far_expiry)));
}

#[test]
fn a_calendar_preview_asks_for_the_chain_it_misses_and_names_its_dte() {
    let ((near, near_expiry), (far, far_expiry)) = calendar_dtes();
    // Only the near chain is loaded.
    let (mut app, commands) = app_for_a_calendar_preview(&[near_expiry.as_str()], near, far);
    app.preview_template_trade();
    assert!(app.instant.template_preview.contains("Market Data requested now"), "{}", app.instant.template_preview);
    assert!(app.instant.template_preview.contains(&far.to_string()));
    assert!(app.instant.template_outcome.is_none());
    let spx = app.chains.symbols.iter().find(|s| s.symbol == "SPX").unwrap();
    assert!(spx.dtes.contains(&far), "the far DTE is subscribed from now on");
    assert!(app.chains.pending_lookups.iter().any(|(_, symbol, expiry)| symbol == "SPX" && *expiry == far_expiry));
    assert!(!app.chains.pending_lookups.iter().any(|(_, _, expiry)| *expiry == near_expiry), "the near chain is already there");
    assert!(!drain(&commands).is_empty(), "the lookup went to TWS");
}

#[test]
fn a_calendar_preview_stops_at_a_weekend_expiration_unless_the_leg_may_move() {
    let today = test_today();
    let weekend = (1..=7).find(|dte| expiry_for_dte(today, *dte).is_none()).expect("a weekend within a week");
    let ((near, near_expiry), _) = calendar_dtes();
    let (mut app, _commands) = app_for_a_calendar_preview(&[near_expiry.as_str()], near, weekend);
    if let Some(template) = app.templates.first_mut() {
        template.leg_dte_b.long_call_dte_adjust = false;
    }
    app.preview_template_trade();
    assert!(app.instant.template_preview.contains("DTE Expiration is not available. Cancelling trade."), "{}", app.instant.template_preview);
    // With the leg allowed to adjust, the DTE moves to the next trading day (its chain is requested
    // or, when that is the near expiration, already there - which one depends on today).
    if let Some(template) = app.templates.first_mut() {
        template.leg_dte_b.long_call_dte_adjust = true;
    }
    let mut log = Vec::new();
    let mut selection = EntrySelection { account: app.instant.account.clone(), test_trade: false, outcome: None };
    let text = app.build_template_preview(app.instant.template_id, &mut selection, &mut log);
    assert!(log.iter().any(|l| l.contains(&format!("{weekend} DTE Expiration is not available. Changing DTE to"))), "{log:?}");
    assert!(!text.contains("Cancelling trade"), "the leg moved instead of cancelling: {text}");
}

#[test]
fn a_put_calendar_needs_only_its_put_expirations() {
    // A PutCalendar must not wait for call chains: with none loaded it asks for the put ones only.
    let ((near, near_expiry), (far, far_expiry)) = calendar_dtes();
    let (mut app, _commands) = app_for_a_calendar_preview(&[], near, far);
    app.templates = vec![TradeTemplate {
        id: 8,
        trade_type: "PutCalendar".into(),
        underlying_symbol: "SPX".into(),
        short_put_dte: near,
        long_put_dte: far,
        // The call DTEs are not used and may be nonsense.
        short_call_dte: 99,
        long_call_dte: 99,
        ..TradeTemplate::default()
    }];
    app.instant.template_id = Some(8);
    app.preview_template_trade();
    assert!(app.instant.template_preview.contains("Market Data requested now"), "{}", app.instant.template_preview);
    let asked: Vec<&String> = app.chains.pending_lookups.iter().map(|(_, _, expiry)| expiry).collect();
    assert!(asked.contains(&&near_expiry) && asked.contains(&&far_expiry), "{asked:?}");
    assert_eq!(asked.len(), 2);
}

#[test]
fn a_stop_on_the_short_price_is_worked_out_from_what_the_short_leg_filled_at() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let sent = filled_trade_with_a_stop_basis(&mut app, &commands, "Short leg price", true);
    let (_, stop) = placed_orders(&sent).remove(0);
    // 1.95 * (2 + 1) = 5.85, on the 0.10 grid of a single SPX option above 3.00: 5.80.
    assert_eq!((stop.order_type.as_str(), stop.aux_price), ("STP", 5.8));
    let trade = app.trades_stored.first().unwrap();
    assert_eq!(trade.stop_price.map(|p| p.normalize().to_string()).as_deref(), Some("5.8"));
    assert_eq!(trade.adjustments.as_ref().and_then(|a| a.basis.clone()), Some("1.95".to_string()));
}

#[test]
fn a_stop_on_the_long_price_uses_what_the_long_leg_cost() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let sent = filled_trade_with_a_stop_basis(&mut app, &commands, "Long leg price", true);
    // 1.65 * 3 = 4.95 -> 4.90.
    assert_eq!(placed_orders(&sent).remove(0).1.aux_price, 4.9);
}

#[test]
fn without_the_fill_prices_the_stop_uses_the_entry_price_and_the_log_says_so() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    let sent = filled_trade_with_a_stop_basis(&mut app, &commands, "Short leg price", false);
    // 0.30 * 3 = 0.90, the same as with the exact entry price.
    assert_eq!(placed_orders(&sent).remove(0).1.aux_price, 0.9);
    assert!(app.log_lines.iter().any(|l| l.contains("Short leg price") && l.contains("exact entry price is used")));
    assert_eq!(app.trades_stored.first().and_then(|t| t.adjustments.as_ref()).and_then(|a| a.basis.clone()), Some("0.3".to_string()));
}

#[test]
fn a_stop_on_the_average_uses_the_mean_of_the_two_sides_entries() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_a_stop_basis(&mut app, &commands, "Mean price", false);
    // The other side of the same opening order, filled at 0.50: a copy without exit orders.
    let mut other = app.trades_stored.first().unwrap().clone();
    other.price_open = rust_decimal::Decimal::from_str_exact("0.5").unwrap();
    other.stop_order_id = None;
    other.profit_order_id = None;
    other.adjustments = None;
    app.trades_stored.push(other);
    let template = TradeTemplate {
        stop_type: "Short".into(),
        stop_multiple: 2.0,
        stop_order_type: "Market stop".into(),
        stop_basis: "Mean price".into(),
        ..TradeTemplate::default()
    };
    drain(&commands);
    app.place_exit_orders(1, &template);
    // (0.30 + 0.50) / 2 = 0.40; 0.40 * 3 = 1.20.
    assert_eq!(placed_orders(&drain(&commands)).remove(0).1.aux_price, 1.2);
    // A single trade of an order has no average: the entry price (log).
    let mut alone = app.trades_stored.first().unwrap().clone();
    alone.order_id = 77;
    alone.stop_order_id = None;
    alone.adjustments = None;
    app.trades_stored.push(alone);
    app.place_exit_orders(2, &template);
    assert_eq!(placed_orders(&drain(&commands)).remove(0).1.aux_price, 0.9);
}

#[test]
fn an_adjustment_by_a_multiple_starts_from_the_basis_of_the_stop() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_a_stop_basis(&mut app, &commands, "Short leg price", true);
    set_spread_value(&mut app, 0.10);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    app.check_stop_adjustments(now);
    // Stop Multiple 0 = the basis (1.95), not the entry price (0.30): 1.95 on the grid of a single option.
    let adjusted = app.trades_stored.first().and_then(|t| t.adjustments.as_ref()).and_then(|a| a.stop.clone());
    assert_eq!(adjusted.as_deref(), Some("1.95"));
}

#[test]
fn the_rel_settings_know_the_minutes_until_the_close_of_the_day_of_the_expiration() {
    let template = TradeTemplate {
        stop_rel_limit: Some(3.0),
        stop_rel_offset: Some(0.25),
        stop_rel_itm: Some(5),
        stop_rel_itm_minutes: Some(20),
        stop_flags: StopSwitches { stop_rel_itm_only: true, stop_guardrail: true, ..StopSwitches::default() },
        stop_guardrail_offset_open: 10,
        stop_guardrail_offset_exp: 8,
        ..TradeTemplate::default()
    };
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    let base = app.trades_stored.first().unwrap().clone();
    let trade = |expiry: &str| StoredTrade { expiry: expiry.into(), ..base.clone() };
    let at = |text: &str| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap();
    // 14:00:30 on the day of the expiration: 119.5 minutes to 16:00, rounded up.
    let rel = rel_settings(&template, &trade("20261116"), Some(Decimal::new(67000, 1)), at("2026-11-16 14:00:30"));
    assert_eq!(rel.minutes_to_close, 120);
    assert_eq!((rel.limit, rel.offset, rel.itm, rel.itm_minutes, rel.itm_only, rel.guardrail), (Some(3.0), Some(0.25), Some(5), Some(20), true, true));
    assert_eq!((rel.offset_open, rel.offset_exp, rel.underlying_open.as_deref(), rel.early_close), (10, 8, Some("6700"), false));
    // An early-close day ends at 13:00: Friday after Thanksgiving.
    let early = rel_settings(&template, &trade("20261127"), None, at("2026-11-27 11:30:00"));
    assert_eq!((early.minutes_to_close, early.early_close, early.underlying_open), (90, true, None));
    // Another day, or after the close: no minutes, so no guardrail steps.
    assert_eq!(rel_settings(&template, &trade("20261117"), None, at("2026-11-16 14:00:00")).minutes_to_close, 0);
    assert_eq!(rel_settings(&template, &trade("20261116"), None, at("2026-11-16 17:00:00")).minutes_to_close, 0);
    assert_eq!(rel_settings(&template, &trade("garbage"), None, at("2026-11-16 14:00:00")).minutes_to_close, 0);
}

#[test]
fn a_stop_after_the_first_level_ends_what_is_left_at_the_average_price() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    filled_trade_with_two_profit_levels(&mut app, &commands);
    fill(&mut app, 12, 2.0, 0.15);
    drain(&commands);
    // The stop fills for the 2 contracts that are left, at 0.90: (2 * 0.15 + 2 * 0.90) / 4 = 0.525.
    fill(&mut app, 11, 2.0, 0.9);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close.map(|p| p.normalize().to_string()).as_deref()), ("Stopped", Some("-0.525")));
}

#[test]
fn a_trade_closed_by_hand_after_the_first_level_closes_only_what_is_left() {
    let (mut app, commands) = app_with_a_sent_template_order_of(true, "4");
    filled_trade_with_two_profit_levels(&mut app, &commands);
    fill(&mut app, 12, 2.0, 0.15);
    drain(&commands);
    app.request_close(0, CloseKind::Market);
    app.confirm_close();
    let sent = drain(&commands);
    let Some(TwsCommand::PlaceOrder { order, .. }) = sent.iter().find(|c| matches!(c, TwsCommand::PlaceOrder { .. })) else {
        panic!("no close order was sent")
    };
    assert_eq!(order.total_quantity, 2.0, "the 2 contracts of level 1 are already closed");
}

#[test]
fn nothing_is_placed_while_the_safety_gate_is_shut_and_the_log_says_so() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.kill_switch.trip();
    let sent = filled_trade_with_exits(&mut app, &commands);
    assert!(placed_limits(&sent).is_empty(), "no order of any kind with the kill switch on");
    assert!(app.log_lines.iter().any(|l| l.contains("NO stop or profit target placed")));
    assert_eq!(app.trades_stored.first().map(|t| t.stop_order_id), Some(None));
}

#[test]
fn a_template_without_stop_and_target_places_nothing() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    assert_eq!(placed_limits(&drain(&commands)), [] as [f64; 0]);
    assert_eq!(app.trades_stored.len(), 1, "the trade is recorded all the same");
}

#[test]
fn a_stopped_short_leaves_its_long_and_sell_sells_it_at_market() {
    let (app, sent) = stopped_short_with_leftover_long(-1, Some(1.55));
    let sales: Vec<(&IbContract, &IbOrder)> = sent
        .iter()
        .filter_map(|c| if let TwsCommand::PlaceOrder { contract, order, .. } = c { Some((contract, &**order)) } else { None })
        .filter(|(_, order)| order.action == "SELL")
        .collect();
    assert_eq!(sales.len(), 1, "one sale: {:?}", app.log_lines);
    let (contract, order) = sales[0];
    assert_eq!((contract.sec_type.as_str(), contract.strike, contract.right.as_str()), ("OPT", 6680.0, "P"), "the long put of the spread");
    assert_eq!((order.order_type.as_str(), order.total_quantity, order.handling.transmit), ("MKT", 1.0, true));
    assert!(order.order_ref.ends_with("(Close)"), "{}", order.order_ref);
    assert!(app.log_lines.iter().any(|l| l.contains("Place leftover long sale")));
}

#[test]
fn ignore_and_a_long_without_a_bid_leave_the_long_alone() {
    let sold = |sent: &[TwsCommand]| sent.iter().any(|c| matches!(c, TwsCommand::PlaceOrder { order, .. } if order.action == "SELL"));
    let (_, ignoring) = stopped_short_with_leftover_long(0, Some(1.55));
    assert!(!sold(&ignoring), "Ignore: nothing is sold");
    let (app, no_bid) = stopped_short_with_leftover_long(-1, None);
    assert!(!sold(&no_bid), "no bid: nothing is sold");
    assert!(app.log_lines.iter().any(|l| l.contains("no bid - kept")), "{:?}", app.log_lines);
}

#[test]
fn a_filled_stop_ends_the_trade_as_stopped_at_the_fill_price() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    // The OCA sibling is cancelled by TWS afterwards: that must not change anything.
    app.tws_sender.send(order_status(12, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.price_close.map(|p| p.normalize().to_string()).as_deref()), ("Stopped", Some("-0.9")));
    // (0.30 - 0.90) * 100 = -60.
    let record = app.trades_today.first().unwrap();
    assert_eq!((record.status, record.current_profit), (TradeStatus::Stopped, Decimal::from(-60)));
    assert!(app.log_lines.iter().any(|l| l.contains("Trade stopped")));
    assert!(!app.log_lines.iter().any(|l| l.contains("WARNING")), "the sibling's cancel is expected");
}
