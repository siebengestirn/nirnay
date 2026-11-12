#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Displayed-liquidity accounting: repeated quotes never replenish consumed
//! liquidity, fills debit a deterministic per-side budget that journal
//! replay reconstructs, and fills beyond the budget reject.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use account_journal::Journal;
use common::*;
use paper_engine::Engine;
use trading_core::{
    AccountId, ChargeComponents, DecisionRecord, EventEnvelope, EventId, EventPayload, EventTimes,
    Fill, FillId, JournalRef, LogicalNanos, Order, OrderId, OrderIntent, OrderStatus, Paise,
    PricePaise, Product, Quantity, RiskAssessment, Side, StoredBody, StrategyId, UtcNanos,
};

/// The decisive regression: consuming the one displayed ask unit, then
/// quoting the identical line under a new source sequence, adds no new
/// liquidity. A genuine displayed-quantity increase releases the increment.
/// (Fills happen on quote ingestion, after orders are resting.)
#[test]
fn displayed_liquidity_is_budgeted_not_refreshed_by_quotes() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;

    engine.on_quote("feed", 1, quote(1, 1)?, times(10))?;
    let first_order = submit(&mut engine, 1, 1, 1_005, Side::Buy, 11)?;
    // The second identical quotation fills exactly the one displayed unit.
    engine.on_quote("feed", 2, quote(1, 1)?, times(12))?;
    assert_eq!(
        engine.order(first_order).expect("order exists").status,
        OrderStatus::Filled
    );

    // A newly submitted order sees the same display, but no remaining
    // budget: identical re-quotes replenish nothing.
    let second_order = submit(&mut engine, 2, 1, 1_005, Side::Buy, 13)?;
    engine.on_quote("feed", 3, quote(1, 1)?, times(14))?;
    let second = engine.order(second_order).expect("order exists");
    assert_eq!(second.status, OrderStatus::Working);
    assert_eq!(second.filled_quantity, Quantity::ZERO);

    // Display increases by one unit: exactly one more unit becomes
    // available, and the pending order fills from it.
    engine.on_quote("feed", 4, quote(2, 1)?, times(15))?;
    let second = engine.order(second_order).expect("order exists");
    assert_eq!(second.status, OrderStatus::Filled);
    assert_eq!(second.filled_quantity, Quantity::new(1)?);
    Ok(())
}

#[test]
fn restart_preserves_consumed_liquidity() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(1, 1)?, times(10))?;
    submit(&mut engine, 1, 1, 1_005, Side::Buy, 11)?;
    // Fill on the next identical quote so the unit is journal-consumed.
    engine.on_quote("feed", 2, quote(1, 1)?, times(12))?;
    engine.set_kill_switch(true, 3, "stop", times(20))?;
    drop(engine);

    // Reopening the journal replays the budget consumption: the unchanged
    // line is still exhausted.
    let mut reopened = Engine::open(directory.path().join("ledger"), config()?, times(20))?;
    reopened.set_kill_switch(false, 4, "restart release", times(21))?;
    let order_id = submit(&mut reopened, 2, 1, 1_005, Side::Buy, 21)?;
    reopened.on_quote("feed", 5, quote(1, 1)?, times(22))?;
    let order = reopened.order(order_id).expect("order exists");
    assert_eq!(order.status, OrderStatus::Working);
    assert_eq!(order.filled_quantity, Quantity::ZERO);
    Ok(())
}

