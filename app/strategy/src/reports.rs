//! The figures of the Reports cards (Details, Daily, Puts, Calls) and the CSV export of the
//! trades of a period. Pure logic on `Decimal` (`coding_guideline.md` sections 2/3): no I/O, no
//! clock, the caller passes the trades.

use crate::records::StoredTrade;
use crate::trade_summary::{TradeRecord, TradeStatus};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// Which side of the market a credit trade sold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Put trades.
    Put,
    /// Call trades.
    Call,
    /// Iron condors, iron flies and strangles sell both: their premium cannot be split.
    Both,
}

/// The side of a trade type ("`PutSpread`", "`ShortCall`", "IronCondor-Full", ...).
#[must_use]
pub fn side_of(trade_type: &str) -> Side {
    match (trade_type.contains("Put"), trade_type.contains("Call")) {
        (true, false) => Side::Put,
        (false, true) => Side::Call,
        _ => Side::Both,
    }
}

/// One trade of the period for the figures: its opening day (yyyy-MM-dd), its side and how the
/// summary counts it.
#[derive(Clone, Debug)]
pub struct ReportRow {
    /// The opening day (yyyy-MM-dd).
    pub day: String,
    /// Which side of the market the trade is on.
    pub side: Side,
    /// How the summary counts the trade.
    pub record: TradeRecord,
}

/// The profit of a trade where it is known: finished trades always, open ones while quoted.
#[must_use]
pub fn profit(record: &TradeRecord) -> Option<Decimal> {
    (record.status != TradeStatus::Open || record.profit_known).then_some(record.current_profit)
}

/// One side's figures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SideStats {
    /// How many trades.
    pub trades: usize,
    /// The premium sold in dollars.
    pub premium_sold: Decimal,
    /// Premium per trade; `None` without trades.
    pub avg_premium: Option<Decimal>,
    /// Share of winning trades among the finished ones (an open trade is not decided yet, as in the Overview), a
    /// fraction; `None` without any.
    pub win_rate: Option<Decimal>,
}

/// The figures of the cards. Only credit trades count, like the Overview.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportStats {
    /// Premium per trade over all credit trades of the period.
    pub avg_premium: Option<Decimal>,
    /// Days of the period on which a trade was opened.
    /// How many trading days the trades fall on.
    pub days: usize,
    /// Days whose trades together ended in profit / in loss (days with an unpriced open trade are
    /// not counted either way).
    pub winning_days: usize,
    /// Days that ended in loss (see `winning_days`).
    pub losing_days: usize,
    /// The figures of the put trades.
    pub puts: SideStats,
    /// The figures of the call trades.
    pub calls: SideStats,
}

fn side_stats(rows: &[&ReportRow]) -> Option<SideStats> {
    let mut sold = Decimal::ZERO;
    let (mut known, mut wins) = (0u32, 0u32);
    for row in rows {
        sold = sold.checked_add(row.record.total_premium)?;
        if let (true, Some(p)) = (row.record.status != TradeStatus::Open, profit(&row.record)) {
            known += 1;
            if p >= Decimal::ZERO {
                wins += 1;
            }
        }
    }
    let count = Decimal::from(u32::try_from(rows.len()).ok()?);
    Some(SideStats {
        trades: rows.len(),
        premium_sold: sold,
        avg_premium: (!rows.is_empty()).then(|| sold.checked_div(count)).flatten(),
        win_rate: (known > 0).then(|| Decimal::from(wins).checked_div(Decimal::from(known))).flatten(),
    })
}

/// A ready-made report period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// Only today.
    Today,
    /// Only yesterday.
    Yesterday,
    /// Monday up to today.
    ThisWeek,
    /// The last seven days including today.
    Last7Days,
    /// The first of the month up to today.
    ThisMonth,
}

impl Preset {
    /// Every preset, in the order the screen shows them.
    pub const ALL: [Self; 5] = [Self::Today, Self::Yesterday, Self::ThisWeek, Self::Last7Days, Self::ThisMonth];

    /// First and last day of the period as seen from `today`.
    #[must_use]
    pub fn period(self, today: chrono::NaiveDate) -> (chrono::NaiveDate, chrono::NaiveDate) {
        use chrono::Datelike;
        let back = |days: i64| today - chrono::Duration::days(days);
        match self {
            Self::Today => (today, today),
            Self::Yesterday => (back(1), back(1)),
            Self::ThisWeek => (back(i64::from(today.weekday().num_days_from_monday())), today),
            Self::Last7Days => (back(6), today),
            Self::ThisMonth => (today.with_day(1).unwrap_or(today), today),
        }
    }

