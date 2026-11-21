//! Order prices on the exchange's tick grid.
//! TWS rejects anything off the grid with error 110 ("The price does not
//! conform to the minimum price variation for this contract") - e.g. an SPX option mid of 4.25,
//! which trades in 0.10 steps above 3.00.
//!
//! Prices are `Decimal` (`coding_guideline.md` section 3: no floats for money); every operation is
//! checked and the rounding direction is spelled out.

use rust_decimal::Decimal;
use std::str::FromStr;

/// A price from TWS or a stored template value (`f64` on the wire / in the template JSON) as an
/// exact decimal: the shortest representation that round-trips (`0.1` stays `0.1`, not
/// `0.1000000000000000055...`). `None` for NaN/infinite values.
#[must_use]
pub fn dec(value: f64) -> Option<Decimal> {
    if !value.is_finite() {
        return None;
    }
    Decimal::from_str(&format!("{value}")).ok().or_else(|| Decimal::from_scientific(&format!("{value:e}")).ok())
}

/// Rounds `price` to multiples of `1 / per_unit`: `up` = toward +infinity (ceiling), otherwise
/// toward -infinity (floor) - `ceil`/`floor(price * n) / n`.
fn steps(price: Decimal, per_unit: i64, up: bool) -> Option<Decimal> {
    let unit = Decimal::from(per_unit);
    let scaled = price.checked_mul(unit)?;
    let whole = if up { scaled.ceil() } else { scaled.floor() };
    whole.checked_div(unit)
}

/// Rounds a limit price onto the symbol's tick grid, in
/// the grid's direction (positive prices down, negative - credits as combo prices - up).
/// `None` only on arithmetic overflow, which real option prices cannot reach.
#[must_use]
pub fn fix_price(price: Decimal, symbol: &str, legs: usize) -> Option<Decimal> {
    let zero = Decimal::ZERO;
    let three = Decimal::from(3);
    let nickel = Decimal::new(5, 2);
    // Single-leg SPX/RUT/NDX: 0.05 below 3.00, 0.10 from 3.00 up (floor; credits ceil).
    let index_single = |mut price: Decimal| {
        if price < nickel && price > zero {
            price = nickel;
        }
        if price > three {
            steps(price, 10, false)
        } else if price < -three {
            steps(price, 10, true)
        } else {
            steps(price, 20, false)
        }
    };
    match symbol {
        "RUT" | "SPX" => {
            if legs > 1 {
                steps(price, 20, price < zero)
            } else {
                index_single(price)
            }
        }
        "ES" => {
            let (per_unit, up) = if price >= Decimal::from(100) {
                (2, false)
            } else if price >= Decimal::from(20) {
                (4, false)
            } else if price >= Decimal::from(5) {
                (10, false)
            } else if price > Decimal::from(-5) {
                (20, false)
            } else if price > Decimal::from(-20) {
                (10, true)
            } else if price > Decimal::from(-100) {
                (4, true)
            } else {
                (2, true)
            };
            steps(price, per_unit, up)
        }
        "NDX" => {
            if legs > 1 {
                steps(price, 100, false)
            } else {
                index_single(price)
            }
        }
        "GLD" | "AAPL" | "AMZN" | "IBIT" | "META" | "MSFT" | "NVDA" | "TSLA" | "SPCX" | "GOOGL" => {
            if legs > 1 {
                return steps(price, 100, false);
            }
            let price = if price > three {
                steps(price, 20, false)?
            } else if price < -three {
                steps(price, 20, true)?
            } else {
                price
            };
            steps(price, 100, false)
        }
        _ => steps(price, 100, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn fixed(price: &str, symbol: &str, legs: usize) -> String {
        fix_price(d(price), symbol, legs).unwrap().normalize().to_string()
    }

    #[test]
    fn spx_single_leg_prices_land_on_the_nickel_and_dime_grid() {
        // The real rejection: mid 4.25 (bid 4.10 / ask 4.40) -> TWS 110. On the grid: 4.20.
        assert_eq!(fixed("4.25", "SPX", 1), "4.2");
        assert_eq!(fixed("4.6", "SPX", 1), "4.6");
        assert_eq!(fixed("2.98", "SPX", 1), "2.95");
        assert_eq!(fixed("3", "SPX", 1), "3");
        assert_eq!(fixed("0.02", "SPX", 1), "0.05", "never below one nickel");
        assert_eq!(fixed("0", "SPX", 1), "0");
        assert_eq!(fixed("1.23", "RUT", 1), "1.2");
        assert_eq!(fixed("-4.25", "SPX", 1), "-4.2", "credits round toward zero");
        assert_eq!(fixed("-2.98", "SPX", 1), "-3", "small negatives: floor");
        assert_eq!(fixed("17.37", "NDX", 1), "17.3");
    }

    #[test]
    fn combos_and_other_symbols_follow_the_grid_steps() {
        assert_eq!(fixed("4.27", "SPX", 4), "4.25");
        assert_eq!(fixed("-1.27", "SPX", 4), "-1.25");
        assert_eq!(fixed("4.27", "NDX", 2), "4.27");
        assert_eq!(fixed("123.4", "ES", 1), "123");
        assert_eq!(fixed("23.4", "ES", 1), "23.25");
        assert_eq!(fixed("7.37", "ES", 1), "7.3");
        assert_eq!(fixed("3.33", "ES", 1), "3.3");
        assert_eq!(fixed("-123.4", "ES", 1), "-123");
        assert_eq!(fixed("5.37", "AAPL", 1), "5.35");
        assert_eq!(fixed("1.237", "IWM", 1), "1.23");
    }

    #[test]
    fn overflow_is_reported_not_a_panic() {
        assert_eq!(fix_price(Decimal::MAX, "SPX", 1), None);
    }

    #[test]
    fn floats_from_tws_become_exact_decimals() {
        assert_eq!(dec(0.1), Some(d("0.1")));
        assert_eq!(dec(4.6), Some(d("4.6")));
        assert_eq!(dec(-0.05), Some(d("-0.05")));
        assert_eq!(dec(f64::NAN), None);
        assert_eq!(dec(f64::INFINITY), None);
    }
}
