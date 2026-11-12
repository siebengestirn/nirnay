#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Fill-provenance closure: journal-level run-format policy, active-book
//! binding, and exact fill-price reproduction.
//!
//! A provenance journal (v2 genesis) rejects legacy fills, stale-quote
//! citations, and prices the matcher would never have generated. A legacy
//! journal (v1 genesis) replays under the pre-provenance contract only —
//! and rejects provenance events mixed into it.

mod common;

use std::collections::BTreeSet;

use account_journal::Journal;
use common::*;
use cost_model::{ChargeBook, ChargeRate};
use paper_engine::{Engine, EngineConfig};
use trading_core::{
    AccountId, ChargeComponents, DecisionRecord, EventEnvelope, EventId, EventPayload, EventTimes,
    Fill, FillId, InstrumentCatalog, JournalRef, LogicalNanos, Order, OrderId, OrderIntent,
    OrderStatus, Paise, PricePaise, Product, Quantity, QuoteSnapshot, RiskAssessment, RunId, Side,
    StoredBody, StrategyId, UtcNanos,
};

const RUN_ID: RunId = RunId::from_bytes([7u8; 16]);

struct Row {
    schema_version: u16,
    causation_sequence: Option<u64>,
    available: u64,
    source_sequence: Option<u64>,
    payload: EventPayload,
}

fn build_journal(path: &std::path::Path, rows: &[Row]) -> Result<(), Box<dyn std::error::Error>> {
    let mut journal = Journal::open(path, STREAM)?;
    let mut sequence = 0u64;
    for row in rows {
        sequence += 1;
        let event_id = EventId::for_run(RUN_ID, sequence);
        let body = StoredBody {
            envelope: EventEnvelope {
                event_id,
                schema_version: row.schema_version,
                stream_id: STREAM.to_owned(),
                source_id: row.source_sequence.map(|_| "feed".to_owned()),
                source_clock_domain: row.source_sequence.map(|_| "UTC".to_owned()),
                session_id: None,
                source_sequence: row.source_sequence,
                correlation_id: None,
                causation_id: row
                    .causation_sequence
                    .map(|source| EventId::for_run(RUN_ID, source)),
                times: EventTimes {
                    source_time_utc: None,
                    received_time_utc: UtcNanos::new(0),
                    available_time: LogicalNanos::new(row.available),
                    recorded_time_utc: UtcNanos::new(0),
                },
                quality: BTreeSet::new(),
            },
            payload: row.payload.clone(),
        };
        journal.append(body)?;
    }
    Ok(())
}

fn quote_at(
    ask_price: i64,
    ask_quantity: i64,
    bid_price: i64,
    bid_quantity: i64,
) -> Result<QuoteSnapshot, Box<dyn std::error::Error>> {
    custom_quote(
        Some(ask_price),
        ask_quantity,
        Some(bid_price),
        bid_quantity,
        trading_core::Continuity::Snapshot,
    )
}

fn custom_quote(
    ask_price: Option<i64>,
    ask_quantity: i64,
    bid_price: Option<i64>,
    bid_quantity: i64,
    continuity: trading_core::Continuity,
) -> Result<QuoteSnapshot, Box<dyn std::error::Error>> {
    Ok(QuoteSnapshot {
        instrument_id: instrument_id(),
        venue: trading_core::Venue::Nse,
        bid_price: bid_price
            .map(|value| PricePaise::new(Paise(value)))
            .transpose()?,
        bid_quantity: Quantity::new(bid_quantity)?,
        ask_price: ask_price
            .map(|value| PricePaise::new(Paise(value)))
            .transpose()?,
        ask_quantity: Quantity::new(ask_quantity)?,
        continuity,
    })
}

fn scaffold(
    side: Side,
    quantity: i64,
    limit_price: i64,
) -> Result<Vec<Row>, Box<dyn std::error::Error>> {
    scaffold_for_product(side, Product::Intraday, quantity, limit_price)
}

