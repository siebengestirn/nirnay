//! Stateful risk authority with one atomic authorization operation.
//!
//! The authority owns every fact that `evaluate_risk` consumes: account-global
//! exposure, positions, pending orders, rate state, daily P&L, the logical
//! clock, snapshot availability, kill switch and reconciliation state. A caller
//! supplies only an [`OrderIntent`] and a raw [`FeedObservation`]; it cannot
//! assert a `Healthy` flag, an exposure figure or an account snapshot.
//!
//! Evaluation and reservation happen under a single mutex lock, so two
//! concurrent callers can never both evaluate against the same stale state.
//! On full acceptance the reservation is applied before the lock is released;
//! on any rejection nothing is reserved.

use std::{
    collections::{
        BTreeMap, //
        BTreeSet,
        VecDeque,
    },
    sync::Mutex,
};

use risk_engine::{
    RiskContext, //
    ValidatedRiskPolicy,
    evaluate_risk,
};
use serde::{
    Deserialize, //
    Serialize,
};
use trading_core::{
    AccountId, //
    CheckedPaise,
    FeedHealth,
    Instrument,
    InstrumentId,
    IntentId,
    LogicalNanos,
    OrderIntent,
    Paise,
    Quantity,
    RiskAssessment,
    Side,
    StrategyId,
    TradeDate,
    Venue,
};

use crate::{
    error::AuthorityError, //
    observation::FeedObservation,
};

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// A committed reservation: the only output that may proceed toward an OMS.
/// It is a permit, not an order submission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    pub permit_id: u64,
    pub intent_id: IntentId,
    pub account_id: AccountId,
    pub strategy_id: StrategyId,
    pub instrument_id: InstrumentId,
    pub venue: Venue,
    pub side: Side,
    pub quantity: Quantity,
    pub reserved_notional_paise: Paise,
    /// Authority state revision against which this permit was granted.
    pub snapshot_revision: u64,
    pub assessment: RiskAssessment,
}

