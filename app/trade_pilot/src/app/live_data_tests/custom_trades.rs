//! Custom trades: preview, entry, stops, profit-target levels, REL and closing.

use super::*;

#[test]
fn a_custom_template_previews_enters_records_values_and_closes_its_trade() {
    let (mut app, commands) = app_with_a_sent_custom_order();
    let sent = drain(&commands);
    let orders: Vec<(&IbContract, &IbOrder)> =
        sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { contract, order, .. } = c { Some((contract, &**order)) } else { None }).collect();
    assert_eq!(orders.len(), 1, "{:?}", app.log_lines);
    let (contract, order) = orders[0];
    assert_eq!((contract.sec_type.as_str(), contract.combo_legs.len()), ("BAG", 2));
    assert_eq!(contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str(), l.ratio)).collect::<Vec<_>>(), [(100, "SELL", 1), (101, "BUY", 1)]);
    assert_eq!((order.action.as_str(), order.order_type.as_str(), order.total_quantity), ("BUY", "LMT", 1.0));
    assert!(order.lmt_price < 0.0, "a credit is a negative limit: {}", order.lmt_price);
    // The fill books a custom trade with its legs.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.30 }).unwrap();
    app.poll_tws();
    let trade = app.trades_stored.first().expect("the trade is recorded").clone();
    assert_eq!((trade.trade_type.as_str(), trade.price_open), ("Custom", Decimal::new(30, 2)));
    assert_eq!(trade.strikes, "-1 6690P +1 6680P");
    assert_eq!(trade.custom_legs.as_ref().map(|legs| legs.iter().map(|l| (l.con_id, l.ratio)).collect::<Vec<_>>()), Some(vec![(100, -1), (101, 1)]));
    // Valued at the chain: closing costs 1.95 (ask of the sold put) - 1.55 (bid of the bought one) = 0.40, so the profit is (0.30 - 0.40) * 100.
    let record = app.trades_today.first().expect("it is on the Dashboard");
    assert_eq!(record.current_profit, Decimal::from(-10));
    // No stop or target for a custom trade, and the log does not pretend there is one.
    assert!(app.trades_stored.first().unwrap().stop_order_id.is_none());
    // Closing it sends the legs the other way round, at market.
    drain(&commands);
    let order_id = app.place_close_order(0, None).expect("the close order is sent");
    let close = drain(&commands);
    let (contract, order) = close
        .iter()
        .find_map(|c| if let TwsCommand::PlaceOrder { contract, order, .. } = c { Some((contract.clone(), (**order).clone())) } else { None })
        .expect("a close order");
    assert_eq!(contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), [(100, "BUY"), (101, "SELL")]);
    assert_eq!((order.order_type.as_str(), order.action.as_str()), ("MKT", "BUY"));
    // Its fill ends the trade at what it cost to close.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.45 }).unwrap();
    app.poll_tws();
    let ended = app.trades_stored.first().unwrap();
    assert_eq!((ended.status.as_str(), ended.price_close), ("Closed", Some(Decimal::new(-45, 2))));
}

#[test]
fn the_stop_and_target_of_a_two_leg_custom_trade_close_every_leg_in_one_combo() {
    let (mut app, commands) = app_with_a_sent_custom_order_with(|t| {
        t.stop_type = "Full".into();
        t.stop_multiple = 2.0;
        t.stop_order_type = "Market stop".into();
        t.profit_target_type = "Percentage".into();
        t.profit_target = Some(50.0);
    });
    drain(&commands);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: -0.30 }).unwrap();
    app.poll_tws();
    let sent = drain(&commands);
    let exits: Vec<(i32, IbContract, IbOrder)> =
        sent.iter()
            .filter_map(|c| {
                if let TwsCommand::PlaceOrder { order_id, contract, order } = c {
                    Some((*order_id, contract.clone(), (**order).clone()))
                } else {
                    None
                }
            })
            .collect();
    assert_eq!(exits.len(), 2, "{:?}", app.log_lines);
    for (_, contract, order) in &exits {
        assert_eq!(contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), [(100, "BUY"), (101, "SELL")]);
        assert_eq!((order.action.as_str(), order.oca_type), ("BUY", 3));
    }
    assert_eq!((exits[0].2.order_type.as_str(), exits[0].2.aux_price), ("STP", 0.9), "0.30 * (2 + 1)");
    assert_eq!((exits[1].2.order_type.as_str(), exits[1].2.lmt_price), ("LMT", 0.15), "half of the credit");
    // The stop fills: the trade ends as stopped at what it cost to close.
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: exits[0].0, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 0.9 }).unwrap();
    app.poll_tws();
    let ended = app.trades_stored.first().unwrap();
    assert_eq!((ended.status.as_str(), ended.price_close), ("Stopped", Some(Decimal::new(-9, 1))));
}

