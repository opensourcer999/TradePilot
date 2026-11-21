//! The Dashboard's "Credit Trades" figures:
//! Premium Sold, Premium Captured, PCR and Win Rate over the trades this app opened
//! today. Pure logic on `Decimal` (`coding_guideline.md` sections 2/3); the market state comes in as
//! a flag.

use rust_decimal::Decimal;

/// Where a trade stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeStatus {
    /// 1 - still open.
    Open,
    /// 2 - closed by its stop.
    Stopped,
    /// 3 - closed (profit target, exit time, manual).
    Closed,
    /// 4 - expired.
    Expired,
    /// 5 - closed otherwise (counted like 3).
    ClosedOther,
    /// Split into one trade per side (the full stop split): the sides carry on, this one is hidden and counts for nothing.
    Split,
}

impl TradeStatus {
    /// Every status.
    pub const ALL: [Self; 6] = [Self::Open, Self::Stopped, Self::Closed, Self::Expired, Self::ClosedOther, Self::Split];

    /// The name a stored trade keeps (`StoredTrade::status`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Stopped => "Stopped",
            Self::Closed => "Closed",
            Self::Expired => "Expired",
            Self::ClosedOther => "ClosedOther",
            Self::Split => "Split",
        }
    }

    /// The status of a stored name; `None` for any other text.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == name)
    }
}

/// The fields of one trade the summary reads.
#[derive(Clone, Debug, PartialEq)]
pub struct TradeRecord {
    /// Where the trade stands.
    pub status: TradeStatus,
    /// Entry price per contract: positive = credit received, negative = debit paid.
    pub price_open: Decimal,
    /// Exit price per contract (negative = paid to close a credit trade), once closed.
    pub price_close: Option<Decimal>,
    /// Number of contracts (> 0).
    pub qty: u32,
    /// Contract multiplier (100; ES 50).
    pub multiplier: u32,
    /// Premium received at entry in dollars (`price_open * qty * multiplier`).
    pub total_premium: Decimal,
    /// Fees of the entry, if known.
    pub commission: Option<Decimal>,
    /// Fees of the exit, if known.
    pub commission_close: Option<Decimal>,
    /// Profit/loss after fees, once known.
    pub profit_loss_net: Option<Decimal>,
    /// Current profit in dollars (open: at today's prices; closed: realized) - `CurrentProfit`.
    pub current_profit: Decimal,
    /// The underlying, e.g. "SPX".
    pub symbol: String,
    /// Delta of the whole trade (`TotalDelta`); a greek, not money.
    pub total_delta: f64,
    /// The current profit is based on real quotes. An open trade without quotes (yet) is not
    /// counted as 0 - the Dashboard then shows "-" instead of a wrong figure.
    pub profit_known: bool,
}

impl TradeRecord {
    /// The record with the fees TWS charged (see `fees`): they count in the summary's captured premium, and a finished
    /// trade's profit is then what is left after them. A fee that is not known (`None`) changes nothing.
    #[must_use]
    pub fn with_fees(mut self, open: Option<Decimal>, close: Option<Decimal>) -> Self {
        self.commission = open;
        self.commission_close = close;
        if self.status != TradeStatus::Open && (open.is_some() || close.is_some()) {
            let fees = open.unwrap_or(Decimal::ZERO).checked_add(close.unwrap_or(Decimal::ZERO));
            if let (Some(gross), Some(fees)) = (self.profit_loss_net, fees) {
                if let Some(net) = gross.checked_sub(fees) {
                    self.profit_loss_net = Some(net);
                    self.current_profit = net;
                }
            }
        }
        self
    }

    /// `Trade.IsDebitTrade`.
    #[must_use]
    pub fn is_debit(&self) -> bool {
        self.price_open < Decimal::ZERO
    }

