#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Cross-venue execution contract: an order may only be filled by a quote
//! observed on its own exchange.
//!
//! The same instrument can list on NSE and BSE with different tick sizes,
//! market lots and prices. Books, displayed liquidity, feed health, trading
//! terms and replay must therefore be scoped by venue at every point of the
//! matching path.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{STARTING_CASH, TestResult, charge_rate, date, times};
use cost_model::ChargeBook;
use paper_engine::{Engine, EngineConfig, PaperProfile, ReconcileInput};
use risk_engine::RiskPolicy;
use trading_core::{
    AccountId, AlgoId, Continuity, DecisionRecord, Instrument, InstrumentCatalog, InstrumentId,
    InstrumentType, IntentId, JournalRef, Listing, ListingStatus, ListingTerms, LogicalNanos,
    OrderId, OrderIntent, OrderStatus, Paise, PricePaise, Product, Quantity, QuoteSnapshot,
    ReconciliationStatus, RunId, Segment, SessionPhase, SettlementRegime, Side, StrategyId,
    TickAssignment, Venue,
};

const RUN: RunId = RunId::from_bytes([
    0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00, 0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88,
]);

/// NSE and BSE terms differ by more than price: NSE has a
/// 5-paise tick and a 1-unit lot, BSE a 1-paise tick and a 5-unit lot.
fn venue_terms(venue: Venue) -> (i64, i64) {
    match venue {
        Venue::Nse => (5, 1),
        Venue::Bse => (1, 5),
    }
}

/// Bid/ask displayed prices differ per venue, so a fill at the wrong venue's
/// book would be visible in the recorded price.
fn venue_prices(venue: Venue) -> (i64, i64) {
    match venue {
        Venue::Nse => (995, 1_005),
        Venue::Bse => (1_095, 1_105),
    }
}

fn instrument_id() -> InstrumentId {
    InstrumentId::new("dual.cash.synidx").expect("synthetic dual-venue instrument id is valid")
}

fn listing(venue: Venue, symbol: &str) -> Result<Listing, trading_core::DomainError> {
    let (tick, lot) = venue_terms(venue);
    Ok(Listing {
        venue,
        segment: Segment::Cash,
        venue_symbol: symbol.to_owned(),
        token: None,
        effective_from: date(2026, 1, 1)?,
        effective_to: Some(date(2026, 12, 31)?),
        status: ListingStatus::Active,
        settlement_regime: SettlementRegime::Standard,
        z_category: false,
        settlement_group: None,
        terms: vec![ListingTerms {
            effective_from: date(2026, 1, 1)?,
            effective_to: Some(date(2026, 12, 31)?),
            tick_assignment: TickAssignment {
                tick_paise: tick,
                reference_close_paise: None,
                reference_close_date: None,
                reference_source: None,
                reference_sha256_hex: None,
                effective_month: None,
                schedule_revision: "dual-venue-fixture".to_owned(),
                schedule_effective_from: None,
                schedule_sha256_hex: None,
                calendar_revision: None,
                calendar_sha256_hex: None,
            },
            market_lot: lot,
            quantity_increment: lot,
        }],
    })
}

