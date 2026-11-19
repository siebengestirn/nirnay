//! Fail-closed contract errors.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContractError {
    #[error("identity preimage must not be empty")]
    EmptyIdentityPreimage,
    #[error("invalid content id `{0}`")]
    InvalidContentId(String),
    #[error("exchange_event origin requires exchange-native evidence")]
    ExchangeEventWithoutEvidence,
    #[error("business event id requires a domain discriminator")]
    MissingDomainDiscriminator,
    #[error("business event id requires a source observation back-reference")]
    MissingSourceObservation,
    #[error("unsupported kernel event schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("connection epoch and receive sequence must advance together")]
    InvalidReceiveFence,
    #[error("reference revision is required when instrument resolution is asserted")]
    MissingReferenceRevision,
    #[error("instrument is unresolved")]
    UnresolvedInstrument,
    #[error("source time and receive time must be distinct domains")]
    TimeDomainCollision,
    #[error("recovered state requires a snapshot or reset evidence token")]
    RecoveryWithoutEvidence,
    #[error("gap state cannot transition directly to clean")]
    GapToClean,
}
