use thiserror::Error;

/// Fail-closed domain error. Variants are stable enough to be matched by the
/// engine and risk layers; messages are diagnostics, not control flow.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("identifier must be non-zero")]
    ZeroIdentifier,
    #[error("price must be greater than zero")]
    ZeroPrice,
    #[error("quantity must be non-negative")]
    NegativeQuantity,
    #[error("quantity must be greater than zero")]
    ZeroQuantity,
    #[error("arithmetic overflow")]
    Overflow,
    #[error("quantity underflow")]
    Underflow,
    #[error("identifier is not canonical: {0}")]
    InvalidIdentifier(String),
    #[error("instrument metadata is invalid: {0}")]
    InvalidInstrument(String),
    #[error("event time is invalid: {0}")]
    InvalidEventTime(String),
    #[error("instrument identity {0} is duplicated in the catalog")]
    DuplicateInstrument(String),
    #[error("conversion would lose information: {0}")]
    NonCanonicalConversion(String),
}
