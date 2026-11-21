//! The stop management of the settings against a fake TWS: the full stop split, the stop conversions, closing a trade beyond its stop
//! or after a margin rejection of its stop, and sending an entry again after a margin rejection.

use super::*;

/// What was sent to TWS: the orders with their contracts.
fn sent_orders(commands: &[TwsCommand]) -> Vec<(i32, IbContract, IbOrder)> {
    commands
        .iter()
        .filter_map(
            |c| if let TwsCommand::PlaceOrder { order_id, contract, order } = c { Some((*order_id, contract.clone(), (**order).clone())) } else { None },
        )
        .collect()
}

/// The orders sent, readable in a failed assertion: id, type, action, quantity, prices and legs.
fn describe(placed: &[(i32, IbContract, IbOrder)]) -> String {
    placed
        .iter()
        .map(|(id, contract, order)| {
            format!(
                "#{id} {} {} {} aux {} lmt {} legs {:?}",
                order.order_type,
                order.action,
                order.total_quantity,
                order.aux_price,
                order.lmt_price,
                legs_of(contract)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The legs an order's contract closes: the contract ids of a combo's legs, else the option itself by strike and right ("6690P"),
/// as TWS is sent a single option.
fn legs_of(contract: &IbContract) -> Vec<String> {
    if contract.combo_legs.is_empty() {
        vec![format!("{}{}", contract.strike, contract.right)]
    } else {
        contract.combo_legs.iter().map(|leg| leg.con_id.to_string()).collect()
    }
}

/// Moves the test clock on by `seconds` from New York 11:00 of the test day.
fn seconds_later(app: &mut App, seconds: u64) {
    let clock = crate::clock::TestClock::at_new_york(test_today().and_hms_opt(11, 0, 0).unwrap());
    clock.advance(Duration::from_secs(seconds));
    app.clock = Box::new(clock);
}

fn d(text: &str) -> Decimal {
    text.parse().unwrap()
}

/// Sets the bid and the ask of the option `con_id` of the chain.
fn quote_leg(app: &mut App, con_id: i32, bid: f64, ask: f64) {
    for contract in app.chains.contracts.iter_mut().filter(|c| c.con_id == con_id) {
        contract.bid = Some(bid);
        contract.ask = Some(ask);
    }
}

fn chain_option(con_id: i32, right: &str, strike: f64, expiry: &str, bid: f64, ask: f64) -> crate::option_chain::OptionContract {
    crate::option_chain::OptionContract {
        con_id,
        symbol: "SPX".into(),
        right: right.into(),
        strike,
        expiry: expiry.to_string(),
        local_symbol: String::new(),
        req_id: Some(500 + con_id),
        bid: Some(bid),
        ask: Some(ask),
        delta: Some(0.1),
        updated: None,
        updated_at: Some(Instant::now()),
    }
}

fn condor_fill(con_id: i32, side: &str, price: f64, expiry: &str) -> StoredExecution {
    StoredExecution {
        exec_id: format!("c{con_id}"),
        time: "20261116 10:00:00 America/New_York".into(),
        account: "DU1234567".into(),
        symbol: "SPX".into(),
        con_id,
        order_id: 40,
        order_ref: crate::trade_rebuild::order_ref(40, ""),
        sec_type: "OPT".into(),
        expiry: expiry.into(),
        right: String::new(),
        strike: 0.0,
        side: side.into(),
        quantity: 1.0,
        price,
    }
}

/// An open IronCondor-Full #40 (1 contract at 3.00: put side 1.80, call side 1.20) with its Short stop #41 at 6.00 on both shorts and
/// its target #42 at 1.50, the template #9 behind it, the fills of its legs, and the full stop split switched on at 0.05.
fn app_with_a_full_condor() -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = connected_app();
    let (_, expiry) = next_expiry();
    app.chains.symbols = ChainSymbol::from_settings(&[]);
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6700.0), price_at: Some(Instant::now()), chain_requested: true });
    app.chains.contracts = vec![
        chain_option(1, "P", 6600.0, &expiry, 0.50, 0.60),
        chain_option(2, "P", 6590.0, &expiry, 0.00, 0.05),
        chain_option(3, "C", 6800.0, &expiry, 1.90, 2.00),
        chain_option(4, "C", 6810.0, &expiry, 0.20, 0.25),
    ];
    app.templates = vec![TradeTemplate {
        id: 9,
        name: "ICF".into(),
        trade_type: "IronCondor-Full".into(),
        underlying_symbol: "SPX".into(),
        stop_type: "Short".into(),
        stop_order_type: "Market stop".into(),
        ..TradeTemplate::default()
    }];
    app.exec_history = vec![
        condor_fill(1, "SLD", 2.10, &expiry),
        condor_fill(2, "BOT", 0.30, &expiry),
        condor_fill(3, "SLD", 1.40, &expiry),
        condor_fill(4, "BOT", 0.20, &expiry),
    ];
    let leg = |con_id: i32, strike: &str| Some(StoredLeg { con_id, strike: strike.into() });
    app.trades_stored = vec![StoredTrade {
        day: "2026-11-16".into(),
        opened: "16-11-2026 10:00:00".into(),
        trade_type: "IronCondor-Full".into(),
        strikes: "6600/6590 P 6800/6810 C".into(),
        order_id: 40,
        qty: 1,
        price_open: d("3.00"),
        short_put: leg(1, "6600"),
        long_put: leg(2, "6590"),
        short_call: leg(3, "6800"),
        long_call: leg(4, "6810"),
        expiry: expiry.clone(),
        stop_order_id: Some(41),
        profit_order_id: Some(42),
        oca_group: Some("TradePilot-41".into()),
        stop_price: Some(d("6")),
        profit_price: Some(d("1.5")),
        adjustments: Some(StopAdjustments {
            order_type: "Market stop".into(),
            stop: Some("6".into()),
            legs: vec![(3, 1, false), (1, 1, false)],
            ..StopAdjustments::default()
        }),
        origin: Some(crate::storage::TradeOrigin { template_id: 9, root: 40, entries: 0, window_minutes: 0, trigger: 0, ratio_open: None }),
        ..stored_put_spread()
    }];
    app.settings.convert_flags.iron_fly_stop_split_enabled = true;
    app.settings.iron_fly_stop_split_price = "0.05".into();
    app.next_order_id = Some(50);
    app.session.account_is_paper = Some(true);
    (app, commands)
}

