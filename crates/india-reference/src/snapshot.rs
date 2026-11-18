//! Versioned reference snapshot payload and its domain rules.
//!
//! This module is pure data and validation: no filesystem and no database IO.
//! It is the foundation of the promoted P1-04 reference-store contract. The
//! encoded envelope bytes are authoritative for `payload_sha256`; the semantic
//! hash covers only the fields that affect a decision, never capture metadata.

use std::collections::{
    BTreeSet, //
};

use india_domain::{
    Instrument, //
    InstrumentId,
    ListingStatus,
    SettlementRegime,
    SourceProvenance,
    TickAssignment,
    TradingCalendar,
    Venue,
    is_sha256_hex,
};
use jiff::{
    Timestamp, //
    civil::Date,
};
use serde::{
    Deserialize, //
    Serialize,
};
use serde_json::json;
use sha2::{
    Digest, //
    Sha256,
};
use thiserror::Error;

/// Version of the encoded snapshot envelope. Changing the encoding requires a
/// new version; old rows are never silently reinterpreted.
pub const PAYLOAD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SnapshotError {
    #[error("snapshot validation failed: {0}")]
    Validation(String),
    #[error("unsupported snapshot payload schema version {actual}; expected {expected}")]
    UnsupportedVersion { actual: u32, expected: u32 },
    #[error("snapshot payload encoding failed: {0}")]
    Encode(String),
    #[error("snapshot payload decoding failed: {0}")]
    Decode(String),
}

/// One point-in-time listing state. Corrections append a new record under a
/// new `recorded_at`; history is never rewritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingStateRecord {
    pub instrument_id: InstrumentId,
    pub venue: Venue,
    pub symbol: String,
    pub token: Option<String>,
    pub status: ListingStatus,
    pub settlement_regime: SettlementRegime,
    pub z_category: bool,
    pub settlement_group: Option<String>,
    pub market_lot: i64,
    pub tick_assignment: TickAssignment,
    pub valid_from: Date,
    pub valid_to: Option<Date>,
    pub recorded_at: Timestamp,
    pub provenance: SourceProvenance,
}

/// The declared difference between this snapshot and its predecessor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reconciliation {
    pub added: Vec<InstrumentId>,
    pub removed: Vec<InstrumentId>,
    pub changed: Vec<InstrumentId>,
}

/// One complete reference snapshot. `instruments` are ordered by canonical
/// identity and `listing_states` by observation key; both orders are part of
/// the encoded payload and therefore of `payload_sha256`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSnapshot {
    pub valid_at: Date,
    pub recorded_at: Timestamp,
    pub provenance: SourceProvenance,
    pub instruments: Vec<Instrument>,
    pub calendar: Option<TradingCalendar>,
    pub reconciliation: Option<Reconciliation>,
    pub listing_states: Vec<ListingStateRecord>,
}

/// The bytes stored in `reference_revision.payload` are exactly the encoding
/// of this envelope, so verification never reserializes decoded data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadEnvelope {
    pub schema_version: u32,
    pub snapshot: ReferenceSnapshot,
}

fn invalid(message: impl Into<String>) -> SnapshotError {
    SnapshotError::Validation(message.into())
}

