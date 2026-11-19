//! Exact rational arithmetic for deterministic strategy state.
//!
//! No floating point. Numerator and denominator are kept in lowest terms with
//! a positive denominator; every operation is checked.

use crate::error::StrategyError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Rational {
    num: i128,
    den: i128,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    if a == 0 { 1 } else { a }
}

impl Rational {
    #[must_use]
    pub fn integer(value: i64) -> Self {
        Self {
            num: i128::from(value),
            den: 1,
        }
    }

    pub fn fraction(numerator: i64, denominator: i64) -> Result<Self, StrategyError> {
        if denominator == 0 {
            return Err(StrategyError::InvalidConfiguration);
        }
        let mut num = i128::from(numerator);
        let mut den = i128::from(denominator);
        if den < 0 {
            num = num.checked_neg().ok_or(StrategyError::Overflow)?;
            den = den.checked_neg().ok_or(StrategyError::Overflow)?;
        }
        let divisor = gcd(num, den);
        Ok(Self {
            num: num / divisor,
            den: den / divisor,
        })
    }

    fn checked(self, num: i128, den: i128) -> Result<Self, StrategyError> {
        if den == 0 {
            return Err(StrategyError::Overflow);
        }
        let mut n = num;
        let mut d = den;
        if d < 0 {
            n = n.checked_neg().ok_or(StrategyError::Overflow)?;
            d = d.checked_neg().ok_or(StrategyError::Overflow)?;
        }
        let divisor = gcd(n, d);
        Ok(Self {
            num: n / divisor,
            den: d / divisor,
        })
    }

    pub fn checked_add(self, other: Self) -> Result<Self, StrategyError> {
        let num = self
            .num
            .checked_mul(other.den)
            .and_then(|a| a.checked_add(other.num.checked_mul(self.den)?))
            .ok_or(StrategyError::Overflow)?;
        let den = self
            .den
            .checked_mul(other.den)
            .ok_or(StrategyError::Overflow)?;
        self.checked(num, den)
    }

    pub fn checked_mul(self, other: Self) -> Result<Self, StrategyError> {
        let num = self
            .num
            .checked_mul(other.num)
            .ok_or(StrategyError::Overflow)?;
        let den = self
            .den
            .checked_mul(other.den)
            .ok_or(StrategyError::Overflow)?;
        self.checked(num, den)
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, StrategyError> {
        let neg = Self {
            num: other.num.checked_neg().ok_or(StrategyError::Overflow)?,
            den: other.den,
        };
        self.checked_add(neg)
    }

    #[must_use]
    pub const fn is_positive(&self) -> bool {
        self.num > 0
    }

    #[must_use]
    pub fn cmp_value(&self, other: &Self) -> std::cmp::Ordering {
        let left = self.num.saturating_mul(other.den);
        let right = other.num.saturating_mul(self.den);
        left.cmp(&right)
    }

    /// Round to the nearest integer, halves away from zero (explicit rule).
    pub fn round_i64(&self) -> Result<i64, StrategyError> {
        let abs_num = self.num.abs();
        let quotient = abs_num / self.den;
        let remainder = abs_num % self.den;
        let rounded = if remainder * 2 >= self.den {
            quotient.checked_add(1).ok_or(StrategyError::Overflow)?
        } else {
            quotient
        };
        let signed = if self.num < 0 {
            rounded.checked_neg().ok_or(StrategyError::Overflow)?
        } else {
            rounded
        };
        i64::try_from(signed).map_err(|_| StrategyError::Overflow)
    }

    #[must_use]
    pub fn as_parts(&self) -> (i128, i128) {
        (self.num, self.den)
    }
}