fn scaffold_for_product(
    side: Side,
    product: Product,
    quantity: i64,
    limit_price: i64,
) -> Result<Vec<Row>, Box<dyn std::error::Error>> {
    let intent = OrderIntent {
        intent_id: trading_core::IntentId::new(1)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: trading_core::AlgoId::new(3)?,
        instrument_id: instrument_id(),
        venue: trading_core::Venue::Nse,
        side,
        product,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(limit_price))?,
        reason: "forged intent".to_owned(),
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
        features_fixed_point: std::collections::BTreeMap::new(),
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
        intent,
        status: OrderStatus::Working,
        filled_quantity: Quantity::ZERO,
        filled_notional: Paise(0),
        average_fill_price: None,
        fill_count: 0,
    };
    Ok(vec![
        Row {
            // Venue-bearing payloads are schema version 3 in every run;
            // fill provenance is a separate, genesis-declared contract.
            schema_version: 3,
            causation_sequence: None,
            available: 3,
            source_sequence: None,
            payload: EventPayload::DecisionMade { decision },
        },
        Row {
            schema_version: 1,
            causation_sequence: None,
            available: 4,
            source_sequence: None,
            payload: EventPayload::RiskAssessed { assessment },
        },
        Row {
            schema_version: 3,
            causation_sequence: None,
            available: 5,
            source_sequence: None,
            payload: EventPayload::OrderCreated { order },
        },
    ])
}

fn genesis(schema_version: u16) -> Result<Row, Box<dyn std::error::Error>> {
    Ok(Row {
        schema_version,
        causation_sequence: (schema_version == 2).then_some(1),
        available: 1,
        source_sequence: None,
        payload: EventPayload::AccountOpened {
            account_id: AccountId::new(1)?,
            starting_cash: Paise(1_000_000),
            trade_date: date(2026, 10, 28)?,
        },
    })
}

fn quote_row(available: u64, source_sequence: u64, quote: QuoteSnapshot) -> Row {
    Row {
        schema_version: 3,
        causation_sequence: None,
        available,
        source_sequence: Some(source_sequence),
        payload: EventPayload::QuoteReceived { quote },
    }
}

fn fill_row(
    schema_version: u16,
    causation_sequence: Option<u64>,
    available: u64,
    side: Side,
    price: i64,
    cash_delta: i64,
) -> Result<Row, Box<dyn std::error::Error>> {
    Ok(Row {
        schema_version,
        causation_sequence,
        available,
        source_sequence: None,
        payload: EventPayload::FillBooked {
            fill: Fill {
                fill_id: FillId::new(1)?,
                order_id: OrderId::new(1)?,
                side,
                quantity: Quantity::new(1)?,
                price: PricePaise::new(Paise(price))?,
                charges: ChargeComponents::default(),
                cash_delta: Paise(cash_delta),
            },
            next_status: OrderStatus::Filled,
        },
    })
}

fn fill_row_with_fees(
    schema_version: u16,
    causation_sequence: Option<u64>,
    available: u64,
    side: Side,
    price: i64,
    cash_delta: i64,
    charges: ChargeComponents,
) -> Result<Row, Box<dyn std::error::Error>> {
    Ok(Row {
        schema_version,
        causation_sequence,
        available,
        source_sequence: None,
        payload: EventPayload::FillBooked {
            fill: Fill {
                fill_id: FillId::new(1)?,
                order_id: OrderId::new(1)?,
                side,
                quantity: Quantity::new(1)?,
                price: PricePaise::new(Paise(price))?,
                charges,
                cash_delta: Paise(cash_delta),
            },
            next_status: OrderStatus::Filled,
        },
    })
}

fn brokerage_charge(paise: i64) -> ChargeComponents {
    ChargeComponents {
        brokerage: Paise(paise),
        ..ChargeComponents::default()
    }
}

fn exchange_charge(paise: i64) -> ChargeComponents {
    ChargeComponents {
        exchange_transaction: Paise(paise),
        ..ChargeComponents::default()
    }
}

