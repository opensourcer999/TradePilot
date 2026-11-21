//! Template previews and orders, valuing and settling open trades, the market data warnings and the mode.

use super::*;

#[test]
fn template_preview_selects_strikes_from_the_live_chain_and_sends_nothing() {
    let (mut app, commands) = connected_app();
    let (dte, expiry) = next_expiry();
    app.chains.symbols = ChainSymbol::from_settings(&[]);
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6700.0), price_at: Some(Instant::now()), chain_requested: true });
    for (i, (strike, bid, ask, delta)) in [(6690.0, 1.85, 1.95, -0.195), (6680.0, 1.55, 1.65, -0.165), (6600.0, 0.25, 0.35, -0.05)].into_iter().enumerate() {
        let con_id = 100 + i32::try_from(i).unwrap();
        app.chains.contracts.push(crate::option_chain::OptionContract {
            con_id,
            symbol: "SPX".into(),
            right: "P".into(),
            strike,
            expiry: expiry.clone(),
            local_symbol: String::new(),
            req_id: Some(500 + con_id),
            bid: Some(bid),
            ask: Some(ask),
            delta: Some(delta),
            updated: None,
            updated_at: Some(Instant::now()),
        });
    }
    app.templates = vec![TradeTemplate {
        id: 3,
        name: "SP 15D".into(),
        trade_type: "ShortPut".into(),
        underlying_symbol: "SPX".into(),
        target_type: "Delta".into(),
        target_min: 10.0,
        target_max: 20.0,
        preference: "Highest premium".into(),
        short_call_dte: dte,
        ..TradeTemplate::default()
    }];
    app.instant.template_id = Some(3);
    drain(&commands);
    app.preview_template_trade();
    let expected_date = format_date(&expiry);
    assert!(app.instant.template_preview.starts_with(&format!("SPX  Price: 6700\nPUT: 1.9 6690 {expected_date} ")), "{}", app.instant.template_preview);
    assert!(app.instant.template_preview.ends_with("Entry price (limit): 1.9") || app.instant.template_preview.ends_with("Einstiegspreis (Limit): 1.9"));
    assert!(drain(&commands).iter().all(|c| !matches!(c, TwsCommand::PlaceOrder { .. })), "PREVIEW never sends an order");

    // A template for a symbol without market data is refused.
    app.templates[0].underlying_symbol = "NDX".into();
    app.preview_template_trade();
    assert_eq!(app.instant.template_preview, "No market data is set up for NDX: trade cancelled. Check the market data symbols in Settings.");
}

