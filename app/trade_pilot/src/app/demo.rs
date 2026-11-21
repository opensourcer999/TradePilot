//! The demo for pictures of the app (`--screenshot <file> --demo`): made-up trades and prices on a paper account with a dummy number, at
//! a fixed time of an open market. Nothing of the person's own data is shown: the settings, trades, schedules and templates read at the
//! start are replaced, and the log is not written to. No order can be sent: the command channel leads nowhere.

use super::{default_settings, App, ChainSymbol, Decimal, Instant, Lang, QuoteRow, StoredLeg, StoredTrade, TradeStatus, Underlying};
use std::sync::mpsc;

/// The New York time the demo shows: late morning of a weekday, the market open.
fn demo_moment() -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2026, 11, 18).and_then(|day| day.and_hms_opt(11, 42, 0)).unwrap_or_default()
}

/// The SPX price of the demo and its one-minute closes since the open: a quiet, slightly rising morning.
fn spx_curve() -> Vec<(String, f64)> {
    (0..128_u32)
        .map(|minute| {
            let t = f64::from(minute);
            let close = 5938.2 + t * 0.09 + (t / 7.0).sin() * 3.4 + (t / 2.3).cos() * 1.1;
            (format!("{:02}:{:02}", 9 + (30 + minute) / 60, (30 + minute) % 60), (close * 100.0).round() / 100.0)
        })
        .collect()
}

fn quote(label: &str, last: f64, close: f64, curve: Vec<(String, f64)>) -> QuoteRow {
    QuoteRow {
        label: label.to_string(),
        req_id: 0,
        bid: Some(last - 0.05),
        ask: Some(last + 0.05),
        last: Some(last),
        last_move: Some(true),
        bid_size: None,
        ask_size: None,
        last_size: None,
        close: Some(close),
        open: Some(close),
        last_update: Some("11:42:00".to_string()),
        acknowledged: true,
        requested_at: Instant::now(),
        attempts: 1,
        history_req_id: None,
        history: None,
        prev_close: Some(close),
        bars_req_id: None,
        curve_req_id: None,
        curve,
    }
}

fn price(text: &str) -> Decimal {
    text.parse().unwrap_or_default()
}

/// A demo trade: `legs` are (contract id, strike) of the short and the long of one side, `right` P or C; `ended` the status and the
/// closing price of a finished one (`None`: open).
fn demo_trade(order_id: i32, kind: &str, right: &str, legs: [(i32, &str); 2], qty: u32, open: &str, ended: Option<(TradeStatus, &str)>) -> StoredTrade {
    let leg = |(con_id, strike): (i32, &str)| Some(StoredLeg { con_id, strike: strike.to_string() });
    let (short, long) = (leg(legs[0]), leg(legs[1]));
    let put = right == "P";
    let open_price = price(open);
    let stop = open_price * Decimal::TWO;
    let target = (open_price / Decimal::TWO).round_dp(2);
    let working = ended.is_none();
    StoredTrade {
        day: demo_moment().format("%Y-%m-%d").to_string(),
        opened: demo_moment().format("%d-%m-%Y 10:%M:00").to_string(),
        account: "DU0000000".into(),
        symbol: "SPX".into(),
        trade_type: kind.into(),
        strikes: format!("{}/{} {right}", legs[0].1, legs[1].1),
        order_id,
        qty,
        multiplier: 100,
        price_open: open_price,
        delta: if put { 4.2 } else { -3.6 },
        short_put: if put { short.clone() } else { None },
        long_put: if put { long.clone() } else { None },
        short_call: if put { None } else { short },
        long_call: if put { None } else { long },
        put_ratio: 1,
        call_ratio: 1,
        expiry: demo_moment().format("%Y%m%d").to_string(),
        status: ended.map_or(TradeStatus::Open, |(status, _)| status).as_str().to_string(),
        price_close: ended.map(|(_, close)| price(close)),
        stop_order_id: working.then_some(order_id + 1),
        profit_order_id: working.then_some(order_id + 2),
        stop_limit_order_id: None,
        exit_levels: None,
        adjustments: None,
        oca_group: None,
        close_order_id: None,
        stop_price: Some(stop),
        profit_price: Some(target),
        entry_unknown: false,
        strategy: "0DTE credit spreads".into(),
        timed_exit: None,
        profit_cancel: None,
        origin: None,
        custom_legs: None,
    }
}