fn brokerage_and_gst_charges(brokerage: i64, gst: i64) -> ChargeComponents {
    ChargeComponents {
        brokerage: Paise(brokerage),
        gst: Paise(gst),
        ..ChargeComponents::default()
    }
}

fn zero_brokerage_config() -> Result<EngineConfig, Box<dyn std::error::Error>> {
    let mut rates = Vec::new();
    for product in [Product::Delivery, Product::Intraday] {
        for side in [Side::Buy, Side::Sell] {
            let mut rate = charge_rate(product, side)?;
            rate.brokerage_ppb = 0;
            rates.push(rate);
        }
    }
    let mut changed = config()?;
    changed.charge_book = ChargeBook::new(rates)?;
    Ok(changed)
}

fn distinct_rate_config() -> Result<EngineConfig, Box<dyn std::error::Error>> {
    let effective_from = date(2026, 1, 1)?;
    let rates = [
        (Product::Intraday, Side::Buy, 5_000_000),
        (Product::Intraday, Side::Sell, 7_000_000),
        (Product::Delivery, Side::Buy, 9_000_000),
        (Product::Delivery, Side::Sell, 11_000_000),
    ]
    .into_iter()
    .map(|(product, side, brokerage_ppb)| ChargeRate {
        effective_from,
        product,
        side,
        brokerage_ppb,
        stt_ppb: 0,
        exchange_ppb: 0,
        sebi_ppb: 0,
        stamp_ppb: 0,
        gst_ppb: 100_000_000,
    })
    .collect::<Vec<_>>();
    let mut changed = config()?;
    changed.charge_book = ChargeBook::new(rates)?;
    Ok(changed)
}

fn open_error(path: &std::path::Path, config: EngineConfig) -> String {
    match Engine::open(path, config, times(1)) {
        Err(error) => error.to_string(),
        Ok(_) => String::from("replay unexpectedly succeeded"),
    }
}

#[test]
fn legacy_journal_with_valid_fill_still_replays() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    let mut rows = vec![genesis(1)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row_with_fees(
        1,
        None,
        6,
        Side::Buy,
        1_005,
        -1_015,
        brokerage_charge(10),
    )?);
    build_journal(&path, &rows)?;

    let engine = Engine::open(&path, config()?, times(7))?;
    let view = account(&engine)?;
    assert_eq!(view.positions[0].quantity, 1);
    Ok(())
}

#[test]
fn provenance_journal_rejects_a_legacy_fill() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // A v1 fill in a provenance journal is a downgrade: the run declared
    // provenance at genesis, so every fill must carry it.
    let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(1, None, 6, Side::Buy, 1_005, -1_005)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("legacy fill in a provenance journal"));
    Ok(())
}

#[test]
fn legacy_journal_rejects_a_provenance_event() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // Mixed journals are not a migration: a v2 fill inside a legacy
    // journal is rejected rather than partially validated.
    let mut rows = vec![genesis(1)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(2), 6, Side::Buy, 1_005, -1_005)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("provenance event in a legacy journal"));
    Ok(())
}

#[test]
fn fill_citing_a_stale_quote_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // Quote A (ask 1005) is replaced by quote B (ask 1010); a fill at
    // 1005 citing A would consume B's budget under the old existence
    // check. The active-book binding rejects it.
    let mut rows = vec![
        genesis(2)?,
        quote_row(2, 1, quote_at(1_005, 1, 995, 1)?),
        quote_row(3, 2, quote_at(1_010, 1, 995, 1)?),
    ];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(2), 7, Side::Buy, 1_005, -1_005)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("not the active book quote"));
    Ok(())
}

