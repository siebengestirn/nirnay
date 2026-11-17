//! Immutable provenance for exchange/broker reference-data source artifacts.
//!
//! Hash the exact downloaded bytes before parsing. Parsed records inherit this
//! provenance; a parser correction must never rewrite the source identity.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Segment, Venue};

pub const PROVENANCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProvenance {
    pub schema_version: u32,
    pub source_id: String,
    pub venue: Venue,
    pub segment: Segment,
    pub retrieved_at: Timestamp,
    /// Lowercase SHA-256 of the exact, unmodified source bytes.
    pub sha256_hex: String,
    pub source_format_version: String,
    pub record_count: usize,
    pub declared_by: String,
}

impl SourceProvenance {
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            self.schema_version == PROVENANCE_SCHEMA_VERSION,
            "unsupported provenance schema version"
        );
        anyhow::ensure!(
            !self.source_id.trim().is_empty(),
            "source_id must not be empty"
        );
        anyhow::ensure!(
            !self.source_format_version.trim().is_empty(),
            "source_format_version must not be empty"
        );
        anyhow::ensure!(
            !self.declared_by.trim().is_empty(),
            "declared_by must not be empty"
        );
        anyhow::ensure!(
            self.sha256_hex.len() == 64
                && self
                    .sha256_hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "sha256_hex must be 64 lowercase hexadecimal characters"
        );
        Ok(())
    }
}

/// Capture provenance from exact raw bytes, before decoding or normalisation.
#[must_use]
pub fn provenance_for_source(
    source_id: &str,
    venue: Venue,
    segment: Segment,
    retrieved_at: Timestamp,
    source_format_version: &str,
    declared_by: &str,
    raw_bytes: &[u8],
) -> SourceProvenance {
    let text = String::from_utf8_lossy(raw_bytes);
    let record_count = text
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .count();
    let digest = Sha256::digest(raw_bytes);
    SourceProvenance {
        schema_version: PROVENANCE_SCHEMA_VERSION,
        source_id: source_id.to_owned(),
        venue,
        segment,
        retrieved_at,
        sha256_hex: format!("{digest:x}"),
        source_format_version: source_format_version.to_owned(),
        record_count,
        declared_by: declared_by.to_owned(),
    }
}
