//! Price bars and the values worked out of them for the trade conditions: the exponential and simple moving
//! averages, the RSI, and the rank and percentile of the VIX. Plain logic on values, no connection.
//!
//! The one-minute bars of a symbol and the daily bars of the SPX are streamed by TWS (the last bar is the one still
//! forming). The averages of the one-minute bars leave the forming bar out, as the reference does; the daily
//! values take every bar.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

/// How many bars a series keeps (the oldest go first): three days of one-minute bars and more than a year of daily ones.
const KEEP_BARS: usize = 6000;

/// One bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bar {
    /// The date as TWS sends it ("20261117 09:31:00" or, for a daily bar, "20261117"): it sorts like time.
    pub date: String,
    /// The highest price of the bar.
    pub high: Decimal,
    /// The lowest price of the bar.
    pub low: Decimal,
    /// The last price of the bar.
    pub close: Decimal,
}

/// The bars of one symbol and size, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BarSeries {
    /// The bars, the newest last.
    pub bars: Vec<Bar>,
}

impl BarSeries {
    /// A bar from TWS: the bar still forming replaces the newest one (same date), a later date is added. A bar older
    /// than the newest is ignored.
    pub fn push(&mut self, bar: Bar) {
        match self.bars.last_mut() {
            Some(last) if last.date == bar.date => *last = bar,
            Some(last) if last.date > bar.date => {}
            _ => {
                self.bars.push(bar);
                if self.bars.len() > KEEP_BARS {
                    let surplus = self.bars.len() - KEEP_BARS;
                    self.bars.drain(..surplus);
                }
            }
        }
    }
}

fn closes(bars: &[Bar]) -> Vec<f64> {
    bars.iter().filter_map(|bar| bar.close.to_f64()).collect()
}

fn to_decimal(value: f64) -> Option<Decimal> {
    Decimal::try_from(value).ok().map(|value| value.round_dp(6))
}

/// The exponential moving average of `length` periods over the closes of `bars` without the newest (it is still
/// forming): the first value is the plain average of the first `length` closes, then each close moves it by
/// `2 / (length + 1)`. `None` with fewer than `length` closes.
#[must_use]
pub fn ema(bars: &[Bar], length: usize) -> Option<Decimal> {
    let closed = closes(bars.get(..bars.len().checked_sub(1)?)?);
    if length == 0 || closed.len() < length {
        return None;
    }
    let (seed, rest) = closed.split_at(length);
    let mut value = seed.iter().sum::<f64>() / f64::from(u32::try_from(length).ok()?);
    let k = 2.0 / (f64::from(u32::try_from(length).ok()?) + 1.0);
    for close in rest {
        value += k * (close - value);
    }
    to_decimal(value)
}

/// The simple moving average of the last `length` closes of `bars`. `None` with fewer closes.
#[must_use]
pub fn sma(bars: &[Bar], length: usize) -> Option<Decimal> {
    let all = closes(bars);
    let window = all.get(all.len().checked_sub(length)?..)?;
    if length == 0 {
        return None;
    }
    to_decimal(window.iter().sum::<f64>() / f64::from(u32::try_from(length).ok()?))
}

/// The RSI of the closes of `bars` over `length` periods (Wilder's smoothing: the first average is the plain one of the
/// first `length` changes). 100 when nothing fell. `None` with fewer than `length + 1` closes.
#[must_use]
pub fn rsi(bars: &[Bar], length: usize) -> Option<Decimal> {
    let all = closes(bars);
    if length == 0 || all.len() <= length {
        return None;
    }
    let n = f64::from(u32::try_from(length).ok()?);
    let changes: Vec<f64> = all.windows(2).map(|pair| pair.get(1).copied().unwrap_or(0.0) - pair.first().copied().unwrap_or(0.0)).collect();
    let (first, rest) = changes.split_at(length);
    let mut gain = first.iter().map(|change| change.max(0.0)).sum::<f64>() / n;
    let mut loss = first.iter().map(|change| (-change).max(0.0)).sum::<f64>() / n;
    for change in rest {
        gain = (gain * (n - 1.0) + change.max(0.0)) / n;
        loss = (loss * (n - 1.0) + (-change).max(0.0)) / n;
    }
    if loss > 0.0 {
        to_decimal(100.0 - 100.0 / (1.0 + gain / loss))
    } else {
        Some(Decimal::ONE_HUNDRED)
    }
}

/// The `length` closed bars that end with the one before the newest (which is still forming), and that last closed bar, the
/// "current value" of rank and percentile. `None` with fewer than `length + 1` bars.
fn window(bars: &[Bar], length: usize) -> Option<(&[Bar], &Bar)> {
    if length == 0 || bars.len() < length.checked_add(1)? {
        return None;
    }
    let last_closed = bars.len().checked_sub(2)?;
    let start = (last_closed + 1).checked_sub(length)?;
    Some((bars.get(start..=last_closed)?, bars.get(last_closed)?))
}

/// Where the last closed bar stands between the lowest low and the highest high of the `length` closed bars ending
/// with it, 0 to 100 and rounded to a tenth; 50 when high and low are the same.
#[must_use]
pub fn rank(bars: &[Bar], length: usize) -> Option<Decimal> {
    let (span, current) = window(bars, length)?;
    let high = span.iter().map(|bar| bar.high).max()?;
    let low = span.iter().map(|bar| bar.low).min()?;
    let range = high.checked_sub(low)?;
    if range.is_zero() {
        return Some(Decimal::from(50));
    }
    Some(current.close.checked_sub(low)?.checked_div(range)?.checked_mul(Decimal::ONE_HUNDRED)?.round_dp(1))
}