#[test]
fn fill_citing_the_active_book_passes() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // Control for the stale-citation test: the same journal shape with
    // the fill citing the active quote at its price replays cleanly.
    let mut rows = vec![
        genesis(2)?,
        quote_row(2, 1, quote_at(1_005, 1, 995, 1)?),
        quote_row(3, 2, quote_at(1_010, 1, 995, 1)?),
    ];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row_with_fees(
        2,
        Some(3),
        7,
        Side::Buy,
        1_010,
        -1_020,
        brokerage_charge(10),
    )?);
    build_journal(&path, &rows)?;

    let engine = Engine::open(&path, config()?, times(7))?;
    assert_eq!(account(&engine)?.positions[0].quantity, 1);
    Ok(())
}

#[test]
fn fill_beyond_configured_slippage_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // Test profile: tick 5, slippage 0. A buy at 1010 — one tick above
    // the 1005 ask and still under the 1020 limit — is a price the
    // matcher would never generate.
    let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(2), 6, Side::Buy, 1_010, -1_010)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("slippage"));
    Ok(())
}

#[test]
fn sell_fill_beyond_configured_slippage_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // Sell at 990 — one tick below the 995 bid, still above the 985
    // limit — exceeds the configured slippage in the sell direction.
    let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Sell, 1, 985)?);
    rows.push(fill_row(2, Some(2), 6, Side::Sell, 990, 990)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("slippage"));
    Ok(())
}

#[test]
fn fill_with_understated_fees_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // The shared fixture charges 1% brokerage, so this otherwise-valid,
    // zero-fee fill understates the schedule.
    let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(2), 6, Side::Buy, 1_005, -1_005)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("fill fees do not match the charge schedule"));
    Ok(())
}

#[test]
fn fill_with_wrong_fee_component_allocation_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // Same total charges (10) and cash impact, wrong component: replay
    // compares each component, not only the balanced total.
    let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row_with_fees(
        2,
        Some(2),
        6,
        Side::Buy,
        1_005,
        -1_015,
        exchange_charge(10),
    )?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("fill fees do not match the charge schedule"));
    Ok(())
}

#[test]
fn reopening_with_a_changed_fee_schedule_fails() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(1, 1)?, times(10))?;
    submit(&mut engine, 1, 1, 1_005, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(1, 1)?, times(12))?;
    drop(engine);

    let error = Engine::open(
        directory.path().join("ledger"),
        zero_brokerage_config()?,
        times(20),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("fill fees do not match"),
        "{error}"
    );
    Ok(())
}

#[test]
fn legacy_journal_rejects_fill_under_changed_fee_schedule() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // Legacy compatibility covers fill provenance, not the fee
    // configuration: a fee-bearing v1 fill opened with the schedule that
    // produced it replays, but a changed schedule rejects it.
    let mut rows = vec![genesis(1)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row_with_fees(
        1,
        None,
        6,
        Side::Buy,
        1_005,
        -1_015,
        brokerage_charge(10),
    )?);
    build_journal(&path, &rows)?;

    {
        let engine = Engine::open(&path, config()?, times(7))?;
        assert_eq!(account(&engine)?.positions[0].quantity, 1);
    }
    let error = Engine::open(&path, zero_brokerage_config()?, times(20)).unwrap_err();
    assert!(
        error.to_string().contains("fill fees do not match"),
        "{error}"
    );
    Ok(())
}