#[test]
fn the_profit_target_levels_of_a_custom_trade_close_every_leg_one_after_the_other() {
    let (mut app, commands) = app_with_a_sent_custom_order_of("4", |t| {
        t.stop_type = "Full".into();
        t.stop_multiple = 2.0;
        t.stop_order_type = "Market stop".into();
        t.profit_target_type = "Percentage".into();
        t.profit_target = Some(50.0);
        t.profit_target_trade_percent = Some(50.0);
        t.profit_target2 = Some(75.0);
        t.profit_target2_trade_percent = Some(50.0);
    });
    drain(&commands);
    fill(&mut app, 10, 4.0, -0.3);
    let sent = drain(&commands);
    let exits: Vec<(i32, IbContract, IbOrder)> =
        sent.iter()
            .filter_map(|c| {
                if let TwsCommand::PlaceOrder { order_id, contract, order } = c {
                    Some((*order_id, contract.clone(), (**order).clone()))
                } else {
                    None
                }
            })
            .collect();
    // The stop covers all 4 contracts, the first target half of them; both close both legs.
    assert_eq!(exits.iter().map(|(id, _, o)| (*id, o.total_quantity)).collect::<Vec<_>>(), [(11, 4.0), (12, 2.0)], "{:?}", app.log_lines);
    for (_, contract, _) in &exits {
        assert_eq!(contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), [(100, "BUY"), (101, "SELL")]);
    }
    let group = exits[0].2.oca_group.clone();
    assert_eq!(app.trades_stored.first().unwrap().exit_levels.as_ref().map(|l| l.qty), Some([2, 2, 0, 0]));
    // Level 1 is hit at 0.15: level 2 follows for the other half, in the same group, with the same two legs.
    fill(&mut app, 12, 2.0, 0.15);
    let sent = drain(&commands);
    let [TwsCommand::PlaceOrder { order_id, contract, order }] = sent.as_slice() else { panic!("one new order expected, got {}", sent.len()) };
    assert_eq!((*order_id, order.total_quantity, order.oca_group.as_str(), order.oca_type), (14, 2.0, group.as_str(), 3));
    assert_eq!(contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), [(100, "BUY"), (101, "SELL")]);
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.status.as_str(), trade.remaining_qty()), ("Open", 2));
    // The profit counts what the first level made: (0.30 - 0.15) * 2 * 100 = 30, plus the open 2 contracts valued live
    // (closing costs 0.40: (0.30 - 0.40) * 2 * 100 = -20).
    app.refresh_trades(app.clock.instant(), true);
    assert_eq!(app.trades_today.first().map(|r| r.current_profit), Some(Decimal::from(10)));
    // Level 2 at 0.05: the trade ends at the average (2 * 0.15 + 2 * 0.05) / 4 = 0.10.
    fill(&mut app, 14, 2.0, 0.05);
    let ended = app.trades_stored.first().unwrap();
    assert_eq!((ended.status.as_str(), ended.price_close.map(|p| p.normalize().to_string()).as_deref()), ("Closed", Some("-0.1")));
}

