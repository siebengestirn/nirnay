//! Paper-engine contract tests.
//!
//! These exercise the order lifecycle end to end through the public engine
//! API: partial fills, cancel races, unknown outcomes, broker reconciliation,
//! duplicate submission, stale and discontinuous feeds, exact cash movement,
//! journal reopen and cash/position reconciliation.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use paper_engine::{CancelOutcome, Engine, EngineError, QuoteOutcome, ReconcileInput};
use trading_core::{
    EventId, OrderId, OrderStatus, Paise, PricePaise, Quantity, ReconciliationStatus, RiskCheck,
    SessionPhase, Side,
};

#[test]
fn partial_fills_and_cash_are_exact() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;

    assert_eq!(
        engine.on_quote("feed", 1, quote(5, 5)?, times(10))?,
        QuoteOutcome::Accepted
    );
    let order_id = submit(&mut engine, 1, 2, 1_005, Side::Buy, 11)?;

    // Displayed ask quantity is one lot, so the first fill is partial.
    assert_eq!(
        engine.on_quote("feed", 2, quote(1, 5)?, times(12))?,
        QuoteOutcome::Accepted
    );
    let order = engine.order(order_id).ok_or("order missing")?;
    assert_eq!(order.status, OrderStatus::PartiallyFilled);
    assert_eq!(order.filled_quantity, Quantity::new(1)?);
    assert_eq!(order.filled_notional, Paise(1_005));
    let view = account(&engine)?;
    assert_eq!(
        view.cash,
        Paise(STARTING_CASH - 1_015),
        "cash after first fill"
    );
    assert_eq!(view.charges_paid, Paise(10));

    // A second displayed lot completes the order at the same price.
    assert_eq!(
        engine.on_quote("feed", 3, quote(5, 5)?, times(13))?,
        QuoteOutcome::Accepted
    );
    let order = engine.order(order_id).ok_or("order missing")?;
    assert_eq!(order.status, OrderStatus::Filled);
    assert_eq!(order.filled_quantity, Quantity::new(2)?);
    assert_eq!(order.filled_notional, Paise(2_010));
    assert_eq!(
        order.average_fill_price,
        Some(PricePaise::new(Paise(1_005))?)
    );
    assert_eq!(order.fill_count, 2);
    let view = account(&engine)?;
    assert_eq!(view.cash, Paise(STARTING_CASH - 2_030));
    assert_eq!(view.charges_paid, Paise(20));
    assert_eq!(view.positions.len(), 1);
    assert_eq!(view.positions[0].quantity, 2);
    Ok(())
}

#[test]
fn cancel_during_a_fill_race_finishes_flat_and_acknowledges_late() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;
    let order_id = submit(&mut engine, 1, 2, 1_005, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(1, 5)?, times(12))?;

    assert_eq!(
        engine.request_cancel(order_id, "operator cancel", times(13))?,
        CancelOutcome::Pending
    );
    // The next quote fills the remaining lot before the cancel acknowledges.
    engine.on_quote("feed", 3, quote(5, 5)?, times(14))?;
    let order = engine.order(order_id).ok_or("order missing")?;
    assert_eq!(order.status, OrderStatus::Filled);

    assert_eq!(
        engine.acknowledge_cancel(order_id, times(15))?,
        CancelOutcome::LateAfterFill
    );
    assert_eq!(
        engine.request_cancel(order_id, "after fill", times(16))?,
        CancelOutcome::AlreadyTerminal
    );
    Ok(())
}

#[test]
fn unknown_outcome_needs_an_authoritative_resolution() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;
    let order_id = submit(&mut engine, 1, 2, 1_005, Side::Buy, 11)?;

    engine.mark_outcome_unknown(order_id, "broker timeout", times(12))?;
    assert!(matches!(
        engine.request_cancel(order_id, "cancel unknown", times(13)),
        Err(EngineError::UnknownOutcomeNeedsResolution)
    ));

    let active_orders: BTreeSet<OrderId> = [order_id].into_iter().collect();
    let report = engine.reconcile(
        ReconcileInput {
            positions: Some(BTreeMap::new()),
            active_orders: Some(active_orders),
        },
        "unknown-reconciliation",
        times(14),
    )?;
    assert_eq!(report.status, ReconciliationStatus::Mismatch);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("unresolved"))
    );

    // A fill cannot be invented from a status lookup alone.
    assert!(matches!(
        engine.resolve_unknown(order_id, OrderStatus::Filled, "query-1", times(15)),
        Err(EngineError::UnknownOutcomeNeedsResolution)
    ));
    engine.resolve_unknown(order_id, OrderStatus::Cancelled, "query-2", times(16))?;
    let order = engine.order(order_id).ok_or("order missing")?;
    assert_eq!(order.status, OrderStatus::Cancelled);

    let report = engine.reconcile(
        ReconcileInput {
            positions: Some(BTreeMap::new()),
            active_orders: Some(BTreeSet::new()),
        },
        "resolved-reconciliation",
        times(17),
    )?;
    assert_eq!(report.status, ReconciliationStatus::Matched);
    Ok(())
}