#[test]
fn fee_schedule_selection_covers_product_side_and_gst() -> TestResult {
    struct FeeCase {
        product: Product,
        side: Side,
        price: i64,
        limit_price: i64,
        correct: ChargeComponents,
        correct_cash_delta: i64,
        wrong: ChargeComponents,
        wrong_cash_delta: i64,
    }

    // Distinct brokerage rates are paired with a 10% GST rate. At these
    // turnovers, every case yields a nonzero GST component, so replay
    // cannot match a schedule by coincidence.
    let cases = [
        FeeCase {
            product: Product::Intraday,
            side: Side::Buy,
            price: 10_050,
            limit_price: 10_100,
            correct: brokerage_and_gst_charges(50, 5),
            correct_cash_delta: -10_105,
            wrong: brokerage_and_gst_charges(90, 9),
            wrong_cash_delta: -10_149,
        },
        FeeCase {
            product: Product::Intraday,
            side: Side::Sell,
            price: 9_950,
            limit_price: 9_900,
            correct: brokerage_and_gst_charges(70, 7),
            correct_cash_delta: 9_873,
            wrong: brokerage_and_gst_charges(109, 11),
            wrong_cash_delta: 9_830,
        },
        FeeCase {
            product: Product::Delivery,
            side: Side::Buy,
            price: 10_050,
            limit_price: 10_100,
            correct: brokerage_and_gst_charges(90, 9),
            correct_cash_delta: -10_149,
            wrong: brokerage_and_gst_charges(50, 5),
            wrong_cash_delta: -10_105,
        },
        FeeCase {
            product: Product::Delivery,
            side: Side::Sell,
            price: 9_950,
            limit_price: 9_900,
            correct: brokerage_and_gst_charges(109, 11),
            correct_cash_delta: 9_830,
            wrong: brokerage_and_gst_charges(70, 7),
            wrong_cash_delta: 9_873,
        },
    ];

    for case in cases {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("ledger");
        let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(10_050, 1, 9_950, 1)?)];
        rows.extend(scaffold_for_product(
            case.side,
            case.product,
            1,
            case.limit_price,
        )?);
        rows.push(fill_row_with_fees(
            2,
            Some(2),
            6,
            case.side,
            case.price,
            case.correct_cash_delta,
            case.correct,
        )?);
        build_journal(&path, &rows)?;
        Engine::open(&path, distinct_rate_config()?, times(7))?;

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("ledger");
        let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(10_050, 1, 9_950, 1)?)];
        rows.extend(scaffold_for_product(
            case.side,
            case.product,
            1,
            case.limit_price,
        )?);
        rows.push(fill_row_with_fees(
            2,
            Some(2),
            6,
            case.side,
            case.price,
            case.wrong_cash_delta,
            case.wrong,
        )?);
        build_journal(&path, &rows)?;
        assert!(
            open_error(&path, distinct_rate_config()?)
                .contains("fill fees do not match the charge schedule")
        );
    }
    Ok(())
}

#[test]
fn reopening_with_a_different_slippage_profile_fails() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(1, 1)?, times(10))?;
    submit(&mut engine, 1, 1, 1_005, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(1, 1)?, times(12))?;
    drop(engine);

    // The journal was priced with slippage 0; reopening under slippage 2
    // must fail rather than re-price history.
    let mut changed: EngineConfig = config()?;
    changed.paper_profile.slippage_ticks = 2;
    let error = Engine::open(directory.path().join("ledger"), changed, times(20)).unwrap_err();
    assert!(error.to_string().contains("slippage"), "{error}");
    Ok(())
}

#[test]
fn reopening_with_a_different_tick_size_fails() -> TestResult {
    let directory = tempfile::tempdir()?;
    // Price history on a 5-paise tick with 1 tick of slippage: fills land
    // one tick beyond the display. Reopening against a 10-paise tick
    // catalog would price the same rule one tick further — the exact
    // reproduction fails.
    let mut profiled = config()?;
    profiled.paper_profile.slippage_ticks = 1;
    let mut engine = running_engine_with(&directory, profiled)?;
    engine.on_quote("feed", 1, quote(1, 1)?, times(10))?;
    let order = submit(&mut engine, 1, 1, 1_020, Side::Buy, 11)?;
    engine.on_quote("feed", 2, quote(1, 1)?, times(12))?;
    assert_eq!(
        engine.order(order).expect("order exists").status,
        OrderStatus::Filled
    );
    drop(engine);

    let mut instrument = instrument()?;
    instrument.listings[0].terms[0].tick_assignment.tick_paise = 10;
    let mut changed: EngineConfig = config()?;
    changed.paper_profile.slippage_ticks = 1;
    changed.instruments = InstrumentCatalog::new(vec![instrument])?;
    let error = Engine::open(directory.path().join("ledger"), changed, times(20)).unwrap_err();
    assert!(error.to_string().contains("slippage"), "{error}");
    Ok(())
}

