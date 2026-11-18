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
pub mod snapshot;
pub mod source;
pub mod store;

pub use archive::{
    ArchiveError, //
    DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS,
    RecoveryReport,
    SourceArchive,
    SourceRecord,
    recover,
};
pub use csv::{CsvDocument, CsvError, CsvRow, Limits};
pub use identity::{IdentityError, IdentityRegistry};
pub use observation::{Exclusion, Rejection, RowOutcome, SnapshotScope, SourceBatch};
pub use snapshot::{
    ListingStateRecord, //
    PayloadEnvelope,
    Reconciliation,
    ReferenceSnapshot,
    SnapshotError,
    collect_lineage_hashes,
    decode_payload,
    encode_payload,
    latest_states_at,
};
pub use store::{
    BACKUP_FORMAT_VERSION, //
    BackupManifest,
    BackupResult,
    DurableReferenceStore,
    Publication,
    PublishOutcome,
    ReferenceStore,
    RestoreOutcome,
    RestoreVerification,
    STORE_SCHEMA_VERSION,
    StoreError,
    StoredRevision,
    VerificationReport,
    backup_verified,
    restore_verified,
    restore_verified_store,
};
#[cfg(debug_assertions)]
pub use store::{durable_connection_for_test, set_fail_point};
