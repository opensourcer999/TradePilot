//! Market data lines, live data, quotes that stay silent, keeping the connection and the accounts.

use super::*;

/// The option lines leave room for every other line: the underlying, the Instant Order quote and
/// the reserve. When TWS still refuses one (101, its own windows use lines too), that option is
/// given up and the budget stays one lower, so it is not asked for again.
#[test]
fn the_option_lines_stay_inside_the_market_data_limit_and_shrink_when_tws_refuses_one() {
    let (mut app, commands) = connected_app();
    app.chains.symbols = ChainSymbol::from_settings(&[]);
    app.chains.underlyings.push(Underlying { symbol: "SPX".into(), req_id: 1, price: Some(6700.0), price_at: Some(Instant::now()), chain_requested: true });
    for strike in 0..120_u32 {
        app.chains.contracts.push(crate::option_chain::OptionContract {
            con_id: 1000 + i32::try_from(strike).unwrap(),
            symbol: "SPX".into(),
            right: "P".into(),
            strike: 6600.0 + f64::from(strike),
            expiry: "20261118".into(),
            local_symbol: String::new(),
            req_id: None,
            bid: None,
            ask: None,
            delta: None,
            updated: None,
            updated_at: None,
        });
    }
    app.instant.quote_req_id = Some(2);
    app.manage_chain_subscriptions(Instant::now(), true);
    drain(&commands);
    // 100 lines - the underlying - the Instant Order quote - 4 in reserve.
    assert_eq!(app.chains.subscribed_count(), 94);
    let refused = app.chains.contracts.iter().find_map(|c| c.req_id).unwrap();
    app.tws_sender.send(TwsEvent::RequestError { req_id: refused, code: 101, message: "Max number of tickers has been reached".into() }).unwrap();
    app.poll_tws();
    assert_eq!(app.chains.subscribed_count(), 93, "{:?}", app.log_lines);
    assert!(app.log_lines.iter().any(|l| l.contains("WARNING: TWS refused a market data line")));
    assert_eq!(app.session.market_data_error, None, "a full line limit is not a missing data subscription");
    // The next planning keeps the lower budget.
    app.manage_chain_subscriptions(Instant::now(), true);
    assert_eq!(app.chains.subscribed_count(), 93);
    // An id that is no market data line of this app changes nothing.
    app.tws_sender.send(TwsEvent::RequestError { req_id: 987_654, code: 101, message: "Max number of tickers has been reached".into() }).unwrap();
    app.poll_tws();
    assert_eq!(app.session.market_data_lines_refused, 1);
}

/// A request that cannot be handed to the connection thread (the session has ended) is no longer
/// dropped silently: the log names how many were not sent.
#[test]
fn market_data_requests_that_cannot_be_handed_over_are_logged() {
    let (mut app, commands) = connected_app();
    drop(commands);
    app.request_live_data_once();
    let line = app.log_lines.iter().find(|l| l.contains("NOT sent")).cloned().unwrap_or_default();
    assert!(line.contains("WARNING: 4 TWS request(s) NOT sent (request live data once)"), "{line}");
    assert!(line.contains("the connection to TWS has ended"), "{line}");
}

#[test]
fn live_data_requests_spx_vix_and_vix9d_as_cboe_indices_and_shows_a_row_for_each() {
    let (mut app, commands) = connected_app();
    app.request_live_data_once();

    let labels: Vec<&str> = app.quotes.iter().map(|q| q.label.as_str()).collect();
    assert_eq!(labels, ["SPX", "VIX", "VIX9D"]);

    let sent = drain(&commands);
    let requested = requested_symbols(&sent);
    assert_eq!(requested.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["SPX", "VIX", "VIX9D"]);
    for command in &sent {
        if let TwsCommand::RequestMarketData { req_id, contract } = command {
            assert_eq!((contract.sec_type.as_str(), contract.exchange.as_str(), contract.currency.as_str()), ("IND", "CBOE", "USD"));
            assert_eq!(quote(&app, &contract.symbol).req_id, *req_id, "row and request must share the ticker id");
        }
    }
    let mut ids: Vec<i32> = requested.iter().map(|(id, _)| *id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "every symbol needs its own ticker id");
}

#[test]
fn live_data_is_requested_only_once_per_session() {
    let (mut app, commands) = connected_app();
    app.request_live_data_once();
    app.request_live_data_once();
    assert_eq!(app.quotes.len(), 3);
    assert_eq!(requested_symbols(&drain(&commands)).len(), 3);
}

