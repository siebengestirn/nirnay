//! Authority failures. Every rejection is fail-closed: no reservation is made.

use thiserror::Error;
use trading_core::RiskAssessment;

use crate::observation::ObservationSource;

/// Which measured operational limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OpsThresholdKind {
    #[error("sequence gaps")]
    SequenceGaps,
    #[error("journal append latency")]
    JournalAppendLatency,
    #[error("journal queue depth")]
    JournalQueueDepth,
    #[error("disk capacity")]
    DiskCapacity,
    #[error("unknown orders")]
    UnknownOrders,
    #[error("reconciliation differences")]
    ReconciliationDifferences,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuthorityError {
    #[error("invalid order intent: {0}")]
    InvalidIntent(String),
    #[error("stale account snapshot: expected revision {expected}, current {current}")]
    StaleSnapshot { expected: u64, current: u64 },
    #[error("intent id `{0}` was reused with different terms")]
    ConflictingIntent(String),
    #[error("intent id `{0}` was already decided and rejected")]
    PreviouslyRejected(String),
    #[error("risk assessment rejected the intent: {0:?}")]
    Rejected(RiskAssessment),
    #[error("order notional overflowed i64 paise")]
    NotionalOverflow,
    #[error("unknown intent id `{0}`")]
    UnknownIntent(String),
    #[error("authority state mutex was poisoned")]
    StatePoisoned,
    #[error("observation receipt time is in the future relative to the authority clock")]
    FutureObservation,
    #[error("accounting overflow in `{0}`")]
    AccountingOverflow(String),
    #[error("invalid reservation: {0}")]
    InvalidReservation(String),
    #[error("intent id `{0}` was already consumed by a fill or release")]
    AlreadyConsumed(String),
    #[error("observation from `{observation_source}` is missing")]
    MissingObservation {
        observation_source: ObservationSource,
    },
    #[error(
        "observation from `{observation_source}` is stale: age {age_ns} ns exceeds {max_age_ns} ns"
    )]
    StaleObservation {
        observation_source: ObservationSource,
        age_ns: u64,
        max_age_ns: u64,
    },
    #[error("observation from `{observation_source}` has unsupported schema version {version}")]
    UnsupportedObservationVersion {
        observation_source: ObservationSource,
        version: u32,
    },
    #[error("observation from `{observation_source}` has invalid clock ordering: {detail}")]
    InvalidObservationTime {
        observation_source: ObservationSource,
        detail: String,
    },
    #[error("observation from `{observation_source}` declares the wrong source")]
    ObservationSourceMismatch {
        observation_source: ObservationSource,
    },
    #[error("digest evidence from `{observation_source}` failed verification")]
    ObservationDigestMismatch {
        observation_source: ObservationSource,
    },
    #[error("producer `{observation_source}` is not registered with this authority")]
    UnregisteredProducer {
        observation_source: ObservationSource,
    },
    #[error("observation token does not belong to producer `{observation_source}`")]
    ProducerTokenMismatch {
        observation_source: ObservationSource,
    },
    #[error("authenticated {action} requires a non-empty evidenced reason")]
    MissingEvidence { action: String },
    #[error("reconciliation report `{report_id}` blocks new risk: {differences:?}")]
    ReconciliationMismatch {
        report_id: String,
        differences: Vec<String>,
    },
    #[error("kill switch engaged by `{actor_id}`: {reason}")]
    KillSwitchEngaged { reason: String, actor_id: String },
    #[error("operational limit exceeded: {kind} observed {observed}, limit {limit}")]
    OpsThresholdExceeded {
        kind: OpsThresholdKind,
        observed: u64,
        limit: u64,
    },
    #[error("kill-switch control store failure: {0}")]
    ControlFailure(String),
}
