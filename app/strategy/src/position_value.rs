//! What a position TWS reports is worth at the market and what it made: its quantity, its price now and TWS's average cost.
//! Pure logic on `Decimal` (`coding_guideline.md` section 3), no I/O.

use crate::pricing::dec;
use rust_decimal::Decimal;

/// The market value of a position and its profit or loss against its cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionValue {
    /// What the position is worth now: quantity x price x multiplier (negative for a short position, which costs that to close).
    pub market_value: Decimal,
    /// The profit (positive) or loss (negative): the market value less the cost.
    pub profit: Decimal,
    /// The profit in percent of the money paid or received for the position; `None` when that is 0.
    pub percent: Option<Decimal>,
}

/// The value of a position of `quantity` (negative = short) at `price` per unit, with TWS's `average_cost` per contract (for an
/// option already the price times the multiplier, e.g. 185.5 for 1.855) and the contract's `multiplier` (none for a stock: 1).
/// `None` without a price or a cost, or with numbers that cannot be computed.
#[must_use]
pub fn position_value(quantity: f64, average_cost: Option<f64>, multiplier: Option<f64>, price: Option<f64>) -> Option<PositionValue> {
    let quantity = dec(quantity)?;
    let average_cost = dec(average_cost?)?;
    let multiplier = multiplier.map_or(Some(Decimal::ONE), dec)?;
    let price = dec(price?)?;
    let market_value = quantity.checked_mul(price)?.checked_mul(multiplier)?;
    let cost = quantity.checked_mul(average_cost)?;
    let profit = market_value.checked_sub(cost)?;
    valued(market_value, profit, cost)
}

/// The value of a position as TWS gives it (`updatePortfolio`): its `market_value` and `unrealized` profit, with the percent of
/// what was paid or received for it (`quantity` x `average_cost`) worked out the same way as in `position_value`.
#[must_use]
pub fn tws_position_value(quantity: f64, average_cost: f64, market_value: f64, unrealized: f64) -> Option<PositionValue> {
    let cost = dec(quantity)?.checked_mul(dec(average_cost)?)?;
    valued(dec(market_value)?, dec(unrealized)?, cost)
}

/// The value with the profit in percent of the money paid or received (`cost`, negative for a short position); no percent of 0.
fn valued(market_value: Decimal, profit: Decimal, cost: Decimal) -> Option<PositionValue> {
    let percent = if cost.is_zero() { None } else { Some(profit.checked_mul(Decimal::ONE_HUNDRED)?.checked_div(cost.abs())?) };
    Some(PositionValue { market_value, profit, percent })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn a_short_option_made_what_it_was_sold_for_less_what_it_costs_to_buy_back() {
        // Sold for 1.855 (185.50 per contract), worth 0.30 now: 30 to buy back, 155.50 made, 83.83 % of the 185.50 received.
        let value = position_value(-1.0, Some(185.5), Some(100.0), Some(0.3)).unwrap();
        assert_eq!((value.market_value, value.profit), (d("-30"), d("155.5")));
        assert_eq!(value.percent.map(|p| p.round_dp(2)), Some(d("83.83")));
    }

    #[test]
    fn a_long_option_and_a_stock_are_valued_the_same_way() {
        // 2 options bought at 1.20 (120 per contract), now 0.90: worth 180, 60 lost, -25 %.
        let option = position_value(2.0, Some(120.0), Some(100.0), Some(0.9)).unwrap();
        assert_eq!((option.market_value, option.profit, option.percent), (d("180"), d("-60"), Some(d("-25"))));
        // 10 shares at 150.50, now 160: no multiplier.
        let stock = position_value(10.0, Some(150.5), None, Some(160.0)).unwrap();
        assert_eq!((stock.market_value, stock.profit), (d("1600"), d("95")));
    }

    #[test]
    fn the_figures_of_tws_are_taken_as_they_are_and_get_their_percent_the_same_way() {
        // The short SPX put of the portfolio: TWS values it at -6120 with 1436.37 not yet realized, the cost was 7556.37.
        let value = tws_position_value(-1.0, 7556.37, -6120.0, 1436.37).unwrap();
        assert_eq!((value.market_value, value.profit), (d("-6120"), d("1436.37")));
        assert_eq!(value.percent.map(|p| p.round_dp(2)), Some(d("19.01")));
        // Computed from a price, the same position gives the same figures.
        let computed = position_value(-1.0, Some(7556.37), Some(100.0), Some(61.2)).unwrap();
        assert_eq!((computed.market_value, computed.profit, computed.percent), (value.market_value, value.profit, value.percent));
    }

    #[test]
    fn without_a_price_or_a_cost_nothing_is_made_up() {
        assert_eq!(position_value(-1.0, Some(185.5), Some(100.0), None), None);
        assert_eq!(position_value(-1.0, None, Some(100.0), Some(0.3)), None);
        assert_eq!(position_value(-1.0, Some(0.0), Some(100.0), Some(0.3)).and_then(|v| v.percent), None, "no percent of nothing");
    }
}
