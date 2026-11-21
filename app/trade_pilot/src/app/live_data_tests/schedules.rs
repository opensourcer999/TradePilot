//! Schedules and their runs, sets, import, conditions of a run and the schedule form.

use super::*;

/// A schedule that runs today gets its run, the run waits inside its window and expires when the
/// window is over; the clock of the app (New York time) decides, not the computer's.
#[test]
fn a_schedule_run_is_made_for_the_day_and_expires_when_its_window_is_over() {
    let (mut app, _commands) = connected_app();
    app.schedule.list = vec![Schedule {
        id: 1,
        template_id: 7,
        account: "DU1".into(),
        hour: 9,
        minute: 35,
        expiration_minutes: 5,
        quantity: QuantityRule::Fixed { count: 1 },
        active: true,
        days: Days::WEEKDAYS,
        ..Schedule::default()
    }];
    let monday = chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
    let at = |app: &mut App, hour: u32, minute: u32| {
        app.clock = Box::new(crate::clock::TestClock::at_new_york(monday.and_hms_opt(hour, minute, 0).unwrap()));
        app.schedule.runs_checked_at = None;
        app.update_schedule_runs();
    };
    at(&mut app, 9, 0);
    assert_eq!(app.schedule.runs.len(), 1);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Waiting);
    // Inside the window (09:35:00 to 09:40:00) nothing changes.
    at(&mut app, 9, 38);
    assert_eq!((app.schedule.runs.len(), app.schedule.runs[0].status), (1, RunStatus::Waiting));
    // After it the run expires, once, and the log says so.
    at(&mut app, 9, 41);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Expired);
    at(&mut app, 9, 42);
    assert_eq!(app.log_lines.iter().filter(|line| line.contains("Schedule run 1 expired")).count(), 1);
    assert_eq!(app.schedule.runs.len(), 1, "no second run for the day");
}

#[test]
fn a_schedule_enters_its_trade_through_the_template_path_and_its_run_is_complete_once_filled() {
    let (mut app, commands) = app_with_a_schedule(5);
    assert_eq!(app.schedule.runs.len(), 1);
    run_the_schedules(&mut app, 5);
    let sent = drain(&commands);
    assert_eq!(placed_quantities(&sent), [1.0], "one order of one contract");
    assert_eq!(placed_limits(&sent), [-0.3], "the credit of the template trade, as sent for a combo");
    assert_eq!((app.schedule.runs[0].status, app.schedule.runs[0].attempts), (RunStatus::Working, 1));
    assert_eq!(app.template.orders.len(), 1);
    assert_eq!(app.template.orders[0].run_id, Some(1));
    assert!(app.schedule.runs[0].notes.contains("order 10"), "{}", app.schedule.runs[0].notes);
    // TWS reports the order filled: the run is complete and the trade is recorded as for a template trade.
    app.tws_sender.send(filled(10)).unwrap();
    app.poll_tws();
    assert_eq!(app.schedule.runs[0].status, RunStatus::Complete);
    assert!(app.schedule.runs[0].finished.is_some());
    assert_eq!(app.trades_stored.len(), 1, "the trade is recorded");
    assert!(app.log_lines.iter().any(|l| l.contains("Schedule run 1 complete: the order is filled")));
}

#[test]
fn a_run_whose_trade_is_not_found_yet_waits_and_tries_again_five_seconds_later() {
    let (mut app, commands) = app_with_a_schedule(5);
    app.chains.contracts.clear();
    run_the_schedules(&mut app, 5);
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new(), "no order without a trade");
    assert_eq!((app.schedule.runs[0].status, app.schedule.runs[0].attempts), (RunStatus::Waiting, 1));
    assert!(app.schedule.runs[0].notes.starts_with("waiting: no trade yet"), "{}", app.schedule.runs[0].notes);
    // Two seconds later it is too early; six seconds later it tries again.
    run_the_schedules(&mut app, 7);
    assert_eq!(app.schedule.runs[0].attempts, 1);
    run_the_schedules(&mut app, 11);
    assert_eq!(app.schedule.runs[0].attempts, 2);
}

#[test]
fn a_run_that_cannot_connect_keeps_waiting_without_using_up_an_attempt() {
    let (mut app, commands) = app_with_a_schedule(5);
    app.tws_cmd_sender = None;
    run_the_schedules(&mut app, 5);
    assert!(drain(&commands).is_empty());
    assert_eq!((app.schedule.runs[0].status, app.schedule.runs[0].attempts), (RunStatus::Waiting, 0));
    assert_eq!(app.schedule.runs[0].notes, "waiting: not connected to TWS");
}