    /// An open trade (`Status = 1`): no exit and no fees known. `current_profit` is its profit at
    /// today's quotes, `None` while a quote is missing. `TotalDelta = Delta * QtyOpen`.
    /// `None` only on arithmetic overflow.
    #[must_use]
    pub fn opened(symbol: &str, price_open: Decimal, qty: u32, multiplier: u32, delta: f64, current_profit: Option<Decimal>) -> Option<Self> {
        let total_premium = price_open.checked_mul(Decimal::from(qty))?.checked_mul(Decimal::from(multiplier))?;
        Some(Self {
            status: TradeStatus::Open,
            price_open,
            price_close: None,
            qty,
            multiplier,
            total_premium,
            commission: None,
            commission_close: None,
            profit_loss_net: None,
            current_profit: current_profit.unwrap_or(Decimal::ZERO),
            symbol: symbol.to_string(),
            total_delta: delta * f64::from(qty),
            profit_known: current_profit.is_some(),
        })
    }

    /// A trade that is over - stopped, closed, marked closed, or expired (`Status = 4`, settled
    /// at expiration) - at `price_close` per contract; its profit is final:
    /// `(PriceOpen + PriceClose) * Qty * Multiplier`.
    /// `None` only on arithmetic overflow.
    #[must_use]
    pub fn finished(status: TradeStatus, symbol: &str, price_open: Decimal, price_close: Decimal, qty: u32, multiplier: u32) -> Option<Self> {
        let total_premium = price_open.checked_mul(Decimal::from(qty))?.checked_mul(Decimal::from(multiplier))?;
        let profit = price_open.checked_add(price_close)?.checked_mul(Decimal::from(qty))?.checked_mul(Decimal::from(multiplier))?;
        Some(Self {
            status,
            price_open,
            price_close: Some(price_close),
            qty,
            multiplier,
            total_premium,
            commission: None,
            commission_close: None,
            profit_loss_net: Some(profit),
            current_profit: profit,
            symbol: symbol.to_string(),
            total_delta: 0.0,
            profit_known: true,
        })
    }
}

/// The four figures of the Dashboard's credit card (the summary fields).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CreditSummary {
    /// Trades counted.
    pub trade_count: usize,
    /// Dollars of premium taken in.
    pub premium_sold: Decimal,
    /// Dollars kept: premium sold + closing prices - fees.
    pub premium_captured: Decimal,
    /// Premium Capture Rate, a fraction (0.5 = 50 %); 0 without premium sold.
    pub pcr: Decimal,
    /// Share of winning trades, a fraction; 0 without trades.
    pub win_rate: Decimal,
    /// An open trade has no quote yet, so `premium_captured` and `pcr` cannot be told: the
    /// Dashboard shows "-" for them.
    pub captured_unknown: bool,
}

