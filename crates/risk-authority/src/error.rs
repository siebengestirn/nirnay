//! Authority failures. Every rejection is fail-closed: no reservation is made.

use thiserror::Error;

use trading_core::RiskAssessment;

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
}
