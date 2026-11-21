//! Executions and rows of the Dashboard, notices, the CSV, and the Instant Order page.

use super::*;

#[test]
fn executions_are_listed_newest_first_and_equal_times_keep_the_later_one_on_top() {
    let at = |time: &str, exec_id: &str| ExecutionRow {
        account: String::new(),
        symbol: "SPX".into(),
        con_id: 0,
        time: time.to_string(),
        exec_id: exec_id.to_string(),
        order_id: 0,
        order_ref: String::new(),
        expiry: String::new(),
        sec_type: "OPT".into(),
        right: "P".into(),
        strike: 0.0,
        side: "BOT".into(),
        quantity: 1.0,
        price: 1.0,
    };
    let list = [at("20261116 09:35:12", "a"), at("20261116 10:02:00", "b"), at("20261116 09:35:12", "c"), at("20261115 15:59:59", "d")];
    let ids: Vec<&str> = newest_first(&list).iter().map(|e| e.exec_id.as_str()).collect();
    assert_eq!(ids, ["b", "c", "a", "d"]);
    assert_eq!(newest_first(&[]).len(), 0);
}

#[test]
fn a_row_of_the_trade_list_knows_its_stored_trade_so_a_click_can_select_it_for_closing() {
    let (mut app, _commands) = connected_app();
    let open = |strikes: &str, status: &str| StoredTrade { strikes: strikes.into(), status: status.into(), ..stored_put_spread() };
    app.trades_stored = vec![open("7000/6990 P", "Open"), open("7100/7090 P", "Expired"), open("7200/7190 P", "Open")];
    // Newest first; the index is the position in the stored list, whatever the filter hides.
    let all: Vec<(usize, String)> = app.trade_row_views().iter().map(|r| (r.index, r.strikes.clone())).collect();
    assert_eq!(all, [(2, "7200/7190 P".to_string()), (1, "7100/7090 P".to_string()), (0, "7000/6990 P".to_string())]);
    app.ui_state.trade_filter = "Open".into();
    assert_eq!(app.trade_row_views().iter().map(|r| r.index).collect::<Vec<_>>(), [2, 0]);
    // The selected trade is the one that the close request then names.
    app.ui_state.selected_trade = Some(2);
    app.request_close(2, CloseKind::Market);
    assert_eq!(app.close_pending.as_ref().map(|p| p.index), Some(2));
    // A trade that is no longer open cannot be closed.
    app.close_pending = None;
    app.request_close(1, CloseKind::Market);
    assert!(app.close_pending.is_none());
}

#[test]
fn a_notification_email_that_cannot_be_sent_says_why_in_the_log_and_on_the_settings_page() {
    let (mut app, _commands) = connected_app();
    app.settings.smtp_host = "securesmtp.example.de".into();
    app.settings.smtp_username = "me@example.de".into();
    // No recipient: not silent.
    app.settings.notification_email = String::new();
    app.notify_trade_event("sample", "hello", true, false);
    app.poll_tws();
    assert!(app.log_lines.iter().any(|l| l.contains("Notification email NOT sent (\"TradePilot: sample\"): no recipient address")), "{:?}", app.log_lines);
    assert!(app.ui_state.mail_status.as_deref().is_some_and(|s| s.contains("NOT sent")));
    // Port 465 with STARTTLS on would hang: it is named at once instead.
    app.settings.notification_email = "you@example.de".into();
    app.settings.smtp_port = "465".into();
    app.settings.mail_flags.smtp_use_tls = true;
    app.notify_trade_event("sample", "hello", true, false);
    app.poll_tws();
    assert!(app.log_lines.iter().any(|l| l.contains("port 465 uses TLS from the start: switch STARTTLS off")), "{:?}", app.log_lines);
    assert!(app.ui_state.mail_status.as_deref().is_some_and(|s| s.contains("switch STARTTLS off")));
}

