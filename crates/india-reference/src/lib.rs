#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Bounded India exchange reference-data ingestion.
//!
//! This crate uses `india-domain` as the canonical model. It owns source
//! parsing, explicit row outcomes, opaque instrument identity allocation and
//! reconciliation. It does not duplicate `Instrument`, `Listing`, `Isin`,
//! `TickAssignment` or `SourceProvenance`.

pub mod archive;
pub mod csv;
pub mod identity;
pub mod observation;
pub mod source;
pub mod store;

pub use archive::{ArchiveError, RecoveryReport, SourceArchive, SourceRecord, recover};
pub use csv::{CsvDocument, CsvError, CsvRow, Limits};
pub use identity::{IdentityError, IdentityRegistry};
pub use observation::{Exclusion, Rejection, RowOutcome, SnapshotScope, SourceBatch};
#[cfg(debug_assertions)]
pub use store::set_fail_point;
pub use store::{Publication, ReferenceStore, StoreError};