fn instrument() -> Result<Instrument, trading_core::DomainError> {
    Ok(Instrument {
        instrument_id: instrument_id(),
        instrument_type: InstrumentType::Index,
        isin: None,
        name: "Dual-venue synthetic index".to_owned(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![
            listing(Venue::Nse, "DUALNSE")?,
            listing(Venue::Bse, "DUALBSE")?,
        ],
        corporate_actions: Vec::new(),
    })
}

fn policy() -> Result<RiskPolicy, trading_core::DomainError> {
    Ok(RiskPolicy {
        version: "risk-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(1)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(2)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(3)?]),
        allowed_instruments: BTreeSet::from([instrument_id()]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(100_000),
        max_position_quantity: Quantity::new(100)?,
        max_gross_exposure_paise: Paise(1_000_000),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 5,
        max_orders_per_second: 5,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: false,
    })
}

fn config() -> Result<EngineConfig, Box<dyn std::error::Error>> {
    let mut rates = Vec::new();
    for product in [Product::Delivery, Product::Intraday] {
        for side in [Side::Buy, Side::Sell] {
            rates.push(charge_rate(product, side)?);
        }
    }
    Ok(EngineConfig {
        stream_id: "paper-engine-venue-test".to_owned(),
        run_id: RUN,
        account_id: AccountId::new(1)?,
        starting_cash: Paise(STARTING_CASH),
        trade_date: date(2026, 10, 28)?,
        instruments: InstrumentCatalog::new(vec![instrument()?])?,
        risk_policy: policy()?,
        charge_book: ChargeBook::new(rates)?,
        // One tick of slippage makes the fill price depend on the venue's
        // assigned tick: NSE 1005 + 5 = 1010, BSE 1105 + 1 = 1106.
        paper_profile: PaperProfile {
            slippage_ticks: 1,
            consume_displayed_liquidity_only: true,
        },
    })
}

fn quote(
    venue: Venue,
    ask_quantity: i64,
    bid_quantity: i64,
    continuity: Continuity,
) -> Result<QuoteSnapshot, trading_core::DomainError> {
    let (bid, ask) = venue_prices(venue);
    Ok(QuoteSnapshot {
        instrument_id: instrument_id(),
        venue,
        bid_price: Some(PricePaise::new(Paise(bid))?),
        bid_quantity: Quantity::new(bid_quantity)?,
        ask_price: Some(PricePaise::new(Paise(ask))?),
        ask_quantity: Quantity::new(ask_quantity)?,
        continuity,
    })
}

fn limit_price(venue: Venue) -> i64 {
    match venue {
        Venue::Nse => 1_015,
        Venue::Bse => 1_115,
    }
}

fn decision(
    intent_id: u64,
    venue: Venue,
    quantity: i64,
    side: Side,
    evidence: Vec<JournalRef>,
    available_time: u64,
) -> Result<DecisionRecord, trading_core::DomainError> {
    let intent = OrderIntent {
        intent_id: IntentId::new(intent_id)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(3)?,
        instrument_id: instrument_id(),
        venue,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(limit_price(venue)))?,
        reason: "cross-venue test intent".to_owned(),
        evidence: evidence.clone(),
        expires_at: LogicalNanos::new(available_time + 1_000_000),
    };
    Ok(DecisionRecord {
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        source_build_hash: "a".repeat(64),
        configuration_hash: "b".repeat(64),
        dataset_hash: "c".repeat(64),
        input_refs: evidence,
        features_fixed_point: BTreeMap::new(),
        rationale: "cross-venue rationale".to_owned(),
        intent: Some(intent),
    })
}

fn submit(
    engine: &mut Engine,
    intent_id: u64,
    venue: Venue,
    quantity: i64,
    side: Side,
    available_time: u64,
) -> Result<OrderId, Box<dyn std::error::Error>> {
    let evidence = engine
        .quote_reference(&instrument_id(), venue)
        .map(|reference| vec![reference])
        .ok_or("the venue has no quote reference")?;
    let order = decision(intent_id, venue, quantity, side, evidence, available_time)?;
    engine
        .submit_decision(order, times(available_time))?
        .order_id
        .ok_or_else(|| "order was rejected".into())
}

fn running_engine(directory: &tempfile::TempDir) -> Result<Engine, Box<dyn std::error::Error>> {
    let mut engine = Engine::open(directory.path().join("ledger"), config()?, times(1))?;
    engine.set_session_phase(SessionPhase::Continuous, "cal-v1", times(2))?;
    engine.observe_risk_state(Paise(0), true, times(3))?;
    let report = engine.reconcile(
        ReconcileInput {
            positions: Some(BTreeMap::new()),
            active_orders: Some(BTreeSet::new()),
        },
        "initial",
        times(4),
    )?;
    assert_eq!(report.status, ReconciliationStatus::Matched);
    engine.set_kill_switch(false, 7, "test release", times(5))?;
    Ok(engine)
}

fn order_status(engine: &Engine, order_id: OrderId) -> (OrderStatus, i64) {
    let order = engine
        .order(order_id)
        .expect("submitted order is projected");
    (order.status, order.filled_quantity.get())
}

fn fill_price(engine: &Engine, order_id: OrderId) -> i64 {
    engine
        .fills()
        .values()
        .find(|fill| fill.order_id == order_id)
        .unwrap_or_else(|| panic!("order {order_id:?} has no fill"))
        .price
        .get()
        .0
}

#[test]
fn a_quote_fills_only_orders_on_its_own_venue() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;

    // Build a two-sided book on each exchange. Each venue has its own feed
    // source and sequence space, its own prices and its own tick/lot terms.
    engine.on_quote(
        "nse-feed",
        1,
        quote(Venue::Nse, 5, 5, Continuity::Snapshot)?,
        times(10),
    )?;
    engine.on_quote(
        "bse-feed",
        1,
        quote(Venue::Bse, 5, 5, Continuity::Snapshot)?,
        times(11),
    )?;

    // The BSE lot is 5: a one-unit BSE order is rejected before matching.
    assert!(
        submit(&mut engine, 99, Venue::Bse, 1, Side::Buy, 12).is_err(),
        "BSE market lot 5 must reject a one-unit order"
    );

    // One working buy order per exchange, each at its own quantity.
    let nse_order = submit(&mut engine, 1, Venue::Nse, 1, Side::Buy, 12)?;
    let bse_order = submit(&mut engine, 2, Venue::Bse, 5, Side::Buy, 13)?;
    assert_eq!(order_status(&engine, nse_order), (OrderStatus::Working, 0));
    assert_eq!(order_status(&engine, bse_order), (OrderStatus::Working, 0));

    // An NSE quote must fill the NSE order and leave the BSE order untouched.
    engine.on_quote(
        "nse-feed",
        2,
        quote(Venue::Nse, 5, 5, Continuity::Delta)?,
        times(14),
    )?;
    assert_eq!(order_status(&engine, nse_order), (OrderStatus::Filled, 1));
    assert_eq!(
        fill_price(&engine, nse_order),
        1_010,
        "NSE fill uses the NSE tick: 1005 + 5"
    );
    assert_eq!(
        order_status(&engine, bse_order),
        (OrderStatus::Working, 0),
        "an NSE quote must never fill a BSE order"
    );

    // A fresh NSE order stays working when the next quote comes from BSE.
    let nse_order_2 = submit(&mut engine, 3, Venue::Nse, 1, Side::Buy, 15)?;
    engine.on_quote(
        "bse-feed",
        2,
        quote(Venue::Bse, 5, 5, Continuity::Delta)?,
        times(16),
    )?;
    assert_eq!(order_status(&engine, bse_order), (OrderStatus::Filled, 5));
    assert_eq!(
        fill_price(&engine, bse_order),
        1_106,
        "BSE fill uses the BSE tick: 1105 + 1"
    );
    assert_eq!(
        order_status(&engine, nse_order_2),
        (OrderStatus::Working, 0),
        "a BSE quote must never fill an NSE order"
    );

    // Only a quote from its own venue completes the remaining NSE order.
    engine.on_quote(
        "nse-feed",
        3,
        quote(Venue::Nse, 5, 5, Continuity::Delta)?,
        times(17),
    )?;
    assert_eq!(order_status(&engine, nse_order_2), (OrderStatus::Filled, 1));
    assert_eq!(fill_price(&engine, nse_order_2), 1_010);
    Ok(())
}