#[test]
fn a_run_is_cancelled_when_the_safety_gate_is_shut_and_nothing_is_sent() {
    let (mut app, commands) = app_with_a_schedule(5);
    app.kill_switch.trip();
    run_the_schedules(&mut app, 5);
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert!(app.schedule.runs[0].notes.contains("safety gate"), "{}", app.schedule.runs[0].notes);
    assert!(app.template.orders.is_empty());
}

#[test]
fn a_run_is_cancelled_for_a_deleted_template_a_switched_off_schedule_and_a_quantity_of_zero() {
    // The template is deleted.
    let (mut app, commands) = app_with_a_schedule(5);
    app.templates.clear();
    run_the_schedules(&mut app, 5);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert!(app.schedule.runs[0].notes.contains("does not exist"), "{}", app.schedule.runs[0].notes);
    assert!(drain(&commands).is_empty());
    // The schedule is switched off after its run was made.
    let (mut app, commands) = app_with_a_schedule(5);
    app.schedule.list[0].active = false;
    app.schedule.runner_checked_at = None;
    app.run_schedules();
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert!(app.schedule.runs[0].notes.contains("switched off"));
    assert!(drain(&commands).is_empty());
    // A budget of 10 dollars buys nothing at 0.30 (30 dollars a contract).
    let (mut app, commands) = app_with_a_schedule(5);
    app.schedule.list[0].quantity = QuantityRule::PremiumAllocation { budget: Decimal::new(10, 0), max: 0, min: 0 };
    run_the_schedules(&mut app, 5);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert!(app.schedule.runs[0].notes.contains("no contracts"), "{}", app.schedule.runs[0].notes);
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
}

#[test]
fn the_premium_allocation_and_the_net_liquidation_rule_decide_the_quantity_of_the_order() {
    // 100 dollars at 0.30 (30 dollars a contract) -> 3 contracts.
    let (mut app, commands) = app_with_a_schedule(5);
    app.schedule.list[0].quantity = QuantityRule::PremiumAllocation { budget: Decimal::new(100, 0), max: 0, min: 0 };
    run_the_schedules(&mut app, 5);
    assert_eq!(placed_quantities(&drain(&commands)), [3.0]);
    // 5 percent of 100 000 = 5 000; a 10-point spread for a 0.30 credit needs 970 -> 5 contracts.
    let (mut app, commands) = app_with_a_schedule(5);
    app.schedule.list[0].quantity = QuantityRule::PctOfNetLiq { percent: Decimal::new(5, 0), max: 0, min: 0 };
    app.account_summary.push(("DU1234567".into(), "NetLiquidation".into(), "100000".into(), "USD".into()));
    run_the_schedules(&mut app, 5);
    assert_eq!(placed_quantities(&drain(&commands)), [5.0]);
    // Without the balance there is no quantity: the run is cancelled.
    let (mut app, commands) = app_with_a_schedule(5);
    app.schedule.list[0].quantity = QuantityRule::PctOfNetLiq { percent: Decimal::new(5, 0), max: 0, min: 0 };
    run_the_schedules(&mut app, 5);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
}

#[test]
fn an_order_that_ends_without_a_fill_sends_the_run_back_to_waiting_and_it_enters_again() {
    let (mut app, commands) = app_with_a_schedule(5);
    run_the_schedules(&mut app, 5);
    assert_eq!(placed_quantities(&drain(&commands)).len(), 1);
    app.tws_sender.send(order_status(10, "Submitted", 0.0, 1.0)).unwrap();
    app.tws_sender.send(order_status(10, "Cancelled", 0.0, 1.0)).unwrap();
    app.poll_tws();
    assert_eq!(app.schedule.runs[0].status, RunStatus::Waiting);
    assert!(app.schedule.runs[0].notes.contains("ended without a fill"), "{}", app.schedule.runs[0].notes);
    // Within the window the run enters again, with a new order id.
    run_the_schedules(&mut app, 20);
    assert_eq!((app.schedule.runs[0].status, app.schedule.runs[0].attempts), (RunStatus::Working, 2));
    let second = drain(&commands);
    assert!(second.iter().any(|c| matches!(c, TwsCommand::PlaceOrder { order_id: 11, .. })), "a new order with id 11");
}