#[test]
fn sell_fills_debit_the_bid_budget_symmetrically() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(5, 1)?, times(10))?;
    // Two longs first (policy disallows shorts; one bid unit displayed).
    let buy_a = submit(&mut engine, 1, 1, 1_005, Side::Buy, 10)?;
    engine.on_quote("feed", 2, quote(5, 1)?, times(11))?;
    let buy_b = submit(&mut engine, 2, 1, 1_005, Side::Buy, 12)?;
    engine.on_quote("feed", 3, quote(5, 1)?, times(13))?;
    assert_eq!(engine.order(buy_a).expect("a").status, OrderStatus::Filled);
    assert_eq!(engine.order(buy_b).expect("b").status, OrderStatus::Filled);

    // Sell one unit against the displayed bid.
    let first_sell = submit(&mut engine, 3, 1, 995, Side::Sell, 14)?;
    engine.on_quote("feed", 4, quote(5, 1)?, times(15))?;
    assert_eq!(
        engine.order(first_sell).expect("order exists").status,
        OrderStatus::Filled
    );

    // An identical bid display under a new sequence cannot fill the next
    // sell: the bid unit it would consume is already gone.
    let later_sell = submit(&mut engine, 4, 1, 995, Side::Sell, 15)?;
    engine.on_quote("feed", 5, quote(5, 1)?, times(16))?;
    let later = engine.order(later_sell).expect("order exists");
    assert_eq!(later.status, OrderStatus::Working);
    assert_eq!(later.filled_quantity, Quantity::ZERO);
    Ok(())
}

#[test]
fn fills_keep_price_and_time_priority_within_a_budget() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(1, 1)?, times(10))?;
    let low = submit(&mut engine, 1, 1, 1_000, Side::Buy, 11)?;
    let high = submit(&mut engine, 2, 1, 1_005, Side::Buy, 12)?;
    // One unit is displayed: only the higher-limit (better-priced) order
    // fills first, even though it was submitted later.
    engine.on_quote("feed", 2, quote(1, 1)?, times(13))?;
    assert_eq!(
        engine.order(high).expect("high order exists").status,
        OrderStatus::Filled
    );
    assert_eq!(
        engine.order(low).expect("low order exists").status,
        OrderStatus::Working
    );
    Ok(())
}

#[test]
fn forged_over_consumption_fails_journal_replay() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    build_journal_with_over_consuming_fill(&path)?;
    let error = Engine::open(path, config()?, times(1)).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("liquidity") || message.contains("budget") || message.contains("exceeds"),
        "unexpected error: {message}"
    );
    Ok(())
}

