//! Kernel projection failures. Every rejected session publishes nothing.

use thiserror::Error;

use market_event::ContractError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KernelError {
    #[error("session schema version {0} is unsupported")]
    UnsupportedSessionSchema(u32),
    #[error("session has no source observations")]
    EmptySession,
    #[error("observation frame bytes must not be empty")]
    EmptyFrame,
    #[error("source id must not be empty")]
    EmptySource,
    #[error("provider instrument id must not be empty")]
    EmptyProviderInstrument,
    #[error("receive time must be positive")]
    InvalidReceiveTime,
    #[error("reference revision is required for every observation")]
    MissingReferenceRevision,
    #[error("session digest mismatch: declared {declared} computed {computed}")]
    SessionDigestMismatch { declared: String, computed: String },
    #[error("output path already exists; publication is exclusive")]
    OutputExists,
    #[error("duplicate observation identity within a verified session")]
    DuplicateObservation,
    #[error("a scoped continuity observation has no source sequence")]
    MissingSequence,
    #[error("kernel identity contract error: {0}")]
    Identity(#[from] ContractError),
}