#[test]
fn only_one_run_is_entered_at_a_time_and_a_linked_schedule_waits_for_the_one_it_is_linked_to() {
    let (mut app, commands) = app_with_a_schedule(5);
    app.schedule.list.push(Schedule { linked_schedule_id: 1, ..a_schedule(2) });
    run_the_schedules(&mut app, 5);
    let statuses: Vec<RunStatus> = app.schedule.runs.iter().map(|run| run.status).collect();
    assert_eq!(statuses, [RunStatus::Working, RunStatus::Waiting], "the first is entered, the second is linked to it");
    assert_eq!(placed_quantities(&drain(&commands)).len(), 1);
    // The linked one still waits while the first is working ...
    run_the_schedules(&mut app, 20);
    assert_eq!(app.schedule.runs[1].status, RunStatus::Waiting);
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
    // ... and runs once the first is complete.
    app.tws_sender.send(filled(10)).unwrap();
    app.poll_tws();
    assert_eq!(app.schedule.runs[0].status, RunStatus::Complete);
    run_the_schedules(&mut app, 30);
    assert_eq!(app.schedule.runs[1].status, RunStatus::Working);
    assert!(app.schedule.runs[1].notes.contains("order 11"), "{}", app.schedule.runs[1].notes);
}

#[test]
fn after_the_maximum_of_attempts_the_alternate_template_is_used_and_then_the_run_is_given_up() {
    let (mut app, commands) = app_with_a_schedule(5);
    let alternate = TradeTemplate { id: 9, name: "ALT".into(), ..app.templates[0].clone() };
    app.templates.push(alternate);
    app.schedule.list[0].max_attempts = 1;
    app.schedule.list[0].alternate_template_id = 9;
    let chain = std::mem::take(&mut app.chains.contracts);
    // Attempt 1 (the template of the schedule): no trade.
    run_the_schedules(&mut app, 5);
    assert_eq!(app.schedule.runs[0].attempts, 1);
    // Attempt 2: the alternate template, with the chain back.
    app.chains.contracts = chain;
    run_the_schedules(&mut app, 11);
    assert!(app.log_lines.iter().any(|l| l.contains("using the alternate template after 1 attempts")));
    assert_eq!(app.template.orders.last().map(|o| o.template.id), Some(9));
    assert_eq!(placed_quantities(&drain(&commands)).len(), 1);
    // Without an alternate template the run is cancelled at the limit.
    let (mut app, _commands) = app_with_a_schedule(5);
    app.schedule.list[0].max_attempts = 1;
    app.chains.contracts.clear();
    run_the_schedules(&mut app, 5);
    run_the_schedules(&mut app, 11);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert!(app.schedule.runs[0].notes.contains("maximum of 1 attempts"), "{}", app.schedule.runs[0].notes);
}

#[test]
fn a_schedule_never_enters_twice_for_the_same_day() {
    let (mut app, commands) = app_with_a_schedule(5);
    run_the_schedules(&mut app, 5);
    app.tws_sender.send(filled(10)).unwrap();
    app.poll_tws();
    assert_eq!(app.schedule.runs[0].status, RunStatus::Complete);
    drain(&commands);
    // Later in the window - and after a restart that makes the runs again - nothing is entered.
    run_the_schedules(&mut app, 40);
    app.schedule.runs_checked_at = None;
    app.update_schedule_runs();
    run_the_schedules(&mut app, 50);
    assert_eq!(app.schedule.runs.len(), 1, "no second run for the day");
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
    assert_eq!(app.template.orders.len(), 1);
}

#[test]
fn a_run_can_be_paused_and_resumed_and_a_paused_one_makes_no_attempt() {
    let (mut app, commands) = app_with_a_schedule(5);
    app.pause_schedule_run(1, true);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Paused);
    assert!(app.log_lines.iter().any(|l| l.contains("Schedule run 1 paused by the user")));
    run_the_schedules(&mut app, 5);
    assert_eq!((app.schedule.runs[0].status, app.schedule.runs[0].attempts), (RunStatus::Paused, 0), "no attempt while paused");
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
    // Pausing again does nothing; resuming lets the next attempt go out.
    app.pause_schedule_run(1, true);
    app.pause_schedule_run(1, false);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Waiting);
    run_the_schedules(&mut app, 12);
    assert_eq!(placed_quantities(&drain(&commands)), [1.0]);
    // A run with an order in TWS cannot be paused or resumed.
    app.pause_schedule_run(1, true);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Working);
    app.pause_schedule_run(1, false);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Working);
}