#[test]
fn a_template_trade_is_confirmed_sent_as_a_bag_and_recorded_when_filled() {
    let (mut app, commands) = connected_app();
    let (dte, expiry) = next_expiry();
    app.chains.symbols = ChainSymbol::from_settings(&[]);
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6700.0), price_at: Some(Instant::now()), chain_requested: true });
    for (con_id, strike, bid, ask, delta) in [(100, 6690.0, 1.85, 1.95, -0.195), (101, 6680.0, 1.55, 1.65, -0.165)] {
        app.chains.contracts.push(crate::option_chain::OptionContract {
            con_id,
            symbol: "SPX".into(),
            right: "P".into(),
            strike,
            expiry: expiry.clone(),
            local_symbol: String::new(),
            req_id: Some(500 + con_id),
            bid: Some(bid),
            ask: Some(ask),
            delta: Some(delta),
            updated: None,
            updated_at: Some(Instant::now()),
        });
    }
    app.templates = vec![TradeTemplate {
        id: 4,
        name: "PCS".into(),
        trade_type: "PutSpread".into(),
        underlying_symbol: "SPX".into(),
        target_type: "Delta".into(),
        target_min: 10.0,
        target_max: 20.0,
        preference: "Highest premium".into(),
        long_type: "Width".into(),
        long_width: "10".into(),
        short_call_dte: dte,
        ..TradeTemplate::default()
    }];
    app.instant.template_id = Some(4);
    app.instant.template_qty = "1".into();
    app.next_order_id = Some(10);
    drain(&commands);

    // An account that does not match the mode: refused before anything is built.
    app.session.account_is_paper = Some(true);
    app.instant.account = "U1234567".into();
    app.enter_template_trade();
    assert!(app.template.pending.is_none());
    assert!(app.instant.template_status.as_deref().is_some_and(|s| s.contains("does not match the Paper Trading mode")));

    app.instant.account = "DU1234567".into();
    app.enter_template_trade();
    assert!(app.template.pending.is_some(), "{:?} / {}", app.instant.template_status, app.instant.template_preview);
    assert!(drain(&commands).iter().all(|c| !matches!(c, TwsCommand::PlaceOrder { .. })), "nothing is sent before the confirmation");
    app.confirm_template_order();
    let sent = drain(&commands);
    let Some(TwsCommand::PlaceOrder { order_id, contract, order }) = sent.into_iter().find(|c| matches!(c, TwsCommand::PlaceOrder { .. })) else {
        panic!("no order sent");
    };
    assert_eq!(order_id, 10);
    assert_eq!((contract.symbol.as_str(), contract.sec_type.as_str(), contract.exchange.as_str()), ("SPX", "BAG", "SMART"));
    let legs: Vec<(i32, &str)> = contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str())).collect();
    assert_eq!(legs, [(101, "BUY"), (100, "SELL")]);
    assert_eq!((order.action.as_str(), order.order_type.as_str(), order.lmt_price, order.account.as_str()), ("BUY", "LMT", -0.3, "DU1234567"));
    assert!(!order.handling.transmit, "held in TWS unless 'Transmit immediately' is ticked");
    assert!(!order.handling.outside_rth, "TWS refuses an extended-hours flag on a combo (error 201)");
    assert_eq!(order.order_ref, "TraPi-10");
    assert!(app.trades_today.is_empty(), "nothing recorded before the fill");

    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Submitted".into(), filled: 0.0, remaining: 1.0, avg_fill_price: 0.0 }).unwrap();
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.3 }).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_today.len(), 1, "recorded once, repeated status ignored");
    let summary = credit_summary(&app.trades_today).unwrap();
    assert_eq!(summary.premium_sold, Decimal::from(30), "0.30 credit x 1 x 100");
    assert_eq!(app.instant.template_status.as_deref(), Some("#10: Filled"));
}

#[test]
fn a_calendar_is_not_settled_at_its_near_expiration_but_its_unprotected_far_leg_is_named_once() {
    let (mut app, _commands) = connected_app();
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6645.0), price_at: Some(Instant::now()), chain_requested: true });
    // A put calendar: the near short put expires today, the far long put later.
    app.trades_stored.push(StoredTrade {
        trade_type: "PutCalendar".into(),
        strikes: "6650 P".into(),
        price_open: rust_decimal::Decimal::from_str_exact("-2").unwrap(),
        stop_order_id: Some(21),
        profit_order_id: Some(22),
        ..stored_put_spread()
    });
    app.settle_expired_trades(true, "20261113");
    app.settle_expired_trades(true, "20261113");
    assert_eq!(app.trades_stored.first().map(|t| t.status.as_str()), Some("Open"), "the far leg lives on");
    let warnings: Vec<&String> = app.log_lines.iter().filter(|l| l.contains("WARNING: calendar SPX PutCalendar 6650 P")).collect();
    assert_eq!(warnings.len(), 1, "said once: {warnings:?}");
    assert!(warnings.iter().all(|l| l.contains("stop and profit target end with the day") && l.contains("NO stop")));
    // Another day's calendar is not named.
    app.settle_expired_trades(true, "20261112");
    assert_eq!(app.log_lines.iter().filter(|l| l.contains("WARNING: calendar")).count(), 1);
}