#[test]
fn duplicate_submission_is_deduplicated_and_conflicts_are_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;

    let first_refs = evidence(&engine)?;
    let first_decision = decision(1, first_refs, 1, 1_005, Side::Buy, "first rationale")?;
    let first = engine.submit_decision(first_decision.clone(), times(11))?;
    assert!(!first.deduplicated);
    let order_id = first.order_id.ok_or("order missing")?;

    let second = engine.submit_decision(first_decision, times(12))?;
    assert!(second.deduplicated);
    assert_eq!(second.order_id, Some(order_id));
    assert_eq!(engine.records().len(), 9);

    let conflicting = decision(
        1,
        evidence(&engine)?,
        1,
        1_005,
        Side::Buy,
        "other rationale",
    )?;
    assert!(matches!(
        engine.submit_decision(conflicting, times(13)),
        Err(EngineError::IdempotencyConflict)
    ));
    assert_eq!(engine.records().len(), 9);
    Ok(())
}

#[test]
fn stale_and_gapped_quotes_block_new_risk() -> TestResult {
    let stale_dir = tempfile::tempdir()?;
    let mut stale = running_engine(&stale_dir)?;
    stale.on_quote("feed", 1, quote(5, 5)?, times(10))?;
    let stale_refs = evidence(&stale)?;
    let stale_decision = decision(1, stale_refs, 1, 1_005, Side::Buy, "stale quote")?;
    let outcome = stale.submit_decision(stale_decision, times(2_000))?;
    assert!(outcome.order_id.is_none());
    let assessment = outcome.risk.ok_or("risk assessment missing")?;
    assert!(!assessment.accepted);
    assert!(
        assessment
            .checks
            .iter()
            .any(|check| check.check == RiskCheck::MarketFeedHealth && !check.passed)
    );

    let gap_dir = tempfile::tempdir()?;
    let mut gapped = running_engine(&gap_dir)?;
    gapped.on_quote("feed", 1, quote(5, 5)?, times(10))?;
    assert_eq!(
        gapped.on_quote("feed", 3, delta_quote(5, 5)?, times(11))?,
        QuoteOutcome::Accepted
    );
    let gap_refs = evidence(&gapped)?;
    let gap_decision = decision(1, gap_refs, 1, 1_005, Side::Buy, "gapped feed")?;
    let outcome = gapped.submit_decision(gap_decision, times(12))?;
    assert!(outcome.order_id.is_none());
    let assessment = outcome.risk.ok_or("risk assessment missing")?;
    assert!(
        assessment
            .checks
            .iter()
            .any(|check| check.check == RiskCheck::MarketFeedHealth && !check.passed)
    );
    Ok(())
}

#[test]
fn broker_reconciliation_reports_matched_mismatch_and_unavailable() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;

    let matched = engine.reconcile(
        ReconcileInput {
            positions: Some(BTreeMap::new()),
            active_orders: Some(BTreeSet::new()),
        },
        "matched",
        times(10),
    )?;
    assert_eq!(matched.status, ReconciliationStatus::Matched);
    assert!(matched.differences.is_empty());

    let observed = BTreeMap::from([(instrument_id(), 3_i64)]);
    let mismatch = engine.reconcile(
        ReconcileInput {
            positions: Some(observed),
            active_orders: Some(BTreeSet::new()),
        },
        "mismatch",
        times(11),
    )?;
    assert_eq!(mismatch.status, ReconciliationStatus::Mismatch);
    assert!(!mismatch.differences.is_empty());

    let unavailable = engine.reconcile(
        ReconcileInput {
            positions: None,
            active_orders: None,
        },
        "unavailable",
        times(12),
    )?;
    assert_eq!(unavailable.status, ReconciliationStatus::Unavailable);
    assert_eq!(unavailable.differences.len(), 2);

    let restored = engine.reconcile(
        ReconcileInput {
            positions: Some(BTreeMap::new()),
            active_orders: Some(BTreeSet::new()),
        },
        "restored",
        times(13),
    )?;
    assert_eq!(restored.status, ReconciliationStatus::Matched);
    Ok(())
}