#[test]
fn a_refusal_fades_after_five_seconds_but_a_newer_status_stays() {
    let (mut app, _commands) = app_with_a_chain_and_a_template(SCHEDULE_EXPIRY, 0);
    let ctx = egui::Context::default();
    app.show_notice("Not sent: no trade found".into());
    app.expire_notice(&ctx);
    assert_eq!(app.instant.template_status.as_deref(), Some("Not sent: no trade found"), "still shown right away");
    let now = app.clock.instant();
    app.instant.template_notice = app.instant.template_notice.take().map(|(text, _)| (text, now));
    app.expire_notice(&ctx);
    assert_eq!(app.instant.template_status, None, "gone once its time is up");
    // An order status that came in meanwhile is not taken away with it.
    app.show_notice("Not sent: x".into());
    app.instant.template_status = Some("#10: Filled".into());
    app.instant.template_notice = app.instant.template_notice.take().map(|(text, _)| (text, now));
    app.expire_notice(&ctx);
    assert_eq!(app.instant.template_status.as_deref(), Some("#10: Filled"));
}

#[test]
fn a_stop_that_covers_fewer_contracts_than_the_trade_holds_is_logged_once_and_shown_on_the_dashboard() {
    let (mut app, _commands) = app_with_a_chain_and_a_template(SCHEDULE_EXPIRY, 0);
    let mut trade: StoredTrade = serde_json::from_str(
        r#"{"day":"2026-11-19","opened":"","account":"DU1","symbol":"SPX","trade_type":"PutSpread","strikes":"7235/7225 P",
            "order_id":138,"qty":5,"multiplier":100,"price_open":"1.3","delta":0.0}"#,
    )
    .unwrap();
    trade.stop_order_id = Some(141);
    app.trades_stored = vec![trade];
    app.exit_coverage.remaining.insert(141, 4.0);
    app.check_stop_coverage();
    app.check_stop_coverage();
    let warnings = app.log_lines.iter().filter(|line| line.contains("covers 4 of 5 contracts")).count();
    assert_eq!(warnings, 1, "said once, not every second: {:?}", app.log_lines);
    let views = app.trade_row_views();
    assert!(views.first().and_then(|view| view.stop_note.as_deref()).is_some_and(|note| note.contains('4') && note.contains('5')));
    // The stop is changed to cover everything: the warning is forgotten, a later gap is said again.
    app.exit_coverage.remaining.insert(141, 5.0);
    app.check_stop_coverage();
    assert!(app.trade_row_views().first().is_some_and(|view| view.stop_note.is_none()));
    app.exit_coverage.remaining.insert(141, 3.0);
    app.check_stop_coverage();
    assert_eq!(app.log_lines.iter().filter(|line| line.contains("covers 3 of 5 contracts")).count(), 1);
}

#[test]
fn the_trades_of_the_day_are_exported_once_after_the_close_and_only_when_it_is_switched_on() {
    let (mut app, _commands) = app_with_a_chain_and_a_template(SCHEDULE_EXPIRY, 0);
    let after_close = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(17, 0, 0).unwrap();
    app.clock = Box::new(crate::clock::TestClock::at_new_york(after_close));
    app.daily_csv_tick();
    assert_eq!(app.csv_exported_day, None, "switched off: nothing is done");
    app.settings.daily_csv_export = true;
    app.clock = Box::new(crate::clock::TestClock::at_new_york(chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(11, 0, 0).unwrap()));
    app.daily_csv_tick();
    assert_eq!(app.csv_exported_day, None, "the market is open: not yet");
    app.clock = Box::new(crate::clock::TestClock::at_new_york(after_close));
    app.daily_csv_tick();
    let day = app.csv_exported_day.clone();
    assert!(day.is_some(), "after the close it is done");
    app.daily_csv_tick();
    assert_eq!(app.csv_exported_day, day, "once a day");
}

#[test]
fn the_csv_names_the_strategy_of_each_trade() {
    let (mut app, _commands) = app_with_a_chain_and_a_template(SCHEDULE_EXPIRY, 0);
    let mut trade: StoredTrade = serde_json::from_str(
        r#"{"day":"2026-11-19","opened":"","account":"DU1","symbol":"SPX","trade_type":"PutSpread","strikes":"7235/7225 P",
            "order_id":138,"qty":1,"multiplier":100,"price_open":"1.3","delta":0.0}"#,
    )
    .unwrap();
    trade.strategy = "BPS".to_string();
    app.trade_log = vec![trade];
    let csv = app.csv_of(app.trade_log.iter().collect());
    let mut lines = csv.lines();
    assert!(lines.next().is_some_and(|header| header.contains("Trade,Strategy,Strikes")), "{csv}");
    assert!(lines.next().is_some_and(|row| row.contains("PutSpread,BPS,7235/7225 P")), "{csv}");
}

