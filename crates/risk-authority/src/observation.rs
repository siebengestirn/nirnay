//! Typed, versioned observations owned by the risk authority.
//!
//! Callers submit raw facts from six independent owners. Every observation
//! carries a source, schema version, observed and received logical times and a
//! revision. The authority alone checks freshness and derives health; no
//! caller-supplied verdict or boolean crosses this boundary.

use std::{
    collections::BTreeMap, //
    fmt,
};

use serde::Serialize;
use trading_core::{
    BorrowAvailability, //
    FeedHealth,
    Greeks,
    InstrumentId,
    LiquidityDepth,
    LogicalNanos,
    Paise,
    QuoteSnapshot,
    ReconciliationReport,
};

/// Version of every M3.6.1 observation payload. An unknown version is denied.
pub const OBSERVATION_SCHEMA_VERSION: u32 = 1;

/// The six owners of risk-relevant facts.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    MarketDataKernel,
    Oms,
    AccountJournal,
    Reconciliation,
    Ops,
    OperatorControl,
}

impl fmt::Display for ObservationSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl ObservationSource {
    pub const ALL: [Self; 6] = [
        Self::MarketDataKernel,
        Self::Oms,
        Self::AccountJournal,
        Self::Reconciliation,
        Self::Ops,
        Self::OperatorControl,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MarketDataKernel => "market_data_kernel",
            Self::Oms => "oms",
            Self::AccountJournal => "account_journal",
            Self::Reconciliation => "reconciliation",
            Self::Ops => "ops",
            Self::OperatorControl => "operator_control",
        }
    }
}

/// Process-local, unguessable capability issued only by [`crate::RiskAuthority`]
/// to a registered producer. It cannot be constructed by a caller.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct OwnerToken(u64);

impl OwnerToken {
    pub(crate) fn from_value(value: u64) -> Self {
        Self(value)
    }
}

/// Tokens for all six owners, issued together by `register_all_owners`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct OwnerTokens {
    pub market: OwnerToken,
    pub oms: OwnerToken,
    pub account_journal: OwnerToken,
    pub reconciliation: OwnerToken,
    pub ops: OwnerToken,
    pub operator: OwnerToken,
}

impl OwnerTokens {
    #[must_use]
    pub const fn token_for(&self, source: ObservationSource) -> OwnerToken {
        match source {
            ObservationSource::MarketDataKernel => self.market,
            ObservationSource::Oms => self.oms,
            ObservationSource::AccountJournal => self.account_journal,
            ObservationSource::Reconciliation => self.reconciliation,
            ObservationSource::Ops => self.ops,
            ObservationSource::OperatorControl => self.operator,
        }
    }
}

/// Versioned envelope for one observed fact. The producer token binds the
/// envelope to a registered in-process owner; a direct caller cannot fabricate
/// one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Observation<T> {
    pub producer_token: OwnerToken,
    pub source: ObservationSource,
    pub schema_version: u32,
    pub observed_at: LogicalNanos,
    pub received_at: LogicalNanos,
    pub revision: u64,
    pub payload: T,
}

impl<T> Observation<T> {
    pub fn new(
        producer_token: OwnerToken,
        source: ObservationSource,
        observed_at: LogicalNanos,
        received_at: LogicalNanos,
        revision: u64,
        payload: T,
    ) -> Self {
        Self {
            producer_token,
            source,
            schema_version: OBSERVATION_SCHEMA_VERSION,
            observed_at,
            received_at,
            revision,
            payload,
        }
    }

    #[must_use]
    pub fn age_ns(&self, now: LogicalNanos) -> u64 {
        now.get().saturating_sub(self.observed_at.get())
    }
}

/// Sequence evidence carried by an observation, used to detect a gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SequenceGap {
    pub expected: Option<u64>,
    pub observed: u64,
}

/// One observed market fact for a single instrument and venue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeedObservation {
    /// The displayed quote as observed. `None` means no quote was received.
    pub quote: Option<QuoteSnapshot>,
    /// Logical time at which the observation was received.
    pub received_at: LogicalNanos,
    /// Gap evidence from continuity tracking, if a gap was detected.
    pub gap: Option<SequenceGap>,
    /// Book completeness reported by the decoder (a raw observation
    /// attribute, not a health judgement).
    pub book_complete: bool,
}

impl FeedObservation {
    /// Derive feed health from observation facts and the authority's clock.
    pub fn health(&self, now: LogicalNanos, max_quote_age_ns: u64) -> FeedHealth {
        let Some(quote) = self.quote.as_ref() else {
            return FeedHealth::NoQuote;
        };
        let (Some(bid), Some(ask)) = (quote.bid_price.as_ref(), quote.ask_price.as_ref()) else {
            return FeedHealth::OneSided;
        };
        if bid.get().0 > ask.get().0 {
            return FeedHealth::Crossed;
        }
        if let Some(gap) = self.gap {
            return FeedHealth::Gapped {
                expected_sequence: gap.expected,
                observed_sequence: gap.observed,
            };
        }
        if !self.book_complete {
            return FeedHealth::Incomplete;
        }
        let age_ns = now.get().saturating_sub(self.received_at.get());
        if age_ns > max_quote_age_ns {
            return FeedHealth::Stale { age_ns };
        }
        FeedHealth::Healthy { age_ns }
    }
}