/// Lets the app look at its trades, as it does every second.
fn look(app: &mut App) {
    app.refresh_trades(app.clock.instant(), true);
}

fn cancelled(app: &mut App, order_id: i32) {
    app.tws_sender.send(order_status(order_id, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
}

#[test]
fn a_full_condor_whose_short_put_is_nearly_worthless_is_split_into_two_spreads_with_exits_of_their_own() {
    let (mut app, commands) = app_with_a_full_condor();
    quote_leg(&mut app, 1, 0.05, 0.10);
    look(&mut app);
    let first = drain(&commands);
    assert_eq!(cancelled_orders(&first), [41], "the stop goes first");
    assert!(sent_orders(&first).is_empty());

    cancelled(&mut app, 41);
    look(&mut app);
    let second = drain(&commands);
    assert_eq!(cancelled_orders(&second), [42], "then the target");
    assert!(app.trades_stored.first().unwrap().is_open(), "not split while an order is working");

    cancelled(&mut app, 42);
    look(&mut app);
    let placed = sent_orders(&drain(&commands));

    let trades = &app.trades_stored;
    assert_eq!(trades.len(), 3);
    assert!(trades[0].is_split(), "the condor is kept, as split");
    let (put, call) = (&trades[1], &trades[2]);
    assert_eq!((put.label().as_str(), put.price_open), ("SPX PutSpread 6600/6590 P", d("1.8")));
    assert_eq!((call.label().as_str(), call.price_open), ("SPX CallSpread 6800/6810 C", d("1.2")));

    // The put side costs 0.10 to close (0.10 ask of the short, no bid of the long): its target is lowered to 0.20; the call side
    // costs 1.80 and keeps 1.50.
    assert_eq!((put.profit_price, call.profit_price), (Some(d("0.2")), Some(d("1.5"))));
    assert_eq!((put.stop_price, call.stop_price), (Some(d("6")), Some(d("6"))), "the condor's stop on each side");
    assert_eq!(placed.len(), 4, "a target and a stop per side: {}", describe(&placed));
    for side in [put, call] {
        let (stop_id, target_id) = (side.stop_order_id.unwrap(), side.profit_order_id.unwrap());
        let (_, stop_contract, stop) = placed.iter().find(|(id, ..)| *id == stop_id).unwrap();
        let (_, target_contract, target) = placed.iter().find(|(id, ..)| *id == target_id).unwrap();
        let short = match (&side.short_put, &side.short_call) {
            (Some(put), _) => format!("{}P", put.strike),
            (_, Some(call)) => format!("{}C", call.strike),
            _ => unreachable!("a side has a short"),
        };
        assert_eq!((stop.order_type.as_str(), stop.aux_price, stop.total_quantity), ("STP", 6.0, 1.0));
        assert_eq!(legs_of(stop_contract), [short], "the stop buys back the side's short");
        assert_eq!(target.order_type, "LMT");
        assert_eq!(legs_of(target_contract).len(), 2, "the target closes both legs of the side");
        assert_eq!(stop.oca_group, target.oca_group, "stop and target of a side cancel each other");
    }
    assert_ne!(
        placed.iter().find(|(id, ..)| Some(*id) == put.stop_order_id).unwrap().2.oca_group,
        placed.iter().find(|(id, ..)| Some(*id) == call.stop_order_id).unwrap().2.oca_group,
        "the sides are apart"
    );

    // The Dashboard and the figures count the sides, not the condor.
    assert_eq!(app.trade_row_views().len(), 2);
    assert_eq!(app.trades_today.len(), 2);
    assert!(app.watch.split_pending.is_empty());
    look(&mut app);
    assert!(drain(&commands).is_empty(), "the sides are not split again");

    // After a restart the executions of the opening order are known: they bring back neither the condor nor a grown side.
    app.import_trades_from_executions();
    assert_eq!(app.trades_stored.len(), 3);
    assert!(app.trades_stored[0].is_split() && app.trades_stored[1].is_open() && app.trades_stored[2].is_open());
}

#[test]
fn a_condor_with_a_level_hit_before_is_split_into_sides_whose_target_ends_what_is_left() {
    let (mut app, commands) = app_with_a_full_condor();
    {
        let condor = &mut app.trades_stored[0];
        condor.qty = 2;
        condor.profit_order_id = Some(42);
        condor.exit_levels = Some(ExitLevels {
            qty: [1, 1, 0, 0],
            level: 1,
            order_ids: [Some(43), Some(42), None, None],
            closed: vec![(1, "-1.5".into())],
            prices: [Some("1.5".into()), Some("1".into()), None, None],
            legs: Vec::new(),
            outside_rth: false,
        });
    }
    for execution in &mut app.exec_history {
        execution.quantity = 2.0;
    }
    quote_leg(&mut app, 1, 0.05, 0.10);
    look(&mut app);
    cancelled(&mut app, 41);
    look(&mut app);
    cancelled(&mut app, 42);
    look(&mut app);
    drain(&commands);
    let call = app.trades_stored[2].clone();
    assert_eq!((call.qty, call.remaining_qty()), (2, 1), "one contract was closed by the level before");
    assert_eq!(call.profit_price, Some(d("1")), "the target of the level that was next");
    let target = call.profit_order_id.unwrap();
    assert_eq!(call.exit_levels.as_ref().unwrap().order_ids[1], Some(target), "the target is the next level");
    fill(&mut app, target, 1.0, 1.0);
    let call = &app.trades_stored[2];
    assert_eq!(call.status, "Closed", "the fill of the target ends the side");
    assert_eq!(call.remaining_qty(), 0);
}

#[test]
fn a_side_that_is_stopped_ends_alone_and_the_other_side_stays_open() {
    let (mut app, commands) = app_with_a_full_condor();
    quote_leg(&mut app, 1, 0.05, 0.10);
    look(&mut app);
    cancelled(&mut app, 41);
    look(&mut app);
    cancelled(&mut app, 42);
    look(&mut app);
    drain(&commands);
    let call_stop = app.trades_stored[2].stop_order_id.unwrap();
    fill(&mut app, call_stop, 1.0, 6.0);
    assert_eq!(app.trades_stored[2].status, "Stopped");
    assert!(app.trades_stored[1].is_open(), "the put side carries on");
    assert!(app.trades_stored[0].is_split());
}

#[test]
fn no_split_when_switched_off_above_the_price_or_without_the_fills_of_the_legs() {
    let (mut app, commands) = app_with_a_full_condor();
    look(&mut app);
    assert!(drain(&commands).is_empty(), "both shorts are worth more than 0.05");

    quote_leg(&mut app, 1, 0.05, 0.10);
    app.settings.convert_flags.iron_fly_stop_split_enabled = false;
    look(&mut app);
    assert!(drain(&commands).is_empty(), "switched off");

    app.settings.convert_flags.iron_fly_stop_split_enabled = true;
    app.exec_history.clear();
    look(&mut app);
    assert!(drain(&commands).is_empty(), "without the fills the sides could not be priced: the stop stays");

    let (mut vertical, commands) = app_with_a_full_condor();
    quote_leg(&mut vertical, 1, 0.05, 0.10);
    vertical.templates[0].stop_type = "Vertical".into();
    look(&mut vertical);
    assert!(drain(&commands).is_empty(), "only a Short stop is split");
}

#[test]
fn a_trade_whose_target_fills_while_the_split_waits_is_not_split() {
    let (mut app, commands) = app_with_a_full_condor();
    quote_leg(&mut app, 1, 0.05, 0.10);
    look(&mut app);
    cancelled(&mut app, 41);
    look(&mut app);
    fill(&mut app, 42, 1.0, 1.5);
    assert_eq!(app.trades_stored[0].status, "Closed");
    drain(&commands);
    look(&mut app);
    assert!(sent_orders(&drain(&commands)).is_empty());
    assert_eq!(app.trades_stored.len(), 1, "nothing is split");
    assert!(app.watch.split_pending.is_empty());
}

#[test]
fn a_cancel_of_the_split_that_is_not_confirmed_is_sent_again_after_thirty_seconds() {
    let (mut app, commands) = app_with_a_full_condor();
    quote_leg(&mut app, 1, 0.05, 0.10);
    look(&mut app);
    assert_eq!(cancelled_orders(&drain(&commands)), [41]);
    look(&mut app);
    assert!(drain(&commands).is_empty(), "waits for TWS");
    seconds_later(&mut app, 31);
    look(&mut app);
    assert_eq!(cancelled_orders(&drain(&commands)), [41]);
}

#[test]
fn the_cancelled_stop_of_a_split_raises_no_missing_stop_warning() {
    let (mut app, commands) = app_with_a_full_condor();
    app.settings.alert_flags.missing_stop_notifications_enabled = true;
    quote_leg(&mut app, 1, 0.05, 0.10);
    look(&mut app);
    cancelled(&mut app, 41);
    drain(&commands);
    assert!(!app.log_lines.iter().any(|line| line.contains("WARNING: the stop 41")), "{:?}", app.log_lines);
}

/// A put spread filled with a Vertical stop (#11 on both legs) at 0.90 and a target (#12).
fn put_spread_with_a_vertical_stop() -> (App, mpsc::Receiver<TwsCommand>) {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    for template in &mut app.templates {
        template.stop_type = "Vertical".into();
    }
    {
        let template = &mut app.template.orders.first_mut().unwrap().template;
        template.stop_type = "Vertical".into();
        template.stop_multiple = 2.0;
        template.stop_order_type = "Market stop".into();
        template.profit_target_type = "Percentage".into();
        template.profit_target = Some(50.0);
    }
    app.trades_stored.clear();
    drain(&commands);
    fill(&mut app, 10, 1.0, -0.3);
    drain(&commands);
    let trade = app.trades_stored.last().unwrap();
    assert_eq!(trade.adjustments.as_ref().unwrap().legs.len(), 2, "the stop sells the long too");
    (app, commands)
}

#[test]
fn a_vertical_stop_whose_long_falls_to_the_price_is_placed_again_on_the_short_alone() {
    let (mut app, commands) = put_spread_with_a_vertical_stop();
    app.settings.convert_flags.convert_vertical_to_short_enabled = true;
    app.settings.convert_vertical_to_short_price = "0.05".into();
    let stop = app.trades_stored.last().unwrap().stop_order_id.unwrap();
    look(&mut app);
    assert!(drain(&commands).is_empty(), "the long is worth 1.55");

    quote_leg(&mut app, 101, 0.05, 0.10);
    look(&mut app);
    assert_eq!(cancelled_orders(&drain(&commands)), [stop]);
    let trade = app.trades_stored.last().unwrap();
    assert!(trade.adjustments.as_ref().unwrap().replace_pending);
    assert_eq!(trade.adjustments.as_ref().unwrap().legs, [(100, 1, false)]);

    cancelled(&mut app, stop);
    let placed = sent_orders(&drain(&commands));
    assert_eq!(placed.len(), 1, "{}", describe(&placed));
    let (id, contract, order) = &placed[0];
    assert_eq!(legs_of(contract), ["6690P"], "the short put alone");
    assert_eq!(order.aux_price, 0.9, "the stop keeps its price");
    assert_eq!(app.trades_stored.last().unwrap().stop_order_id, Some(*id));
}

#[test]
fn a_short_stop_takes_the_long_back_once_it_has_a_bid_of_ten_cents_but_not_after_the_zero_dte_time() {
    let (mut app, commands) = put_spread_with_a_vertical_stop();
    app.settings.convert_flags.convert_vertical_to_short_enabled = true;
    app.settings.convert_vertical_to_short_price = "0.05".into();
    app.settings.convert_flags.convert_short_to_vertical_enabled = true;
    let stop = app.trades_stored.last().unwrap().stop_order_id.unwrap();
    quote_leg(&mut app, 101, 0.05, 0.10);
    look(&mut app);
    cancelled(&mut app, stop);
    drain(&commands);
    let short_stop = app.trades_stored.last().unwrap().stop_order_id.unwrap();

    quote_leg(&mut app, 101, 0.09, 0.12);
    app.watch.conversion_tried.clear();
    look(&mut app);
    assert!(drain(&commands).is_empty(), "0.09 is not enough");

    quote_leg(&mut app, 101, 0.10, 0.12);
    look(&mut app);
    assert_eq!(cancelled_orders(&drain(&commands)), [short_stop]);
    cancelled(&mut app, short_stop);
    let placed = sent_orders(&drain(&commands));
    assert_eq!(legs_of(&placed[0].1), ["100", "101"], "the long is back in the stop");

    // After the 0-DTE time (15:00 by default) the long does not come back.
    let (mut late, commands) = put_spread_with_a_vertical_stop();
    late.settings.convert_flags.convert_vertical_to_short_enabled = true;
    late.settings.convert_vertical_to_short_price = "0.05".into();
    late.settings.convert_flags.convert_short_to_vertical_enabled = true;
    let stop = late.trades_stored.last().unwrap().stop_order_id.unwrap();
    quote_leg(&mut late, 101, 0.05, 0.10);
    look(&mut late);
    cancelled(&mut late, stop);
    drain(&commands);
    at_new_york(&mut late, 15, 30, 0);
    quote_leg(&mut late, 101, 0.50, 0.60);
    late.watch.conversion_tried.clear();
    look(&mut late);
    assert_eq!(cancelled_orders(&drain(&commands)), Vec::<i32>::new());
}

#[test]
fn on_the_expiration_day_from_the_set_time_the_stop_buys_back_the_short_only() {
    let (mut app, commands) = put_spread_with_a_vertical_stop();
    app.settings.time_flags.convert_0dte_enabled = true;
    app.settings.convert_0dte_hour = "3".into();
    app.settings.convert_0dte_minute = "00".into();
    app.settings.time_flags.convert_0dte_pm = true;
    let stop = app.trades_stored.last().unwrap().stop_order_id.unwrap();
    at_new_york(&mut app, 14, 59, 0);
    look(&mut app);
    assert!(drain(&commands).is_empty(), "before 15:00");
    at_new_york(&mut app, 15, 0, 0);
    look(&mut app);
    assert_eq!(cancelled_orders(&drain(&commands)), [stop]);
    assert_eq!(app.trades_stored.last().unwrap().adjustments.as_ref().unwrap().legs, [(100, 1, false)]);
}

#[test]
fn a_trade_far_enough_beyond_its_stop_for_the_wait_is_closed_at_market() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.settings.stop_flags.close_beyond_stop_enabled = true;
    app.settings.close_beyond_stop_pct = "10".into();
    app.settings.close_beyond_stop_wait_seconds = "5".into();
    app.settings.stop_flags.close_beyond_stop_market_order = true;
    // The stop is at 0.90; the trade is valued at the short's bid less the long's ask: 2.65 - 1.65 = 1.00 (11 % above it).
    quote_leg(&mut app, 100, 2.65, 2.75);
    quote_leg(&mut app, 101, 1.55, 1.65);
    app.run_watchers();
    assert!(sent_orders(&drain(&commands)).is_empty(), "not for the wait yet");
    seconds_later(&mut app, 6);
    app.run_watchers();
    let placed = sent_orders(&drain(&commands));
    assert_eq!(placed.len(), 1, "{}", describe(&placed));
    assert_eq!(placed[0].2.order_type, "MKT");
    assert_eq!(app.trades_stored.last().unwrap().close_order_id, Some(placed[0].0));
    seconds_later(&mut app, 20);
    app.run_watchers();
    assert!(sent_orders(&drain(&commands)).is_empty(), "closed once");
}

#[test]
fn a_trade_beyond_its_stop_by_less_than_the_percent_or_switched_off_stays_open() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.settings.close_beyond_stop_pct = "10".into();
    app.settings.close_beyond_stop_wait_seconds = "0".into();
    quote_leg(&mut app, 100, 2.65, 2.75);
    quote_leg(&mut app, 101, 1.55, 1.65);
    app.run_watchers();
    seconds_later(&mut app, 2);
    app.run_watchers();
    assert!(sent_orders(&drain(&commands)).is_empty(), "switched off: a warning only");
    app.settings.stop_flags.close_beyond_stop_enabled = true;
    quote_leg(&mut app, 100, 2.60, 2.70);
    seconds_later(&mut app, 4);
    app.run_watchers();
    seconds_later(&mut app, 6);
    app.run_watchers();
    assert!(sent_orders(&drain(&commands)).is_empty(), "0.95 is only 5.6 % above the stop");
    quote_leg(&mut app, 100, 2.65, 2.75);
    seconds_later(&mut app, 8);
    app.run_watchers();
    assert_eq!(sent_orders(&drain(&commands)).len(), 1, "at 1.00 it is closed (wait 0)");
}