/// `new TradeSummary(trades, isMarketOpen, includeFees: true, isDebit: false)` - credit trades
/// only. `None` only on arithmetic overflow.
#[must_use]
pub fn credit_summary(trades: &[TradeRecord]) -> Option<CreditSummary> {
    // A split trade lives on in its two sides: counting it too would count its premium twice.
    let trades: Vec<&TradeRecord> = trades.iter().filter(|t| !t.is_debit() && t.status != TradeStatus::Split).collect();
    let mut summary = CreditSummary { trade_count: trades.len(), ..CreditSummary::default() };
    if trades.is_empty() {
        return Some(summary);
    }
    let mut premium_sold = Decimal::ZERO;
    let mut commissions = Decimal::ZERO;
    let mut closing = Decimal::ZERO;
    // Live: what is kept already is the premium of finished trades + their closing prices; an
    // open trade has captured what it is worth now (`current_profit`).
    let mut finished_sold = Decimal::ZERO;
    let mut open_profit = Decimal::ZERO;
    let mut captured_unknown = false;
    for t in &trades {
        premium_sold = premium_sold.checked_add(t.total_premium)?;
        if t.status == TradeStatus::Open {
            if t.profit_known {
                open_profit = open_profit.checked_add(t.current_profit)?;
            } else {
                captured_unknown = true;
            }
        } else {
            finished_sold = finished_sold.checked_add(t.total_premium)?;
        }
        for fee in [t.commission, t.commission_close].into_iter().flatten() {
            commissions = commissions.checked_add(fee)?;
        }
        // Closing prices of finished trades (expired ones only when they had one other than 0).
        let counts = match t.status {
            TradeStatus::Closed | TradeStatus::Stopped | TradeStatus::ClosedOther => t.price_close.is_some(),
            TradeStatus::Expired => t.price_close.is_some_and(|p| p != Decimal::ZERO),
            TradeStatus::Open | TradeStatus::Split => false,
        };
        if let (true, Some(close)) = (counts, t.price_close) {
            let contracts = Decimal::from(t.qty).checked_mul(Decimal::from(t.multiplier))?;
            closing = closing.checked_add(contracts.checked_mul(close)?)?;
        }
    }
    // The formula for finished trades (premium + closing prices - fees), plus the open
    // trades at today's prices: Premium Captured runs with the market instead of reading 100 %
    // for as long as nothing has closed.
    let captured = finished_sold.checked_add(closing)?.checked_add(open_profit)?.checked_sub(commissions)?;
    summary.premium_sold = premium_sold;
    summary.captured_unknown = captured_unknown;
    if !captured_unknown {
        summary.premium_captured = captured;
        if premium_sold > Decimal::ZERO {
            summary.pcr = captured.checked_div(premium_sold)?;
        }
    }
    // Win rate: only finished trades count (a winner if it made money or broke even). An open trade is not decided
    // yet, however it stands now, so it neither lifts nor lowers the rate; with no finished trade the rate is 0.
    let finished: Vec<&&TradeRecord> = trades.iter().filter(|t| t.status != TradeStatus::Open).collect();
    let finished_winners = finished.iter().filter(|t| t.profit_loss_net.is_some_and(|p| p >= Decimal::ZERO)).count();
    if !finished.is_empty() {
        summary.win_rate = Decimal::from(finished_winners).checked_div(Decimal::from(finished.len()))?;
    }
    Some(summary)
}

/// Profit of today's trades (`TradeSummary.PLOpen` / `PLClosed`, all trades, credit and debit).
#[derive(Clone, Debug, PartialEq)]
pub struct ProfitSummary {
    /// Open trades at current prices; `None` while an open trade has no quote yet (no figure rather than a wrong one).
    pub open: Option<Decimal>,
    /// Trades no longer open.
    pub closed: Decimal,
}

impl ProfitSummary {
    /// "Live P/L": open + closed. `None` if an open trade has no quote yet, or on overflow.
    #[must_use]
    pub fn total(&self) -> Option<Decimal> {
        self.open?.checked_add(self.closed)
    }
}

/// `PLOpen` and `PLClosed` of `new TradeSummary(trades, isMarketOpen)`. The closed part is always known; the open part is `None` while an open trade has no quote. `None` only on overflow.
#[must_use]
pub fn profit_summary(trades: &[TradeRecord]) -> Option<ProfitSummary> {
    let mut summary = ProfitSummary { open: Some(Decimal::ZERO), closed: Decimal::ZERO };
    for t in trades {
        if t.status == TradeStatus::Open {
            // No quote for an open trade yet: no open figure, rather than counting it as 0.
            summary.open = if t.profit_known { summary.open.and_then(|open| open.checked_add(t.current_profit)) } else { None };
        } else {
            summary.closed = summary.closed.checked_add(t.current_profit)?;
        }
    }
    Some(summary)
}

/// The Dashboard's delta: the trades' total delta
/// on `symbol` while one of them is open, else 0. `None` = no value (open trades whose
/// deltas sum to exactly 0 - it then keeps showing the previous value).
#[must_use]
pub fn delta_total(trades: &[TradeRecord], symbol: &str) -> Option<f64> {
    if !trades.iter().any(|t| t.status == TradeStatus::Open && t.symbol == symbol) {
        return Some(0.0);
    }
    let sum: f64 = trades.iter().filter(|t| t.symbol == symbol).map(|t| t.total_delta).sum();
    (sum != 0.0).then_some(sum)
}

