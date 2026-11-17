use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ArchiveError {
    #[error("required field `{0}` is empty")]
    EmptyField(&'static str),
    #[error("payload hash does not match raw bytes")]
    PayloadHashMismatch,
    #[error("payload base64 is invalid")]
    PayloadEncoding,
    #[error("malformed journal record: {0}")]
    MalformedRecord(String),
    #[error("journal has an incomplete final record (torn tail)")]
    TornTail,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("commit outcome unknown after {context}; the record may or may not be durable")]
    CommitOutcomeUnknown {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("journal writer is poisoned after an uncertain commit; drop and reopen to recover")]
    Poisoned,
    #[error("manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("manifest content hash does not match dataset_id")]
    DatasetIdMismatch,
    #[error("dataset is not research-grade: {0}")]
    NotResearchGrade(String),
}
