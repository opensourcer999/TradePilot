//! Pre-trade risk checks and a kill switch (`coding_guideline.md` section 8, MUSS): every order
//! runs through a risk check before being sent, and there is a kill switch that stops new orders
//! independent of any other logic.
//!
//! This is a first, minimal real check (order quantity against a configured limit) rather than a
//! stub - the full set from the guideline (max position, max loss, price bands, order rate) is a
//! separate, larger follow-up stage once this app tracks the position/P&L state those checks
//! would need. No network, file or time access here (section 2) - the caller supplies everything.
#![forbid(unsafe_code)]
// coding_guideline.md section 15: no floating-point arithmetic in a crate with money logic.
#![deny(clippy::float_arithmetic)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing))]

use core_types::Quantity;

/// Configured limits a proposed order is checked against before it may be sent.
#[derive(Debug, Clone, Copy)]
pub struct OrderRiskLimits {
    /// The largest quantity one order may have (contracts or combos).
    pub max_quantity: Quantity,
}

/// Why a risk check rejected an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskRejection {
    /// The order is for more than `OrderRiskLimits::max_quantity`.
    QuantityExceedsLimit {
        /// The quantity of the order.
        requested: Quantity,
        /// The configured limit.
        limit: Quantity,
    },
    /// The kill switch is on: no order may be sent.
    KillSwitchTripped,
}

impl std::fmt::Display for RiskRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskRejection::QuantityExceedsLimit { requested, limit } => {
                write!(f, "order quantity {requested} exceeds the configured limit of {limit}")
            }
            RiskRejection::KillSwitchTripped => write!(f, "the kill switch is active; no new orders are allowed"),
        }
    }
}

impl std::error::Error for RiskRejection {}

/// # Errors
/// Returns [`RiskRejection::QuantityExceedsLimit`] if `quantity` is greater than
/// `limits.max_quantity`.
pub fn check_order_quantity(quantity: Quantity, limits: &OrderRiskLimits) -> Result<(), RiskRejection> {
    if quantity > limits.max_quantity {
        return Err(RiskRejection::QuantityExceedsLimit { requested: quantity, limit: limits.max_quantity });
    }
    Ok(())
}

/// A kill switch that can stop new orders independent of any other logic (section 8, MUSS).
/// Shareable (`Clone` is cheap - it's an `Arc` under the hood) so the GUI thread and anything
/// else that needs to trip or check it can hold their own handle to the same switch.
#[derive(Debug, Clone, Default)]
pub struct KillSwitch(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl KillSwitch {
    /// A switch that is not tripped.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Trips the switch: every handle reports it tripped from now on.
    pub fn trip(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Resets the switch: orders may be sent again.
    pub fn reset(&self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether the switch is tripped.
    #[must_use]
    pub fn is_tripped(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// # Errors
    /// Returns [`RiskRejection::KillSwitchTripped`] if the switch is currently tripped.
    pub fn check(&self) -> Result<(), RiskRejection> {
        if self.is_tripped() {
            return Err(RiskRejection::KillSwitchTripped);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    #[test]
    fn quantity_over_the_limit_is_rejected() {
        let limits = OrderRiskLimits { max_quantity: Quantity::new(Decimal::from(10)).unwrap() };
        let ok = Quantity::new(Decimal::from(5)).unwrap();
        let too_much = Quantity::new(Decimal::from(11)).unwrap();
        assert!(check_order_quantity(ok, &limits).is_ok());
        assert_eq!(check_order_quantity(too_much, &limits), Err(RiskRejection::QuantityExceedsLimit { requested: too_much, limit: limits.max_quantity }));
    }

    #[test]
    fn kill_switch_blocks_once_tripped_and_clears_on_reset() {
        let switch = KillSwitch::new();
        assert!(switch.check().is_ok());
        switch.trip();
        assert_eq!(switch.check(), Err(RiskRejection::KillSwitchTripped));
        switch.reset();
        assert!(switch.check().is_ok());
    }

    #[test]
    fn kill_switch_clones_share_the_same_underlying_state() {
        let switch = KillSwitch::new();
        let handle = switch.clone();
        handle.trip();
        assert!(switch.is_tripped(), "a clone shares state - it is not an independent switch");
    }
}