#[test]
fn fills_funded_by_one_quote_share_its_event_id() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;
    engine.on_quote("feed", 1, quote(2, 1)?, times(10))?;
    submit(&mut engine, 1, 1, 1_005, Side::Buy, 11)?;
    submit(&mut engine, 2, 1, 1_005, Side::Buy, 12)?;
    engine.on_quote("feed", 2, quote(2, 1)?, times(14))?;
    drop(engine);

    let journal = Journal::open(directory.path().join("ledger"), STREAM)?;
    let quote_events: Vec<_> = journal
        .records()
        .iter()
        .filter(|record| matches!(record.body.payload, EventPayload::QuoteReceived { .. }))
        .map(|record| record.body.envelope.event_id.clone())
        .collect();
    let fill_causations: Vec<_> = journal
        .records()
        .iter()
        .filter(|record| matches!(record.body.payload, EventPayload::FillBooked { .. }))
        .map(|record| {
            record
                .body
                .envelope
                .causation_id
                .clone()
                .expect("fill carries provenance")
        })
        .collect();
    assert_eq!(fill_causations.len(), 2);
    assert_eq!(fill_causations[0], fill_causations[1]);
    // Both fills name the quote that currently owns the book: the latest
    // applied quote event for the instrument.
    assert_eq!(fill_causations[0], *quote_events.last().unwrap());
    Ok(())
}

#[test]
fn v2_genesis_with_unrelated_causation_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    let mut row = genesis(2)?;
    row.causation_sequence = Some(99);
    let error =
        build_journal(&path, &[row]).expect_err("unrelated genesis causation must be rejected");
    assert!(error.to_string().contains("self-causative"), "{error}");
    Ok(())
}

#[test]
fn fill_against_gapped_delta_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // A Snapshot initializes the source at sequence 1. Sequence 3 is
    // therefore a gapped Delta: it updates the book but leaves the source
    // ineligible until recovery.
    let mut rows = vec![
        genesis(2)?,
        quote_row(2, 1, quote_at(1_005, 1, 995, 1)?),
        quote_row(
            3,
            3,
            custom_quote(
                Some(1_005),
                1,
                Some(995),
                1,
                trading_core::Continuity::Delta,
            )?,
        ),
    ];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(3), 7, Side::Buy, 1_005, -1_005)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("not healthy"));
    Ok(())
}

#[test]
fn fill_after_quote_becomes_stale_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    // The shared risk policy permits a maximum quote age of 1000 ns. The
    // fill's availability time is later than that after the quote.
    let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(1_005, 1, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(2), 2_000, Side::Buy, 1_005, -1_005)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("not healthy"));
    Ok(())
}

#[test]
fn fill_against_crossed_book_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(995, 1, 1_000, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(2), 6, Side::Buy, 995, -995)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("not healthy"));
    Ok(())
}

#[test]
fn fill_against_one_sided_book_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    let mut rows = vec![
        genesis(2)?,
        quote_row(
            2,
            1,
            custom_quote(None, 0, Some(995), 1, trading_core::Continuity::Snapshot)?,
        ),
    ];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(2), 6, Side::Buy, 1_005, -1_005)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("not healthy"));
    Ok(())
}

#[test]
fn fill_against_incomplete_book_is_rejected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("ledger");
    let mut rows = vec![genesis(2)?, quote_row(2, 1, quote_at(1_005, 0, 995, 1)?)];
    rows.extend(scaffold(Side::Buy, 1, 1_020)?);
    rows.push(fill_row(2, Some(2), 6, Side::Buy, 1_005, -1_005)?);
    build_journal(&path, &rows)?;

    assert!(open_error(&path, config()?).contains("not healthy"));
    Ok(())
}
