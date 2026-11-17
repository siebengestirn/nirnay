//! Source provenance for reference files.
//!
//! Every source file is described once, before parsing:

use india_domain::{Segment, Venue};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::util::sha256_hex;

/// Schema version of the provenance manifest itself.
pub const PROVENANCE_SCHEMA_VERSION: u32 = 1;

/// Provenance of one raw source file.
///
/// Provenance is recorded at ingestion, before any parsing, and is carried on
/// the resulting snapshot. It is never derived from parsed content, so a
/// parse disagreement cannot rewrite what was actually downloaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProvenance {
    /// Version of this manifest format.
    pub schema_version: u32,
    /// Stable source identity, e.g. `nse-cm-master`, `bse-security-master`.
    pub source_id: String,
    /// Venue the file was issued for (exchange, never a broker).
    pub venue: Venue,
    /// Market segment the file covers.
    pub segment: Segment,
    /// Wall-clock time the bytes were retrieved (knowledge time).
    pub retrieved_at: Timestamp,
    /// Lowercase hex SHA-256 of the exact raw bytes.
    pub sha256_hex: String,
    /// Source file layout version, e.g. `nse-cm-master-v1`.
    pub source_format_version: String,
    /// Number of non-header records observed in the raw bytes.
    pub record_count: usize,
    /// Who/what declared this retrieval, e.g. `manual-download`,
    /// `automated-fetch`. Free-form for audit; never empty.
    pub declared_by: String,
}

impl SourceProvenance {
    /// Validate manifest structure. The hash is recomputed by callers that
    /// still hold the raw bytes; this check covers format only.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            self.schema_version == PROVENANCE_SCHEMA_VERSION,
            "provenance schema_version {} is unsupported",
            self.schema_version
        );
        anyhow::ensure!(
            !self.source_id.trim().is_empty(),
            "provenance source_id must not be empty"
        );
        anyhow::ensure!(
            !self.source_format_version.trim().is_empty(),
            "provenance source_format_version must not be empty"
        );
        anyhow::ensure!(
            !self.declared_by.trim().is_empty(),
            "provenance declared_by must not be empty"
        );
        anyhow::ensure!(
            self.sha256_hex.len() == 64 && self.sha256_hex.chars().all(|c| c.is_ascii_hexdigit()),
            "provenance sha256_hex must be 64 hex digits"
        );
        Ok(())
    }
}

/// Build provenance for `raw_bytes` without parsing them.
///
/// `record_count` is a syntactic count of non-empty lines after the header;
/// semantic validation happens in the venue parsers.
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
    SourceProvenance {
        schema_version: PROVENANCE_SCHEMA_VERSION,
        source_id: source_id.to_owned(),
        venue,
        segment,
        retrieved_at,
        sha256_hex: sha256_hex(raw_bytes),
        source_format_version: source_format_version.to_owned(),
        record_count,
        declared_by: declared_by.to_owned(),
    }
}
