//! The bars the trade conditions work their values out of (see `bars`): the one-minute bars of the underlying and of the VIX
//! and the daily bars of the SPX. A rule that needs them has them requested from TWS once; the bars then stream in (the
//! newest one forming), and a request that has been silent for six minutes is made again. Until the bars are there the
//! rule's value is not known and the check says so.

use super::{App, Duration, Instant};
use crate::bars::{Bar, BarSeries};
use crate::condition::{Condition, Input, Snapshot};
use broker::{IbContract, TwsCommand};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};

/// A stream with no new bar for this long is requested again (while the market is open).
const SILENT_AFTER: Duration = Duration::from_secs(360);
/// A stream is not requested more often than this.
const ASK_EVERY: Duration = Duration::from_secs(60);
/// The one-minute bars of a session.
const SESSION_BARS: u32 = 390;

/// The bars received so far and the requests that bring them.
#[derive(Default)]
pub(super) struct IndicatorState {
    /// The bars by key (`min:SYMBOL`, `day:SPX`).
    series: BTreeMap<String, BarSeries>,
    /// Which key a request id feeds.
    requests: HashMap<i32, String>,
    /// When each key was asked for last, and for how many bars of history.
    asked: HashMap<String, (Instant, u32)>,
    /// When a bar of each key arrived last.
    received: HashMap<String, Instant>,
}

/// What `App::ema_reading` finds: the averages at the last closed bar (`now`) and at the one before (`before`), as (fast, slow).
pub(super) struct EmaReading {
    /// The date of the last closed bar.
    pub date: String,
    /// How many bars the series has.
    pub count: usize,
    pub before: (Decimal, Decimal),
    pub now: (Decimal, Decimal),
}

/// What a condition needs: the key of the series, bar size and days of history.
fn needs(condition: &Condition, symbol: &str) -> Vec<(String, &'static str, u32)> {
    let mut found: Vec<(String, &'static str, u32)> = Vec::new();
    let mut add = |key: String, size: &'static str, days: u32| match found.iter_mut().find(|(known, _, _)| *known == key) {
        Some(entry) => entry.2 = entry.2.max(days),
        None => found.push((key, size, days)),
    };
    for rule in &condition.rules {
        let sides = [
            (Some(rule.input), rule.input_length),
            (
                match rule.comparison {
                    crate::condition::Comparison::Input(input) => Some(input),
                    crate::condition::Comparison::Value(_) => None,
                },
                rule.comparison_length,
            ),
        ];
        for (input, length) in sides {
            match input {
                Some(Input::Ema5 | Input::Ema20 | Input::Ema40 | Input::Ema | Input::Ema750) => add(format!("min:{symbol}"), "1 min", 4),
                Some(Input::SmaDaily | Input::Rsi) => add("day:SPX".to_string(), "1 day", 365),
                Some(Input::VixRank | Input::VixPercentile) => add("min:VIX".to_string(), "1 min", (length / SESSION_BARS).saturating_add(3)),
                _ => {}
            }
        }
    }
    found
}

/// Whether a rule of `condition` compares a delta total of the open trades.
fn uses_delta_totals(condition: &Condition) -> bool {
    let is_total = |input: Input| matches!(input, Input::DeltaTotal | Input::DeltaTotalSymbol | Input::DeltaAccount | Input::DeltaAccountSymbol);
    condition.rules.iter().any(|rule| {
        is_total(rule.input)
            || match rule.comparison {
                crate::condition::Comparison::Input(input) => is_total(input),
                crate::condition::Comparison::Value(_) => false,
            }
    })
}

/// The contract whose bars are asked for.
fn bars_contract(symbol: &str) -> IbContract {
    let (sec_type, exchange) = match symbol {
        "SPX" | "VIX" | "XSP" | "VIX9D" => ("IND", "CBOE"),
        "RUT" => ("IND", "RUSSELL"),
        "NDX" => ("IND", "NASDAQ"),
        _ => ("STK", "SMART"),
    };
    IbContract {
        symbol: symbol.to_string(),
        sec_type: sec_type.to_string(),
        exchange: exchange.to_string(),
        currency: "USD".to_string(),
        ..IbContract::default()
    }
}

fn decimal(value: f64) -> Option<Decimal> {
    Decimal::try_from(value).ok()
}

impl App {
    /// The snapshot with the bars the rules of `condition` need for the underlying `symbol`; bars that are not there yet are
    /// asked for (the rule is then not known until they arrive).
    pub(super) fn with_bars(&mut self, mut snapshot: Snapshot, condition: &Condition, symbol: &str) -> Snapshot {
        snapshot.symbol = symbol.to_string();
        if snapshot.underlying.is_none() {
            snapshot.underlying = self.chains.price(symbol).and_then(decimal).filter(|price| *price > Decimal::ZERO);
        }
        if uses_delta_totals(condition) {
            snapshot.open_deltas = self.open_deltas();
        }
        for (key, size, days) in needs(condition, symbol) {
            self.ensure_bars(&key, size, days);
            if let Some(series) = self.indicators.series.get(&key) {
                snapshot.bars.insert(key, series.bars.clone());
            }
        }
        snapshot
    }

