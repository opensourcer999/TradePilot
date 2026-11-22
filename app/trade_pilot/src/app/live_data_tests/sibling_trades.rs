//! Two trades of one opening order (the put and the call side of an `IronCondor`, or the sides of a split trade) are two trades: what
//! the app remembers about one of them while it runs must never be taken for the other.

use super::*;

/// Moves the test clock on by `seconds` from New York 11:00 of the test day.
fn seconds_later(app: &mut App, seconds: u64) {
    let clock = crate::clock::TestClock::at_new_york(test_today().and_hms_opt(11, 0, 0).unwrap());
    clock.advance(Duration::from_secs(seconds));
    app.clock = Box::new(clock);
}

fn log_has(app: &App, text: &str) -> bool {
    app.log_lines.iter().any(|line| line.contains(text))
}

/// The put side beyond its stop of 0.90 (it is valued at 2.65 - 1.65 = 1.00), the call side far below its stop of 2.50.
fn put_side_beyond_its_stop() -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    with_a_second_side(&mut app);
    for contract in &mut app.chains.contracts {
        match contract.con_id {
            100 => (contract.bid, contract.ask) = (Some(2.65), Some(2.75)),
            101 => (contract.bid, contract.ask) = (Some(1.55), Some(1.65)),
            _ => {}
        }
    }
    drain(&commands);
    (app, commands)
}

#[test]
fn the_beyond_stop_warning_comes_for_one_side_while_the_other_side_is_fine() {
    let (mut app, _commands) = put_side_beyond_its_stop();
    for second in [0, 30, 61] {
        seconds_later(&mut app, second);
        app.run_watchers();
    }
    let warning = app.log_lines.iter().find(|line| line.contains("Trade Beyond Stop")).cloned().unwrap_or_default();
    assert!(warning.contains("PutSpread 6690/6680 P"), "{warning}");
    assert!(!warning.contains("CallSpread"), "only the side beyond its stop: {warning}");
}

#[test]
fn a_side_far_beyond_its_stop_is_closed_while_the_other_side_stays_open() {
    let (mut app, commands) = put_side_beyond_its_stop();
    app.settings.stop_flags.close_beyond_stop_enabled = true;
    app.settings.close_beyond_stop_pct = "10".into();
    app.settings.close_beyond_stop_wait_seconds = "5".into();
    app.settings.stop_flags.close_beyond_stop_market_order = true;
    for second in [0, 6] {
        seconds_later(&mut app, second);
        app.run_watchers();
    }
    let closes: Vec<i32> =
        drain(&commands).iter().filter_map(|c| if let TwsCommand::PlaceOrder { order_id, .. } = c { Some(*order_id) } else { None }).collect();
    assert_eq!(closes.len(), 1, "one close order");
    assert_eq!(app.trades_stored[0].close_order_id, closes.first().copied(), "for the put side");
    assert_eq!(app.trades_stored[1].close_order_id, None, "the call side is left alone");
}

#[test]
fn the_limit_exits_of_two_sides_go_out_together_and_each_moves_from_its_own_price() {
    let (mut app, commands) = filled_trade_with_a_timed_exit(true);
    with_a_second_side(&mut app);
    at_new_york(&mut app, 11, 31, 0);
    app.check_timed_exits(app.clock.new_york());
    let sent = orders_sent_now(&commands);
    // The put side costs 0.40 / brings 0.20 (middle 0.30), the call side 0.60 / 0.40 (middle 0.50).
    assert_eq!(sent.iter().map(|(_, order)| order.lmt_price).collect::<Vec<_>>(), [0.3, 0.5], "both sides in one look");
    let (put_close, call_close) = (sent[0].0, sent[1].0);
    for trade in &app.trades_stored {
        let state = app.watch.progressive_close.get_mut(&trade.key()).unwrap();
        state.moved = state.moved.checked_sub(Duration::from_secs(4)).unwrap();
    }
    app.check_timed_exits(app.clock.new_york());
    let moved: Vec<(i32, f64)> = orders_sent_now(&commands).iter().map(|(id, order)| (*id, order.lmt_price)).collect();
    assert_eq!(moved, [(put_close, 0.35), (call_close, 0.55)], "each side one step up from its own price");
}

#[test]
fn a_repeated_fill_report_of_the_order_that_closed_one_side_is_no_over_fill() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    with_a_second_side(&mut app);
    fill(&mut app, 11, 1.0, 0.9);
    fill(&mut app, 22, 1.0, 0.1);
    assert_eq!((app.trades_stored[0].status.as_str(), app.trades_stored[1].status.as_str()), ("Stopped", "Closed"));
    // TWS reports the fill of the put side's stop once more.
    fill(&mut app, 11, 1.0, 0.9);
    assert!(!log_has(&app, "Over Fill"), "the same order again is no second fill");
    // The put side's target fills after its stop: the put side was closed twice.
    fill(&mut app, 12, 1.0, 0.15);
    assert!(log_has(&app, "Over Fill: SPX PutSpread"), "{:?}", app.log_lines);
}