#[test]
fn open_trades_are_valued_at_the_live_quotes_and_settle_at_the_close() {
    let (mut app, _commands) = connected_app();
    app.chains.contracts.push(quoted_option(11, "P", 6650.0, 1.00, 1.10, -0.15));
    app.chains.contracts.push(quoted_option(12, "P", 6640.0, 0.70, 0.80, -0.11));
    app.trades_stored = vec![stored_put_spread()];
    app.refresh_trades(Instant::now(), true);

    // Closing costs 1.10 - 0.70 = 0.40: (1.00 - 0.40) x 2 x 100 = 120 up, delta 4 x 2 contracts.
    let profit = profit_summary(&app.trades_today).unwrap();
    assert_eq!((profit.open, profit.closed), (Some(Decimal::from(120)), Decimal::ZERO));
    assert!((delta_total(&app.trades_today, "SPX").unwrap() - 8.0).abs() < 1e-9);
    // PCR runs live too: 120 of the 200 sold are captured. The win rate waits for a finished trade.
    let credit = credit_summary(&app.trades_today).unwrap();
    assert_eq!((credit.premium_captured, credit.pcr, credit.win_rate), (Decimal::from(120), "0.6".parse().unwrap(), Decimal::ZERO));

    // The trade list shows it with its live bid / ask to close and its profit.
    let rows = app.trade_row_views();
    assert_eq!(rows.len(), 1);
    let row = rows.first().unwrap();
    assert_eq!((row.kind.as_str(), row.strikes.as_str(), row.qty.as_str(), row.open.as_str()), ("PutSpread", "6650/6640 P", "2", "1.00"));
    assert_eq!((row.bid.as_str(), row.ask.as_str(), row.profit.as_str()), ("0.20", "0.40", "$120.00"));
    assert_eq!(row.profit_color, STATUS_OK);
    app.ui_state.trade_filter = "Closed".into();
    assert!(app.trade_row_views().is_empty(), "an open trade is not in the Closed filter");
    app.ui_state.trade_filter = "All".into();

    // The market moves against the trade: the short put now asks 2.50 -> (1.00 - 1.80) x 200.
    if let Some(c) = app.chains.contracts.iter_mut().find(|c| c.con_id == 11) {
        c.bid = Some(2.40);
        c.ask = Some(2.50);
    }
    app.refresh_trades(Instant::now(), true);
    assert_eq!(profit_summary(&app.trades_today).unwrap().open, Some(Decimal::from(-160)));
    let credit = credit_summary(&app.trades_today).unwrap();
    assert_eq!((credit.pcr, credit.win_rate), ("-0.8".parse().unwrap(), Decimal::ZERO), "losing: PCR negative, no winner");

    // A leg loses its subscription (no quote): no figure instead of a wrong one.
    if let Some(c) = app.chains.contracts.iter_mut().find(|c| c.con_id == 12) {
        c.req_id = None;
    }
    app.refresh_trades(Instant::now(), true);
    assert_eq!(profit_summary(&app.trades_today).unwrap().open, None, "no open figure without a quote");
    assert!(credit_summary(&app.trades_today).unwrap().captured_unknown, "PCR is unknown without a quote");
    if let Some(c) = app.chains.contracts.iter_mut().find(|c| c.con_id == 12) {
        c.req_id = Some(912);
    }

    // Before the close, or on another day's expiry, nothing settles.
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6645.0), price_at: Some(Instant::now()), chain_requested: true });
    app.settle_expired_trades(false, "20261113");
    app.settle_expired_trades(true, "20261112");
    assert_eq!(app.trades_stored.first().map(|t| t.status.as_str()), Some("Open"));

    // After the close SPX is 6645: the short put is 5 in the money -> PriceClose -5.
    app.settle_expired_trades(true, "20261113");
    let settled = app.trades_stored.first().cloned().unwrap();
    assert_eq!((settled.status.as_str(), settled.price_close.map(|p| p.normalize().to_string()).as_deref()), ("Expired", Some("-5")));
    app.refresh_trades(Instant::now(), true);
    // (1.00 - 5) x 2 x 100 = -800, now closed; nothing left open.
    let profit = profit_summary(&app.trades_today).unwrap();
    assert_eq!((profit.open, profit.closed), (Some(Decimal::ZERO), Decimal::from(-800)));
    assert_eq!(delta_total(&app.trades_today, "SPX"), Some(0.0));
    // Finished: 200 sold, 1000 paid at settlement -> -800 captured, PCR -4, not a winner.
    let credit = credit_summary(&app.trades_today).unwrap();
    assert_eq!((credit.premium_captured, credit.pcr, credit.win_rate), (Decimal::from(-800), Decimal::from(-4), Decimal::ZERO));
    let rows = app.trade_row_views();
    let row = rows.first().unwrap();
    assert_eq!(
        (row.kind.as_str(), row.status.as_str(), row.profit.as_str(), row.bid.as_str(), row.profit_color),
        ("PutSpread", "Expired", "-$800.00", "-", STATUS_ERROR)
    );
    app.ui_state.trade_filter = "Open".into();
    assert_eq!(app.trade_row_views(), [] as [crate::widgets::TradeRowView; 0]);
    app.ui_state.trade_filter = "Closed".into();
    assert_eq!(app.trade_row_views().len(), 1);
    // Settling again changes nothing.
    app.settle_expired_trades(true, "20261113");
    assert_eq!(app.trades_stored.first().and_then(|t| t.price_close.map(|p| p.normalize().to_string())).as_deref(), Some("-5"));
}