/// The share of the closes of the `length` closed bars ending with the last one that lie below its close (those equal to
/// it count half), in percent.
#[must_use]
pub fn percentile(bars: &[Bar], length: usize) -> Option<Decimal> {
    let (span, current) = window(bars, length)?;
    let below = span.iter().filter(|bar| bar.close < current.close).count();
    let equal = span.iter().filter(|bar| bar.close == current.close).count();
    let score = Decimal::from(below).checked_add(Decimal::from(equal).checked_mul(Decimal::new(5, 1))?)?;
    score.checked_div(Decimal::from(length))?.checked_mul(Decimal::ONE_HUNDRED)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(date: &str, high: &str, low: &str, close: &str) -> Bar {
        Bar { date: date.into(), high: high.parse().unwrap(), low: low.parse().unwrap(), close: close.parse().unwrap() }
    }

    fn closes_only(values: &[&str]) -> Vec<Bar> {
        values.iter().enumerate().map(|(i, v)| bar(&format!("20261117 09:{:02}:00", 30 + i), v, v, v)).collect()
    }

    #[test]
    fn a_bar_that_is_still_forming_replaces_the_newest_and_old_ones_are_ignored() {
        let mut series = BarSeries::default();
        series.push(bar("20261117 09:30:00", "11", "9", "10"));
        series.push(bar("20261117 09:31:00", "12", "10", "11"));
        series.push(bar("20261117 09:31:00", "13", "10", "12"));
        series.push(bar("20261117 09:29:00", "1", "1", "1"));
        assert_eq!(series.bars.len(), 2);
        assert_eq!(series.bars.last().map(|b| b.close), Some(Decimal::from(12)));
    }

    #[test]
    fn a_series_keeps_only_its_newest_bars() {
        let mut series = BarSeries::default();
        for i in 0..(KEEP_BARS + 5) {
            series.push(bar(&format!("{i:08}"), "1", "1", "1"));
        }
        assert_eq!(series.bars.len(), KEEP_BARS);
        assert_eq!(series.bars.first().map(|b| b.date.as_str()), Some("00000005"));
    }

    #[test]
    fn the_ema_starts_with_the_plain_average_leaves_out_the_forming_bar_and_needs_enough_bars() {
        // Closes 1..=6 and a forming bar of 100 that must not count. Length 3: seed (1+2+3)/3 = 2, k = 0.5:
        // 4 -> 3, 5 -> 4, 6 -> 5.
        let bars = closes_only(&["1", "2", "3", "4", "5", "6", "100"]);
        assert_eq!(ema(&bars, 3), Some(Decimal::from(5)));
        assert_eq!(ema(&bars, 6), Some(Decimal::new(35, 1)), "six closed closes: just the seed");
        assert_eq!(ema(&bars, 7), None, "only six closed bars");
        assert_eq!(ema(&bars, 0), None);
        assert_eq!(ema(&[], 3), None);
    }

    #[test]
    fn the_sma_is_the_average_of_the_last_closes_and_needs_enough_bars() {
        let bars = closes_only(&["1", "2", "3", "4", "5"]);
        assert_eq!(sma(&bars, 3), Some(Decimal::from(4)));
        assert_eq!(sma(&bars, 5), Some(Decimal::from(3)));
        assert_eq!(sma(&bars, 6), None);
        assert_eq!(sma(&bars, 0), None);
    }

    #[test]
    fn the_rsi_is_100_without_a_fall_and_follows_wilders_smoothing() {
        let rising = closes_only(&["1", "2", "3", "4", "5"]);
        assert_eq!(rsi(&rising, 4), Some(Decimal::ONE_HUNDRED));
        assert_eq!(rsi(&rising, 5), None, "needs length + 1 closes");
        // Changes +2 -1 +2 -1 over four periods: gain 1, loss 0.5 -> RS 2 -> RSI 66.666667.
        let mixed = closes_only(&["10", "12", "11", "13", "12"]);
        assert_eq!(rsi(&mixed, 4).map(|v| v.round_dp(2)), Some(Decimal::new(6667, 2)));
        // One more period of -1: gain (1*3 + 0)/4 = 0.75, loss (0.5*3 + 1)/4 = 0.625 -> RSI 54.54.
        let longer = closes_only(&["10", "12", "11", "13", "12", "11"]);
        assert_eq!(rsi(&longer, 4).map(|v| v.round_dp(2)), Some(Decimal::new(5455, 2)));
    }

    #[test]
    fn the_rank_places_the_last_closed_bar_between_the_lowest_low_and_the_highest_high() {
        // Three closed bars and a forming one. Length 3: lows 10/8/9, highs 12/14/13, the last closed close is 11.
        let bars = vec![bar("1", "12", "10", "11"), bar("2", "14", "8", "9"), bar("3", "13", "9", "11"), bar("4", "99", "1", "50")];
        assert_eq!(rank(&bars, 3), Some(Decimal::from(50)), "(11 - 8) / (14 - 8) * 100");
        assert_eq!(rank(&bars, 4), None, "needs length + 1 bars");
        let flat = vec![bar("1", "5", "5", "5"), bar("2", "5", "5", "5"), bar("3", "5", "5", "5")];
        assert_eq!(rank(&flat, 2), Some(Decimal::from(50)), "no range");
    }

    #[test]
    fn the_percentile_counts_closes_below_and_half_of_those_equal() {
        // Closed closes 10, 12, 11, 11; the last closed is 11: below = 1 (10), equal = 2 (itself and the other 11).
        let bars = vec![bar("1", "1", "1", "10"), bar("2", "1", "1", "12"), bar("3", "1", "1", "11"), bar("4", "1", "1", "11"), bar("5", "1", "1", "99")];
        assert_eq!(percentile(&bars, 4), Some(Decimal::from(50)), "(1 + 2 * 0.5) / 4 * 100");
        assert_eq!(percentile(&bars, 5), None);
    }
}