    /// The preset whose period is exactly `start`..`end`; `None` for a custom period.
    #[must_use]
    pub fn of(start: chrono::NaiveDate, end: chrono::NaiveDate, today: chrono::NaiveDate) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.period(today) == (start, end))
    }
}

/// One day of the period (a day on which trades were opened).
#[derive(Clone, Debug, PartialEq)]
pub struct DayTotal {
    /// yyyy-MM-dd.
    pub day: String,
    /// How many trades.
    pub trades: usize,
    /// The premium sold in dollars.
    pub premium_sold: Decimal,
    /// The day's total profit; `None` while one of its trades has no price yet.
    pub profit: Option<Decimal>,
    /// Trades with a known profit that won / lost.
    pub wins: usize,
    /// Trades with a known profit that lost.
    pub losses: usize,
}

/// The credit trades of `rows` grouped by opening day, oldest day first. `None` only on arithmetic
/// overflow.
#[must_use]
pub fn daily_totals(rows: &[ReportRow]) -> Option<Vec<DayTotal>> {
    let mut days: BTreeMap<&str, DayTotal> = BTreeMap::new();
    for row in rows.iter().filter(|r| !r.record.is_debit()) {
        let day = days.entry(row.day.as_str()).or_insert_with(|| DayTotal {
            day: row.day.clone(),
            trades: 0,
            premium_sold: Decimal::ZERO,
            profit: Some(Decimal::ZERO),
            wins: 0,
            losses: 0,
        });
        day.trades += 1;
        day.premium_sold = day.premium_sold.checked_add(row.record.total_premium)?;
        let p = profit(&row.record);
        match p {
            Some(p) if p >= Decimal::ZERO => day.wins += 1,
            Some(_) => day.losses += 1,
            None => {}
        }
        day.profit = match (day.profit, p) {
            (Some(sum), Some(p)) => Some(sum.checked_add(p)?),
            _ => None,
        };
    }
    Some(days.into_values().collect())
}

/// The running total of the profit over the days, as (day, total so far). A day whose profit is
/// not known yet is left out and adds nothing.
#[must_use]
pub fn equity_curve(days: &[DayTotal]) -> Option<Vec<(String, Decimal)>> {
    let mut total = Decimal::ZERO;
    let mut curve = Vec::new();
    for day in days {
        if let Some(p) = day.profit {
            total = total.checked_add(p)?;
            curve.push((day.day.clone(), total));
        }
    }
    Some(curve)
}

/// The fees of the trades of `rows` together; `None` when no trade has a known fee yet (a missing fee is never shown as 0).
#[must_use]
pub fn total_fees(rows: &[ReportRow]) -> Option<Decimal> {
    let known: Vec<Decimal> = rows
        .iter()
        .filter(|row| row.record.commission.is_some() || row.record.commission_close.is_some())
        .filter_map(|row| row.record.commission.unwrap_or(Decimal::ZERO).checked_add(row.record.commission_close.unwrap_or(Decimal::ZERO)))
        .collect();
    if known.is_empty() {
        return None;
    }
    known.into_iter().try_fold(Decimal::ZERO, Decimal::checked_add)
}

/// The cards' figures for `rows`. `None` only on arithmetic overflow.
#[must_use]
pub fn report_stats(rows: &[ReportRow]) -> Option<ReportStats> {
    let credit: Vec<&ReportRow> = rows.iter().filter(|r| !r.record.is_debit()).collect();
    let all = side_stats(&credit)?;
    let of_side = |side: Side| -> Option<SideStats> { side_stats(&credit.iter().copied().filter(|r| r.side == side).collect::<Vec<_>>()) };
    let days = daily_totals(rows)?;
    Some(ReportStats {
        avg_premium: all.avg_premium,
        days: days.len(),
        winning_days: days.iter().filter(|d| d.profit.is_some_and(|p| p >= Decimal::ZERO)).count(),
        losing_days: days.iter().filter(|d| d.profit.is_some_and(|p| p < Decimal::ZERO)).count(),
        puts: of_side(Side::Put)?,
        calls: of_side(Side::Call)?,
    })
}

/// A CSV field: quoted when it holds a comma, a quote or a line break.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// One line of the export, already formatted.
pub struct CsvTrade<'a> {
    /// The trade.
    pub trade: &'a StoredTrade,
    /// The profit in dollars after fees, where it is known.
    pub profit: Option<Decimal>,
    /// The fees TWS charged for the trade's fills, where they are known.
    pub fees: Option<Decimal>,
}