#[test]
fn ticks_for_each_symbol_land_in_that_symbols_row() {
    let (mut app, _commands) = connected_app();
    app.request_live_data_once();
    let tws = app.tws_sender.clone();
    for (label, price) in [("SPX", 7590.57), ("VIX", 17.12), ("VIX9D", 17.59)] {
        let ticker_id = quote(&app, label).req_id;
        tws.send(TwsEvent::TickPrice { ticker_id, tick_type: TICK_LAST, price }).unwrap();
    }
    app.poll_tws();
    assert_eq!(quote(&app, "SPX").last, Some(7590.57));
    assert_eq!(quote(&app, "VIX").last, Some(17.12));
    assert_eq!(quote(&app, "VIX9D").last, Some(17.59));
}

#[test]
fn a_symbol_without_any_tick_shows_its_last_daily_close_until_a_real_tick_arrives() {
    let (mut app, commands) = connected_app();
    app.request_live_data_once();
    drain(&commands);
    let spx_id = quote(&app, "SPX").req_id;
    app.tws_sender.send(TwsEvent::MarketDataAck { ticker_id: spx_id }).unwrap();
    app.poll_tws();
    let start = quote(&app, "VIX9D").requested_at;

    // Before the timeout nothing is asked; after it, once per unanswered symbol.
    app.request_unanswered_history(start + Duration::from_secs(1));
    assert!(drain(&commands).is_empty());
    app.request_unanswered_history(start + QUOTE_ACK_TIMEOUT);
    let asked: Vec<(i32, String)> = drain(&commands)
        .iter()
        .filter_map(|c| if let TwsCommand::RequestDailyBars { req_id, contract } = c { Some((*req_id, contract.symbol.clone())) } else { None })
        .collect();
    assert_eq!(asked.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["VIX", "VIX9D"], "SPX was answered");
    app.request_unanswered_history(start + QUOTE_ACK_TIMEOUT * 3);
    assert!(drain(&commands).is_empty(), "asked only once");

    // The last bar (they arrive oldest first) is shown as a close, never as a live price.
    let vix_req = asked.first().map(|(id, _)| *id).unwrap();
    for (date, close) in [("20261112", 18.0), ("20261113", 19.3)] {
        app.tws_sender.send(TwsEvent::HistoricalBar { req_id: vix_req, date: date.into(), high: close, low: close, close }).unwrap();
    }
    app.poll_tws();
    assert_eq!(quote(&app, "VIX").history, Some((19.3, "20261113".to_string())));
    assert_eq!(quote(&app, "VIX").last, None, "a daily close is not a live tick");
    assert_eq!(app.underlying_price("VIX"), None, "and it is never used as the price to trade on");
    assert_eq!(quote(&app, "VIX").reference_close(), Some(19.3), "the last bar before today is yesterday's close");
    // Today's bar, still forming while the market is open, is no yesterday's close: it shows as the latest value only.
    app.tws_sender.send(TwsEvent::HistoricalBar { req_id: vix_req, date: "20261116".into(), high: 25.5, low: 25.5, close: 25.5 }).unwrap();
    app.poll_tws();
    assert_eq!(quote(&app, "VIX").history, Some((25.5, "20261116".to_string())));
    assert_eq!(quote(&app, "VIX").reference_close(), Some(19.3), "unchanged by today's bar");
    app.tws_sender.send(TwsEvent::HistoricalBar { req_id: vix_req, date: "20261113".into(), high: 19.3, low: 19.3, close: 19.3 }).unwrap();
    app.poll_tws();

    // A real tick takes over the display.
    app.tws_sender.send(TwsEvent::TickPrice { ticker_id: quote(&app, "VIX").req_id, tick_type: TICK_LAST, price: 19.5 }).unwrap();
    app.poll_tws();
    assert_eq!(quote(&app, "VIX").last, Some(19.5));
}

#[test]
fn a_volatility_index_without_ticks_while_the_market_is_open_is_fed_by_its_one_minute_bars() {
    let (mut app, commands) = connected_app();
    app.request_live_data_once();
    drain(&commands);
    let start = quote(&app, "VIX").requested_at;
    app.request_unanswered_history(start + QUOTE_ACK_TIMEOUT);
    let asked: Vec<(i32, String)> = drain(&commands)
        .iter()
        .filter_map(|c| if let TwsCommand::RequestLiveBars { req_id, contract } = c { Some((*req_id, contract.symbol.clone())) } else { None })
        .collect();
    assert_eq!(asked.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["VIX", "VIX9D"], "SPX is traded on and is never fed by bars");
    app.request_unanswered_history(start + QUOTE_ACK_TIMEOUT * 3);
    assert!(!drain(&commands).iter().any(|c| matches!(c, TwsCommand::RequestLiveBars { .. })), "asked only once");

    // The bars arrive oldest first and the one still forming keeps updating the newest.
    let vix_req = asked.first().map(|(id, _)| *id).unwrap();
    for (date, close) in [("20261117 09:30:00", 23.0), ("20261117 09:31:00", 23.07)] {
        app.tws_sender.send(TwsEvent::HistoricalBar { req_id: vix_req, date: date.into(), high: close, low: close, close }).unwrap();
    }
    app.poll_tws();
    assert_eq!(quote(&app, "VIX").last, Some(23.07));
    assert!(quote(&app, "VIX").acknowledged, "the subscription is no longer reported as silent");
    assert_eq!(quote(&app, "SPX").last, None);
    assert_eq!(app.condition_snapshot().vix, Some(Decimal::new(2307, 2)), "a condition on the VIX can be checked now");
}