#[test]
fn a_hit_adjustment_moves_the_stop_of_a_custom_trade_and_the_new_stop_closes_both_legs() {
    let (mut app, commands) = app_with_a_sent_custom_order_with(|t| {
        t.stop_type = "Full".into();
        t.stop_multiple = 2.0;
        t.stop_order_type = "Market stop".into();
        t.profit_target_type = "Percentage".into();
        t.profit_target = Some(50.0);
        t.adjustment1_type = "Profit percent".into();
        t.adjustment1 = Some(50.0);
        t.adjustment1_order_type = "Same".into();
        t.adjustment1_change_type = "Stop factor".into();
        t.adjustment1_change = Some(0.0);
    });
    drain(&commands);
    fill(&mut app, 10, 1.0, -0.3);
    drain(&commands);
    assert!(app.trades_stored.first().unwrap().adjustments.is_some(), "the adjustment watches the custom trade: {:?}", app.log_lines);
    let now = new_york_noon(&app.trades_stored.first().unwrap().expiry.clone());
    // The spread is worth 0.20 (short bid 1.85 - long ask 1.65): the 50 % level (0.15) is not reached.
    app.check_stop_adjustments(now);
    assert!(drain(&commands).is_empty());
    // 0.10: hit. The old stop #11 is cancelled; once TWS confirms, the stop at the entry price (0.30) takes its place.
    set_spread_value(&mut app, 0.10);
    app.check_stop_adjustments(now);
    assert_eq!(cancelled_orders(&drain(&commands)), [11]);
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 11, status: "Cancelled".into(), filled: 0.0, remaining: 1.0, avg_fill_price: 0.0 }).unwrap();
    app.poll_tws();
    let sent = drain(&commands);
    let [TwsCommand::PlaceOrder { contract, order, .. }] = sent.as_slice() else { panic!("one new stop expected, got {}: {:?}", sent.len(), app.log_lines) };
    assert_eq!((order.order_type.as_str(), order.aux_price, order.action.as_str()), ("STP", 0.3, "BUY"));
    assert_eq!(contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), [(100, "BUY"), (101, "SELL")]);
    assert!(!app.log_lines.iter().any(|l| l.contains("WARNING")), "{:?}", app.log_lines);
}

#[test]
fn a_rel_stop_of_a_single_leg_custom_trade_watches_that_leg_and_names_the_underlying() {
    let (mut app, commands) = app_with_a_sent_custom_order_with(|t| {
        t.custom_leg2_action = "None".into();
        t.stop_type = "Short".into();
        t.stop_multiple = 2.0;
        t.stop_order_type = "Market stop".into();
        t.stop_flags.stop_rel = true;
        t.stop_rel_limit = Some(3.0);
        t.stop_rel_offset = Some(0.25);
        t.stop_rel_itm = Some(5);
        t.stop_rel_itm_minutes = Some(20);
    });
    drain(&commands);
    fill(&mut app, 10, 1.0, 1.9);
    let orders = placed_orders(&drain(&commands));
    let (id, stop) = orders.first().expect("a stop is placed");
    assert_eq!((*id, stop.order_type.as_str(), stop.action.as_str(), stop.aux_price), (11, "REL", "BUY", 0.25), "{:?}", app.log_lines);
    // The sold 6690 put (contract 100) at or above the stop price 5.70, and the underlying in the money by 5 points.
    assert!(
        matches!(&stop.conditions[0], broker::IbCondition::Price { is_more: true, price, con_id: 100, .. } if price.total_cmp(&5.7).is_eq()),
        "{:?}",
        stop.conditions
    );
    assert!(
        matches!(&stop.conditions[1], broker::IbCondition::Price { and_next: true, is_more: false, price, con_id: 416_904, .. } if price.total_cmp(&6695.0).is_eq()),
        "{:?}",
        stop.conditions
    );
    assert!(!app.log_lines.iter().any(|l| l.contains("WARNING")), "{:?}", app.log_lines);
}