/// Formats a dollar amount: "$1,234.56", negative "-$12.30".
#[must_use]
pub fn display_currency(value: Decimal) -> String {
    let rounded = value.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    let text = format!("{:.2}", rounded.abs());
    let (whole, cents) = text.split_once('.').unwrap_or((text.as_str(), "00"));
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if rounded < Decimal::ZERO { "-" } else { "" };
    format!("{sign}${grouped}.{cents}")
}

/// `Strings.FormatPercent(fraction, 1)`: 0.1234 -> "12.3%".
#[must_use]
pub fn display_percent(fraction: Decimal) -> String {
    let pct = fraction.checked_mul(Decimal::ONE_HUNDRED).unwrap_or(Decimal::ZERO);
    format!("{:.1}%", pct.round_dp_with_strategy(1, rust_decimal::RoundingStrategy::MidpointAwayFromZero))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_is_stored_by_its_name_and_read_back() {
        for status in TradeStatus::ALL {
            assert_eq!(TradeStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(TradeStatus::ALL.map(TradeStatus::as_str), ["Open", "Stopped", "Closed", "Expired", "ClosedOther", "Split"]);
        assert_eq!(TradeStatus::parse("open"), None, "the stored names are exact");
        // A stored trade with a status that is no known one is not open (and not finished either).
        let mut trade: crate::records::StoredTrade = serde_json::from_str(
            r#"{"day":"2026-11-13","opened":"","account":"DU1","symbol":"SPX","trade_type":"PutSpread","strikes":"","order_id":1,"qty":1,"multiplier":100,"price_open":"1","delta":0.0}"#,
        )
        .unwrap();
        assert!(trade.is_open(), "a stored trade without a status is open");
        trade.status = "Gone".into();
        assert_eq!((trade.is_open(), trade.state()), (false, None));
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn trade(status: TradeStatus, open: &str, close: Option<&str>, pl: Option<&str>) -> TradeRecord {
        let price_open = d(open);
        TradeRecord {
            status,
            price_open,
            price_close: close.map(d),
            qty: 1,
            multiplier: 100,
            total_premium: price_open * Decimal::ONE_HUNDRED,
            commission: Some(d("2.6")),
            commission_close: close.map(|_| d("2.6")),
            profit_loss_net: pl.map(d),
            current_profit: pl.map_or(Decimal::ZERO, d),
            symbol: "SPX".into(),
            total_delta: 0.0,
            profit_known: true,
        }
    }

    #[test]
    fn an_open_trade_without_a_quote_hides_only_the_open_part_and_the_total_not_the_closed_profit() {
        let unpriced = TradeRecord { profit_known: false, ..trade(TradeStatus::Open, "1", None, None) };
        let priced = TradeRecord { current_profit: d("35"), ..trade(TradeStatus::Open, "1", None, None) };
        let closed = trade(TradeStatus::Closed, "1.5", Some("-0.4"), Some("104.8"));
        let p = profit_summary(&[closed.clone(), unpriced.clone()]).unwrap();
        assert_eq!((p.open, p.closed, p.total()), (None, d("104.8"), None));
        // Priced and unpriced open trades together: still unknown, in either order.
        assert_eq!(profit_summary(&[priced.clone(), unpriced.clone(), closed.clone()]).unwrap().open, None);
        assert_eq!(profit_summary(&[unpriced, priced, closed]).unwrap().open, None);
    }

    #[test]
    fn open_and_closed_profit_and_the_live_total() {
        let mut open = trade(TradeStatus::Open, "1", None, None);
        open.current_profit = d("35");
        let trades = [open, trade(TradeStatus::Closed, "1.5", Some("-0.4"), Some("104.8")), trade(TradeStatus::Stopped, "2", Some("-4"), Some("-205.2"))];
        let p = profit_summary(&trades).unwrap();
        assert_eq!((p.open, p.closed, p.total()), (Some(d("35")), d("-100.4"), Some(d("-65.4"))));
        assert_eq!(profit_summary(&[]).unwrap(), ProfitSummary { open: Some(d("0")), closed: d("0") });
    }

    #[test]
    fn dashboard_delta_counts_only_while_a_trade_on_the_symbol_is_open() {
        let mut open = trade(TradeStatus::Open, "1", None, None);
        open.total_delta = 1.25;
        let mut closed = trade(TradeStatus::Closed, "1", Some("-0.2"), Some("77"));
        closed.total_delta = -0.5;
        assert_eq!(delta_total(&[closed.clone()], "SPX"), Some(0.0), "nothing open: 0");
        assert_eq!(delta_total(&[open.clone(), closed.clone()], "SPX"), Some(0.75));
        assert_eq!(delta_total(&[open.clone()], "NDX"), Some(0.0), "other symbol");
        closed.total_delta = -1.25;
        assert_eq!(delta_total(&[open, closed], "SPX"), None, "sums to 0: no value");
    }

    #[test]
    fn no_trades_today_means_zero_everywhere() {
        assert_eq!(credit_summary(&[]), Some(CreditSummary::default()));
    }

    #[test]
    fn premium_captured_is_premium_plus_closing_prices_minus_fees() {
        // Still open, worth +35 at today's prices.
        let mut open = trade(TradeStatus::Open, "1", None, None);
        open.current_profit = d("35");
        let trades = [
            // Sold for 1.50, bought back for 0.40: kept 110 - fees 5.20.
            trade(TradeStatus::Closed, "1.5", Some("-0.4"), Some("104.8")),
            // Sold for 2.00, stopped at 4.00: lost 200 - fees.
            trade(TradeStatus::Stopped, "2", Some("-4"), Some("-205.2")),
            open,
            // A debit trade is not part of the credit card.
            trade(TradeStatus::Closed, "-0.5", Some("0.8"), Some("24.8")),
        ];
        let s = credit_summary(&trades).unwrap();
        assert_eq!(s.trade_count, 3);
        assert_eq!(s.premium_sold, d("450"));
        // Finished: 350 - 40 - 400 = -90; the open trade has captured +35 so far; fees
        // (2.6 * 2 + 2.6 * 2 + 2.6) = 13: -90 + 35 - 13 = -68.
        assert!(!s.captured_unknown);
        assert_eq!(s.premium_captured, d("-68"));
        assert_eq!(s.pcr.round_dp(4), d("-0.1511"));
        assert_eq!(display_currency(s.premium_sold), "$450.00");
        assert_eq!(display_currency(s.premium_captured), "-$68.00");
    }

    #[test]
    fn premium_captured_follows_the_open_trades_live_instead_of_reading_100_percent() {
        let mut open = trade(TradeStatus::Open, "1", None, None);
        open.commission = None;
        // Sold for 100 and worth nothing yet: nothing captured, PCR 0 - not 100 %.
        let s = credit_summary(std::slice::from_ref(&open)).unwrap();
        assert_eq!((s.premium_captured, s.pcr), (d("0"), d("0")));
        // The market moves in its favour: 60 of the 100 are captured.
        open.current_profit = d("60");
        let s = credit_summary(std::slice::from_ref(&open)).unwrap();
        assert_eq!((s.premium_captured, s.pcr), (d("60"), d("0.6")));
        // And against it.
        open.current_profit = d("-120");
        let s = credit_summary(std::slice::from_ref(&open)).unwrap();
        assert_eq!((s.premium_captured, s.pcr), (d("-120"), d("-1.2")));
        // No quote yet: unknown, not a made-up figure.
        open.profit_known = false;
        let s = credit_summary(std::slice::from_ref(&open)).unwrap();
        assert!(s.captured_unknown);
        assert_eq!((s.premium_captured, s.pcr), (d("0"), d("0")));
    }

    #[test]
    fn only_finished_trades_count_for_the_win_rate_whatever_the_open_ones_do() {
        let open = |profit: &str, known: bool| TradeRecord { current_profit: d(profit), profit_known: known, ..trade(TradeStatus::Open, "1", None, None) };
        let closed_win = trade(TradeStatus::Closed, "1.5", Some("-0.4"), Some("104.8"));
        let stopped = trade(TradeStatus::Stopped, "2", Some("-4"), Some("-205.2"));
        // One winner of two finished trades, with open trades in profit, flat, in loss or not priced.
        let trades = [closed_win.clone(), stopped, open("40", true), open("0", true), open("-30", true), open("0", false)];
        assert_eq!(credit_summary(&trades).unwrap().win_rate, d("0.5"));
        // Only open trades, even in profit: nothing is decided, the rate is 0 and not 100 %.
        assert_eq!(credit_summary(&[open("40", true)]).unwrap().win_rate, d("0"));
        assert_eq!(credit_summary(&[open("0", true), open("0", false)]).unwrap().win_rate, d("0"));
        // The open ones do not change a finished trade's rate.
        assert_eq!(credit_summary(&[closed_win, open("-99", true)]).unwrap().win_rate, d("1"));
        assert_eq!(display_percent(d("0.75")), "75.0%");
        assert_eq!(display_percent(d("0.12345")), "12.3%");
    }

    #[test]
    fn fees_lower_a_finished_trades_profit_and_stay_unknown_when_not_reported() {
        let finished = trade(TradeStatus::Stopped, "2", Some("-4"), Some("-200"));
        let with = finished.clone().with_fees(Some(d("1.3")), Some(d("1.4")));
        assert_eq!((with.commission, with.commission_close), (Some(d("1.3")), Some(d("1.4"))));
        assert_eq!((with.profit_loss_net, with.current_profit), (Some(d("-202.7")), d("-202.7")));
        let none = finished.clone().with_fees(None, None);
        assert_eq!((none.profit_loss_net, none.current_profit), (Some(d("-200")), d("-200")), "no fee known: the profit is as it was");
        // An open trade keeps its live profit; its entry fee counts in the summary only.
        let open = trade(TradeStatus::Open, "1", None, None).with_fees(Some(d("1.3")), None);
        assert_eq!((open.current_profit, open.commission), (Decimal::ZERO, Some(d("1.3"))));
    }

    #[test]
    fn expired_trades_count_their_closing_price_only_when_not_zero() {
        let worthless = trade(TradeStatus::Expired, "1", Some("0"), Some("97.4"));
        let settled = trade(TradeStatus::Expired, "1", Some("-0.3"), Some("67.4"));
        let s = credit_summary(&[worthless.clone(), settled]).unwrap();
        // 200 premium - 30 settled - fees (2.6 + 2.6 + 2.6 + 2.6)
        assert_eq!(s.premium_captured, d("159.6"));
        // Status 5 counts its closing price like a regular close.
        let other = trade(TradeStatus::ClosedOther, "1", Some("-0.3"), Some("67.4"));
        assert_eq!(credit_summary(&[other]).unwrap().premium_captured, d("64.8"));
        assert_eq!(credit_summary(&[worthless]).unwrap().premium_captured, d("94.8"));
    }

    #[test]
    fn currency_groups_thousands_and_rounds_explicitly() {
        assert_eq!(display_currency(d("1234567.891")), "$1,234,567.89");
        assert_eq!(display_currency(d("0.005")), "$0.01");
        assert_eq!(display_currency(Decimal::ZERO), "$0.00");
        assert_eq!(display_currency(d("-1000")), "-$1,000.00");
    }
}