#[test]
fn a_paused_run_can_be_cancelled_but_one_with_an_order_cannot() {
    let (mut app, _commands) = app_with_a_schedule(0);
    app.pause_schedule_run(1, true);
    app.schedule.confirm_cancel_run = Some(1);
    app.cancel_schedule_run(1);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    app.schedule.runs[0].status = RunStatus::Working;
    app.cancel_schedule_run(1);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Working);
}

#[test]
fn a_waiting_run_can_be_cancelled_and_an_expired_one_is_reported_only_when_asked_connected_and_open() {
    let (mut app, _commands) = app_with_a_schedule(0);
    // Before the window: cancel it; a run that is not waiting stays as it is.
    app.cancel_schedule_run(1);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert_eq!(app.schedule.runs[0].notes, "cancelled by the user");
    app.schedule.runs[0].status = RunStatus::Complete;
    app.cancel_schedule_run(1);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Complete);
    // The report: only if the schedule asks, TWS is connected and the market is open (09:35 is).
    app.schedule.runs[0].status = RunStatus::Expired;
    assert_eq!(app.expired_run_report(1), None, "the schedule does not ask for it");
    app.schedule.list[0].notify_on_expiry = true;
    let report = app.expired_run_report(1).unwrap();
    assert!(report.contains("Schedule run 1") && report.contains("expired without an entry"), "{report}");
    assert_eq!(app.expired_run_report(99), None, "an unknown run");
    app.clock = Box::new(crate::clock::TestClock::at_new_york(chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(18, 0, 0).unwrap()));
    assert_eq!(app.expired_run_report(1), None, "the market is closed");
}

#[test]
fn a_double_click_target_opens_the_schedule_for_editing() {
    let (mut app, _commands) = app_with_a_schedule(0);
    app.page = Page::Dashboard;
    app.edit_schedule(1);
    assert!(app.page == Page::Schedules);
    assert_eq!(app.schedule.selected, Some(1));
    assert_eq!(app.schedule.editing.as_ref().map(|f| f.id), Some(1));
    app.schedule.editing = None;
    app.edit_schedule(99);
    assert!(app.schedule.editing.is_none(), "an unknown schedule does not open");
}

#[test]
fn only_the_schedules_of_the_active_set_run_and_switching_sets_swaps_the_waiting_runs() {
    let (mut app, _commands) = app_with_a_schedule(0);
    app.schedule.list.push(Schedule { group: 2, hour: 10, ..a_schedule(2) });
    assert_eq!(app.schedule.runs.iter().map(|r| r.schedule_id).collect::<Vec<_>>(), [1], "only the default set has a run");
    assert_eq!(app.shown_schedules().iter().map(|s| s.id).collect::<Vec<_>>(), [1]);
    app.set_active_group(2);
    assert_eq!(app.schedule.group, 2);
    assert_eq!(app.schedule.runs.iter().map(|r| r.schedule_id).collect::<Vec<_>>(), [2], "the waiting run of set 0 is gone, set 2 has its own");
    assert!(app.log_lines.iter().any(|l| l.contains("Schedule set Alternate 2 is active")));
    // A set that does not exist is refused; the same set again does nothing.
    app.set_active_group(7);
    app.set_active_group(2);
    assert_eq!(app.schedule.group, 2);
    // A run that has begun stays when the set changes.
    app.schedule.runs[0].status = RunStatus::Complete;
    app.set_active_group(0);
    assert_eq!(app.schedule.runs.iter().map(|r| (r.schedule_id, r.status)).collect::<Vec<_>>(), [(2, RunStatus::Complete), (1, RunStatus::Waiting)]);
}

#[test]
fn a_schedule_is_copied_to_another_set_and_a_new_one_starts_in_the_active_set() {
    let (mut app, _commands) = app_with_a_schedule(0);
    app.schedule.selected = Some(1);
    app.copy_selected_to_group(3);
    assert_eq!(app.schedule.list.iter().map(|s| (s.id, s.group)).collect::<Vec<_>>(), [(1, 0), (2, 3)]);
    app.copy_selected_to_group(0);
    app.copy_selected_to_group(9);
    assert_eq!(app.schedule.list.len(), 2, "the same set and a set that does not exist copy nothing");
    // A schedule saved from the form lands in the set of the form.
    app.save_schedule_form(ScheduleForm { account: "DU1234567".into(), template_id: 4, group: 3, ..ScheduleForm::new_default() });
    assert_eq!(app.schedule.list.last().map(|s| s.group), Some(3));
}

