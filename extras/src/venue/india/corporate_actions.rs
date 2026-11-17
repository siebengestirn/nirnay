//! Standalone point-in-time corporate-action event ledger.
//!
//! [`india_domain::Instrument`] carries an embedded, ordered corporate-action
//! *history* — right for displaying what already happened. It cannot answer
//! "what information was available on date D?", because it has no knowledge time. This
//! ledger is the event-level counterpart:
//!
//! * every record is bitemporal (`effective_date` + `recorded_at`),
//! * every record carries the [`SourceProvenance`] of the announcement file,
//! * adjustment ratios are exact integers (numerator/denominator), never
//!   floats — the same rule `adjusted_price_paise` enforces downstream.
//!
//! A record announced after its effective date is accepted explicitly (with
//! its real `recorded_at`), never back-dated to look known earlier.

use std::collections::{BTreeSet, HashMap};

use india_domain::{CorporateActionKind, InstrumentId};
use jiff::{Timestamp, civil::Date};
use serde::{Deserialize, Serialize};

use super::provenance::SourceProvenance;

/// One bitemporal corporate-action event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorporateActionRecord {
    pub instrument_id: InstrumentId,
    pub kind: CorporateActionKind,
    /// Valid time: when the action takes effect (ex-date).
    pub effective_date: Date,
    /// Exact adjustment numerator.
    pub numerator: i64,
    /// Exact adjustment denominator.
    pub denominator: i64,
    /// Per-share amount in paise (dividend/rights); `None` when the action
    /// carries no amount.
    pub amount_paise: Option<i64>,
    /// Knowledge time: when this record was ingested.
    pub recorded_at: Timestamp,
    /// Raw announcement provenance.
    pub provenance: SourceProvenance,
}

impl CorporateActionRecord {
    /// Validate the record's financial invariants, including the
    /// knowledge-time ordering: a record cannot claim to have been recorded
    /// before its source was retrieved — that is backdating, and every
    /// historical query would silently trust it.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            self.numerator > 0 && self.denominator > 0,
            "corporate action for {} has a non-positive adjustment ratio",
            self.instrument_id
        );
        anyhow::ensure!(
            self.amount_paise.is_none_or(|amount| amount >= 0),
            "corporate action for {} has a negative amount",
            self.instrument_id
        );
        self.provenance.validate()?;
        anyhow::ensure!(
            self.recorded_at >= self.provenance.retrieved_at,
            "corporate action for {} is backdated: recorded_at precedes the source retrieval time",
            self.instrument_id
        );
        Ok(())
    }
}

/// Append-only corporate-action event ledger.
///
/// Records are kept sorted by `(instrument, effective_date, kind)`. Exact
/// duplicates and same-slot contradictions (two records for one instrument,
/// kind and date) are rejected: which copy is right cannot be guessed.
#[derive(Debug, Clone, Default)]
pub struct CorporateActionLedger {
    records: Vec<CorporateActionRecord>,
}

impl CorporateActionLedger {
    /// Append one validated record; fails on ambiguity.
    pub fn append(&mut self, record: CorporateActionRecord) -> Result<(), anyhow::Error> {
        record.validate()?;
        let kind = format!("{:?}", record.kind);
        anyhow::ensure!(
            !self.records.iter().any(|existing| {
                (
                    existing.instrument_id.clone(),
                    existing.effective_date,
                    format!("{:?}", existing.kind),
                ) == (
                    record.instrument_id.clone(),
                    record.effective_date,
                    kind.clone(),
                )
            }),
            "corporate action for {} already recorded for {} on {}",
            record.instrument_id,
            kind,
            record.effective_date
        );
        self.records.push(record);
        self.records.sort_by(|a, b| {
            a.instrument_id
                .cmp(&b.instrument_id)
                .then(a.effective_date.cmp(&b.effective_date))
                .then(format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)))
        });
        Ok(())
    }

    /// Every record, in canonical order.
    #[must_use]
    pub fn records(&self) -> &[CorporateActionRecord] {
        &self.records
    }

    /// Records effective on or before `date` that were known by `knowledge_at`.
    #[must_use]
    pub fn known_by(&self, date: Date, knowledge_at: Timestamp) -> Vec<&CorporateActionRecord> {
        self.records
            .iter()
            .filter(|record| record.effective_date <= date && record.recorded_at <= knowledge_at)
            .collect()
    }

    /// Instruments that at least one record names (deduplicated).
    #[must_use]
    pub fn instruments(&self) -> BTreeSet<InstrumentId> {
        let map: HashMap<InstrumentId, ()> = self
            .records
            .iter()
            .map(|record| (record.instrument_id.clone(), ()))
            .collect();
        map.into_keys().collect()
    }
}