#[test]
fn the_daily_close_is_asked_for_right_after_the_first_timeout_not_after_the_last_retry() {
    // The retry of the live request restarts its clock every 5 s; the daily close must not
    // wait for those retries to stop.
    let (mut app, commands) = connected_app();
    app.request_live_data_once();
    drain(&commands);
    let start = quote(&app, "VIX9D").requested_at;
    app.retry_unanswered_quotes(start + QUOTE_ACK_TIMEOUT, true);
    drain(&commands);
    app.request_unanswered_history(start + QUOTE_ACK_TIMEOUT);
    let asked = drain(&commands).iter().filter(|c| matches!(c, TwsCommand::RequestDailyBars { .. })).count();
    assert_eq!(asked, 3, "all three unanswered symbols, right after the first retry");
}

#[test]
fn silent_subscriptions_are_not_re_requested_while_the_market_is_closed() {
    // The real session of Saturday 14-11-2026: no ticks for SPX and VIX, five retries each and
    // a hint about missing permissions - while nothing was wrong.
    let (mut app, commands) = connected_app();
    app.request_live_data_once();
    drain(&commands);
    let vix_id = quote(&app, "VIX").req_id;
    let mut now = quote(&app, "VIX9D").requested_at;
    for _ in 0..QUOTE_MAX_ATTEMPTS + 2 {
        now += QUOTE_ACK_TIMEOUT;
        app.retry_unanswered_quotes(now, false);
    }
    assert!(drain(&commands).is_empty(), "no retry outside the market hours");
    assert_eq!((quote(&app, "VIX").req_id, quote(&app, "VIX").attempts), (vix_id, 1));
    assert!(!app.log_lines.iter().any(|l| l.contains("market data permissions")), "no wrong hint");
    // The latest daily close is still asked for, once.
    app.request_unanswered_history(now);
    assert!(drain(&commands).iter().any(|c| matches!(c, TwsCommand::RequestDailyBars { .. })));
    // The market opens: the long-silent subscriptions are re-requested at once.
    app.retry_unanswered_quotes(now, true);
    let re_requested = requested_symbols(&drain(&commands));
    assert!(re_requested.iter().any(|(_, s)| s == "VIX"), "{re_requested:?}");
    assert_eq!(quote(&app, "VIX").attempts, 2);
}

#[test]
fn a_subscription_tws_never_answers_is_re_requested_under_a_new_id() {
    // Reproduces the real session: TWS answered SPX (tickReqParams/tickString) but sent
    // nothing at all for VIX and VIX9D.
    let (mut app, commands) = connected_app();
    app.request_live_data_once();
    drain(&commands);
    let spx_id = quote(&app, "SPX").req_id;
    let old_vix_id = quote(&app, "VIX").req_id;
    let old_vix9d_id = quote(&app, "VIX9D").req_id;
    app.tws_sender.send(TwsEvent::MarketDataAck { ticker_id: spx_id }).unwrap();
    app.poll_tws();

    // The latest of the three request times, so every row's timeout has fully elapsed below.
    let start = quote(&app, "VIX9D").requested_at;
    app.retry_unanswered_quotes(start + Duration::from_secs(1), true);
    assert!(drain(&commands).is_empty(), "must wait for the timeout before retrying");

    app.retry_unanswered_quotes(start + QUOTE_ACK_TIMEOUT, true);
    let sent = drain(&commands);
    let cancelled: Vec<i32> = sent
        .iter()
        .filter_map(|c| match c {
            TwsCommand::CancelMarketData { req_id } => Some(*req_id),
            _ => None,
        })
        .collect();
    assert_eq!(cancelled, [old_vix_id, old_vix9d_id]);
    let re_requested = requested_symbols(&sent);
    assert_eq!(re_requested.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["VIX", "VIX9D"]);
    assert_eq!(quote(&app, "SPX").req_id, spx_id, "an answered subscription is left alone");
    assert_ne!(quote(&app, "VIX").req_id, old_vix_id);
    assert_eq!(quote(&app, "VIX").req_id, re_requested[0].0);
    assert_eq!(quote(&app, "VIX9D").req_id, re_requested[1].0);

    // TWS now answers the retried VIX request: its ticks reach the VIX row.
    let new_vix_id = quote(&app, "VIX").req_id;
    app.tws_sender.send(TwsEvent::TickPrice { ticker_id: new_vix_id, tick_type: TICK_DELAYED_LAST, price: 17.12 }).unwrap();
    app.poll_tws();
    assert_eq!(quote(&app, "VIX").last, Some(17.12));
    assert!(quote(&app, "VIX").acknowledged);
}

