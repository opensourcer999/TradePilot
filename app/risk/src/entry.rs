//! The checks of a new position before its opening order is sent (`coding_guideline.md` section 8): how many contracts of the symbol
//! are open already, whether today's loss has reached its limit, and how many new orders went out in the last minute. Orders that
//! close or protect a position (stops, targets, close orders) are never checked here: blocking them would only add risk.
//!
//! No clock and no state of the app: the caller hands in what it knows.

use rust_decimal::Decimal;

/// The limits of new positions; 0 switches a limit off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntryLimits {
    /// The most contracts (or combos) of one symbol that may be open at the same time, the new ones included.
    pub max_open_per_symbol: u32,
    /// The loss of today (in dollars, a positive amount) from which on no new position is opened today.
    pub max_daily_loss: Decimal,
    /// The most new opening orders within one minute.
    pub max_orders_per_minute: u32,
}

/// What is known when a new position is to be opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntryState {
    /// The contracts of the symbol open now.
    pub open_contracts: u32,
    /// The contracts the new order opens.
    pub new_contracts: u32,
    /// Today's profit or loss so far (closed trades and the open ones at their current value); `None` while it cannot be told.
    pub today_profit: Option<Decimal>,
    /// The opening orders sent within the last minute.
    pub orders_last_minute: u32,
}

/// Why a new position is not opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryRejection {
    /// The contracts of the symbol would exceed the limit.
    TooManyOpen {
        /// The contracts open now.
        open: u32,
        /// The contracts the order would add.
        new: u32,
        /// The limit.
        limit: u32,
    },
    /// Today's loss has reached the limit.
    DailyLossReached {
        /// Today's loss (a positive amount).
        loss: Decimal,
        /// The limit.
        limit: Decimal,
    },
    /// Too many opening orders within the last minute.
    TooManyOrders {
        /// The orders sent within the last minute.
        count: u32,
        /// The limit.
        limit: u32,
    },
}

impl std::fmt::Display for EntryRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyOpen { open, new, limit } => {
                write!(f, "{open} contract(s) of the symbol are open, {new} more would exceed the limit of {limit}")
            }
            Self::DailyLossReached { loss, limit } => write!(f, "today's loss of {loss} has reached the limit of {limit}: no new position today"),
            Self::TooManyOrders { count, limit } => write!(f, "{count} opening order(s) within the last minute, the limit is {limit}"),
        }
    }
}

impl std::error::Error for EntryRejection {}

/// Whether today's loss has reached the limit: `today_profit` is at or below minus `max_daily_loss` (off at 0, unknown profit never
/// blocks).
#[must_use]
pub fn daily_loss_reached(today_profit: Option<Decimal>, max_daily_loss: Decimal) -> bool {
    max_daily_loss > Decimal::ZERO && today_profit.is_some_and(|profit| profit <= -max_daily_loss)
}

/// Checks a new position against the limits.
///
/// # Errors
///
/// The first limit the position would break: the daily loss, then the order rate, then the contracts of the symbol.
pub fn check_entry(state: EntryState, limits: EntryLimits) -> Result<(), EntryRejection> {
    if daily_loss_reached(state.today_profit, limits.max_daily_loss) {
        let loss = state.today_profit.map_or(Decimal::ZERO, |profit| -profit);
        return Err(EntryRejection::DailyLossReached { loss, limit: limits.max_daily_loss });
    }
    if limits.max_orders_per_minute > 0 && state.orders_last_minute >= limits.max_orders_per_minute {
        return Err(EntryRejection::TooManyOrders { count: state.orders_last_minute, limit: limits.max_orders_per_minute });
    }
    let after = state.open_contracts.saturating_add(state.new_contracts);
    if limits.max_open_per_symbol > 0 && after > limits.max_open_per_symbol {
        return Err(EntryRejection::TooManyOpen { open: state.open_contracts, new: state.new_contracts, limit: limits.max_open_per_symbol });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> EntryLimits {
        EntryLimits { max_open_per_symbol: 20, max_daily_loss: Decimal::from(1000), max_orders_per_minute: 10 }
    }

    fn state(open: u32, new: u32, profit: Option<i64>, orders: u32) -> EntryState {
        EntryState { open_contracts: open, new_contracts: new, today_profit: profit.map(Decimal::from), orders_last_minute: orders }
    }

    #[test]
    fn a_position_within_every_limit_is_allowed() {
        assert_eq!(check_entry(state(18, 2, Some(-999), 9), limits()), Ok(()));
        assert_eq!(check_entry(state(0, 1, None, 0), limits()), Ok(()), "an unknown profit never blocks");
    }

    #[test]
    fn more_contracts_of_the_symbol_than_the_limit_are_refused() {
        assert_eq!(check_entry(state(19, 2, Some(0), 0), limits()), Err(EntryRejection::TooManyOpen { open: 19, new: 2, limit: 20 }));
    }

    #[test]
    fn a_loss_at_the_limit_stops_new_positions() {
        assert_eq!(
            check_entry(state(0, 1, Some(-1000), 0), limits()),
            Err(EntryRejection::DailyLossReached { loss: Decimal::from(1000), limit: Decimal::from(1000) })
        );
        assert!(!daily_loss_reached(Some(Decimal::from(-999)), Decimal::from(1000)));
        assert!(!daily_loss_reached(Some(Decimal::from(-5000)), Decimal::ZERO), "0 switches it off");
    }

    #[test]
    fn the_order_rate_stops_at_its_limit() {
        assert_eq!(check_entry(state(0, 1, Some(0), 10), limits()), Err(EntryRejection::TooManyOrders { count: 10, limit: 10 }));
    }

    #[test]
    fn every_limit_at_zero_is_off() {
        assert_eq!(check_entry(state(500, 500, Some(-100_000), 500), EntryLimits::default()), Ok(()));
    }
}