fn build_journal_with_over_consuming_fill(
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = trading_core::RunId::from_bytes([7u8; 16]);
    let instrument_id = instrument_id();
    let intent = OrderIntent {
        intent_id: trading_core::IntentId::new(1)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: trading_core::AlgoId::new(3)?,
        instrument_id: instrument_id.clone(),
        venue: trading_core::Venue::Nse,
        side: Side::Buy,
        product: Product::Intraday,
        quantity: Quantity::new(5)?,
        limit_price: PricePaise::new(Paise(1_005))?,
        reason: "forged fill".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "a".repeat(64),
        }],
        expires_at: LogicalNanos::new(10_000_000),
    };
    let decision = DecisionRecord {
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        source_build_hash: "a".repeat(64),
        configuration_hash: "b".repeat(64),
        dataset_hash: "c".repeat(64),
        input_refs: intent.evidence.clone(),
        features_fixed_point: BTreeMap::new(),
        rationale: "forged".to_owned(),
        intent: Some(intent.clone()),
    };
    let assessment = RiskAssessment {
        intent_id: intent.intent_id,
        policy_version: "risk-v1".to_owned(),
        accepted: true,
        checks: Vec::new(),
    };
    let order = Order {
        order_id: OrderId::new(1)?,
        intent: intent.clone(),
        status: OrderStatus::Working,
        filled_quantity: Quantity::ZERO,
        filled_notional: Paise(0),
        average_fill_price: None,
        fill_count: 0,
    };
    let forged_order_id = order.order_id;

    let mut journal = Journal::open(path, STREAM)?;
    let mut sequence = 0u64;
    // Market-source path fields are record-scoped: quote records carry the
    // source identity and sequence the market state requires; other
    // records carry neither (validated together at the journal boundary).
    let mut append = |payload: EventPayload,
                      available: u64,
                      source: Option<(String, u64)>|
     -> Result<(), Box<dyn std::error::Error>> {
        sequence += 1;
        // The forged journal is a legacy (v1) run, but venue-bearing
        // payloads must declare the version that carries their venue.
        let schema_version = if payload.carries_venue_contract() {
            payload.required_schema_version()
        } else {
            1
        };
        let body = StoredBody {
            envelope: EventEnvelope {
                event_id: EventId::for_run(run_id, sequence),
                schema_version,
                stream_id: STREAM.to_owned(),
                source_id: source.as_ref().map(|(id, _)| id.clone()),
                source_clock_domain: source.as_ref().map(|_| "UTC".to_owned()),
                session_id: None,
                source_sequence: source.as_ref().map(|(_, seq)| *seq),
                correlation_id: None,
                causation_id: None,
                times: EventTimes {
                    source_time_utc: None,
                    received_time_utc: UtcNanos::new(0),
                    available_time: LogicalNanos::new(available),
                    recorded_time_utc: UtcNanos::new(0),
                },
                quality: BTreeSet::new(),
            },
            payload,
        };
        journal.append(body)?;
        Ok(())
    };

    append(
        EventPayload::AccountOpened {
            account_id: AccountId::new(1)?,
            starting_cash: Paise(1_000_000),
            trade_date: date(2026, 10, 28)?,
        },
        1,
        None,
    )?;
    // Displayed ask: 1 unit at 1005 paise.
    append(
        EventPayload::QuoteReceived {
            quote: quote(1, 1)?,
        },
        2,
        Some(("feed".to_owned(), 1)),
    )?;
    append(EventPayload::DecisionMade { decision }, 3, None)?;
    append(EventPayload::RiskAssessed { assessment }, 4, None)?;
    append(EventPayload::OrderCreated { order }, 5, None)?;
    // Forged fill: claims the whole 5-unit order from a 1-unit display.
    // Turnover = 5 * 1005 = 5025 paise; zero charges; buy cash is out.
    let fill = Fill {
        fill_id: FillId::new(1)?,
        order_id: forged_order_id,
        side: Side::Buy,
        quantity: Quantity::new(5)?,
        price: PricePaise::new(Paise(1_005))?,
        charges: ChargeComponents::default(),
        cash_delta: Paise(-5_025),
    };
    append(
        EventPayload::FillBooked {
            fill,
            next_status: OrderStatus::Filled,
        },
        6,
        None,
    )?;
    Ok(())
}

#[test]
fn engine_written_fills_carry_resolvable_quote_provenance() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(1, 1)?, times(10))?;
    submit(&mut engine, 1, 1, 1_005, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(1, 1)?, times(12))?;
    drop(engine);

    // Read the journal back: fills must carry schema version 2 and a
    // causation that resolves to a QuoteReceived with the same instrument.
    let journal = account_journal::Journal::open(directory.path().join("ledger"), STREAM)?;
    let (mut found_quote, mut found_fill) = (false, false);
    for record in journal.records() {
        for part in [&record.body.envelope, &record.body.envelope] {
            let _ = part;
        }
        if let EventPayload::QuoteReceived { .. } = &record.body.payload {
            found_quote = true;
        }
        if let EventPayload::FillBooked { .. } = &record.body.payload {
            found_fill = true;
            assert_eq!(record.body.envelope.schema_version, 2);
            let causation = record
                .body
                .envelope
                .causation_id
                .clone()
                .expect("v2 fill carries causation");
            let referenced = journal
                .records()
                .iter()
                .find(|entry| entry.body.envelope.event_id == causation)
                .expect("caused quote is in the journal");
            assert!(matches!(
                &referenced.body.payload,
                EventPayload::QuoteReceived { .. }
            ));
        }
    }
    assert!(found_quote && found_fill);
    Ok(())
}
