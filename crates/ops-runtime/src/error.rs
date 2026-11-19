//! Operating-environment failures.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpsError {
    #[error("runtime root must be outside the source tree")]
    RootInsideSourceTree,
    #[error("runtime path escaped its root")]
    PathEscape,
    #[error("runtime layout is incomplete: missing `{0}`")]
    IncompleteLayout(String),
    #[error("filesystem error: {0}")]
    Io(String),
    #[error("backup manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("backup entry digest mismatch: `{0}`")]
    DigestMismatch(String),
    #[error("backup entry missing: `{0}`")]
    MissingEntry(String),
    #[error("restore target already exists: `{0}`")]
    TargetExists(String),
}