#[test]
fn schedules_are_imported_into_the_active_set_switched_off() {
    let (mut app, _commands) = app_with_a_schedule(0);
    let export = schedule_export::build_export(&app.schedule.list, |id| app.template_name_of(id));
    let guard = crate::test_support::TempDir::new("app-schedule-import");
    let dir = guard.to_path_buf();
    let path = schedule_export::write_export(&export, &dir, "t").unwrap();
    app.set_active_group(1);
    app.import_schedule_file(&path);
    let imported: Vec<&Schedule> = app.schedule.list.iter().filter(|s| s.group == 1).collect();
    assert_eq!(imported.len(), 1);
    assert!(!imported[0].active);
    assert_eq!(imported[0].template_id, 4, "found again by its name");
    assert!(app.log_lines.iter().any(|l| l.contains("Imported 1 schedule(s) into set Alternate 1")));
    app.import_schedule_file(&dir.join("missing.json"));
    assert!(app.schedule.status.as_deref().is_some_and(|s| s.contains("Could not import")));
}

#[test]
fn a_schedule_with_a_true_condition_enters_its_trade_and_a_false_one_cancels_the_run() {
    let (mut app, commands) = app_with_a_schedule(5);
    app.conditions.list = vec![vix_below(20)];
    app.schedule.list[0].condition_id = 1;
    push_vix(&mut app, 18.0);
    run_the_schedules(&mut app, 5);
    assert_eq!(placed_quantities(&drain(&commands)), [1.0], "VIX 18 < 20: the trade is entered");
    assert!(app.log_lines.iter().any(|l| l.contains("Schedule run 1: Condition Calm: group 1: VIX 18 < 20 -> true")), "{:?}", app.log_lines);
    // The VIX is above the limit: the run is cancelled and nothing is sent.
    let (mut app, commands) = app_with_a_schedule(5);
    app.conditions.list = vec![vix_below(20)];
    app.schedule.list[0].condition_id = 1;
    push_vix(&mut app, 25.0);
    run_the_schedules(&mut app, 5);
    assert_eq!((app.schedule.runs[0].status, app.schedule.runs[0].notes.as_str()), (RunStatus::Cancelled, "the condition is false"));
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
    // A condition that does not exist is false too.
    let (mut app, commands) = app_with_a_schedule(5);
    app.schedule.list[0].condition_id = 7;
    run_the_schedules(&mut app, 5);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert!(app.log_lines.iter().any(|l| l.contains("condition 7 does not exist")));
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
}

#[test]
fn a_false_condition_uses_the_template_for_that_case_and_an_unknown_value_waits_without_an_attempt() {
    let (mut app, commands) = app_with_a_schedule(5);
    app.conditions.list = vec![vix_below(20)];
    app.schedule.list[0].condition_id = 1;
    app.schedule.list[0].condition_failure_template_id = 4;
    push_vix(&mut app, 25.0);
    run_the_schedules(&mut app, 5);
    assert!(app.log_lines.iter().any(|l| l.contains("the condition is false - using template 4 instead")), "{:?}", app.log_lines);
    assert_eq!(placed_quantities(&drain(&commands)), [1.0]);
    // No VIX price yet: the run waits, no attempt is counted, nothing is sent.
    let (mut app, commands) = app_with_a_schedule(5);
    app.conditions.list = vec![vix_below(20)];
    app.schedule.list[0].condition_id = 1;
    run_the_schedules(&mut app, 5);
    assert_eq!((app.schedule.runs[0].status, app.schedule.runs[0].attempts), (RunStatus::Waiting, 0));
    assert!(app.schedule.runs[0].notes.contains("condition"), "{}", app.schedule.runs[0].notes);
    assert_eq!(placed_quantities(&drain(&commands)), Vec::<f64>::new());
    // The price arrives: the next attempt enters the trade.
    push_vix(&mut app, 18.0);
    run_the_schedules(&mut app, 12);
    assert_eq!(placed_quantities(&drain(&commands)), [1.0]);
    // A failure template that does not exist ends the run.
    let (mut app, _commands) = app_with_a_schedule(5);
    app.conditions.list = vec![vix_below(20)];
    app.schedule.list[0].condition_id = 1;
    app.schedule.list[0].condition_failure_template_id = 99;
    push_vix(&mut app, 25.0);
    run_the_schedules(&mut app, 5);
    assert_eq!(app.schedule.runs[0].status, RunStatus::Cancelled);
    assert!(app.schedule.runs[0].notes.contains("template 99"), "{}", app.schedule.runs[0].notes);
}

