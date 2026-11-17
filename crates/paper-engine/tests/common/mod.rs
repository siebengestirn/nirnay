#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

//! Shared fixtures for paper-engine integration tests.
//!
//! One synthetic instrument, a 1% brokerage book and a permissive risk policy
//! keep each scenario focused on lifecycle behaviour rather than setup.

use std::collections::{BTreeMap, BTreeSet};

use cost_model::{ChargeBook, ChargeRate};
use paper_engine::{Engine, EngineConfig, PaperProfile, ReconcileInput};
use risk_engine::RiskPolicy;
use tempfile::TempDir;
use trading_core::{
    AccountId, AlgoId, Continuity, DecisionRecord, DomainError, EventTimes, Instrument,
    InstrumentCatalog, InstrumentId, InstrumentType, IntentId, JournalRef, Listing, ListingStatus,
    ListingTerms, LogicalNanos, OrderId, Paise, PricePaise, Product, Quantity, QuoteSnapshot,
    ReconciliationStatus, RunId, Segment, SessionPhase, SettlementRegime, Side, StrategyId,
    TickAssignment, TradeDate, UtcNanos, Venue,
};

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

pub const RUN: RunId = RunId::from_bytes([
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
]);
pub const STREAM: &str = "paper-engine-test";
pub const STARTING_CASH: i64 = 1_000_000;

pub fn instrument_id() -> InstrumentId {
    InstrumentId::new("nse.cash.synidx").expect("synthetic test instrument id is valid")
}

pub fn date(year: i32, month: u32, day: u32) -> Result<TradeDate, DomainError> {
    TradeDate::new(year as i16, month as i8, day as i8)
        .ok()
        .ok_or_else(|| DomainError::InvalidEventTime("invalid calendar date".to_owned()))
}

pub fn times(available: u64) -> EventTimes {
    EventTimes {
        source_time_utc: None,
        received_time_utc: UtcNanos::new(available as i64),
        available_time: LogicalNanos::new(available),
        recorded_time_utc: UtcNanos::new(available as i64),
    }
}

pub fn instrument() -> Result<Instrument, DomainError> {
    Ok(Instrument {
        instrument_id: instrument_id(),
        instrument_type: InstrumentType::Index,
        isin: None,
        name: "Synthetic index".to_owned(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: Segment::Cash,
            venue_symbol: "SYNIDX".to_owned(),
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
                    tick_paise: 5,
                    reference_close_paise: None,
                    reference_close_date: None,
                    reference_source: None,
                    reference_sha256_hex: None,
                    effective_month: None,
                    schedule_revision: "test-fixture".to_owned(),
                    schedule_effective_from: None,
                    schedule_sha256_hex: None,
                    calendar_revision: None,
                    calendar_sha256_hex: None,
                },
                market_lot: 1,
                quantity_increment: 1,
            }],
        }],
        corporate_actions: Vec::new(),
    })
}