#[test]
fn enter_trade_without_a_found_trade_says_why_none_was_found() {
    let (mut app, commands) = app_with_a_chain_and_a_template(SCHEDULE_EXPIRY, 0);
    for underlying in &mut app.chains.underlyings {
        underlying.price = None;
    }
    app.session.account_is_paper = Some(true);
    app.instant.template_id = Some(4);
    app.instant.template_qty = "1".into();
    app.instant.account = "DU1234567".into();
    app.enter_template_trade();
    let status = app.instant.template_status.clone().unwrap_or_default();
    assert!(status.contains("no trade found - SPX: no price from TWS yet"), "{status}");
    assert!(app.template.pending.is_none());
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
}

#[test]
fn an_execution_time_is_shown_with_the_date_of_the_screens_and_odd_text_stays() {
    assert_eq!(
        execution_time_text("20261116 09:35:12"),
        crate::i18n::show_date_time(chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(9, 35, 12).unwrap())
    );
    assert_eq!(execution_time_text("  20261116 09:35:12 US/Eastern"), execution_time_text("20261116 09:35:12"), "a zone name after the time is ignored");
    assert_eq!(execution_time_text(""), "");
    assert_eq!(execution_time_text("yesterday"), "yesterday");
}

#[test]
fn dates_are_shown_in_the_style_of_the_language_and_typed_ones_go_to_tws_as_yyyymmdd() {
    assert_eq!(format_date("20261113"), show_date(chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()));
    assert_eq!(format_date("202612"), "202612", "a contract month stays as it is");
    assert_eq!(parse_date_input("13.11.2026").as_deref(), Some("20261113"));
    assert_eq!(parse_date_input("2026-11-13").as_deref(), Some("20261113"));
    assert_eq!(parse_date_input("13.11.2026").as_deref(), Some("20261113"));
    assert_eq!(parse_date_input("2026-11-13").as_deref(), Some("20261113"));
    assert_eq!(parse_date_input("20261113").as_deref(), Some("20261113"));
    assert_eq!(parse_date_input(""), None);
}

#[test]
fn the_strike_list_opens_at_the_strike_nearest_the_price() {
    let strikes = [7490.0, 7495.0, 7500.0, 7505.0, 7510.0];
    assert_eq!(nearest_strike_index(&strikes, 7531.97), Some(4));
    assert_eq!(nearest_strike_index(&strikes, 7503.1), Some(3));
    assert_eq!(nearest_strike_index(&strikes, 7497.5), Some(1), "tie: the lower one");
    assert_eq!(nearest_strike_index(&[], 7500.0), None);

    // The price comes from Live Data while no option chain is loaded.
    let (mut app, _commands) = connected_app();
    app.request_live_data_once();
    let spx = quote(&app, "SPX").req_id;
    app.tws_sender.send(TwsEvent::TickPrice { ticker_id: spx, tick_type: TICK_LAST, price: 7531.97 }).unwrap();
    app.poll_tws();
    assert_eq!(app.underlying_price("SPX"), Some(7531.97));
}

#[test]
fn a_filled_instant_option_order_becomes_a_trade_on_the_dashboard() {
    let (mut app, _commands) = app_with_a_sent_instant_option("SELL", "P");
    // Not yet filled, or only partly: nothing is recorded.
    app.tws_sender.send(order_status(30, "Submitted", 0.0, 2.0)).unwrap();
    app.tws_sender.send(order_status(30, "Filled", 1.0, 1.0)).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.len(), 0);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 30, status: "Filled".into(), filled: 2.0, remaining: 0.0, avg_fill_price: 1.45 }).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.len(), 1);
    let trade = &app.trades_stored[0];
    assert_eq!((trade.trade_type.as_str(), trade.strikes.as_str(), trade.qty, trade.expiry.as_str()), ("ShortPut", "7400 P", 2, "20261113"));
    assert_eq!((trade.price_open, trade.multiplier), (Decimal::new(145, 2), 100), "a sold option is a credit trade");
    assert!(trade.short_put.is_some() && trade.long_put.is_none());
    assert_eq!(app.trades_today.len(), 1, "it is on the Dashboard");
    assert!(app.log_lines.iter().any(|l| l.contains("Trade opened: SPX ShortPut 7400 P x2 @ 1.45 (instant order 30)")));
    // The same report again records nothing a second time.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 30, status: "Filled".into(), filled: 2.0, remaining: 0.0, avg_fill_price: 1.45 }).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.len(), 1);
}