#[test]
fn sell_fills_book_cash_net_of_charges() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;
    submit(&mut engine, 1, 1, 1_005, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(5, 5)?, times(12))?;
    engine.on_quote("feed", 3, quote(5, 5)?, times(13))?;

    let sell_id = submit(&mut engine, 2, 1, 995, Side::Sell, 14)?;
    engine.on_quote("feed", 4, quote(5, 5)?, times(15))?;
    let sell = engine.order(sell_id).ok_or("sell order missing")?;
    assert_eq!(sell.status, OrderStatus::Filled);

    // Buy: 1,005 + 10 charge = 1,015 outflow. Sell: 995 - 10 charge = 985
    // inflow. The round trip costs the 10 paise price difference plus 20 in
    // charges, so cash falls by 30.
    let view = account(&engine)?;
    assert_eq!(view.cash, Paise(STARTING_CASH - 30));
    assert_eq!(view.charges_paid, Paise(20));
    assert!(view.positions.is_empty(), "round trip is flat");
    Ok(())
}

#[test]
fn reopened_engine_replays_the_same_account_and_event_identities() -> TestResult {
    let directory = tempfile::tempdir()?;
    let ledger = directory.path().join("ledger");

    let (first_event_id, records, cash, positions) = {
        let mut engine = Engine::open(&ledger, config()?, times(1))?;
        engine.set_session_phase(SessionPhase::Continuous, "cal-v1", times(2))?;
        engine.observe_risk_state(Paise(0), true, times(3))?;
        engine.reconcile(
            ReconcileInput {
                positions: Some(BTreeMap::new()),
                active_orders: Some(BTreeSet::new()),
            },
            "initial",
            times(4),
        )?;
        engine.set_kill_switch(false, 7, "test release", times(5))?;
        engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;
        submit(&mut engine, 1, 2, 1_005, Side::Buy, 11)?;
        engine.on_quote("feed", 2, quote(5, 5)?, times(12))?;

        let view = account(&engine)?;
        let first_event_id = engine
            .records()
            .first()
            .ok_or("journal has no records")?
            .body
            .envelope
            .event_id
            .clone();
        (
            first_event_id,
            engine.records().len(),
            view.cash,
            view.positions,
        )
    };

    assert_eq!(first_event_id, EventId::for_run(RUN, 1));

    let engine = Engine::open(&ledger, config()?, times(1))?;
    let view = account(&engine)?;
    assert_eq!(engine.records().len(), records);
    assert_eq!(view.cash, cash);
    assert_eq!(view.positions, positions);
    assert_eq!(
        engine
            .records()
            .first()
            .ok_or("journal has no records")?
            .body
            .envelope
            .event_id,
        first_event_id
    );
    Ok(())
}

#[test]
fn cash_and_position_reconciliation_matches_a_filled_order() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 5)?, times(10))?;
    let order_id = submit(&mut engine, 1, 2, 1_005, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(5, 5)?, times(12))?;

    let view = account(&engine)?;
    assert_eq!(view.cash, Paise(STARTING_CASH - 2_030));
    assert_eq!(view.positions.len(), 1);
    assert_eq!(view.positions[0].quantity, 2);

    let expected_positions = BTreeMap::from([(instrument_id(), 2_i64)]);
    let matched = engine.reconcile(
        ReconcileInput {
            positions: Some(expected_positions),
            active_orders: Some(BTreeSet::new()),
        },
        "cash-position-matched",
        times(13),
    )?;
    assert_eq!(matched.status, ReconciliationStatus::Matched);

    let wrong_positions = BTreeMap::from([(instrument_id(), 1_i64)]);
    let mismatch = engine.reconcile(
        ReconcileInput {
            positions: Some(wrong_positions),
            active_orders: Some(BTreeSet::new()),
        },
        "cash-position-mismatch",
        times(14),
    )?;
    assert_eq!(mismatch.status, ReconciliationStatus::Mismatch);

    let active: BTreeSet<OrderId> = [order_id].into_iter().collect();
    let stale_active_orders = engine.reconcile(
        ReconcileInput {
            positions: Some(BTreeMap::from([(instrument_id(), 2_i64)])),
            active_orders: Some(active),
        },
        "terminal-order-still-active",
        times(15),
    )?;
    assert_eq!(stale_active_orders.status, ReconciliationStatus::Mismatch);
    Ok(())
}

#[test]
fn kill_switch_cannot_release_before_reconciliation() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = Engine::open(directory.path().join("ledger"), config()?, times(1))?;
    engine.set_session_phase(SessionPhase::Continuous, "cal-v1", times(2))?;
    engine.observe_risk_state(Paise(0), true, times(3))?;

    assert!(matches!(
        engine.set_kill_switch(false, 7, "premature release", times(4)),
        Err(EngineError::InvalidOrderTransition)
    ));

    engine.reconcile(
        ReconcileInput {
            positions: Some(BTreeMap::new()),
            active_orders: Some(BTreeSet::new()),
        },
        "initial",
        times(5),
    )?;
    engine.set_kill_switch(false, 7, "reconciled release", times(6))?;
    let view = account(&engine)?;
    assert!(!view.kill_switch_engaged);
    Ok(())
}