#[test]
fn a_trade_stored_before_v097_without_legs_stays_open_without_a_profit_figure() {
    let (mut app, _commands) = connected_app();
    app.trades_stored = vec![StoredTrade { short_put: None, long_put: None, ..stored_put_spread() }];
    app.refresh_trades(Instant::now(), true);
    assert_eq!(app.trades_today.len(), 1);
    assert_eq!(profit_summary(&app.trades_today).unwrap().open, None, "no open figure without a quote");
    // Its premium is still counted.
    assert_eq!(credit_summary(&app.trades_today).unwrap().premium_sold, Decimal::from(200));
}

#[test]
fn a_cancelled_confirmation_sends_nothing_and_a_tripped_kill_switch_still_blocks() {
    let (mut app, commands) = connected_app();
    app.session.account_is_paper = Some(true);
    app.next_order_id = Some(1);
    app.instant.sec_type = "OPT".into();
    app.instant.expiry = "20261113".into();
    app.instant.strike = "7400".into();
    app.instant.qty = "1".into();
    app.instant.limit_price = "1.00".into();
    app.submit_instant_order();
    app.instant.pending = None; // "Abbrechen"
    app.confirm_instant_order();
    assert!(drain(&commands).is_empty());

    // The gate is checked again on "Send now": the kill switch tripped while the dialog was open.
    app.submit_instant_order();
    app.kill_switch.trip();
    app.confirm_instant_order();
    assert!(drain(&commands).iter().all(|c| !matches!(c, TwsCommand::PlaceOrder { .. })));
    assert_eq!(app.instant.order_error.as_deref(), Some("paper-account safety gate not satisfied"));
}

#[test]
fn a_double_clicked_template_opens_in_the_editor() {
    let (mut app, _commands) = connected_app();
    app.templates = vec![
        TradeTemplate { id: 7, name: "IC 0DTE".into(), trade_type: "IronCondor".into(), ..TradeTemplate::default() },
        TradeTemplate { id: 8, name: "gone".into(), list_flags: ListSwitches { is_deleted: true, ..ListSwitches::default() }, ..TradeTemplate::default() },
    ];
    app.edit_template(7);
    assert_eq!(app.editing_template.as_ref().map(|f| f.name.as_str()), Some("IC 0DTE"));
    app.editing_template = None;
    app.edit_template(8);
    assert!(app.editing_template.is_none(), "deleted templates do not open");
}