#[test]
fn restart_reconstructs_venue_fills_and_liquidity() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut engine = running_engine(&directory)?;

    engine.on_quote(
        "nse-feed",
        1,
        quote(Venue::Nse, 5, 5, Continuity::Snapshot)?,
        times(10),
    )?;
    engine.on_quote(
        "bse-feed",
        1,
        quote(Venue::Bse, 5, 5, Continuity::Snapshot)?,
        times(11),
    )?;
    let nse_order = submit(&mut engine, 1, Venue::Nse, 5, Side::Buy, 12)?;
    let bse_order = submit(&mut engine, 2, Venue::Bse, 5, Side::Buy, 13)?;
    engine.on_quote(
        "nse-feed",
        2,
        quote(Venue::Nse, 5, 5, Continuity::Delta)?,
        times(14),
    )?;
    engine.on_quote(
        "bse-feed",
        2,
        quote(Venue::Bse, 5, 5, Continuity::Delta)?,
        times(16),
    )?;
    assert_eq!(order_status(&engine, nse_order), (OrderStatus::Filled, 5));
    assert_eq!(order_status(&engine, bse_order), (OrderStatus::Filled, 5));
    assert_eq!(
        engine.positions().get(&instrument_id()).copied(),
        Some(10),
        "positions are instrument-level across both venues"
    );
    engine.set_kill_switch(true, 8, "restart", times(20))?;
    drop(engine);

    // Replay must reproduce each venue's fills, prices and consumed budgets.
    let mut reopened = Engine::open(directory.path().join("ledger"), config()?, times(20))?;
    reopened.set_kill_switch(false, 9, "restart release", times(21))?;
    assert_eq!(order_status(&reopened, nse_order), (OrderStatus::Filled, 5));
    assert_eq!(order_status(&reopened, bse_order), (OrderStatus::Filled, 5));
    assert_eq!(fill_price(&reopened, nse_order), 1_010);
    assert_eq!(fill_price(&reopened, bse_order), 1_106);
    assert_eq!(
        reopened.positions().get(&instrument_id()).copied(),
        Some(10)
    );

    // Re-quoting the same displayed size releases no liquidity on either
    // venue: both budgets stay exhausted after replay.
    let nse_again = submit(&mut reopened, 3, Venue::Nse, 5, Side::Buy, 22)?;
    let bse_again = submit(&mut reopened, 4, Venue::Bse, 5, Side::Buy, 23)?;
    reopened.on_quote(
        "nse-feed",
        3,
        quote(Venue::Nse, 5, 5, Continuity::Delta)?,
        times(24),
    )?;
    reopened.on_quote(
        "bse-feed",
        3,
        quote(Venue::Bse, 5, 5, Continuity::Delta)?,
        times(25),
    )?;
    assert_eq!(
        order_status(&reopened, nse_again),
        (OrderStatus::Working, 0)
    );
    assert_eq!(
        order_status(&reopened, bse_again),
        (OrderStatus::Working, 0)
    );

    // A genuine displayed-size increase releases one new budget per venue,
    // and each fill again uses that venue's tick.
    reopened.on_quote(
        "nse-feed",
        4,
        quote(Venue::Nse, 10, 5, Continuity::Delta)?,
        times(26),
    )?;
    reopened.on_quote(
        "bse-feed",
        4,
        quote(Venue::Bse, 10, 5, Continuity::Delta)?,
        times(27),
    )?;
    assert_eq!(order_status(&reopened, nse_again), (OrderStatus::Filled, 5));
    assert_eq!(order_status(&reopened, bse_again), (OrderStatus::Filled, 5));
    assert_eq!(fill_price(&reopened, nse_again), 1_010);
    assert_eq!(fill_price(&reopened, bse_again), 1_106);
    assert_eq!(
        reopened.positions().get(&instrument_id()).copied(),
        Some(20)
    );
    Ok(())
}