/// Market-data kernel owner: raw quote plus session and continuity evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MarketObservation {
    pub feed: FeedObservation,
    pub session_open: bool,
    pub kernel_schema_version: u32,
    pub source_sequence: Option<u64>,
    pub connection_epoch: Option<u64>,
    /// Synthetic displayed depth and ADV. `None` fails the liquidity check
    /// closed once a participation cap is configured.
    pub depth: Option<LiquidityDepth>,
    /// Quote-derived exact marks for stress projection. Missing marks fail
    /// the stress check closed.
    pub marks_paise: BTreeMap<InstrumentId, Paise>,
    /// Synthetic per-instrument Greeks. Missing greeks fail closed for a
    /// derivative-flagged instrument.
    pub greeks: Option<Greeks>,
    /// Synthetic short-borrow evidence. Missing borrow fails a short closed
    /// once a borrow cap is configured.
    pub borrow: Option<BorrowAvailability>,
    /// True only for paper/synthetic wiring. Never a health verdict.
    pub synthetic: bool,
}

/// OMS owner: digest of the accepted fill set and poison/unresolved evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OmsObservation {
    pub fills_digest: String,
    pub unresolved_count: u32,
    pub last_journal_digest: String,
    pub is_poisoned: bool,
}

/// Account-journal owner: verified chain head and ledger state digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AccountJournalObservation {
    pub head_hash_hex: String,
    pub record_count: u64,
    pub ledger_state_digest: String,
    pub opening_balance_evidence_ref: Option<String>,
}

/// Independent reconciliation owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReconciliationObservation {
    pub report: ReconciliationReport,
    pub replay_digest_matched: bool,
}

/// Ops owner: typed measurements only, never a `safe_for_new_risk` verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpsObservation {
    pub feed_age_ns: u64,
    pub sequence_gaps: u32,
    pub journal_append_latency_ns: u64,
    pub journal_queue_depth: u64,
    pub disk_available_bytes: u64,
    pub unknown_orders: u32,
    pub reconciliation_differences: u32,
}

/// Operator-control owner: authored kill-switch state and release evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperatorObservation {
    pub kill_switch_engaged: bool,
    pub actor_id: String,
    pub reason: String,
    /// Required when releasing an engaged switch; absent when engaging.
    pub release_evidence_ref: Option<String>,
}

/// The complete observation bundle. Every field is required: an owner that
/// supplies no observation denies new risk through `MissingObservation`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Observations {
    pub market: Option<Observation<MarketObservation>>,
    pub oms: Option<Observation<OmsObservation>>,
    pub account_journal: Option<Observation<AccountJournalObservation>>,
    pub reconciliation: Option<Observation<ReconciliationObservation>>,
    pub ops: Option<Observation<OpsObservation>>,
    pub operator: Option<Observation<OperatorObservation>>,
}

/// Freshness and operational limits, owned by the authority. Each source has
/// its own maximum age; operational thresholds are expressed as typed numbers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ObservationPolicy {
    pub max_market_age_ns: u64,
    pub max_oms_age_ns: u64,
    pub max_account_journal_age_ns: u64,
    pub max_reconciliation_age_ns: u64,
    pub max_ops_age_ns: u64,
    pub max_operator_age_ns: u64,
    pub max_sequence_gaps: u32,
    pub max_journal_append_latency_ns: u64,
    pub max_journal_queue_depth: u64,
    pub min_disk_available_bytes: u64,
    pub max_unknown_orders: u32,
    pub max_reconciliation_differences: u32,
}

impl Default for ObservationPolicy {
    fn default() -> Self {
        Self {
            max_market_age_ns: 1_000_000_000,
            max_oms_age_ns: 1_000_000_000,
            max_account_journal_age_ns: 1_000_000_000,
            max_reconciliation_age_ns: 1_000_000_000,
            max_ops_age_ns: 1_000_000_000,
            max_operator_age_ns: 1_000_000_000,
            max_sequence_gaps: 0,
            max_journal_append_latency_ns: 10_000_000,
            max_journal_queue_depth: 100,
            min_disk_available_bytes: 1_000_000,
            max_unknown_orders: 0,
            max_reconciliation_differences: 0,
        }
    }
}

impl ObservationPolicy {
    #[must_use]
    pub const fn max_age_ns(&self, source: ObservationSource) -> u64 {
        match source {
            ObservationSource::MarketDataKernel => self.max_market_age_ns,
            ObservationSource::Oms => self.max_oms_age_ns,
            ObservationSource::AccountJournal => self.max_account_journal_age_ns,
            ObservationSource::Reconciliation => self.max_reconciliation_age_ns,
            ObservationSource::Ops => self.max_ops_age_ns,
            ObservationSource::OperatorControl => self.max_operator_age_ns,
        }
    }
}