impl Reservation {
    /// A permit must be self-consistent before any downstream consumer acts
    /// on it. Deserialized or fabricated values are not authoritative.
    pub fn validate_invariants(&self) -> Result<(), AuthorityError> {
        if self.permit_id == 0 {
            return Err(AuthorityError::InvalidReservation(
                "zero permit id".to_owned(),
            ));
        }
        if !self.assessment.accepted {
            return Err(AuthorityError::InvalidReservation(
                "assessment was not accepted".to_owned(),
            ));
        }
        if self.assessment.intent_id != self.intent_id {
            return Err(AuthorityError::InvalidReservation(
                "intent id mismatch".to_owned(),
            ));
        }
        if self.reserved_notional_paise.0 <= 0 {
            return Err(AuthorityError::InvalidReservation(
                "non-positive notional".to_owned(),
            ));
        }
        if self.quantity.is_zero() {
            return Err(AuthorityError::InvalidReservation(
                "zero quantity".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Read-only view of authority state for operating evidence and tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritySnapshot {
    pub revision: u64,
    pub now: LogicalNanos,
    pub gross_exposure_paise: Paise,
    pub pending_gross_exposure_paise: Paise,
    pub open_orders: u32,
    pub orders_last_second: u32,
    pub positions: BTreeMap<InstrumentId, i64>,
    pub kill_switch_engaged: bool,
    pub reconciliation_blocked: bool,
    pub account_snapshot_available: bool,
    pub reservation_count: usize,
}

/// All mutable account-global facts. Never constructed from caller claims.
#[derive(Debug)]
struct AuthorityState {
    now: LogicalNanos,
    revision: u64,
    session_open: bool,
    account_snapshot_available: bool,
    kill_switch_engaged: bool,
    reconciliation_blocked: bool,
    daily_pnl: Paise,
    positions: BTreeMap<InstrumentId, i64>,
    pending_buy: BTreeMap<InstrumentId, i64>,
    pending_sell: BTreeMap<InstrumentId, i64>,
    gross_exposure_paise: Paise,
    pending_gross_exposure_paise: Paise,
    open_orders: u32,
    order_times: VecDeque<LogicalNanos>,
    reservations: BTreeMap<IntentId, Reservation>,
    decided: BTreeMap<IntentId, OrderIntent>,
    consumed: BTreeSet<IntentId>,
    next_permit: u64,
}

impl Default for AuthorityState {
    fn default() -> Self {
        Self {
            now: LogicalNanos::ZERO,
            revision: 0,
            session_open: true,
            account_snapshot_available: true,
            kill_switch_engaged: false,
            reconciliation_blocked: false,
            daily_pnl: Paise(0),
            positions: BTreeMap::new(),
            pending_buy: BTreeMap::new(),
            pending_sell: BTreeMap::new(),
            gross_exposure_paise: Paise(0),
            pending_gross_exposure_paise: Paise(0),
            open_orders: 0,
            order_times: VecDeque::new(),
            reservations: BTreeMap::new(),
            decided: BTreeMap::new(),
            consumed: BTreeSet::new(),
            next_permit: 1,
        }
    }
}

impl AuthorityState {
    fn orders_last_second(&self) -> u32 {
        let cutoff = self.now.get().saturating_sub(NANOS_PER_SECOND);
        let count = self
            .order_times
            .iter()
            .filter(|at| at.get() >= cutoff)
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    fn pending_quantity(
        quantities: &BTreeMap<InstrumentId, i64>,
        id: &InstrumentId,
    ) -> Result<Quantity, AuthorityError> {
        let value = quantities.get(id).copied().unwrap_or(0);
        Quantity::new(value).map_err(|error| AuthorityError::InvalidIntent(error.to_string()))
    }

    /// Keep only order times inside the rate window so memory stays bounded.
    fn prune_order_times(&mut self) {
        let cutoff = self.now.get().saturating_sub(NANOS_PER_SECOND);
        while self.order_times.front().is_some_and(|at| at.get() < cutoff) {
            self.order_times.pop_front();
        }
    }
}

/// The authorization boundary. Shareable across threads; all state transitions
/// are serialized internally.
pub struct RiskAuthority {
    account_id: AccountId,
    policy: ValidatedRiskPolicy,
    instruments: BTreeMap<InstrumentId, Instrument>,
    trade_date: TradeDate,
    state: Mutex<AuthorityState>,
}

impl RiskAuthority {
    /// Build an authority with validated policy and owned effective-dated
    /// reference data. No order, OMS, broker or live dependency.
    pub fn new(
        account_id: AccountId,
        policy: ValidatedRiskPolicy,
        instruments: Vec<Instrument>,
        trade_date: TradeDate,
    ) -> Self {
        Self {
            account_id,
            policy,
            instruments: instruments
                .into_iter()
                .map(|instrument| (instrument.instrument_id.clone(), instrument))
                .collect(),
            trade_date,
            state: Mutex::new(AuthorityState::default()),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, AuthorityState>, AuthorityError> {
        self.state.lock().map_err(|_| AuthorityError::StatePoisoned)
    }

    /// Current state revision, used as an optimistic-concurrency fence by
    /// callers that hold a previously observed account snapshot.
    pub fn revision(&self) -> Result<u64, AuthorityError> {
        Ok(self.lock()?.revision)
    }

    pub fn set_now(&self, now: LogicalNanos) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        state.now = now;
        state.prune_order_times();
        state.revision += 1;
        Ok(())
    }

    pub fn set_session_open(&self, open: bool) -> Result<(), AuthorityError> {
        self.lock()?.session_open = open;
        Ok(())
    }

    pub fn set_snapshot_available(&self, available: bool) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        state.account_snapshot_available = available;
        state.revision += 1;
        Ok(())
    }

    pub fn set_kill_switch(&self, engaged: bool) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        state.kill_switch_engaged = engaged;
        state.revision += 1;
        Ok(())
    }

    pub fn set_reconciliation_blocked(&self, blocked: bool) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        state.reconciliation_blocked = blocked;
        state.revision += 1;
        Ok(())
    }

    pub fn set_daily_pnl(&self, pnl: Paise) -> Result<(), AuthorityError> {
        let mut state = self.lock()?;
        state.daily_pnl = pnl;
        state.revision += 1;
        Ok(())
    }

    /// The single atomic gate: evaluate the intent against authority-owned
    /// facts, then reserve exposure only on full acceptance.
    pub fn evaluate_and_reserve(
        &self,
        intent: OrderIntent,
        observation: FeedObservation,
        expected_revision: Option<u64>,
    ) -> Result<Reservation, AuthorityError> {
        let mut state = self.lock()?;
        intent
            .validate()
            .map_err(|error| AuthorityError::InvalidIntent(error.to_string()))?;

        // Idempotency: an accepted replay returns its original permit without
        // reserving again; a reused id with different terms is conflicting.
        if let Some(existing) = state.reservations.get(&intent.intent_id) {
            let prior = state
                .decided
                .get(&intent.intent_id)
                .ok_or(AuthorityError::StatePoisoned)?;
            if *prior != intent {
                return Err(AuthorityError::ConflictingIntent(
                    intent.intent_id.to_string(),
                ));
            }
            return Ok(existing.clone());
        }
        if let Some(prior) = state.decided.get(&intent.intent_id) {
            if *prior != intent {
                return Err(AuthorityError::ConflictingIntent(
                    intent.intent_id.to_string(),
                ));
            }
            if state.consumed.contains(&intent.intent_id) {
                return Err(AuthorityError::AlreadyConsumed(
                    intent.intent_id.to_string(),
                ));
            }
            return Err(AuthorityError::PreviouslyRejected(
                intent.intent_id.to_string(),
            ));
        }

        if let Some(expected) = expected_revision
            && expected != state.revision
        {
            return Err(AuthorityError::StaleSnapshot {
                expected,
                current: state.revision,
            });
        }

        if observation.received_at > state.now {
            return Err(AuthorityError::FutureObservation);
        }

        let context = self.build_context(&state, &intent, &observation)?;
        let assessment = evaluate_risk(&intent, &self.policy, &context);
        if !assessment.accepted {
            state.decided.insert(intent.intent_id, intent);
            return Err(AuthorityError::Rejected(assessment));
        }

        self.commit_reservation(&mut state, &intent, &assessment)
    }

    fn build_context(
        &self,
        state: &AuthorityState,
        intent: &OrderIntent,
        observation: &FeedObservation,
    ) -> Result<RiskContext, AuthorityError> {
        let instrument = self.instruments.get(&intent.instrument_id).cloned();
        let quote = observation.quote.clone().filter(|value| {
            value.instrument_id == intent.instrument_id && value.venue == intent.venue
        });
        let feed_health = if quote.is_some() {
            observation.health(state.now, self.policy.max_quote_age_ns)
        } else {
            FeedHealth::NoQuote
        };
        let position_quantity = state
            .positions
            .get(&intent.instrument_id)
            .copied()
            .unwrap_or(0);
        let pending_buy_quantity =
            AuthorityState::pending_quantity(&state.pending_buy, &intent.instrument_id)?;
        let pending_sell_quantity =
            AuthorityState::pending_quantity(&state.pending_sell, &intent.instrument_id)?;

        Ok(RiskContext {
            account_id: self.account_id,
            strategy_id: intent.strategy_id,
            venue: intent.venue,
            instrument,
            quote,
            feed_health,
            trade_date: self.trade_date,
            now: state.now,
            position_quantity,
            pending_buy_quantity,
            pending_sell_quantity,
            gross_exposure_paise: state.gross_exposure_paise,
            pending_gross_exposure_paise: state.pending_gross_exposure_paise,
            daily_pnl: state.daily_pnl,
            open_orders: state.open_orders,
            orders_last_second: state.orders_last_second(),
            session_open: state.session_open,
            account_snapshot_available: state.account_snapshot_available,
            kill_switch_engaged: state.kill_switch_engaged,
            reconciliation_blocked: state.reconciliation_blocked,
            duplicate_intent: false,
        })
    }

    fn commit_reservation(
        &self,
        state: &mut AuthorityState,
        intent: &OrderIntent,
        assessment: &RiskAssessment,
    ) -> Result<Reservation, AuthorityError> {
        let notional = i128::from(intent.limit_price.get().0) * i128::from(intent.quantity.get());
        let notional_paise =
            Paise(i64::try_from(notional).map_err(|_| AuthorityError::NotionalOverflow)?);
        state.pending_gross_exposure_paise = state
            .pending_gross_exposure_paise
            .checked_add(notional_paise)
            .map_err(|_| AuthorityError::NotionalOverflow)?;
        let pending = match intent.side {
            Side::Buy => &mut state.pending_buy,
            Side::Sell => &mut state.pending_sell,
        };
        let entry = pending.entry(intent.instrument_id.clone()).or_insert(0);
        *entry = (*entry).checked_add(intent.quantity.get()).ok_or(
            AuthorityError::AccountingOverflow("pending quantity".to_owned()),
        )?;

        state.open_orders = state
            .open_orders
            .checked_add(1)
            .ok_or(AuthorityError::AccountingOverflow("open orders".to_owned()))?;
        state.order_times.push_back(state.now);
        state.prune_order_times();
        state.revision = state.revision.saturating_add(1);
        let permit_id = state.next_permit;
        state.next_permit = state.next_permit.saturating_add(1);

        let reservation = Reservation {
            permit_id,
            intent_id: intent.intent_id,
            account_id: self.account_id,
            strategy_id: intent.strategy_id,
            instrument_id: intent.instrument_id.clone(),
            venue: intent.venue,
            side: intent.side,
            quantity: intent.quantity,
            reserved_notional_paise: notional_paise,
            snapshot_revision: state.revision,
            assessment: assessment.clone(),
        };
        reservation.validate_invariants()?;
        state
            .reservations
            .insert(intent.intent_id, reservation.clone());
        state.decided.insert(intent.intent_id, intent.clone());
        Ok(reservation)
    }

    /// Idempotent replay of an accepted intent returns its original permit.
    pub fn reservation(&self, intent_id: IntentId) -> Result<Reservation, AuthorityError> {
        let state = self.lock()?;
        let reservation = state
            .reservations
            .get(&intent_id)
            .cloned()
            .ok_or(AuthorityError::UnknownIntent(intent_id.to_string()))?;
        reservation.validate_invariants()?;
        Ok(reservation)
    }

    /// Release a reservation (e.g. an order that never reached the market).
    pub fn release(&self, intent_id: IntentId) -> Result<Reservation, AuthorityError> {
        let mut state = self.lock()?;
        let reservation = state
            .reservations
            .remove(&intent_id)
            .ok_or(AuthorityError::UnknownIntent(intent_id.to_string()))?;
        state.pending_gross_exposure_paise = Paise(
            state
                .pending_gross_exposure_paise
                .0
                .checked_sub(reservation.reserved_notional_paise.0)
                .ok_or(AuthorityError::AccountingOverflow(
                    "pending exposure".to_owned(),
                ))?,
        );
        self.remove_pending(&mut state, &reservation)?;
        state.open_orders = state
            .open_orders
            .checked_sub(1)
            .ok_or(AuthorityError::AccountingOverflow("open orders".to_owned()))?;
        state.consumed.insert(intent_id);
        state.revision = state.revision.saturating_add(1);
        Ok(reservation)
    }

    /// Convert a reservation into a filled position.
    pub fn apply_fill(&self, intent_id: IntentId) -> Result<Reservation, AuthorityError> {
        let mut state = self.lock()?;
        let reservation = state
            .reservations
            .remove(&intent_id)
            .ok_or(AuthorityError::UnknownIntent(intent_id.to_string()))?;
        state.pending_gross_exposure_paise = Paise(
            state
                .pending_gross_exposure_paise
                .0
                .checked_sub(reservation.reserved_notional_paise.0)
                .ok_or(AuthorityError::AccountingOverflow(
                    "pending exposure".to_owned(),
                ))?,
        );
        state.gross_exposure_paise = Paise(
            state
                .gross_exposure_paise
                .0
                .checked_add(reservation.reserved_notional_paise.0)
                .ok_or(AuthorityError::AccountingOverflow(
                    "gross exposure".to_owned(),
                ))?,
        );
        self.remove_pending(&mut state, &reservation)?;
        let signed = match reservation.side {
            Side::Buy => reservation.quantity.get(),
            Side::Sell => -reservation.quantity.get(),
        };
        let position = state
            .positions
            .entry(reservation.instrument_id.clone())
            .or_insert(0);
        *position = position
            .checked_add(signed)
            .ok_or(AuthorityError::AccountingOverflow("position".to_owned()))?;
        state.open_orders = state
            .open_orders
            .checked_sub(1)
            .ok_or(AuthorityError::AccountingOverflow("open orders".to_owned()))?;
        state.consumed.insert(intent_id);
        state.revision = state.revision.saturating_add(1);
        Ok(reservation)
    }

    fn remove_pending(
        &self,
        state: &mut AuthorityState,
        reservation: &Reservation,
    ) -> Result<(), AuthorityError> {
        let pending = match reservation.side {
            Side::Buy => &mut state.pending_buy,
            Side::Sell => &mut state.pending_sell,
        };
        if let std::collections::btree_map::Entry::Occupied(mut entry) =
            pending.entry(reservation.instrument_id.clone())
        {
            let remaining = entry.get().checked_sub(reservation.quantity.get()).ok_or(
                AuthorityError::AccountingOverflow("pending quantity".to_owned()),
            )?;
            if remaining <= 0 {
                entry.remove();
            } else {
                *entry.get_mut() = remaining;
            }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Result<AuthoritySnapshot, AuthorityError> {
        let state = self.lock()?;
        Ok(AuthoritySnapshot {
            revision: state.revision,
            now: state.now,
            gross_exposure_paise: state.gross_exposure_paise,
            pending_gross_exposure_paise: state.pending_gross_exposure_paise,
            open_orders: state.open_orders,
            orders_last_second: state.orders_last_second(),
            positions: state.positions.clone(),
            kill_switch_engaged: state.kill_switch_engaged,
            reconciliation_blocked: state.reconciliation_blocked,
            account_snapshot_available: state.account_snapshot_available,
            reservation_count: state.reservations.len(),
        })
    }

    pub fn policy_version(&self) -> &str {
        &self.policy.version
    }
}
