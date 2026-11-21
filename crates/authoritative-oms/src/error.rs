//! OMS failures. Every error leaves the transactional store unchanged.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OmsError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("another process holds the OMS journal writer lock")]
    WriterLocked,
    #[error("OMS writer is poisoned after an uncertain commit; recover before writing")]
    WriterPoisoned,
    #[error("client order id `{0}` already exists with different terms")]
    ConflictingClientOrder(String),
    #[error("unknown client order id `{0}`")]
    UnknownOrder(String),
    #[error("unknown submission attempt `{0}`")]
    UnknownAttempt(i64),
    #[error("illegal order transition {from:?} -> {to:?}")]
    IllegalOrderTransition {
        from: super::lifecycle::OmsOrderState,
        to: super::lifecycle::OmsOrderState,
    },
    #[error("illegal outbox transition {from:?} -> {to:?}")]
    IllegalOutboxTransition {
        from: super::lifecycle::OutboxState,
        to: super::lifecycle::OutboxState,
    },
    #[error("broker event `{0}` was already recorded")]
    DuplicateBrokerEvent(String),
    #[error("execution `{execution_id}` was delivered again with different content")]
    FillContentConflict { execution_id: String },
    #[error("fill quantity exceeds the remaining order quantity")]
    OverFill,
    #[error("an ambiguous submission must be reconciled before retry; found `{0}`")]
    UnresolvedUnknown(String),
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),
    #[error("corrupt stored record: {0}")]
    CorruptRecord(String),
}
