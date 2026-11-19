//! Deterministic rational EMA crossover strategy.
//!
//! This is a systems fixture, not validated alpha. It emits intents only and
//! never accesses cash, OMS, risk, broker state or positions beyond the
//! caller-supplied context. Arithmetic uses exact rationals with an explicit
//! rounding rule, so a Python `Fraction` implementation can reproduce it.

use crate::{
    artifact::FrozenArtifact, //
    error::StrategyError,
    rational::Rational,
};

pub const SUPPORTED_RULE: &str = "rational_ema_cross_v1";

/// Narrow strategy interface. Strategies propose; they never execute.
pub trait Strategy {
    type Intent;
    fn on_event(&mut self, event: &StrategyEvent) -> Result<Option<Self::Intent>, StrategyError>;
}

/// The only inputs a strategy may observe.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct StrategyEvent {
    pub observation_sequence: u64,
    pub instrument_id: String,
    pub midpoint: Rational,
    pub best_bid: i64,
    pub best_ask: i64,
    pub position_quantity: i64,
    pub has_open_order: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeIntent {
    pub intent_id: String,
    pub candidate_id: String,
    pub instrument_id: String,
    pub side: String,
    pub quantity: i64,
    pub limit_price_paise: i64,
}

pub struct RationalEmaCross {
    candidate_id: String,
    universe: Vec<String>,
    fast: i64,
    slow: i64,
    quantity: i64,
    min_observations: u32,
    aggressive_limit_bps: i64,
    fast_ema: Option<Rational>,
    slow_ema: Option<Rational>,
    previous_fast: Option<Rational>,
    previous_slow: Option<Rational>,
    observations: u32,
    intent_sequence: u32,
    last_sequence: Option<u64>,
}

impl RationalEmaCross {
    /// Construct only from a gated frozen artifact plus bounded parameters.
    pub fn from_artifact(
        artifact: &FrozenArtifact,
        fast: i64,
        slow: i64,
        quantity: i64,
        min_observations: u32,
        aggressive_limit_bps: i64,
    ) -> Result<Self, StrategyError> {
        // Re-gate here: a struct-literal artifact must not bypass load().
        artifact
            .validate_runtime(SUPPORTED_RULE)
            .map_err(|error| StrategyError::ArtifactRejected(error.to_string()))?;
        if fast < 2
            || slow <= fast
            || quantity <= 0
            || aggressive_limit_bps < 0
            || min_observations < slow as u32
        {
            return Err(StrategyError::InvalidConfiguration);
        }
        Ok(Self {
            candidate_id: artifact.candidate_id.clone(),
            universe: artifact.universe.clone(),
            fast,
            slow,
            quantity,
            min_observations,
            aggressive_limit_bps,
            fast_ema: None,
            slow_ema: None,
            previous_fast: None,
            previous_slow: None,
            observations: 0,
            intent_sequence: 0,
            last_sequence: None,
        })
    }

    fn update(
        previous: Option<Rational>,
        value: Rational,
        span: i64,
    ) -> Result<Rational, StrategyError> {
        let Some(previous) = previous else {
            return Ok(value);
        };
        let alpha = Rational::fraction(2, span + 1)?;
        let one = Rational::integer(1);
        let complement = one.checked_sub(alpha)?;
        value
            .checked_mul(alpha)?
            .checked_add(previous.checked_mul(complement)?)
    }

    fn limit_price(side: &str, reference: i64, bps: i64) -> Result<i64, StrategyError> {
        let basis = Rational::fraction(10_000 + if side == "buy" { bps } else { -bps }, 10_000)?;
        Rational::integer(reference).checked_mul(basis)?.round_i64()
    }
}

impl Strategy for RationalEmaCross {
    type Intent = RuntimeIntent;

    fn on_event(&mut self, event: &StrategyEvent) -> Result<Option<Self::Intent>, StrategyError> {
        if self
            .last_sequence
            .is_some_and(|last| event.observation_sequence <= last)
        {
            return Err(StrategyError::NonMonotonicObservation);
        }
        self.last_sequence = Some(event.observation_sequence);

        if !self.universe.contains(&event.instrument_id) {
            return Ok(None);
        }
        if !event.midpoint.is_positive() {
            return Ok(None);
        }

        self.previous_fast = self.fast_ema;
        self.previous_slow = self.slow_ema;
        self.fast_ema = Some(Self::update(self.fast_ema, event.midpoint, self.fast)?);
        self.slow_ema = Some(Self::update(self.slow_ema, event.midpoint, self.slow)?);
        self.observations += 1;

        if self.observations < self.min_observations || event.has_open_order {
            return Ok(None);
        }
        let Some(previous_fast) = self.previous_fast else {
            return Ok(None);
        };
        let Some(previous_slow) = self.previous_slow else {
            return Ok(None);
        };
        let Some(fast) = self.fast_ema else {
            return Ok(None);
        };
        let Some(slow) = self.slow_ema else {
            return Ok(None);
        };

        let crossed_up = previous_fast.cmp_value(&previous_slow) != std::cmp::Ordering::Greater
            && fast.cmp_value(&slow) == std::cmp::Ordering::Greater;
        let crossed_down = previous_fast.cmp_value(&previous_slow) != std::cmp::Ordering::Less
            && fast.cmp_value(&slow) == std::cmp::Ordering::Less;

        let (side, quantity, reference) = if crossed_up && event.position_quantity == 0 {
            ("buy", self.quantity, event.best_ask)
        } else if crossed_down && event.position_quantity > 0 {
            ("sell", event.position_quantity, event.best_bid)
        } else {
            return Ok(None);
        };

        self.intent_sequence += 1;
        Ok(Some(RuntimeIntent {
            intent_id: format!("{}:{:06}", self.candidate_id, self.intent_sequence),
            candidate_id: self.candidate_id.clone(),
            instrument_id: event.instrument_id.clone(),
            side: side.to_owned(),
            quantity,
            limit_price_paise: Self::limit_price(side, reference, self.aggressive_limit_bps)?,
        }))
    }
}