#[test]
fn a_stop_refused_for_margin_closes_the_trade_at_market_but_a_margin_warning_does_not() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    filled_trade_with_exits(&mut app, &commands);
    app.settings.stop_flags.close_on_stop_margin_rejection = true;
    let stop = app.trades_stored.last().unwrap().stop_order_id.unwrap();
    app.tws_sender.send(TwsEvent::RequestError { req_id: stop, code: 2148, message: "Margin warning: the account is close to a margin call".into() }).unwrap();
    app.poll_tws();
    assert!(sent_orders(&drain(&commands)).is_empty(), "a warning leaves the stop working");
    app.tws_sender.send(TwsEvent::RequestError { req_id: stop, code: 201, message: "Order rejected - reason: insufficient margin".into() }).unwrap();
    app.poll_tws();
    let placed = sent_orders(&drain(&commands));
    assert_eq!(placed.len(), 1, "{}", describe(&placed));
    assert_eq!(placed[0].2.order_type, "MKT");
}

#[test]
fn an_entry_refused_for_margin_is_sent_again_three_times_then_given_up() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    app.settings.alert_flags.retry_on_margin_rejection = true;
    drain(&commands);
    let mut order_id = 10;
    for attempt in 1..=3 {
        app.tws_sender.send(TwsEvent::RequestError { req_id: order_id, code: 201, message: "Order rejected - reason: insufficient margin".into() }).unwrap();
        app.tws_sender.send(order_status(order_id, "Cancelled", 0.0, 1.0)).unwrap();
        app.poll_tws();
        app.margin_retry_tick(app.clock.instant());
        assert!(sent_orders(&drain(&commands)).is_empty(), "attempt {attempt}: waits 20 seconds");
        seconds_later(&mut app, 21 * attempt);
        app.margin_retry_tick(app.clock.instant());
        let placed = sent_orders(&drain(&commands));
        assert_eq!(placed.len(), 1, "attempt {attempt}: {}", describe(&placed));
        assert_ne!(placed[0].0, order_id, "a new order id");
        assert_eq!(placed[0].2.lmt_price, app.template.orders[0].entry.limit.to_f64().unwrap(), "at the limit the entry started with");
        order_id = placed[0].0;
    }
    app.tws_sender.send(TwsEvent::RequestError { req_id: order_id, code: 201, message: "Order rejected - reason: insufficient margin".into() }).unwrap();
    app.poll_tws();
    seconds_later(&mut app, 200);
    app.margin_retry_tick(app.clock.instant());
    assert!(sent_orders(&drain(&commands)).is_empty(), "used up");
}

#[test]
fn an_entry_refused_for_margin_is_not_sent_again_when_switched_off() {
    let (mut app, commands) = app_with_a_sent_template_order(true);
    drain(&commands);
    app.tws_sender.send(TwsEvent::RequestError { req_id: 10, code: 201, message: "Order rejected - reason: insufficient margin".into() }).unwrap();
    app.poll_tws();
    seconds_later(&mut app, 60);
    app.margin_retry_tick(app.clock.instant());
    assert!(sent_orders(&drain(&commands)).is_empty());
}