#[test]
fn a_bought_instant_option_is_a_debit_trade_and_a_buy_that_closes_a_short_is_not_a_new_trade() {
    let (mut app, _commands) = app_with_a_sent_instant_option("BUY", "C");
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 30, status: "Filled".into(), filled: 2.0, remaining: 0.0, avg_fill_price: 0.80 }).unwrap();
    app.poll_tws();
    let trade = &app.trades_stored[0];
    assert_eq!((trade.trade_type.as_str(), trade.price_open), ("LongCall", Decimal::new(-80, 2)), "a bought option is a debit trade");
    assert!(trade.long_call.is_some());
    // Buying back an option of an open short trade is a closing order.
    let (mut app, _commands) = app_with_a_sent_instant_option("BUY", "P");
    app.trades_stored.push(StoredTrade {
        account: "DU1234567".into(),
        expiry: "20261113".into(),
        short_put: Some(StoredLeg { con_id: 1, strike: "7400".into() }),
        ..stored_put_spread()
    });
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 30, status: "Filled".into(), filled: 2.0, remaining: 0.0, avg_fill_price: 0.40 }).unwrap();
    app.poll_tws();
    assert_eq!(app.trades_stored.len(), 1, "no new trade");
    assert!(app.log_lines.iter().any(|l| l.contains("closes an open short option")));
}

#[test]
fn instant_orders_that_are_no_single_options_are_not_recorded() {
    let (now, avg) = (chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap().with_timezone(&chrono::Local), Decimal::ONE);
    let option =
        IbContract { symbol: "SPX".into(), sec_type: "OPT".into(), right: "P".into(), strike: 7400.0, multiplier: "100".into(), ..IbContract::default() };
    let order = |action: &str, quantity: f64| IbOrder { action: action.into(), total_quantity: quantity, ..IbOrder::default() };
    assert!(orders::instant_option_trade(&option, &order("SELL", 1.0), 1, avg, 0, now).is_some());
    assert!(orders::instant_option_trade(&IbContract { sec_type: "STK".into(), ..option.clone() }, &order("SELL", 1.0), 1, avg, 0, now).is_none());
    assert!(orders::instant_option_trade(&option, &order("HOLD", 1.0), 1, avg, 0, now).is_none());
    assert!(orders::instant_option_trade(&option, &order("SELL", 0.0), 1, avg, 0, now).is_none());
    assert!(orders::instant_option_trade(&IbContract { right: String::new(), ..option }, &order("SELL", 1.0), 1, avg, 0, now).is_none());
}

#[test]
fn a_limit_price_with_a_decimal_comma_is_accepted_and_a_rejection_is_shown() {
    let (mut app, commands) = connected_app();
    app.session.account_is_paper = Some(true);
    app.next_order_id = Some(1);
    app.instant.sec_type = "OPT".into();
    app.instant.expiry = "20261113".into();
    app.instant.strike = "7400".into();
    app.instant.right = "P".into();
    app.instant.qty = "1".into();
    app.instant.order_type = "LMT".into();

    app.instant.limit_price = "abc".into();
    app.submit_instant_order();
    assert_eq!(app.instant.order_error.as_deref(), Some("invalid limit price 'abc'"));
    assert!(drain(&commands).is_empty(), "nothing sent");

    app.instant.limit_price = "0,05".into();
    app.submit_instant_order();
    assert_eq!(app.instant.order_error, None);
    // Enter Trade only opens the summary - nothing goes to TWS before "Send now".
    assert!(app.instant.pending.is_some(), "confirmation shown");
    assert!(drain(&commands).is_empty(), "not sent before confirming");
    app.confirm_instant_order();
    assert!(app.instant.pending.is_none());
    let sent = drain(&commands);
    let Some(TwsCommand::PlaceOrder { contract, order, .. }) = sent.first() else { panic!("order not built: {:?}", app.instant.order_error) };
    assert!((order.lmt_price - 0.05).abs() < 1e-12);
    assert_eq!((contract.last_trade_date_or_contract_month.as_str(), contract.strike), ("20261113", 7400.0));
}