/// The trades as CSV text (header + one line per trade, `\n` line ends, `.` as decimal mark):
/// dates as DD-MM-YYYY, the account masked, an unknown entry price as "?".
pub fn trades_csv(rows: &[CsvTrade<'_>], mask_account: &dyn Fn(&str) -> String) -> String {
    let mut out = String::from("Date,Opened,Account,Symbol,Trade,Strategy,Strikes,Expiry,Qty,Entry,Close,Profit,Fees,Status,OrderID\n");
    let dmy = |ymd: &str| ymd.split('-').rev().collect::<Vec<_>>().join("-");
    let money = |value: &Option<Decimal>| {
        value.map(|p| p.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero).to_string()).unwrap_or_default()
    };
    for CsvTrade { trade: t, profit, fees } in rows {
        let expiry = match (t.expiry.get(6..8), t.expiry.get(4..6), t.expiry.get(0..4)) {
            (Some(d), Some(m), Some(y)) => format!("{d}-{m}-{y}"),
            _ => t.expiry.clone(),
        };
        let fields = [
            dmy(&t.day),
            t.opened.clone(),
            mask_account(&t.account),
            t.symbol.clone(),
            t.trade_type.clone(),
            t.strategy.clone(),
            t.strikes.clone(),
            expiry,
            t.qty.to_string(),
            if t.entry_unknown { "?".to_string() } else { t.price_open.normalize().to_string() },
            t.price_close.map(|p| p.normalize().to_string()).unwrap_or_default(),
            money(profit),
            money(fees),
            t.status.clone(),
            t.order_id.to_string(),
        ];
        out.push_str(&fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    /// A credit trade of 1 contract (x100) at `open` with `profit` (None = open and unpriced).
    fn row(day: &str, kind: &str, open: &str, close: Option<&str>) -> ReportRow {
        let open = d(open);
        let record = match close {
            Some(c) => TradeRecord::finished(TradeStatus::Expired, "SPX", open, d(c), 1, 100).unwrap(),
            None => TradeRecord::opened("SPX", open, 1, 100, 0.0, None).unwrap(),
        };
        ReportRow { day: day.into(), side: side_of(kind), record }
    }

    #[test]
    fn presets_give_their_period_and_are_recognised_again() {
        let day = |d: u32| chrono::NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let wednesday = day(18);
        assert_eq!(Preset::Today.period(wednesday), (wednesday, wednesday));
        assert_eq!(Preset::Yesterday.period(wednesday), (day(17), day(17)));
        assert_eq!(Preset::ThisWeek.period(wednesday), (day(16), wednesday));
        assert_eq!(Preset::Last7Days.period(wednesday), (day(12), wednesday));
        assert_eq!(Preset::ThisMonth.period(wednesday), (day(1), wednesday));
        assert_eq!(Preset::of(day(17), day(17), wednesday), Some(Preset::Yesterday));
        assert_eq!(Preset::of(day(3), day(5), wednesday), None);
    }

    #[test]
    fn the_side_comes_from_the_trade_type() {
        assert_eq!(side_of("PutSpread"), Side::Put);
        assert_eq!(side_of("ShortPut"), Side::Put);
        assert_eq!(side_of("CallDebitSpread"), Side::Call);
        assert_eq!(side_of("IronCondor-Full"), Side::Both);
        assert_eq!(side_of("ShortStrangle"), Side::Both);
    }

    #[test]
    fn the_cards_count_premium_wins_and_days_per_side() {
        let rows = [
            row("2026-11-12", "PutSpread", "1", Some("0")),       // +100, puts
            row("2026-11-12", "PutSpread", "2", Some("-5")),      // -300, puts
            row("2026-11-13", "CallSpread", "1.5", Some("-0.5")), // +100, calls
            row("2026-11-13", "IronCondor-Full", "3", Some("0")), // +300, both sides
            row("2026-11-13", "PutSpread", "1", None),            // open, unpriced
        ];
        let s = report_stats(&rows).unwrap();
        // (100 + 200 + 150 + 300 + 100) / 5 = 170 per trade.
        assert_eq!(s.avg_premium, Some(d("170")));
        assert_eq!((s.days, s.winning_days, s.losing_days), (2, 0, 1), "12-11: -200 loss; 13-11 has an unpriced trade");
        // Puts: three trades, 100 + 200 + 100 = 400 sold; of the two known, one won.
        assert_eq!((s.puts.trades, s.puts.premium_sold, s.puts.avg_premium), (3, d("400"), Some(d("400") / d("3"))));
        assert_eq!(s.puts.win_rate, Some(d("0.5")));
        assert_eq!((s.calls.trades, s.calls.premium_sold, s.calls.win_rate), (1, d("150"), Some(Decimal::ONE)));
    }

    #[test]
    fn an_open_trade_does_not_change_a_win_rate() {
        let mut open = row("2026-11-13", "PutSpread", "1", None);
        open.record.profit_known = true;
        open.record.current_profit = d("-500");
        let rows = [row("2026-11-13", "PutSpread", "1", Some("0")), open];
        let s = report_stats(&rows).unwrap();
        assert_eq!((s.puts.trades, s.puts.win_rate), (2, Some(Decimal::ONE)), "only the finished trade counts");
    }

    #[test]
    fn the_days_are_totalled_oldest_first_and_the_curve_runs_over_them() {
        let rows = [
            row("2026-11-13", "PutSpread", "1", Some("0")),       // +100
            row("2026-11-12", "PutSpread", "2", Some("-5")),      // -300
            row("2026-11-12", "CallSpread", "1.5", Some("-0.5")), // +100
            row("2026-11-14", "PutSpread", "1", None),            // open, unpriced
        ];
        let days = daily_totals(&rows).unwrap();
        let summary: Vec<(&str, usize, Option<Decimal>, usize, usize)> = days.iter().map(|d| (d.day.as_str(), d.trades, d.profit, d.wins, d.losses)).collect();
        assert_eq!(summary, [("2026-11-12", 2, Some(d("-200")), 1, 1), ("2026-11-13", 1, Some(d("100")), 1, 0), ("2026-11-14", 1, None, 0, 0)]);
        assert_eq!(days.first().map(|d| d.premium_sold), Some(d("350")));
        // The unpriced day is not on the curve: -200, then -200 + 100 = -100.
        assert_eq!(equity_curve(&days).unwrap(), [("2026-11-12".to_string(), d("-200")), ("2026-11-13".to_string(), d("-100"))]);
        assert_eq!(equity_curve(&[]).unwrap(), [] as [(std::string::String, rust_decimal::Decimal); 0]);
    }
    #[test]
    fn debit_trades_and_empty_periods_give_nothing() {
        let s = report_stats(&[row("2026-11-12", "LongPut", "-1", Some("2"))]).unwrap();
        assert_eq!(s, ReportStats::default());
        assert_eq!(report_stats(&[]).unwrap(), ReportStats::default());
    }

    #[test]
    fn csv_fields_are_quoted_when_needed_and_the_header_leads() {
        let trade = StoredTrade {
            day: "2026-11-13".into(),
            opened: "13-11-2026 21:30:05".into(),
            account: "DU1234567".into(),
            symbol: "SPX".into(),
            trade_type: "IronCondor-Full".into(),
            strikes: "7535/7525 P  7600/7610 C".into(),
            order_id: 7,
            qty: 2,
            multiplier: 100,
            price_open: rust_decimal::Decimal::from_str_exact("1.2").unwrap(),
            delta: 0.0,
            short_put: None,
            long_put: None,
            short_call: None,
            long_call: None,
            put_ratio: 1,
            call_ratio: 1,
            expiry: "20261113".into(),
            status: "Expired".into(),
            price_close: Some(rust_decimal::Decimal::from_str_exact("0").unwrap()),
            stop_order_id: None,
            profit_order_id: None,
            stop_limit_order_id: None,
            exit_levels: None,
            adjustments: None,
            oca_group: None,
            close_order_id: None,
            stop_price: None,
            profit_price: None,
            entry_unknown: false,
            strategy: String::new(),
            timed_exit: None,
            profit_cancel: None,
            origin: None,
            custom_legs: None,
        };
        let unknown = StoredTrade {
            entry_unknown: true,
            strategy: String::new(),
            timed_exit: None,
            profit_cancel: None,
            origin: None,
            custom_legs: None,
            strikes: "7400/7390 \"P\", x".into(),
            price_open: rust_decimal::Decimal::from_str_exact("0").unwrap(),
            ..trade.clone()
        };
        let csv = trades_csv(
            &[CsvTrade { trade: &trade, profit: Some(d("240")), fees: Some(d("2.6")) }, CsvTrade { trade: &unknown, profit: None, fees: None }],
            &|a| format!("{}***", a.get(0..4).unwrap_or("")),
        );
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some("Date,Opened,Account,Symbol,Trade,Strategy,Strikes,Expiry,Qty,Entry,Close,Profit,Fees,Status,OrderID"));
        assert_eq!(
            lines.next(),
            Some("13-11-2026,13-11-2026 21:30:05,DU12***,SPX,IronCondor-Full,,7535/7525 P  7600/7610 C,13-11-2026,2,1.2,0,240,2.6,Expired,7")
        );
        assert_eq!(lines.next(), Some("13-11-2026,13-11-2026 21:30:05,DU12***,SPX,IronCondor-Full,,\"7400/7390 \"\"P\"\", x\",13-11-2026,2,?,0,,,Expired,7"));
        assert_eq!(lines.next(), None);
    }
}
