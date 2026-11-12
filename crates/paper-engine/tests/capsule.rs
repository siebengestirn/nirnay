//! Decision-capsule contract tests.
//!
//! A capsule must reconstruct one decision's evidence from the journal alone:
//! cited input events, the risk assessment, the resulting order and fills, and
//! the reconciliation history. Every gap is surfaced as an uncertainty flag
//! rather than hidden.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use paper_engine::{Engine, EngineError, QuoteOutcome, ReconcileInput};
use trading_core::{
    IntentId, OrderStatus, Paise, PricePaise, Quantity, ReconciliationStatus, Side,
};

fn capsule_for(engine: &Engine, intent: u64) -> Result<paper_engine::DecisionCapsule, EngineError> {
    let intent_id = IntentId::new(intent).map_err(EngineError::Domain)?;
    engine.decision_capsule(intent_id)
}

#[test]
fn capsule_records_decision_risk_order_fills_and_reconciliation() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    assert_eq!(
        engine.on_quote("feed", 1, quote(5, 5)?, times(10))?,
        QuoteOutcome::Accepted
    );
    let order_id = submit(&mut engine, 1, 2, 1_005, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(5, 5)?, times(12))?;

    let expected_positions = BTreeMap::from([(instrument_id(), 2_i64)]);
    let report = engine.reconcile(
        ReconcileInput {
            positions: Some(expected_positions),
            active_orders: Some(BTreeSet::new()),
        },
        "post-trade",
        times(13),
    )?;
    assert_eq!(report.status, ReconciliationStatus::Matched);

    let capsule = capsule_for(&engine, 1)?;
    assert_eq!(capsule.capsule_version, 1);
    assert_eq!(capsule.stream_id, STREAM);
    assert_eq!(capsule.intent_id, IntentId::new(1)?);
    assert!(capsule.decision_sequence > 0);
    assert_eq!(capsule.decision_hash_hex.len(), 64);
    assert!(capsule.input_references_verified);
    assert!(!capsule.input_events.is_empty());
    assert!(
        capsule
            .input_events
            .iter()
            .all(|event| event.hash_hex.len() == 64 && event.sequence <= capsule.decision_sequence)
    );

    let risk = capsule.risk.as_ref().ok_or("risk evidence missing")?;
    assert!(risk.assessment.accepted);
    assert_eq!(risk.assessment.intent_id, IntentId::new(1)?);

    let order = capsule.order.as_ref().ok_or("order evidence missing")?;
    assert_eq!(order.order_request.order_id, order_id);
    assert_eq!(order.final_status, OrderStatus::Filled);
    assert_eq!(order.filled_quantity, Quantity::new(2)?);
    assert_eq!(order.filled_notional, Paise(2_010));
    assert_eq!(
        order.average_fill_price,
        Some(PricePaise::new(Paise(1_005))?)
    );
    assert_eq!(order.fill_count, 1);
    assert_eq!(order.fills.len(), 1);
    assert_eq!(order.fills[0].fill.fill_id.get(), 1);
    assert!(!order.events.is_empty());

    let latest = capsule
        .reconciliations
        .last()
        .ok_or("reconciliation evidence missing")?;
    assert_eq!(latest.report.status, ReconciliationStatus::Matched);
    assert!(
        capsule.uncertainty_flags.is_empty(),
        "unexpected flags: {:?}",
        capsule.uncertainty_flags
    );

    let head = engine.records().last().ok_or("journal has no records")?;
    assert_eq!(capsule.journal_head_sequence, engine.last_sequence());
    assert_eq!(capsule.journal_head_sequence, head.sequence);
    assert_eq!(capsule.journal_head_hash_hex, head.hash.hex());

    assert_eq!(
        engine.positions(),
        BTreeMap::from([(instrument_id(), 2_i64)])
    );
    assert_eq!(engine.fills().len(), 1);
    assert_eq!(engine.calendar_version(), Some("cal-v1"));
    Ok(())
}

#[test]
fn capsule_records_rejected_risk_without_an_order() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;

    // Eleven lots exceed the policy's ten-lot per-order limit.
    let refs = evidence(&engine)?;
    let rejected = decision(1, refs, 11, 1_005, Side::Buy, "too large")?;
    let outcome = engine.submit_decision(rejected, times(11))?;
    assert!(outcome.order_id.is_none());

    let capsule = capsule_for(&engine, 1)?;
    let risk = capsule.risk.as_ref().ok_or("risk evidence missing")?;
    assert!(!risk.assessment.accepted);
    assert!(capsule.order.is_none());
    assert!(
        !capsule
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "accepted_risk_has_no_order_record"),
        "a rejected decision must not claim a missing order"
    );
    Ok(())
}

#[test]
fn capsule_flags_gapped_source_continuity() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;
    // Sequence 2 never arrives, so the delta at 3 opens a continuity gap.
    assert_eq!(
        engine.on_quote("feed", 3, delta_quote(5, 5)?, times(11))?,
        QuoteOutcome::Accepted
    );

    let refs = evidence(&engine)?;
    let gapped = decision(1, refs, 1, 1_005, Side::Buy, "gapped input")?;
    engine.submit_decision(gapped, times(12))?;

    let capsule = capsule_for(&engine, 1)?;
    assert!(
        capsule
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "source_continuity_not_established:feed"),
        "flags: {:?}",
        capsule.uncertainty_flags
    );
    assert!(
        capsule
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "input_quality:SequenceGap"),
        "flags: {:?}",
        capsule.uncertainty_flags
    );
    Ok(())
}

#[test]
fn capsule_flags_an_unapplied_broker_report() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;
    let order_id = submit(&mut engine, 1, 1, 1_005, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(5, 5)?, times(12))?;
    engine.reconcile(
        ReconcileInput {
            positions: Some(BTreeMap::from([(instrument_id(), 1_i64)])),
            active_orders: Some(BTreeSet::new()),
        },
        "matched-after-fill",
        times(13),
    )?;

    // The order already filled, so this acknowledgement is recorded but not
    // applied; the capsule must not hide that.
    engine.acknowledge_cancel(order_id, times(14))?;

    let capsule = capsule_for(&engine, 1)?;
    assert!(
        capsule
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "unapplied_broker_report_seen"),
        "flags: {:?}",
        capsule.uncertainty_flags
    );
    Ok(())
}

#[test]
fn capsule_rejects_an_unknown_decision() -> TestResult {
    let directory = tempfile::tempdir()?;
    let engine = running_engine(&directory)?;
    assert!(matches!(
        capsule_for(&engine, 999),
        Err(EngineError::Capsule(_))
    ));
    Ok(())
}