pub fn policy() -> Result<RiskPolicy, DomainError> {
    Ok(RiskPolicy {
        version: "risk-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(1)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(2)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(3)?]),
        allowed_instruments: BTreeSet::from([instrument_id()]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(100_000),
        max_position_quantity: Quantity::new(10)?,
        max_gross_exposure_paise: Paise(100_000),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 5,
        max_orders_per_second: 5,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: false,
    })
}

pub fn charge_rate(product: Product, side: Side) -> Result<ChargeRate, DomainError> {
    Ok(ChargeRate {
        effective_from: date(2026, 1, 1)?,
        product,
        side,
        // 1% brokerage on turnover, rounded to the nearest paise half up.
        brokerage_ppb: 10_000_000,
        stt_ppb: 0,
        exchange_ppb: 0,
        sebi_ppb: 0,
        stamp_ppb: 0,
        gst_ppb: 0,
    })
}

pub fn config() -> Result<EngineConfig, Box<dyn std::error::Error>> {
    let mut rates = Vec::new();
    for product in [Product::Delivery, Product::Intraday] {
        for side in [Side::Buy, Side::Sell] {
            rates.push(charge_rate(product, side)?);
        }
    }
    Ok(EngineConfig {
        stream_id: STREAM.to_owned(),
        run_id: RUN,
        account_id: AccountId::new(1)?,
        starting_cash: Paise(STARTING_CASH),
        trade_date: date(2026, 10, 28)?,
        instruments: InstrumentCatalog::new(vec![instrument()?])?,
        risk_policy: policy()?,
        charge_book: ChargeBook::new(rates)?,
        paper_profile: PaperProfile {
            slippage_ticks: 0,
            consume_displayed_liquidity_only: true,
        },
    })
}

pub fn quote(ask_quantity: i64, bid_quantity: i64) -> Result<QuoteSnapshot, DomainError> {
    Ok(QuoteSnapshot {
        instrument_id: instrument_id(),
        venue: trading_core::Venue::Nse,
        bid_price: Some(PricePaise::new(Paise(995))?),
        bid_quantity: Quantity::new(bid_quantity)?,
        ask_price: Some(PricePaise::new(Paise(1_005))?),
        ask_quantity: Quantity::new(ask_quantity)?,
        continuity: Continuity::Snapshot,
    })
}

pub fn delta_quote(ask_quantity: i64, bid_quantity: i64) -> Result<QuoteSnapshot, DomainError> {
    let mut value = quote(ask_quantity, bid_quantity)?;
    value.continuity = Continuity::Delta;
    Ok(value)
}

pub fn decision(
    intent: u64,
    evidence: Vec<JournalRef>,
    quantity: i64,
    limit_price: i64,
    side: Side,
    rationale: &str,
) -> Result<DecisionRecord, DomainError> {
    let intent = trading_core::OrderIntent {
        intent_id: IntentId::new(intent)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(3)?,
        instrument_id: instrument_id(),
        venue: trading_core::Venue::Nse,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(limit_price))?,
        reason: "test intent".to_owned(),
        evidence: evidence.clone(),
        expires_at: LogicalNanos::new(10_000_000),
    };
    Ok(DecisionRecord {
        strategy_id: StrategyId::new(2)?,
        strategy_version: "strategy-v1".to_owned(),
        source_build_hash: "a".repeat(64),
        configuration_hash: "b".repeat(64),
        dataset_hash: "c".repeat(64),
        input_refs: evidence,
        features_fixed_point: BTreeMap::new(),
        rationale: rationale.to_owned(),
        intent: Some(intent),
    })
}

pub fn evidence(engine: &Engine) -> Result<Vec<JournalRef>, Box<dyn std::error::Error>> {
    let record = engine.records().last().ok_or("journal has no records")?;
    Ok(vec![JournalRef {
        sequence: record.sequence,
        hash_hex: record.hash.hex(),
    }])
}

pub fn running_engine(directory: &TempDir) -> Result<Engine, Box<dyn std::error::Error>> {
    running_engine_with(directory, config()?)
}

pub fn running_engine_with(
    directory: &TempDir,
    config: EngineConfig,
) -> Result<Engine, Box<dyn std::error::Error>> {
    let mut engine = Engine::open(directory.path().join("ledger"), config, times(1))?;
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

pub fn submit(
    engine: &mut Engine,
    intent: u64,
    quantity: i64,
    limit_price: i64,
    side: Side,
    decision_time: u64,
) -> Result<OrderId, Box<dyn std::error::Error>> {
    let evidence = evidence(engine)?;
    let decision = decision(
        intent,
        evidence,
        quantity,
        limit_price,
        side,
        "test rationale",
    )?;
    let outcome = engine.submit_decision(decision, times(decision_time))?;
    outcome.order_id.ok_or_else(|| "order was rejected".into())
}

pub fn account(engine: &Engine) -> Result<paper_engine::AccountView, Box<dyn std::error::Error>> {
    engine
        .account_view()
        .ok_or_else(|| "account view missing".into())
}