#[test]
fn a_complete_instant_order_contract_gets_its_mid_as_the_limit_price() {
    let (mut app, commands) = connected_app();
    app.instant.sec_type = "OPT".into();
    app.instant.symbol = "SPX".into();
    app.instant.right = "P".into();
    app.instant.order_type = "LMT".into();
    app.instant.expiry = "20261113".into();
    app.sync_instant_quote();
    assert!(drain(&commands).is_empty(), "no strike yet: nothing requested");

    app.instant.strike = "7500".into();
    app.sync_instant_quote();
    app.sync_instant_quote();
    let sent = drain(&commands);
    let requests: Vec<(i32, IbContract)> = sent
        .into_iter()
        .filter_map(|c| match c {
            TwsCommand::RequestMarketData { req_id, contract } => Some((req_id, contract)),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1, "requested once");
    let (req_id, contract) = &requests[0];
    assert_eq!(
        (
            contract.symbol.as_str(),
            contract.trading_class.as_str(),
            contract.last_trade_date_or_contract_month.as_str(),
            contract.strike,
            contract.right.as_str()
        ),
        ("SPX", "SPXW", "20261113", 7500.0, "P")
    );

    let tws = app.tws_sender.clone();
    tws.send(TwsEvent::TickPrice { ticker_id: *req_id, tick_type: TICK_BID, price: 4.1 }).unwrap();
    tws.send(TwsEvent::TickPrice { ticker_id: *req_id, tick_type: TICK_ASK, price: 4.4 }).unwrap();
    app.poll_tws();
    let t0 = Instant::now();
    app.refresh_instant_limit(t0);
    // Mid 4.25 is off the SPX grid (0.10 steps above 3.00) - TWS answered it with error 110
    // in a real paper session; on the tick grid it becomes 4.20.
    assert_eq!(app.instant.limit_price, "4.20", "the first quote fills the field at once");

    // The mid moves (ask 4.80 -> mid 4.45 -> 4.40): the field waits 5 s before following.
    tws.send(TwsEvent::TickPrice { ticker_id: *req_id, tick_type: TICK_ASK, price: 4.8 }).unwrap();
    app.poll_tws();
    app.refresh_instant_limit(t0 + Duration::from_secs(4));
    assert_eq!(app.instant.limit_price, "4.20", "not on every tick");
    app.refresh_instant_limit(t0 + INSTANT_LIMIT_REFRESH);
    assert_eq!(app.instant.limit_price, "4.40");
    tws.send(TwsEvent::TickPrice { ticker_id: *req_id, tick_type: TICK_ASK, price: 4.4 }).unwrap();
    app.poll_tws();

    // A typed off-grid price is put on the grid before it is sent.
    app.session.account_is_paper = Some(true);
    app.next_order_id = Some(5);
    app.instant.qty = "1".into();
    app.instant.limit_price = "4.25".into();
    app.submit_instant_order();
    app.confirm_instant_order();
    let order = drain(&commands).into_iter().find_map(|c| match c {
        TwsCommand::PlaceOrder { order, .. } => Some(order),
        _ => None,
    });
    assert!((order.expect("order sent").lmt_price - 4.2).abs() < 1e-9);
    assert_eq!(app.instant.limit_price, "4.20", "the field shows what was sent");

    // A TWS rejection of that order shows up on the ticket, not only in the log.
    app.tws_sender
        .send(TwsEvent::RequestError {
            req_id: 5,
            code: 110,
            message: "Der Kurs hält nicht die mindestens erforderliche Kursabweichung für diesen Kontrakt ein.".into(),
        })
        .unwrap();
    app.poll_tws();
    assert_eq!(
        app.instant.last_order_status.as_deref(),
        Some("TWS 110: Der Kurs hält nicht die mindestens erforderliche Kursabweichung für diesen Kontrakt ein.")
    );

    // A typed price is kept even when the quote moves.
    app.instant.limit_price = "4.20".into();
    app.instant_flags.instant_limit_follows_mid = false;
    tws.send(TwsEvent::TickPrice { ticker_id: *req_id, tick_type: TICK_ASK, price: 4.6 }).unwrap();
    app.poll_tws();
    app.refresh_instant_limit(t0 + Duration::from_secs(60));
    assert_eq!(app.instant.limit_price, "4.20");

    // Another strike: the old quote is cancelled, the mid follows again.
    app.instant.strike = "7495".into();
    app.sync_instant_quote();
    let sent = drain(&commands);
    assert!(sent.iter().any(|c| matches!(c, TwsCommand::CancelMarketData { req_id: old } if old == req_id)));
    assert!(app.instant_flags.instant_limit_follows_mid);
    assert_eq!((app.instant.bid, app.instant.ask), (None, None));
}

#[test]
fn the_top_bar_reports_missing_market_data_with_twss_reason() {
    let (mut app, _commands) = connected_app();
    app.settings.lang = Lang::En;
    let start = Instant::now();
    app.session.connected_at = Some(start);
    app.request_live_data_once();
    assert_eq!(app.market_data_warning(start + Duration::from_secs(5), false), None, "too early to tell");

    // The real case: TWS answers every subscription with 10197.
    let spx = quote(&app, "SPX").req_id;
    app.tws_sender.send(TwsEvent::RequestError { req_id: spx, code: 10197, message: "Keine Marktdaten während konkurrierender Live-Sitzung".into() }).unwrap();
    // An error about something else (an order, a lookup) is not a market data problem.
    app.tws_sender.send(TwsEvent::RequestError { req_id: 99999, code: 201, message: "Order rejected".into() }).unwrap();
    app.poll_tws();
    assert_eq!(
        app.market_data_warning(start + Duration::from_secs(5), false).as_deref(),
        Some("TWS 10197: Keine Marktdaten während konkurrierender Live-Sitzung")
    );

    // Prices arrive again (the other session ended): the notice disappears.
    app.tws_sender.send(TwsEvent::TickPrice { ticker_id: spx, tick_type: TICK_LAST, price: 7531.97 }).unwrap();
    app.poll_tws();
    assert_eq!(app.market_data_warning(start + Duration::from_secs(60), false), None);
}

#[test]
fn a_broken_data_farm_is_shown_until_it_is_ok_and_then_quotes_are_requested_again() {
    let (mut app, commands) = connected_app();
    app.settings.lang = Lang::De;
    app.session.connected_at = Some(Instant::now());
    app.request_live_data_once();
    let old_ids: Vec<i32> = app.quotes.iter().map(|q| q.req_id).collect();
    drain(&commands);
    // The real sequence from 13-11-2026 19:58:28-30.
    for (farm, ok) in [("usopt", false), ("usfarm.nj", false), ("usfarm", false), ("usopt", true), ("usfarm.nj", true)] {
        app.tws_sender.send(TwsEvent::FarmStatus { farm: farm.into(), ok }).unwrap();
    }
    app.poll_tws();
    assert_eq!(app.market_data_warning(Instant::now(), true).as_deref(), Some("Datenverbindung von der TWS unterbrochen (usfarm) - Kurse sind eingefroren"));
    assert!(drain(&commands).is_empty(), "nothing re-requested while a farm is still down");

    app.tws_sender.send(TwsEvent::FarmStatus { farm: "usfarm".into(), ok: true }).unwrap();
    app.poll_tws();
    assert_eq!(app.session.broken_data_farms, [] as [std::string::String; 0]);
    let sent = drain(&commands);
    let cancelled: Vec<i32> = sent.iter().filter_map(|c| if let TwsCommand::CancelMarketData { req_id } = c { Some(*req_id) } else { None }).collect();
    assert_eq!(cancelled, old_ids, "every Live Data quote is requested again");
    assert_eq!(requested_symbols(&sent).len(), 3);
    assert!(app.quotes.iter().all(|q| !old_ids.contains(&q.req_id)), "under new ids");
}

#[test]
fn prices_that_stop_while_the_market_is_open_are_reported_as_frozen() {
    let (mut app, _commands) = connected_app();
    app.settings.lang = Lang::En;
    let last = Instant::now();
    app.session.connected_at = Some(last);
    app.session.last_price_tick_at = Some(last);
    assert_eq!(app.market_data_warning(last + Duration::from_secs(29), true), None);
    assert_eq!(app.market_data_warning(last + STALE_PRICES_AFTER, true).as_deref(), Some("no new prices from TWS for 30s"));
    assert_eq!(app.market_data_warning(last + Duration::from_secs(600), false), None, "quiet is normal after the close");
}

#[test]
fn the_offset_to_new_york_time_is_rounded_to_whole_minutes() {
    let seconds = chrono::Duration::seconds;
    assert_eq!(App::whole_minutes(seconds(21_599)), 360, "6 hours read as 5:59:59 because the two clocks are read a moment apart");
    assert_eq!(App::whole_minutes(seconds(21_601)), 360);
    assert_eq!(App::whole_minutes(seconds(-21_599)), -360);
    assert_eq!(App::whole_minutes(seconds(-21_601)), -360);
    assert_eq!(App::whole_minutes(seconds(0)), 0);
}

#[test]
fn a_closed_market_is_explained_instead_of_warned_about() {
    let (mut app, _commands) = connected_app();
    app.settings.lang = Lang::En;
    let note = |app: &App, session| app.market_closed_note(session).map(|(text, _)| text);
    assert_eq!(note(&app, Session::Open), None);
    assert_eq!(note(&app, Session::Weekend).as_deref(), Some("US market closed: weekend"));
    assert_eq!(note(&app, Session::Holiday("Thanksgiving", "Thanksgiving")).as_deref(), Some("US market closed: holiday (Thanksgiving)"));
    at_new_york(&mut app, 8, 15, 30);
    assert_eq!(note(&app, Session::BeforeOpen).as_deref(), Some("US market closed: opens in 01:14:30"));
    at_new_york(&mut app, 9, 29, 59);
    assert_eq!(note(&app, Session::BeforeOpen).as_deref(), Some("US market closed: opens in 00:00:01"));
    assert_eq!(note(&app, Session::AfterClose).as_deref(), Some("US market closed: trading is over"));
    app.settings.lang = Lang::De;
    assert_eq!(note(&app, Session::Holiday("Christmas Day", "Weihnachten")).as_deref(), Some("US-Börse geschlossen: Feiertag (Weihnachten)"));
    app.session.server_version = None;
    assert_eq!(note(&app, Session::Weekend), None, "nothing to say while disconnected");
}

#[test]
fn silence_from_tws_after_connecting_is_reported_too() {
    let (mut app, _commands) = connected_app();
    app.settings.lang = Lang::En;
    let start = Instant::now();
    app.session.connected_at = Some(start);
    assert_eq!(app.market_data_warning(start + Duration::from_secs(14), true), None);
    assert_eq!(
        app.market_data_warning(start + NO_MARKET_DATA_AFTER, true).as_deref(),
        Some("no prices from TWS for 15s - check the market data subscriptions in TWS")
    );
    // With the market closed (weekend, holiday, outside hours) the silence is normal: no warning.
    assert_eq!(app.market_data_warning(start + Duration::from_secs(600), false), None);
    app.session.server_version = None;
    assert_eq!(app.market_data_warning(start + Duration::from_secs(60), false), None, "nothing to say while disconnected");
}

#[test]
fn a_live_account_in_paper_mode_ends_the_session_and_flips_the_switch() {
    let (mut app, _commands) = connected_app();
    let stop = Arc::new(AtomicBool::new(false));
    app.session.stop = Some(stop.clone());
    app.apply_managed_accounts(accounts(&["U1111111"]));
    assert!(stop.load(Ordering::Relaxed), "session ended");
    assert!(app.link.trading_live, "switched to Live");
    assert!(app.session.managed_accounts.is_empty(), "no account offered");
    assert!(app.session.mode_mismatch.as_deref().unwrap_or_default().starts_with("Mismatch between Live/Paper trading modes."));
}

#[test]
fn a_paper_account_in_live_mode_is_a_mismatch_and_orders_stay_blocked_in_live_mode() {
    let (mut app, _commands) = connected_app();
    app.link.trading_live = true;
    app.apply_managed_accounts(accounts(&["DUR123456"]));
    assert!(!app.link.trading_live, "switched to Paper Trading");
    assert!(app.session.mode_mismatch.is_some());

    // Matching mode: accepted, no dialog.
    app.session.mode_mismatch = None;
    app.apply_managed_accounts(accounts(&["DUR123456"]));
    assert_eq!(app.instant.account, "DUR123456");
    assert!(app.session.mode_mismatch.is_none());

    // Even a paper account never gets orders while the switch says Live.
    app.session.account_is_paper = Some(true);
    assert!(app.trading_allowed());
    app.link.trading_live = true;
    assert!(!app.trading_allowed());
}

#[test]
fn paper_trading_starts_from_a_copy_of_the_existing_data_once() {
    let guard = crate::test_support::TempDir::new("seed");
    let root = guard.to_path_buf();
    let (live, paper) = (root.clone(), root.join("paper"));
    fs::create_dir_all(&live).unwrap();
    fs::write(live.join("settings.json"), "{\"live\":1}").unwrap();
    fs::write(live.join("templates.json"), "[]").unwrap();
    assert!(seed_mode_dir(&live, &paper).unwrap());
    assert_eq!(fs::read_to_string(paper.join("settings.json")).unwrap(), "{\"live\":1}");
    fs::write(paper.join("settings.json"), "{\"paper\":1}").unwrap();
    assert!(!seed_mode_dir(&live, &paper).unwrap(), "never overwrites paper data");
    assert_eq!(fs::read_to_string(paper.join("settings.json")).unwrap(), "{\"paper\":1}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn ignored_accounts_from_settings_are_left_out_of_the_dropdown() {
    let (mut app, _commands) = connected_app();
    app.link.trading_live = true;
    app.settings.settings_ignore_accounts = "U2222222".to_string();
    app.apply_managed_accounts(accounts(&["U1111111", "U2222222"]));
    assert_eq!(app.session.managed_accounts, ["U1111111"]);
    assert_eq!(app.instant.account, "U1111111");
}

#[test]
fn after_the_quick_attempts_a_silent_subscription_is_asked_for_again_every_ten_seconds_until_it_answers() {
    let (mut app, commands) = connected_app();
    app.request_live_data_once();
    drain(&commands);
    let mut now = quote(&app, "VIX9D").requested_at;
    for _ in 1..QUOTE_MAX_ATTEMPTS {
        now += QUOTE_ACK_TIMEOUT;
        app.retry_unanswered_quotes(now, true);
    }
    assert_eq!(quote(&app, "VIX").attempts, QUOTE_MAX_ATTEMPTS, "the quick attempts are used up");
    let quick = requested_symbols(&drain(&commands)).iter().filter(|(_, s)| s == "VIX").count();
    assert_eq!(u32::try_from(quick).unwrap(), QUOTE_MAX_ATTEMPTS - 1, "initial request + quick retries = max attempts");
    // Within ten seconds nothing is sent again; after ten seconds it is, every time.
    now += QUOTE_ACK_TIMEOUT;
    app.retry_unanswered_quotes(now, true);
    assert_eq!(requested_symbols(&drain(&commands)), Vec::<(i32, String)>::new(), "5 s after the last request is too soon");
    for round in 1..=7_u32 {
        now += QUOTE_SLOW_RETRY;
        app.retry_unanswered_quotes(now, true);
        let asked = requested_symbols(&drain(&commands)).iter().filter(|(_, s)| s == "VIX").count();
        assert_eq!(asked, 1, "round {round}");
        assert_eq!(quote(&app, "VIX").attempts, QUOTE_MAX_ATTEMPTS + round);
    }
    // The log has the hint once and the slow retries only once a minute (every sixth request).
    let still = app.log_lines.iter().filter(|l| l.contains("Still no answer from TWS for VIX - asked again")).count();
    assert_eq!(still, 1);
    assert_eq!(app.log_lines.iter().filter(|l| l.contains("asking again every 10s")).count(), 3, "one hint per silent subscription");
    // An answer ends it.
    let id = quote(&app, "VIX").req_id;
    app.tws_sender.send(TwsEvent::MarketDataAck { ticker_id: id }).unwrap();
    app.poll_tws();
    now += QUOTE_SLOW_RETRY;
    app.retry_unanswered_quotes(now, true);
    assert!(!requested_symbols(&drain(&commands)).iter().any(|(_, s)| s == "VIX"));
}
