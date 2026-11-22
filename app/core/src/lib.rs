//! Domain newtypes (`coding_guideline.md` section 3 and 5): no bare `f64`/`String`/`i32` for
//! money, quantities or order identity, and constructors validate instead of letting invalid
//! values exist. This crate holds only pure domain types - no network, file or time access
//! (section 2) - plus the Black-Scholes/Greeks calculator, which is pure computation with no I/O.
//!
//! This is the first of several migration stages (see `coding_guideline.md`): the wire-protocol
//! layer (`broker`) and the GUI (`trade_pilot`) still use plain `f64`/`String` internally for now
//! (mirroring `IBApi`'s own `double` fields, and user-typed text fields) and only convert to these
//! types at specific boundaries (e.g. before a risk check). Converting every `f64` price/quantity
//! field throughout the whole codebase to these types is a separate, larger follow-up stage.
#![forbid(unsafe_code)]
// coding_guideline.md section 4: unwrap/expect/panic/indexing are denied in production code, but
// explicitly exempted for tests - this crate's workspace-inherited clippy lints would otherwise
// also flag the `.unwrap()`s this crate's own `#[cfg(test)]` modules use.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing))]

mod greeks;

pub use greeks::{calculate_greeks, IV_TOLERANCE};

use rust_decimal::Decimal;
use std::fmt;

/// A validated, non-negative order quantity. Rejects zero, negative and non-finite values at
/// construction instead of letting an invalid `Quantity` exist anywhere in the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Quantity(Decimal);

/// Why a raw value could not become a validated domain type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// The value is zero or negative where it must be greater than zero.
    NotPositive,
    /// The value is NaN or infinite.
    NotFinite,
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DomainError::NotPositive => write!(f, "value must be greater than zero"),
            DomainError::NotFinite => write!(f, "value must be a finite number"),
        }
    }
}

impl std::error::Error for DomainError {}

impl Quantity {
    /// # Errors
    /// Returns [`DomainError::NotPositive`] if `value` is zero or negative.
    pub fn new(value: Decimal) -> Result<Self, DomainError> {
        if value <= Decimal::ZERO {
            return Err(DomainError::NotPositive);
        }
        Ok(Self(value))
    }

    /// # Errors
    /// Returns [`DomainError::NotFinite`] if `value` is NaN or infinite (as reported by
    /// [`f64::is_finite`]), or [`DomainError::NotPositive`] if it is zero or negative.
    ///
    /// Exists only as a bridge from the GUI's still-`f64`-typed text fields (see the module doc)
    /// to this validated type - new code that already has a `Decimal` should call [`Self::new`].
    pub fn from_f64(value: f64) -> Result<Self, DomainError> {
        if !value.is_finite() {
            return Err(DomainError::NotFinite);
        }
        let decimal = Decimal::try_from(value).map_err(|_| DomainError::NotFinite)?;
        Self::new(decimal)
    }

    /// A whole number of contracts; it cannot fail, the type already rules out 0.
    #[must_use]
    pub fn from_count(count: std::num::NonZeroU32) -> Self {
        Self(Decimal::from(count.get()))
    }

    /// The quantity as a decimal.
    #[must_use]
    pub fn get(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A validated price (may be negative - e.g. some spreads/credits are quoted negative - but must
/// be a real, finite number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Price(Decimal);

impl Price {
    /// A price of `value` (every `Decimal` is finite).
    #[must_use]
    pub fn new(value: Decimal) -> Self {
        Self(value)
    }

    /// # Errors
    /// Returns [`DomainError::NotFinite`] if `value` is NaN or infinite.
    pub fn from_f64(value: f64) -> Result<Self, DomainError> {
        if !value.is_finite() {
            return Err(DomainError::NotFinite);
        }
        Decimal::try_from(value).map(Self).map_err(|_| DomainError::NotFinite)
    }

    /// The price as a decimal.
    #[must_use]
    pub fn get(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An IB order id (`IBApi.EClient`'s own `nextValidId` sequence - see the `broker` crate). A
/// newtype instead of a bare `i32` so it can't be confused with a request id, quantity, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderId(i32);

impl OrderId {
    /// The order id `value`.
    #[must_use]
    pub fn new(value: i32) -> Self {
        Self(value)
    }

    /// The order id as TWS knows it.
    #[must_use]
    pub fn get(self) -> i32 {
        self.0
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quantity_from_a_count_is_that_whole_number() {
        assert_eq!(Quantity::from_count(std::num::NonZeroU32::MIN).get(), Decimal::ONE);
        assert_eq!(Quantity::from_count(std::num::NonZeroU32::new(1000).unwrap()).get(), Decimal::from(1000));
    }

    #[test]
    fn quantity_rejects_zero_and_negative() {
        assert_eq!(Quantity::new(Decimal::ZERO), Err(DomainError::NotPositive));
        assert_eq!(Quantity::new(Decimal::from(-1)), Err(DomainError::NotPositive));
        assert!(Quantity::new(Decimal::from(1)).is_ok());
    }

    #[test]
    fn quantity_from_f64_rejects_non_finite() {
        assert_eq!(Quantity::from_f64(f64::NAN), Err(DomainError::NotFinite));
        assert_eq!(Quantity::from_f64(f64::INFINITY), Err(DomainError::NotFinite));
        assert_eq!(Quantity::from_f64(0.0), Err(DomainError::NotPositive));
        assert!(Quantity::from_f64(2.5).is_ok());
    }

    #[test]
    fn price_allows_negative_but_not_non_finite() {
        assert!(Price::from_f64(-12.5).is_ok());
        assert_eq!(Price::from_f64(f64::NAN), Err(DomainError::NotFinite));
    }
}