impl ReferenceSnapshot {
    /// Enforce the contract's structural invariants. Every rejection names the
    /// failed invariant so a caller cannot mistake a parse error for a rule.
    pub fn validate(&self) -> Result<(), SnapshotError> {
        self.provenance
            .validate()
            .map_err(|error| invalid(format!("top-level provenance: {error}")))?;
        if self.provenance.venue != Venue::Nse && self.provenance.venue != Venue::Bse {
            return Err(invalid("provenance venue must be NSE or BSE"));
        }
        if self.provenance.source_id.trim().is_empty()
            || self.provenance.source_format_version.trim().is_empty()
        {
            return Err(invalid("provenance identity must not be empty"));
        }

        let mut previous: Option<&InstrumentId> = None;
        let mut symbols = BTreeSet::new();
        for instrument in &self.instruments {
            instrument.validate().map_err(|error| {
                invalid(format!("instrument {}: {error}", instrument.instrument_id))
            })?;
            if previous.is_some_and(|id| id >= &instrument.instrument_id) {
                return Err(invalid("instruments must be strictly ascending by id"));
            }
            previous = Some(&instrument.instrument_id);
            for listing in &instrument.listings {
                let key = (listing.venue, listing.venue_symbol.clone());
                if !symbols.insert(key) {
                    return Err(invalid("duplicate (venue, symbol) across instruments"));
                }
                if let Some(end) = listing.effective_to
                    && end < listing.effective_from
                {
                    return Err(invalid("listing interval ends before it starts"));
                }
            }
        }

        let mut previous_state: Option<(&InstrumentId, Venue, Date, Timestamp)> = None;
        for state in &self.listing_states {
            state
                .provenance
                .validate()
                .map_err(|error| invalid(format!("listing-state provenance: {error}")))?;
            if let Some(end) = state.valid_to
                && end < state.valid_from
            {
                return Err(invalid("listing state interval ends before it starts"));
            }
            if state.market_lot <= 0 {
                return Err(invalid("listing state market lot must be positive"));
            }
            if state.tick_assignment.tick_paise <= 0 {
                return Err(invalid("listing state tick size must be positive"));
            }
            validate_tick_lineage(&state.tick_assignment)?;
            let key = (
                &state.instrument_id,
                state.venue,
                state.valid_from,
                state.recorded_at,
            );
            if previous_state.is_some_and(|old| old >= key) {
                return Err(invalid(
                    "listing states must be strictly ordered by observation key",
                ));
            }
            previous_state = Some(key);
        }

        if let Some(calendar) = &self.calendar {
            let mut previous_day = None;
            for day in &calendar.days {
                if previous_day.is_some_and(|date| date >= day.date) {
                    return Err(invalid("calendar days must be strictly ascending"));
                }
                previous_day = Some(day.date);
            }
        }
        Ok(())
    }

    /// Semantic hash: exactly the four decision-relevant fields. Capture
    /// provenance, reconciliation and `recorded_at` are deliberately excluded.
    pub fn snapshot_hash(&self) -> Result<String, SnapshotError> {
        let value = json!({
            "valid_at": self.valid_at.to_string(),
            "instruments": self.instruments,
            "calendar": self.calendar,
            "listing_states": self.listing_states,
        });
        let bytes =
            serde_json::to_vec(&value).map_err(|error| SnapshotError::Encode(error.to_string()))?;
        Ok(hex_digest(&bytes))
    }
}

fn validate_tick_lineage(tick: &TickAssignment) -> Result<(), SnapshotError> {
    if tick.schedule_revision.trim().is_empty() {
        return Err(invalid("tick schedule_revision must not be empty"));
    }
    if let Some(source) = &tick.reference_source
        && source.trim().is_empty()
    {
        return Err(invalid("tick reference_source must not be empty"));
    }
    if let Some(hash) = &tick.reference_sha256_hex
        && !is_sha256_hex(hash)
    {
        return Err(invalid("tick reference_sha256_hex is not a sha256 value"));
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut text, format_args!("{byte:02x}"));
    }
    text
}

/// Encode the authoritative payload bytes. The caller stores these bytes
/// verbatim and checksums them; decoding never reconstructs them.
pub fn encode_payload(snapshot: &ReferenceSnapshot) -> Result<Vec<u8>, SnapshotError> {
    snapshot.validate()?;
    let envelope = PayloadEnvelope {
        schema_version: PAYLOAD_SCHEMA_VERSION,
        snapshot: snapshot.clone(),
    };
    serde_json::to_vec(&envelope).map_err(|error| SnapshotError::Encode(error.to_string()))
}

/// Decode and validate stored bytes. An unsupported version or a validation
/// failure is refused; no field is defaulted or inferred.
pub fn decode_payload(bytes: &[u8]) -> Result<ReferenceSnapshot, SnapshotError> {
    let envelope: PayloadEnvelope =
        serde_json::from_slice(bytes).map_err(|error| SnapshotError::Decode(error.to_string()))?;
    if envelope.schema_version != PAYLOAD_SCHEMA_VERSION {
        return Err(SnapshotError::UnsupportedVersion {
            actual: envelope.schema_version,
            expected: PAYLOAD_SCHEMA_VERSION,
        });
    }
    envelope.snapshot.validate()?;
    Ok(envelope.snapshot)
}