#[test]
fn a_single_leg_custom_trade_with_a_ratio_buys_that_many_options_at_the_price_of_one() {
    // Buy 2 of the put nearest 20 delta (6690, mid 1.90): a debit of 3.80 for the combo of the template.
    let (mut app, commands) = app_with_a_sent_custom_order_with(|t| {
        (t.custom_leg1_action, t.custom_leg1_ratio) = ("Buy".into(), 2);
        t.custom_leg2_action = "None".into();
    });
    let sent = drain(&commands);
    let orders: Vec<(&IbContract, &IbOrder)> =
        sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { contract, order, .. } = c { Some((contract, &**order)) } else { None }).collect();
    let [(contract, order)] = orders.as_slice() else { panic!("one order expected: {:?}", app.log_lines) };
    assert_eq!((contract.sec_type.as_str(), contract.strike, contract.trading_class.as_str()), ("OPT", 6690.0, "SPXW"));
    assert_eq!((order.action.as_str(), order.total_quantity, order.lmt_price), ("BUY", 2.0, 1.9), "two options at the price of one");
    // The trade holds two options with ratio 1 at the price of one.
    fill(&mut app, 10, 2.0, 1.9);
    let trade = app.trades_stored.first().expect("the trade is recorded");
    assert_eq!((trade.qty, trade.price_open, trade.strikes.as_str()), (2, Decimal::new(-19, 1), "+1 6690P"));
    assert_eq!(trade.custom_legs.as_ref().map(|legs| legs.iter().map(|l| l.ratio).collect::<Vec<_>>()), Some(vec![1]));
    // Valued at the bid of 1.85: (1.85 - 1.90) * 2 * 100 = -10.
    assert_eq!(app.trades_today.first().map(|r| r.current_profit), Some(Decimal::from(-10)));
}

/// "Close Trade" and the stop/target window value a custom trade by its own legs, as the Dashboard does.
#[test]
fn the_close_suggestion_of_a_custom_trade_comes_from_its_legs() {
    let (mut credit, credit_commands) = app_with_a_sent_custom_order();
    drain(&credit_commands);
    fill(&mut credit, 10, 1.0, -0.3);
    credit.request_close(0, CloseKind::Limit);
    assert_eq!(credit.close_pending.as_ref().map(|p| p.price.as_str()), Some("0.4"), "buying back: 1.95 ask - 1.55 bid");
    let trade = credit.trades_stored.first().unwrap().clone();
    assert_eq!(credit.closing_price(&trade), Some(Decimal::new(4, 1)));
    // A debit custom trade receives money when it is closed: 1.85 bid - 1.65 ask.
    let (mut debit, debit_commands) = app_with_a_sent_custom_order_with(|t| {
        t.custom_leg1_action = "Buy".into();
        (t.custom_leg2_action, t.custom_leg2_target) = ("Sell".into(), -10.0);
    });
    drain(&debit_commands);
    fill(&mut debit, 10, 1.0, 0.3);
    debit.request_close(0, CloseKind::Limit);
    assert_eq!(debit.close_pending.as_ref().map(|p| p.price.as_str()), Some("0.2"));
}

