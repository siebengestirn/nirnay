//! Strategy runtime errors. Fail closed before any intent can be emitted.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeArtifactError {
    #[error("strategy artifact could not be read")]
    UnreadableArtifact,
    #[error("strategy artifact is not a valid v1 document")]
    InvalidArtifact,
    #[error("strategy artifact is incomplete")]
    IncompleteArtifact,
    #[error("strategy artifact has no experiment lineage")]
    MissingExperimentLineage,
    #[error("strategy artifact validation fields are incomplete")]
    IncompleteValidation,
    #[error("unsupported strategy rule: {0}")]
    UnsupportedRule(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StrategyError {
    #[error("invalid rational strategy configuration")]
    InvalidConfiguration,
    #[error("event instrument is outside the artifact universe")]
    InstrumentOutsideUniverse,
    #[error("event has no usable midpoint")]
    NoMidpoint,
    #[error("strategy arithmetic overflowed")]
    Overflow,
    #[error("observation sequence is not strictly increasing")]
    NonMonotonicObservation,
    #[error("frozen artifact failed runtime gating: {0}")]
    ArtifactRejected(String),
}