impl App {
    /// Screenshot mode only (`--demo`): replaces everything the Dashboard shows by the demo (see the module).
    pub fn preview_demo(&mut self) {
        if self.shot.is_none() {
            return;
        }
        self.clock = Box::new(crate::clock::ShiftedClock::from_new_york(demo_moment()));
        self.log_path = None;
        self.log_lines.clear();
        let mut settings = default_settings();
        settings.lang = Lang::En;
        self.apply_settings(settings);
        self.settings_saved = self.current_settings();
        self.templates.clear();
        self.schedule.list.clear();
        self.schedule.runs.clear();
        self.today_trades.clear();
        self.positions.clear();
        self.exec_history.clear();

        let (sender, _receiver) = mpsc::channel();
        self.tws_cmd_sender = Some(sender);
        self.session.server_version = Some("demo".to_string());
        self.session.account_is_paper = Some(true);
        self.link.trading_live = false;
        self.instant.account = "DU0000000".into();

        let expiry = demo_moment().format("%Y%m%d").to_string();
        let spx = 5951.4;
        self.quotes = vec![quote("SPX", spx, 5938.2, spx_curve()), quote("VIX", 14.85, 15.32, Vec::new()), quote("VIX9D", 13.9, 14.6, Vec::new())];
        self.chains.symbols = ChainSymbol::from_settings(&[]);
        self.chains.underlyings = vec![Underlying { symbol: "SPX".into(), req_id: 1, price: Some(spx), price_at: Some(Instant::now()), chain_requested: true }];
        let option = |con_id: i32, right: &str, strike: f64, bid: f64, ask: f64| crate::option_chain::OptionContract {
            con_id,
            symbol: "SPX".into(),
            right: right.into(),
            strike,
            expiry: expiry.clone(),
            local_symbol: String::new(),
            req_id: Some(con_id),
            bid: Some(bid),
            ask: Some(ask),
            delta: Some(if right == "P" { -0.08 } else { 0.07 }),
            updated: None,
            updated_at: Some(Instant::now()),
        };
        self.chains.contracts = vec![
            option(9001, "P", 5880.0, 0.55, 0.60),
            option(9002, "P", 5870.0, 0.25, 0.30),
            option(9003, "C", 6020.0, 0.50, 0.55),
            option(9004, "C", 6030.0, 0.20, 0.25),
            option(9005, "P", 5900.0, 0.95, 1.00),
            option(9006, "P", 5890.0, 0.55, 0.60),
        ];
        self.trades_stored = vec![
            demo_trade(1031, "PutSpread", "P", [(9007, "5850"), (9008, "5840")], 1, "0.90", Some((TradeStatus::Closed, "-0.45"))),
            demo_trade(1036, "CallSpread", "C", [(9009, "5990"), (9010, "6000")], 1, "1.05", Some((TradeStatus::Stopped, "-2.10"))),
            demo_trade(1041, "PutSpread", "P", [(9001, "5880"), (9002, "5870")], 2, "1.15", None),
            demo_trade(1044, "CallSpread", "C", [(9003, "6020"), (9004, "6030")], 2, "0.95", None),
            demo_trade(1052, "PutSpread", "P", [(9005, "5900"), (9006, "5890")], 1, "0.80", None),
        ];
        self.trade_log = self.trades_stored.clone();
        self.ui_state.trade_filter = "All".into();
        self.demo_schedules();
        self.refresh_trades(self.clock.instant(), true);
    }

    /// Four schedules of the demo with today's runs: two done in the morning, two waiting for the afternoon. The runs are never saved.
    fn demo_schedules(&mut self) {
        use crate::schedule::{Days, QuantityRule, Schedule};
        use crate::schedule_run::{RunStatus, ScheduleRun};
        use chrono::TimeZone;
        self.schedule.runs_save_blocked = true;
        self.templates = vec![
            super::TradeTemplate { id: 1, name: "SPX 0DTE Put Spread 10 delta".into(), trade_type: "PutSpread".into(), ..super::TradeTemplate::default() },
            super::TradeTemplate { id: 2, name: "SPX 0DTE Call Spread 10 delta".into(), trade_type: "CallSpread".into(), ..super::TradeTemplate::default() },
        ];
        let day = demo_moment().date();
        let plan =
            [(1, 10, 0, 1, RunStatus::Complete), (2, 10, 30, 2, RunStatus::Complete), (3, 13, 0, 1, RunStatus::Waiting), (4, 14, 30, 2, RunStatus::Waiting)];
        for (id, hour, minute, template_id, status) in plan {
            self.schedule.list.push(Schedule {
                id,
                name: format!("{hour:02}:{minute:02}"),
                template_id,
                account: "DU0000000".into(),
                hour,
                minute,
                expiration_minutes: 10,
                quantity: QuantityRule::Fixed { count: 1 },
                active: true,
                days: Days::WEEKDAYS,
                ..Schedule::default()
            });
            let start = day
                .and_hms_opt(u32::from(hour), u32::from(minute), 0)
                .and_then(|time| chrono_tz::America::New_York.from_local_datetime(&time).earliest())
                .map(|time| time.with_timezone(&chrono::Utc));
            let Some(start) = start else { continue };
            let done = status == RunStatus::Complete;
            self.schedule.runs.push(ScheduleRun {
                id,
                schedule_id: id,
                day,
                window_start: start,
                window_end: start + chrono::Duration::minutes(10),
                status,
                attempts: u32::from(done),
                started: done.then_some(start),
                finished: done.then_some(start + chrono::Duration::seconds(40)),
                notes: if done { "filled".into() } else { String::new() },
                reentry: None,
                trigger: None,
            });
        }
    }
}
