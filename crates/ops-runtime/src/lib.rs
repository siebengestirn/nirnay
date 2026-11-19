#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.7 operating-environment primitives for the paper/shadow runtime.
//!
//! This crate validates a filesystem layout that keeps evidence out of the
//! source tree, produces digest-verified backups with verified fresh-path
//! restore, and evaluates operational health into a gate on new risk. It does
//! not deploy anything, contact a broker or authorize live trading.

pub mod backup;
pub mod error;
pub mod health;
pub mod layout;

pub use backup::{
    BackupEntry, //
    BackupManifest,
    create_backup,
    restore_backup,
    verify_backup,
};
pub use error::OpsError;
pub use health::{
    HealthSnapshot, //
    HealthVerdict,
    RuntimeThresholds,
};
pub use layout::{
    RuntimeDir, //
    RuntimeLayout,
};
