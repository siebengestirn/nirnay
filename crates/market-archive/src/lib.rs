#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Deterministic primitives for the raw market-data archive.
//!
//! This crate does not connect to a provider and does not claim data rights.
//! Provider adapters must pass original payload bytes and a reference to the
//! account's applicable entitlement. A raw frame is not a normalized event.

mod continuity;
mod error;
mod frame;
mod journal;
mod manifest;

pub use continuity::{ContinuityTracker, Observation, StreamQuality};
pub use error::ArchiveError;
pub use frame::RawFrame;
pub use journal::{JournalScan, JournalSummary, JournalWriter, scan_journal, verify_journal};
pub use manifest::{DatasetManifest, ManifestDraft, StreamEntry};