#[test]
fn after_the_user_disconnects_maintain_connection_does_not_reconnect_until_connect_is_clicked() {
    let mut app = offline_app("Always");
    let start = app.session.last_broker_check;
    let stop = Arc::new(AtomicBool::new(false));
    app.session.stop = Some(stop.clone());
    app.disconnect_by_user();
    assert!(stop.load(Ordering::Relaxed), "session told to end");
    app.session.stop = None; // the worker has ended
    for minute in 1..=5 {
        assert_eq!(app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL * minute, true), MaintainConnectionAction::None, "minute {minute}");
    }
    // Clicking Connect (as the top bar's "Connect" action does) re-arms Maintain Connection.
    app.link.user_disconnected = false;
    assert_eq!(app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL * 7, true), MaintainConnectionAction::Reconnect { notify: false });
}

#[test]
fn maintain_always_reconnects_once_a_minute_and_notifies_on_the_third_attempt() {
    let mut app = offline_app("Always");
    let start = app.session.last_broker_check;
    assert_eq!(app.maintain_connection_due(start + Duration::from_secs(59), false), MaintainConnectionAction::None);
    let results: Vec<MaintainConnectionAction> =
        (1..=4).map(|minute| app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL * minute, false)).collect();
    assert_eq!(
        results,
        [
            MaintainConnectionAction::Reconnect { notify: false },
            MaintainConnectionAction::Reconnect { notify: false },
            MaintainConnectionAction::Reconnect { notify: true },
            MaintainConnectionAction::Reconnect { notify: false },
        ]
    );
}

#[test]
fn maintain_market_hours_only_reconnects_while_the_market_is_open() {
    let mut app = offline_app("Market Hours");
    let start = app.session.last_broker_check;
    assert_eq!(app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL, false), MaintainConnectionAction::None);
    assert_eq!(app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL * 2, true), MaintainConnectionAction::Reconnect { notify: false });
}

#[test]
fn maintain_disabled_never_reconnects() {
    let mut app = offline_app("Disabled");
    let start = app.session.last_broker_check;
    for minute in 1..=5 {
        assert_eq!(app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL * minute, true), MaintainConnectionAction::None);
    }
}

#[test]
fn maintain_leaves_an_active_or_connecting_session_alone_and_a_connect_resets_the_count() {
    let mut app = offline_app("Always");
    let start = app.session.last_broker_check;
    app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL, false);
    app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL * 2, false);
    assert_eq!(app.session.broker_connection_retries, 2);
    app.session.stop = Some(Arc::new(AtomicBool::new(false)));
    assert_eq!(app.maintain_connection_due(start + MAINTAIN_CONNECTION_INTERVAL * 3, false), MaintainConnectionAction::None);
    assert!(app.gauge_sweep().is_none(), "no sweep before connecting");
    app.tws_sender.send(TwsEvent::Connected { server_version: "200".to_string() }).unwrap();
    app.poll_tws();
    assert_eq!(app.session.broker_connection_retries, 0);
    // Connecting starts the gauges' sweep; a few seconds later it is over.
    assert!(app.gauge_sweep().is_some_and(|t| t < 1.0));
    app.gauge_sweep_start = Instant::now().checked_sub(Duration::from_secs(5));
    assert!(app.gauge_sweep().is_none());
}

#[test]
fn a_saved_enabled_maintain_setting_from_earlier_builds_becomes_always() {
    let mut app = offline_app("Disabled");
    let mut settings = app.current_settings();
    settings.settings_maintain_connection = "Enabled".to_string();
    app.apply_settings(settings);
    assert_eq!(app.settings.settings_maintain_connection, "Always");
}

#[test]
fn account_dropdown_preselects_the_second_account_when_there_are_several() {
    let (mut app, _commands) = connected_app();
    app.link.trading_live = true;
    app.settings.settings_ignore_accounts.clear();
    app.tws_sender.send(TwsEvent::ManagedAccounts(accounts(&["U1111111", "U2222222"]))).unwrap();
    app.poll_tws();
    assert_eq!(app.session.managed_accounts, ["U1111111", "U2222222"]);
    assert_eq!(app.instant.account, "U2222222");
}

#[test]
fn account_dropdown_preselects_the_only_account() {
    let (mut app, _commands) = connected_app();
    app.settings.settings_ignore_accounts.clear();
    app.apply_managed_accounts(accounts(&["DU1234567"]));
    assert_eq!(app.instant.account, "DU1234567");
}
