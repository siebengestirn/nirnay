//! Explicit source outcomes and snapshot coverage.
//!
//! A source record is never silently skipped. Every row is either accepted as
//! an observation, explicitly excluded by a versioned source rule, or rejected
//! as malformed or contradictory.

use std::collections::BTreeSet;

use trading_core::Venue;

/// What a source snapshot claims to cover. Completeness is declared by the
/// source contract and is never inferred from the rows that happened to arrive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotScope {
    Partial,
    Full { covered_venues: BTreeSet<Venue> },
}

/// Why a row was excluded by a declared source rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclusion {
    pub line: usize,
    pub rule: String,
}

/// Why a row was rejected as malformed or contradictory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub line: usize,
    pub reason: String,
}

/// One row's disposition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowOutcome<T> {
    Accepted(T),
    Excluded(Exclusion),
    Rejected(Rejection),
}

/// A parsed source with explicit coverage and per-row outcomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceBatch<T> {
    pub source_id: String,
    pub source_version: String,
    pub scope: SnapshotScope,
    pub accepted: Vec<T>,
    pub excluded: Vec<Exclusion>,
    pub rejected: Vec<Rejection>,
    pub physical_rows: usize,
}

impl<T> SourceBatch<T> {
    /// Assert that every physical data row has exactly one disposition.
    pub fn is_complete(&self) -> bool {
        self.accepted.len() + self.excluded.len() + self.rejected.len() == self.physical_rows
    }
}