#[test]
fn conditions_are_saved_from_the_form_and_one_that_a_schedule_uses_is_not_deleted() {
    let (mut app, _commands) = app_with_a_schedule(0);
    let mut form = ConditionForm::new_default();
    form.name = "Calm".into();
    form.rules[0].comparison_value = "20".into();
    app.save_condition_form(form.clone());
    assert_eq!(app.conditions.list.iter().map(|c| (c.id, c.name.as_str())).collect::<Vec<_>>(), [(1, "Calm")]);
    assert_eq!(app.conditions.selected, Some(1));
    // The same name again is refused and the page stays open.
    app.save_condition_form(form);
    assert_eq!(app.conditions.list.len(), 1);
    assert!(app.conditions.editing.is_some() && app.conditions.error_scroll);
    // Used by a schedule: not deleted.
    app.schedule.list[0].condition_id = 1;
    app.delete_condition(1);
    assert_eq!(app.conditions.list.len(), 1);
    assert!(app.conditions.status.as_deref().is_some_and(|s| s.contains("is used by the schedule")));
    app.schedule.list[0].condition_id = 0;
    app.delete_condition(1);
    assert_eq!(app.conditions.list, Vec::<Condition>::new());
    // A schedule form checks that its condition exists.
    app.save_schedule_form(ScheduleForm { account: "DU1234567".into(), template_id: 4, condition_id: 5, ..ScheduleForm::new_default() });
    assert_eq!(app.schedule.errors.len(), 1);
    assert!(app.schedule.errors[0].message.contains("condition"));
    // Blocked file: not written.
    app.conditions.save_blocked = true;
    app.save_conditions();
    assert!(app.conditions.status.as_deref().is_some_and(|s| s.contains("NOT saved")));
}

#[test]
fn a_valid_form_is_saved_as_a_schedule_and_a_faulty_one_keeps_the_page_open() {
    let (mut app, _commands) = app_with_a_chain_and_a_template(SCHEDULE_EXPIRY, 0);
    let form = ScheduleForm { account: "DU1234567".into(), template_id: 4, ..ScheduleForm::new_default() };
    app.save_schedule_form(form.clone());
    assert_eq!(app.schedule.list.len(), 1);
    assert_eq!((app.schedule.list[0].id, app.schedule.list[0].template_id), (1, 4));
    assert_eq!(app.schedule.selected, Some(1));
    assert!(app.schedule.editing.is_none());
    assert!(app.log_lines.iter().any(|l| l.contains("Schedule 1 saved")));
    // A second one with an unknown template is refused, outlined, and not stored.
    app.save_schedule_form(ScheduleForm { template_id: 99, ..form });
    assert_eq!(app.schedule.list.len(), 1);
    assert!(app.schedule.editing.is_some(), "the page stays open");
    assert_eq!(app.schedule.errors.len(), 1);
    assert!(app.schedule.error_scroll);
}

#[test]
fn deleting_a_schedule_frees_the_schedules_that_waited_for_it_and_a_blocked_file_is_never_written() {
    let (mut app, _commands) = app_with_a_chain_and_a_template(SCHEDULE_EXPIRY, 0);
    app.schedule.list = vec![a_schedule(1), Schedule { linked_schedule_id: 1, ..a_schedule(2) }];
    app.schedule.selected = Some(1);
    app.schedule.confirm_delete = Some(1);
    app.delete_schedule(1);
    assert_eq!(app.schedule.list.len(), 1);
    assert_eq!((app.schedule.list[0].linked_schedule_id, app.schedule.selected, app.schedule.confirm_delete), (0, None, None));
    app.schedule.selected = Some(2);
    app.set_schedule_active(false);
    assert!(!app.schedule.list[0].active);
    app.schedule.save_blocked = true;
    app.save_schedules();
    assert!(app.schedule.status.as_deref().is_some_and(|s| s.contains("NOT saved")));
}