#[test]
fn instant_order_fetch_resolves_indices_on_their_own_exchange() {
    assert_eq!(underlying_lookup_exchange("SPX", "IND", "SMART"), "CBOE");
    assert_eq!(underlying_lookup_exchange("rut", "IND", "SMART"), "RUSSELL");
    assert_eq!(underlying_lookup_exchange("NDX", "IND", "SMART"), "NASDAQ");
    assert_eq!(underlying_lookup_exchange("DAX", "IND", "SMART"), "", "unknown index: any exchange");
    assert_eq!(underlying_lookup_exchange("AAPL", "STK", "CBOE"), "SMART");
    assert_eq!(underlying_lookup_exchange("ES", "FUT", "GLOBEX"), "GLOBEX");
}

#[test]
fn instant_order_fetch_goes_from_contract_lookup_to_expiries_and_strikes() {
    let (mut app, commands) = connected_app();
    app.request_option_params();
    let lookup = drain(&commands)
        .into_iter()
        .find_map(|c| match c {
            TwsCommand::RequestContractDetails { req_id, contract } => Some((req_id, contract)),
            _ => None,
        })
        .expect("contract lookup sent");
    assert_eq!((lookup.1.symbol.as_str(), lookup.1.sec_type.as_str(), lookup.1.exchange.as_str()), ("SPX", "IND", "CBOE"));

    let mut index = spx_option(lookup.0, 416_904, "", 0.0, "");
    index.sec_type = "IND".into();
    index.trading_class = String::new();
    app.tws_sender.send(TwsEvent::ContractDetails(index)).unwrap();
    app.tws_sender.send(TwsEvent::ContractDetailsEnd { req_id: lookup.0 }).unwrap();
    app.poll_tws();
    let params_id = drain(&commands)
        .into_iter()
        .find_map(|c| match c {
            TwsCommand::RequestOptionParams { req_id, underlying_con_id, underlying_sec_type, .. } => {
                assert_eq!((underlying_con_id, underlying_sec_type.as_str()), (416_904, "IND"));
                Some(req_id)
            }
            _ => None,
        })
        .expect("option params requested with the resolved conId");
    app.tws_sender
        .send(TwsEvent::OptionParams {
            req_id: params_id,
            exchange: "SMART".into(),
            underlying_con_id: 416_904,
            trading_class: "SPXW".into(),
            multiplier: "100".into(),
            expirations: vec!["20261116".into(), "20261113".into()],
            strikes: vec![7500.0, 7495.0],
        })
        .unwrap();
    app.tws_sender.send(TwsEvent::OptionParamsEnd { req_id: params_id }).unwrap();
    app.poll_tws();
    assert_eq!(app.instant.option_expirations, ["20261113", "20261116"]);
    assert_eq!(app.instant.option_strikes, [7495.0, 7500.0]);
}

#[test]
fn a_failed_instant_order_lookup_shows_the_tws_reason() {
    let (mut app, commands) = connected_app();
    app.request_option_params();
    let req_id = drain(&commands)
        .into_iter()
        .find_map(|c| match c {
            TwsCommand::RequestContractDetails { req_id, .. } => Some(req_id),
            _ => None,
        })
        .unwrap();
    app.tws_sender.send(TwsEvent::RequestError { req_id, code: 200, message: "No security definition has been found for the request".into() }).unwrap();
    app.poll_tws();
    assert_eq!(app.instant.option_params_status, "TWS 200: No security definition has been found for the request");
    assert_eq!(app.instant.contract_details_req_id, None);
}

