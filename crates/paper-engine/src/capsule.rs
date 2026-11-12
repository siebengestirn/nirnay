//! Reconstructable decision evidence assembled from journal records.
//!
//! Derived from the internal Nirnay `crates/nirnay-engine/src/capsule.rs` at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! A capsule is a read-only projection: it resolves the decision's cited input
//! events, its risk assessment, the resulting order and fills, and the
//! reconciliation history, then records every gap it could not verify as an
//! explicit uncertainty flag instead of hiding it.

use std::collections::{BTreeMap, BTreeSet};

use account_journal::StoredRecord;
use serde::{Deserialize, Serialize};
use trading_core::{
    CheckedPaise, Continuity, DataQualityFlag, DecisionRecord, EventPayload, Fill, IntentId,
    JournalRef, Order, OrderStatus, Paise, PricePaise, Quantity, ReconciliationReport,
    ReconciliationStatus, RiskAssessment,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceEvent {
    pub sequence: u64,
    pub event_id: String,
    pub kind: String,
    pub hash_hex: String,
    pub source_id: Option<String>,
    pub source_sequence: Option<u64>,
    pub available_time_ns: u64,
    pub quality: BTreeSet<DataQualityFlag>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillEvidence {
    pub sequence: u64,
    pub event_hash_hex: String,
    pub fill: Fill,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderEvidence {
    pub created_sequence: u64,
    pub order_request: Order,
    pub final_status: OrderStatus,
    pub filled_quantity: Quantity,
    pub filled_notional: Paise,
    pub average_fill_price: Option<PricePaise>,
    pub fill_count: u32,
    pub events: Vec<EvidenceEvent>,
    pub fills: Vec<FillEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskEvidence {
    pub sequence: u64,
    pub event_hash_hex: String,
    pub assessment: RiskAssessment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationEvidence {
    pub sequence: u64,
    pub event_hash_hex: String,
    pub report: ReconciliationReport,
}

/// A reconstructable account of one strategy decision and its journalled consequences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionCapsule {
    pub capsule_version: u16,
    pub stream_id: String,
    pub intent_id: IntentId,
    pub decision_sequence: u64,
    pub decision_hash_hex: String,
    pub decision: DecisionRecord,
    pub input_events: Vec<EvidenceEvent>,
    pub input_references_verified: bool,
    pub risk: Option<RiskEvidence>,
    pub order: Option<OrderEvidence>,
    pub reconciliations: Vec<ReconciliationEvidence>,
    pub uncertainty_flags: Vec<String>,
    pub journal_head_sequence: u64,
    pub journal_head_hash_hex: String,
}

impl DecisionCapsule {
    pub fn build(records: &[StoredRecord], intent_id: IntentId) -> Result<Self, CapsuleError> {
        let decision_record = records
            .iter()
            .find(|record| {
                matches!(
                    &record.body.payload,
                    EventPayload::DecisionMade { decision }
                        if decision
                            .intent
                            .as_ref()
                            .is_some_and(|intent| intent.intent_id == intent_id)
                )
            })
            .ok_or(CapsuleError::DecisionNotFound)?;
        let decision = match &decision_record.body.payload {
            EventPayload::DecisionMade { decision } => decision.clone(),
            _ => return Err(CapsuleError::DecisionNotFound),
        };
        let references = merged_references(
            &decision,
            &decision
                .intent
                .as_ref()
                .map(|intent| intent.evidence.clone())
                .unwrap_or_default(),
        );
        let mut uncertainty_flags = BTreeSet::new();
        let mut input_events = Vec::with_capacity(references.len());
        let mut refs_verified = true;
        let mut sources = BTreeSet::new();
        for reference in &references {
            match resolve_reference(
                records,
                reference,
                decision_record.body.envelope.times.available_time.get(),
            ) {
                Some(record) => {
                    if let Some(source) = &record.body.envelope.source_id {
                        sources.insert(source.clone());
                    }
                    for flag in &record.body.envelope.quality {
                        uncertainty_flags.insert(format!("input_quality:{flag:?}"));
                    }
                    input_events.push(event_summary(record));
                }
                None => {
                    refs_verified = false;
                    uncertainty_flags.insert(format!(
                        "unresolved_or_future_input_reference:{}",
                        reference.sequence
                    ));
                }
            }
        }
        for source in sources {
            if source_is_gapped(records, decision_record.sequence, &source) {
                uncertainty_flags.insert(format!("source_continuity_not_established:{source}"));
            }
        }
        if decision.input_refs.is_empty() {
            uncertainty_flags.insert("decision_has_no_input_references".to_owned());
        }
        if !refs_verified {
            uncertainty_flags.insert("input_reference_integrity_failed".to_owned());
        }

        let risk = records
            .iter()
            .find_map(|record| match &record.body.payload {
                EventPayload::RiskAssessed { assessment } if assessment.intent_id == intent_id => {
                    Some(RiskEvidence {
                        sequence: record.sequence,
                        event_hash_hex: record.hash.hex(),
                        assessment: assessment.clone(),
                    })
                }
                _ => None,
            });
        if risk.is_none() {
            uncertainty_flags.insert("risk_assessment_missing".to_owned());
        }

        let created = records
            .iter()
            .find_map(|record| match &record.body.payload {
                EventPayload::OrderCreated { order } if order.intent.intent_id == intent_id => {
                    Some((record.sequence, order.clone()))
                }
                _ => None,
            });
        let order = if let Some((created_sequence, order_request)) = created {
            let order_id = order_request.order_id;
            let mut final_status = order_request.status;
            let mut filled_quantity = order_request.filled_quantity;
            let mut filled_notional = order_request.filled_notional;
            let mut average_fill_price = order_request.average_fill_price;
            let mut fill_count = order_request.fill_count;
            let mut events = Vec::new();
            let mut fills = Vec::new();
            for record in records
                .iter()
                .filter(|record| record.sequence >= created_sequence)
            {
                match &record.body.payload {
                    EventPayload::OrderTransitioned {
                        order_id: observed,
                        to,
                        ..
                    } if *observed == order_id => {
                        final_status = *to;
                        events.push(event_summary(record));
                    }
                    EventPayload::BrokerReportObserved {
                        order_id: observed, ..
                    } if *observed == order_id => events.push(event_summary(record)),
                    EventPayload::FillBooked { fill, next_status } if fill.order_id == order_id => {
                        filled_quantity = filled_quantity
                            .checked_add(fill.quantity)
                            .map_err(|_| CapsuleError::InvalidFillHistory)?;
                        let turnover = fill
                            .price
                            .notional(fill.quantity)
                            .map_err(|_| CapsuleError::InvalidFillHistory)?;
                        filled_notional = filled_notional
                            .checked_add(turnover)
                            .map_err(|_| CapsuleError::InvalidFillHistory)?;
                        let weighted_price = i64::try_from(
                            i128::from(filled_notional.0) / i128::from(filled_quantity.get()),
                        )
                        .map_err(|_| CapsuleError::InvalidFillHistory)?;
                        average_fill_price = Some(
                            PricePaise::new(Paise(weighted_price))
                                .map_err(|_| CapsuleError::InvalidFillHistory)?,
                        );
                        fill_count = fill_count
                            .checked_add(1)
                            .ok_or(CapsuleError::InvalidFillHistory)?;
                        final_status = *next_status;
                        fills.push(FillEvidence {
                            sequence: record.sequence,
                            event_hash_hex: record.hash.hex(),
                            fill: fill.clone(),
                        });
                        events.push(event_summary(record));
                    }
                    _ => {}
                }
            }
            Some(OrderEvidence {
                created_sequence,
                order_request,
                final_status,
                filled_quantity,
                filled_notional,
                average_fill_price,
                fill_count,
                events,
                fills,
            })
        } else {
            None
        };
        if order.is_none() && risk.as_ref().is_some_and(|entry| entry.assessment.accepted) {
            uncertainty_flags.insert("accepted_risk_has_no_order_record".to_owned());
        }

        let order_id = order.as_ref().map(|entry| entry.order_request.order_id);
        let reconciliations = records
            .iter()
            .filter_map(|record| {
                if record.sequence < decision_record.sequence {
                    return None;
                }
                match &record.body.payload {
                    EventPayload::ReconciliationObserved { report } => {
                        Some(ReconciliationEvidence {
                            sequence: record.sequence,
                            event_hash_hex: record.hash.hex(),
                            report: report.clone(),
                        })
                    }
                    EventPayload::BrokerReportObserved {
                        order_id: observed,
                        applied: false,
                        ..
                    } if Some(*observed) == order_id => {
                        uncertainty_flags.insert("unapplied_broker_report_seen".to_owned());
                        None
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>();
        if reconciliations
            .last()
            .is_none_or(|entry| entry.report.status != ReconciliationStatus::Matched)
        {
            uncertainty_flags.insert("latest_reconciliation_not_matched_or_missing".to_owned());
        }

        let head = records.last().ok_or(CapsuleError::EmptyJournal)?;
        Ok(Self {
            capsule_version: 1,
            stream_id: decision_record.body.envelope.stream_id.clone(),
            intent_id,
            decision_sequence: decision_record.sequence,
            decision_hash_hex: decision_record.hash.hex(),
            decision,
            input_events,
            input_references_verified: refs_verified,
            risk,
            order,
            reconciliations,
            uncertainty_flags: uncertainty_flags.into_iter().collect(),
            journal_head_sequence: head.sequence,
            journal_head_hash_hex: head.hash.hex(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CapsuleError {
    #[error("the requested decision is absent from the event journal")]
    DecisionNotFound,
    #[error("the event journal has no records")]
    EmptyJournal,
    #[error("order fill history violates exact accounting invariants")]
    InvalidFillHistory,
}

fn merged_references(decision: &DecisionRecord, intent_refs: &[JournalRef]) -> Vec<JournalRef> {
    let mut refs: BTreeMap<u64, String> = BTreeMap::new();
    for reference in decision.input_refs.iter().chain(intent_refs) {
        refs.entry(reference.sequence)
            .or_insert_with(|| reference.hash_hex.clone());
    }
    refs.into_iter()
        .map(|(sequence, hash_hex)| JournalRef { sequence, hash_hex })
        .collect()
}

fn resolve_reference<'a>(
    records: &'a [StoredRecord],
    reference: &JournalRef,
    decision_available_time: u64,
) -> Option<&'a StoredRecord> {
    let index = usize::try_from(reference.sequence.checked_sub(1)?).ok()?;
    records.get(index).filter(|record| {
        record.sequence == reference.sequence
            && record.hash.hex() == reference.hash_hex
            && record.body.envelope.times.available_time.get() <= decision_available_time
    })
}

fn source_is_gapped(records: &[StoredRecord], end_sequence: u64, source: &str) -> bool {
    let mut gapped = false;
    for record in records
        .iter()
        .filter(|record| record.sequence <= end_sequence)
    {
        if record.body.envelope.source_id.as_deref() != Some(source) {
            continue;
        }
        match &record.body.payload {
            EventPayload::FeedGapDetected { .. } | EventPayload::LateQuoteObserved { .. } => {
                gapped = true;
            }
            EventPayload::QuoteReceived { quote } if quote.continuity == Continuity::Snapshot => {
                gapped = false;
            }
            _ => {}
        }
    }
    gapped
}

fn event_summary(record: &StoredRecord) -> EvidenceEvent {
    EvidenceEvent {
        sequence: record.sequence,
        event_id: record.body.envelope.event_id.to_string(),
        kind: record.body.payload.kind().to_owned(),
        hash_hex: record.hash.hex(),
        source_id: record.body.envelope.source_id.clone(),
        source_sequence: record.body.envelope.source_sequence,
        available_time_ns: record.body.envelope.times.available_time.get(),
        quality: record.body.envelope.quality.clone(),
    }
}
