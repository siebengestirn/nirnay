#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Deterministic primitives for the raw market-data archive.
//!
//! This crate does not connect to a provider and does not claim data rights.
//! Provider adapters must pass original payload bytes and a reference to the
//! account's applicable entitlement. A raw frame is not a normalized event.

mod capture;
mod continuity;
mod error;
mod frame;
mod journal;
mod manifest;

pub use capture::{
    CaptureManifest, //
    SessionVerification,
    verified_journal_path,
    verify_capture_session,
};
pub use continuity::{ContinuityTracker, Observation, StreamQuality};
pub use error::ArchiveError;
pub use frame::{
    MAX_PAYLOAD_BYTES, //
    RawFrame,
};
pub use journal::{
    JournalReader, //
    JournalScan,
    JournalSummary,
    JournalWriter,
    scan_journal,
    verify_journal,
};
pub use manifest::{DatasetManifest, ManifestDraft, StreamEntry};