#[test]
fn a_debit_custom_combo_closes_at_a_limit_that_brings_the_typed_price() {
    // Buy the 6690 put (mid 1.90), sell the 6680 put (mid 1.60): a debit of 0.30.
    let (mut app, commands) = app_with_a_sent_custom_order_with(|t| {
        t.custom_leg1_action = "Buy".into();
        (t.custom_leg2_action, t.custom_leg2_target) = ("Sell".into(), -10.0);
    });
    drain(&commands);
    fill(&mut app, 10, 1.0, 0.3);
    assert_eq!(app.trades_stored.first().map(|t| t.price_open), Some(Decimal::new(-3, 1)), "a debit");
    // Close with a limit of 0.25: the legs the other way round, bought for a credit of 0.25.
    let order_id = app.place_close_order(0, Some(Decimal::new(25, 2))).expect("the close order is sent");
    let sent = drain(&commands);
    let (contract, order) = sent
        .iter()
        .find_map(|c| if let TwsCommand::PlaceOrder { contract, order, .. } = c { Some((contract.clone(), (**order).clone())) } else { None })
        .expect("a close order");
    assert_eq!(contract.combo_legs.iter().map(|l| (l.con_id, l.action.as_str())).collect::<Vec<_>>(), [(100, "SELL"), (101, "BUY")]);
    assert_eq!((order.action.as_str(), order.order_type.as_str(), order.lmt_price), ("BUY", "LMT", -0.25), "a credit of at least 0.25");
    // The fill brings 0.25: the trade ends at +0.25 against its debit of 0.30.
    fill(&mut app, order_id, 1.0, -0.25);
    let ended = app.trades_stored.first().unwrap();
    assert_eq!((ended.status.as_str(), ended.price_close), ("Closed", Some(Decimal::new(25, 2))));
    // A credit custom combo still pays the typed price.
    let (mut credit, credit_commands) = app_with_a_sent_custom_order();
    drain(&credit_commands);
    fill(&mut credit, 10, 1.0, -0.3);
    credit.place_close_order(0, Some(Decimal::new(25, 2))).expect("sent");
    let price = drain(&credit_commands).iter().find_map(|c| if let TwsCommand::PlaceOrder { order, .. } = c { Some(order.lmt_price) } else { None });
    assert_eq!(price, Some(0.25));
}

#[test]
fn a_custom_trade_is_stopped_and_targeted_as_a_whole_in_one_oca_group() {
    let (dte, expiry) = next_expiry();
    let (mut app, commands) = app_with_a_chain_and_a_template(&expiry, dte);
    {
        let t = app.templates.first_mut().unwrap();
        t.trade_type = "Custom".into();
        (t.custom_leg1_action, t.custom_leg1_put_call, t.custom_leg1_ratio, t.custom_leg1_target, t.custom_leg1_target_type, t.custom_leg1_dte) =
            ("Sell".into(), "P".into(), 1, 20.0, "Delta".into(), dte);
        t.custom_leg2_action = "None".into();
        t.custom_leg3_action = "None".into();
        t.custom_leg4_action = "None".into();
        t.stop_type = "Full".into();
        t.stop_multiple = 2.0;
        t.stop_order_type = "Market stop".into();
        t.profit_target_type = "Percentage".into();
        t.profit_target = Some(50.0);
    }
    app.instant.template_id = Some(4);
    app.instant.template_qty = "1".into();
    app.template_order_flags.instant_template_transmit = true;
    app.instant.account = "DU1234567".into();
    app.enter_template_trade();
    app.confirm_template_order();
    let sent = drain(&commands);
    let single: Vec<&IbContract> = sent.iter().filter_map(|c| if let TwsCommand::PlaceOrder { contract, .. } = c { Some(contract) } else { None }).collect();
    assert_eq!(single.len(), 1, "{:?}", app.log_lines);
    assert_eq!((single[0].sec_type.as_str(), single[0].strike), ("OPT", 6690.0), "one leg is a plain option order");
    app.tws_sender.send(TwsEvent::OrderStatus { order_id: 10, status: "Filled".into(), filled: 1.0, remaining: 0.0, avg_fill_price: 1.90 }).unwrap();
    app.poll_tws();
    let exits = placed_orders(&drain(&commands));
    assert_eq!(exits.len(), 2, "a stop and a profit target: {:?}", app.log_lines);
    let (stop, profit) = (&exits[0].1, &exits[1].1);
    assert_eq!((stop.order_type.as_str(), stop.action.as_str(), stop.aux_price), ("STP", "BUY", 5.7), "1.90 * (2 + 1)");
    assert_eq!((profit.order_type.as_str(), profit.action.as_str(), profit.lmt_price), ("LMT", "BUY", 0.95));
    assert_eq!((stop.oca_group.as_str(), stop.oca_type), (profit.oca_group.as_str(), 3), "one group");
    assert_ne!(stop.oca_group, "");
    let trade = app.trades_stored.first().unwrap();
    assert_eq!((trade.stop_order_id, trade.profit_order_id), (Some(exits[0].0), Some(exits[1].0)));
}
