//! Exact money, price and quantity conversions.
//!
//! Canonical cash is `india_domain::Paise` and canonical quantity is
//! `india_domain::Lots`. This module adds only validated wrappers
//! ([`PricePaise`], [`Quantity`]) and checked arithmetic. Every conversion is
//! explicit and rejects loss: no floats, no silent truncation, no sign flip.

use serde::{Deserialize, Serialize};

use crate::DomainError;

pub use india_domain::{Lots, Paise};

/// Checked arithmetic over canonical paise. `india-domain` keeps `Paise` a
/// plain exact value; the trading layer needs overflow-safe operations at the
/// accounting boundary.
pub trait CheckedPaise {
    fn checked_add(self, other: Paise) -> Result<Paise, DomainError>;
    fn checked_sub(self, other: Paise) -> Result<Paise, DomainError>;
    fn checked_neg(self) -> Result<Paise, DomainError>;
    fn checked_scale(self, factor: i64) -> Result<Paise, DomainError>;
}

impl CheckedPaise for Paise {
    fn checked_add(self, other: Paise) -> Result<Paise, DomainError> {
        self.0
            .checked_add(other.0)
            .map(Paise)
            .ok_or(DomainError::Overflow)
    }

    fn checked_sub(self, other: Paise) -> Result<Paise, DomainError> {
        self.0
            .checked_sub(other.0)
            .map(Paise)
            .ok_or(DomainError::Overflow)
    }

    fn checked_neg(self) -> Result<Paise, DomainError> {
        self.0.checked_neg().map(Paise).ok_or(DomainError::Overflow)
    }

    fn checked_scale(self, factor: i64) -> Result<Paise, DomainError> {
        let product = i128::from(self.0) * i128::from(factor);
        i64::try_from(product)
            .map(Paise)
            .map_err(|_| DomainError::Overflow)
    }
}

/// Non-negative quantity in lots. Negative quantities have no meaning for an
/// order; positions retain signed `i64` lot counts in the ledger.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct Quantity(i64);

impl Quantity {
    pub const ZERO: Self = Self(0);
    /// Largest representable quantity, used by projections to represent an
    /// unavailable limit without hiding the failure (risk will reject it).
    pub const MAX: Self = Self(i64::MAX);

    pub fn new(value: i64) -> Result<Self, DomainError> {
        if value < 0 {
            return Err(DomainError::NegativeQuantity);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, DomainError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(DomainError::Overflow)
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, DomainError> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or(DomainError::Underflow)
    }

    /// Lossless conversion to the canonical lot unit.
    #[must_use]
    pub const fn as_lots(self) -> Lots {
        Lots(self.0)
    }
}

impl TryFrom<i64> for Quantity {
    type Error = DomainError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<Lots> for Quantity {
    type Error = DomainError;

    fn try_from(value: Lots) -> Result<Self, Self::Error> {
        Self::new(value.0)
    }
}

impl From<Quantity> for Lots {
    fn from(value: Quantity) -> Self {
        value.as_lots()
    }
}

impl From<Quantity> for i64 {
    fn from(value: Quantity) -> Self {
        value.0
    }
}

/// Positive quoted price in canonical paise.
///
/// The inner value is private so only [`PricePaise::new`] can create a price;
/// deserialization also goes through validation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "Paise", into = "Paise")]
pub struct PricePaise(Paise);

impl PricePaise {
    pub fn new(value: Paise) -> Result<Self, DomainError> {
        if value.0 <= 0 {
            return Err(DomainError::ZeroPrice);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> Paise {
        self.0
    }

    /// Exact turnover: price (paise) times quantity (lots). The result is in
    /// paise only because one lot is one unit of the underlying contract;
    /// contract multipliers belong to instrument metadata, not this type.
    pub fn notional(self, quantity: Quantity) -> Result<Paise, DomainError> {
        let product = i128::from(self.0.0) * i128::from(quantity.get());
        i64::try_from(product)
            .map(Paise)
            .map_err(|_| DomainError::Overflow)
    }

    /// Exact midpoint of two whole-paise prices, as doubled paise (M₂ = b + a).
    /// A mid price is never rounded into whole paise here: comparisons and
    /// valuation must be able to see the half exactly.
    pub fn midpoint_doubled(self, other: Self) -> DoubledPaise {
        DoubledPaise::from_paise_pair(self.get(), other.get())
    }

    pub fn shifted_ticks(self, tick: Paise, ticks: i64) -> Result<Self, DomainError> {
        let shifted = i128::from(self.0.0) + i128::from(tick.0) * i128::from(ticks);
        let value = i64::try_from(shifted).map_err(|_| DomainError::Overflow)?;
        Self::new(Paise(value))
    }
}

/// Exact midpoint between two whole-paise prices, stored as doubled paise
/// (M₂ = b + a, so the true midpoint is M₂ / 2). Rounding this value into
/// integer paise is a documented, explicit step at the accounting boundary,
/// never an implicit side effect of comparison or arithmetic.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DoubledPaise {
    value: i128,
}

impl DoubledPaise {
    /// Twice the midpoint of two prices. i128 arithmetic: the sum of two
    /// i64 paise values cannot overflow.
    #[must_use]
    pub fn from_paise_pair(bid: Paise, ask: Paise) -> Self {
        Self {
            value: i128::from(bid.0) + i128::from(ask.0),
        }
    }

    /// M₂ as an integer (twice the midpoint, in paise).
    #[must_use]
    pub const fn value(self) -> i128 {
        self.value
    }

    /// Round the midpoint to the nearest whole paise, ties to even
    /// (round-half-even: the unbiased choice when half-paise cases are
    /// systematic). Fails with overflow only outside the representable
    /// i64 pair range, which checked domain prices cannot reach.
    pub fn rounded_paise(self) -> Result<Paise, DomainError> {
        let quotient = self.value.div_euclid(2);
        let remainder = self.value.rem_euclid(2);
        let rounded = if remainder == 1 && quotient % 2 != 0 {
            quotient + 1
        } else {
            quotient
        };
        i64::try_from(rounded)
            .map(Paise)
            .map_err(|_| DomainError::Overflow)
    }
}

impl TryFrom<Paise> for PricePaise {
    type Error = DomainError;

    fn try_from(value: Paise) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PricePaise> for Paise {
    fn from(value: PricePaise) -> Self {
        value.get()
    }
}
