//! Effective-dated Indian transaction cost schedules.
//!
//! Derived from the internal Nirnay `crates/nirnay-engine/src/fees.rs` at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! Every rate is basis points scaled by one billion (parts per billion of
//! turnover) and every charge is rounded to the nearest paise, half up, in
//! integer arithmetic. The schedule is effective-dated and selected by
//! product and side, so a historical calculation never uses a future rate.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use trading_core::{ChargeComponents, CheckedPaise, DomainError, Paise, Product, Side, TradeDate};

const RATE_SCALE: i128 = 1_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChargeRate {
    pub effective_from: TradeDate,
    pub product: Product,
    pub side: Side,
    pub brokerage_ppb: u64,
    pub stt_ppb: u64,
    pub exchange_ppb: u64,
    pub sebi_ppb: u64,
    pub stamp_ppb: u64,
    pub gst_ppb: u64,
}

impl ChargeRate {
    fn validate(&self) -> Result<(), FeeError> {
        let rates = [
            self.brokerage_ppb,
            self.stt_ppb,
            self.exchange_ppb,
            self.sebi_ppb,
            self.stamp_ppb,
            self.gst_ppb,
        ];
        if rates.into_iter().any(|rate| rate > RATE_SCALE as u64) {
            return Err(FeeError::InvalidRate);
        }
        Ok(())
    }
}

/// Validated, ordered set of charge schedules.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Vec<ChargeRate>")]
pub struct ChargeBook {
    rates: Vec<ChargeRate>,
}

#[derive(Debug, Error)]
pub enum FeeError {
    #[error("charge rate is outside the accepted range")]
    InvalidRate,
    #[error("two charge schedules share the same effective date, product and side")]
    DuplicateEffectiveDate,
    #[error("no charge schedule covers the trade date, product and side")]
    MissingSchedule,
    #[error("charge arithmetic failed: {0}")]
    Arithmetic(#[from] DomainError),
}

impl ChargeBook {
    pub fn new(mut rates: Vec<ChargeRate>) -> Result<Self, FeeError> {
        for rate in &rates {
            rate.validate()?;
        }
        rates.sort_by_key(|rate| (rate.product, side_rank(rate.side), rate.effective_from));
        for pair in rates.windows(2) {
            if let [previous, next] = pair
                && previous.product == next.product
                && previous.side == next.side
                && previous.effective_from == next.effective_from
            {
                return Err(FeeError::DuplicateEffectiveDate);
            }
        }
        Ok(Self { rates })
    }

    /// Select the latest schedule effective on or before `trade_date`, then
    /// compute every charge component for `turnover`.
    pub fn calculate(
        &self,
        trade_date: TradeDate,
        product: Product,
        side: Side,
        turnover: Paise,
    ) -> Result<ChargeComponents, FeeError> {
        let rate = self
            .rates
            .iter()
            .rev()
            .find(|rate| {
                rate.effective_from <= trade_date && rate.product == product && rate.side == side
            })
            .ok_or(FeeError::MissingSchedule)?;
        let brokerage = levy(turnover, rate.brokerage_ppb)?;
        let stt = levy(turnover, rate.stt_ppb)?;
        let exchange = levy(turnover, rate.exchange_ppb)?;
        let sebi = levy(turnover, rate.sebi_ppb)?;
        let stamp = levy(turnover, rate.stamp_ppb)?;
        let taxable = brokerage.checked_add(exchange)?.checked_add(sebi)?;
        let gst = levy(taxable, rate.gst_ppb)?;
        Ok(ChargeComponents {
            brokerage,
            securities_transaction_tax: stt,
            exchange_transaction: exchange,
            sebi_turnover: sebi,
            stamp_duty: stamp,
            gst,
            other: Paise(0),
        })
    }

    #[must_use]
    pub fn schedule_count(&self) -> usize {
        self.rates.len()
    }
}

fn side_rank(side: Side) -> u8 {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}

fn levy(turnover: Paise, rate_ppb: u64) -> Result<Paise, DomainError> {
    let scaled = i128::from(turnover.0)
        .checked_mul(i128::from(rate_ppb))
        .and_then(|value| value.checked_add(RATE_SCALE / 2))
        .ok_or(DomainError::Overflow)?;
    let paise = scaled.div_euclid(RATE_SCALE);
    i64::try_from(paise)
        .map(Paise)
        .map_err(|_| DomainError::Overflow)
}

impl TryFrom<Vec<ChargeRate>> for ChargeBook {
    type Error = FeeError;

    fn try_from(rates: Vec<ChargeRate>) -> Result<Self, Self::Error> {
        Self::new(rates)
    }
}