#[test]
fn option_chain_runs_from_underlying_price_to_live_option_quotes() {
    let (mut app, commands) = connected_app();
    app.start_option_chains();
    app.start_option_chains();
    let sent = drain(&commands);
    let underlying: Vec<(i32, IbContract)> = sent
        .iter()
        .filter_map(|c| match c {
            TwsCommand::RequestMarketData { req_id, contract } => Some((*req_id, contract.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(underlying.len(), 1, "SPX default, requested once per session");
    let (spx_id, spx) = &underlying[0];
    assert_eq!((spx.symbol.as_str(), spx.sec_type.as_str(), spx.exchange.as_str()), ("SPX", "IND", "CBOE"));

    // First price -> one OPT/SMART lookup for today's 0 DTE expiry (or none on a weekend).
    app.tws_sender.send(TwsEvent::TickPrice { ticker_id: *spx_id, tick_type: TICK_LAST, price: 6700.0 }).unwrap();
    app.poll_tws();
    let lookups: Vec<(i32, IbContract)> = drain(&commands)
        .into_iter()
        .filter_map(|c| match c {
            TwsCommand::RequestContractDetails { req_id, contract } => Some((req_id, contract)),
            _ => None,
        })
        .collect();
    let today = test_today();
    let Some(expiry) = expiry_for_dte(today, 0) else {
        assert!(lookups.is_empty(), "no 0 DTE lookup on a weekend");
        return;
    };
    assert_eq!(lookups.len(), 1);
    let (lookup_id, lookup) = &lookups[0];
    assert_eq!(
        (lookup.symbol.as_str(), lookup.sec_type.as_str(), lookup.exchange.as_str(), lookup.last_trade_date_or_contract_month.as_str()),
        ("SPX", "OPT", "SMART", expiry.as_str())
    );

    // Contracts arrive; one inside the put window, one far outside, one AM-settled SPX.
    let tws = app.tws_sender.clone();
    tws.send(TwsEvent::ContractDetails(spx_option(*lookup_id, 11, "P", 6600.0, &expiry))).unwrap();
    tws.send(TwsEvent::ContractDetails(spx_option(*lookup_id, 12, "P", 6000.0, &expiry))).unwrap();
    let mut monthly = spx_option(*lookup_id, 13, "P", 6650.0, &expiry);
    monthly.trading_class = "SPX".into();
    tws.send(TwsEvent::ContractDetails(monthly)).unwrap();
    tws.send(TwsEvent::ContractDetailsEnd { req_id: *lookup_id }).unwrap();
    app.poll_tws();
    assert_eq!(app.chains.contracts.len(), 2);
    assert_eq!(app.chains.pending_lookups, [] as [(i32, std::string::String, std::string::String); 0]);

    app.manage_chain_subscriptions(Instant::now(), true);
    let subscribed: Vec<(i32, IbContract)> = drain(&commands)
        .into_iter()
        .filter_map(|c| match c {
            TwsCommand::RequestMarketData { req_id, contract } => Some((req_id, contract)),
            _ => None,
        })
        .collect();
    assert_eq!(subscribed.len(), 1, "only the 6600 put lies inside 6480..6720");
    let (option_id, option) = &subscribed[0];
    assert_eq!((option.con_id, option.exchange.as_str(), option.right.as_str()), (11, "SMART", "P"));
    assert_eq!(app.chains.subscribed_count(), 1);

    tws.send(TwsEvent::TickPrice { ticker_id: *option_id, tick_type: TICK_BID, price: 1.2 }).unwrap();
    tws.send(TwsEvent::TickPrice { ticker_id: *option_id, tick_type: TICK_ASK, price: 1.4 }).unwrap();
    tws.send(TwsEvent::OptionDelta { ticker_id: *option_id, tick_type: 13, delta: Some(-0.07) }).unwrap();
    app.poll_tws();
    let put = app.chains.contracts.iter().find(|c| c.con_id == 11).unwrap();
    assert_eq!((put.bid, put.ask, put.delta), (Some(1.2), Some(1.4), Some(-0.07)));

    // The underlying rallies 200 points: the 6600 put leaves the window and is cancelled.
    tws.send(TwsEvent::TickPrice { ticker_id: *spx_id, tick_type: TICK_LAST, price: 6900.0 }).unwrap();
    app.poll_tws();
    app.manage_chain_subscriptions(Instant::now(), true);
    let cancelled: Vec<i32> = drain(&commands)
        .into_iter()
        .filter_map(|c| match c {
            TwsCommand::CancelMarketData { req_id } => Some(req_id),
            _ => None,
        })
        .collect();
    assert_eq!(cancelled, [*option_id]);
    assert_eq!(app.chains.subscribed_count(), 0);

    // A disconnect forgets the whole chain.
    tws.send(TwsEvent::Disconnected("gone".into())).unwrap();
    app.poll_tws();
    assert!(!app.chains.is_started() && app.chains.contracts.is_empty());
}