    /// Asks TWS for the bars under `key` unless that was done lately and they are still coming.
    fn ensure_bars(&mut self, key: &str, size: &str, days: u32) {
        let Some(sender) = self.tws_cmd_sender.clone() else { return };
        let now = self.clock.instant();
        let asked = self.indicators.asked.get(key).copied();
        let enough = asked.is_some_and(|(_, asked_days)| asked_days >= days);
        let last_sign = self.indicators.received.get(key).copied().or(asked.map(|(at, _)| at));
        let silent = self.market_is_open() && last_sign.is_some_and(|at| now.saturating_duration_since(at) >= SILENT_AFTER);
        let due = asked.is_none_or(|(at, _)| now.saturating_duration_since(at) >= ASK_EVERY);
        if enough && !silent || !due {
            return;
        }
        let symbol = key.split_once(':').map_or("", |(_, symbol)| symbol).to_string();
        let req_id = self.next_request_id();
        let duration = format!("{days} D");
        self.log(&format!("Requesting {size} bars of {symbol} for the conditions ({duration}, reqId {req_id})"));
        if sender.send(TwsCommand::RequestIndicatorBars { req_id, contract: bars_contract(&symbol), bar_size: size.to_string(), duration }).is_err() {
            self.log("The bar request could not be sent: not connected to TWS");
            return;
        }
        self.indicators.requests.insert(req_id, key.to_string());
        self.indicators.asked.insert(key.to_string(), (now, days));
    }

    /// Has the one-minute bars of `symbol` asked for, for a trigger that watches them for an EMA crossover.
    pub(super) fn watch_minute_bars(&mut self, symbol: &str) {
        self.ensure_bars(&format!("min:{symbol}"), "1 min", 4);
    }

    /// How many one-minute bars of `symbol` there are and the date of the newest (the one still forming): it changes when a bar is added.
    pub(super) fn minute_bar_mark(&self, symbol: &str) -> Option<(usize, String)> {
        let bars = &self.indicators.series.get(&format!("min:{symbol}"))?.bars;
        Some((bars.len(), bars.last()?.date.clone()))
    }

    /// The averages of the one-minute bars of `symbol` (the EMAs of `fast` and of `slow` bars) at the last closed bar and at the one before
    /// it, the date of the last closed bar and how many bars there are. `None` until the bars are there and long enough.
    pub(super) fn ema_reading(&self, symbol: &str, fast: usize, slow: usize) -> Option<EmaReading> {
        let bars = &self.indicators.series.get(&format!("min:{symbol}"))?.bars;
        let count = bars.len();
        let now = (crate::bars::ema(bars, fast)?, crate::bars::ema(bars, slow)?);
        let earlier = bars.get(..count.checked_sub(1)?)?;
        let before = (crate::bars::ema(earlier, fast)?, crate::bars::ema(earlier, slow)?);
        let date = bars.get(count.checked_sub(2)?)?.date.clone();
        Some(EmaReading { date, count, before, now })
    }

    /// Gives the bars of `key` to the app as if TWS had sent them.
    #[cfg(test)]
    pub(super) fn feed_test_bars(&mut self, key: &str, bars: Vec<Bar>) {
        let series = self.indicators.series.entry(key.to_string()).or_default();
        for bar in bars {
            series.push(bar);
        }
    }

    /// A bar of a request made by `ensure_bars`: `true` if it belonged to one.
    pub(super) fn on_indicator_bar(&mut self, req_id: i32, date: &str, high: f64, low: f64, close: f64) -> bool {
        let Some(key) = self.indicators.requests.get(&req_id).cloned() else { return false };
        if let (Some(high), Some(low), Some(close)) = (decimal(high), decimal(low), decimal(close)) {
            self.indicators.series.entry(key.clone()).or_default().push(Bar { date: date.to_string(), high, low, close });
            self.indicators.received.insert(key, self.clock.instant());
        }
        true
    }

    /// A new session with TWS: the bars and the requests of the old one are void.
    pub(super) fn forget_indicator_bars(&mut self) {
        self.indicators = IndicatorState::default();
    }
}

#[cfg(test)]
impl IndicatorState {
    /// How many series have bars.
    pub(super) fn series_len(&self) -> usize {
        self.series.len()
    }

    /// Makes every request and bar look `by` older.
    pub(super) fn age(&mut self, by: Duration) {
        for (at, _) in self.asked.values_mut() {
            *at = at.checked_sub(by).unwrap_or(*at);
        }
        for at in self.received.values_mut() {
            *at = at.checked_sub(by).unwrap_or(*at);
        }
    }
}
