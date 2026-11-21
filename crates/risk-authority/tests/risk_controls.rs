#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Typed fixture falsifiers for the institutional risk controls. All sectors,
//! depth, marks, greeks and borrow values are synthetic; no real master or live
//! instrument is referenced.

use std::collections::{
    BTreeMap, //
    BTreeSet,
};

use risk_authority::{
    AccountJournalObservation, //
    AuthorityError,
    FeedObservation,
    MarketObservation,
    Observation,
    ObservationSource,
    Observations,
    OmsObservation,
    OperatorObservation,
    OpsObservation,
    ReconciliationObservation,
    RiskAuthority,
};
use risk_engine::{
    GreekLimits, //
    RiskPolicy,
    StressScenario,
    ValidatedRiskPolicy,
};
use trading_core::{
    AccountId, //
    AlgoId,
    BorrowAvailability,
    Continuity,
    Greeks,
    Instrument,
    InstrumentId,
    InstrumentType,
    IntentId,
    JournalRef,
    LiquidityDepth,
    Listing,
    ListingStatus,
    ListingTerms,
    LogicalNanos,
    OrderIntent,
    Paise,
    PricePaise,
    Product,
    Quantity,
    QuoteSnapshot,
    ReconciliationReport,
    ReconciliationStatus,
    RiskCheck,
    Segment,
    SettlementRegime,
    Side,
    StrategyId,
    TickAssignment,
    TradeDate,
    Venue,
    permit_digest,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ACCOUNT: u64 = 1;
const STRATEGY: u64 = 2;
const OTHER_STRATEGY: u64 = 20;
const ALGO: u64 = 3;
const PRICE: i64 = 1_000;

fn date() -> Result<TradeDate, Box<dyn std::error::Error>> {
    Ok(TradeDate::new(2026, 10, 28)?)
}

fn instrument(id: &str, derivative: bool) -> Result<Instrument, Box<dyn std::error::Error>> {
    let from = TradeDate::new(2026, 1, 1)?;
    let to = TradeDate::new(2026, 12, 31)?;
    Ok(Instrument {
        instrument_id: InstrumentId::new(id)?,
        instrument_type: InstrumentType::Index,
        isin: None,
        name: "Synthetic control fixture".to_owned(),
        underlying_instrument_id: None,
        expiry_date: derivative.then_some(date()?),
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: Segment::Cash,
            venue_symbol: "SYN-CTRL".to_owned(),
            token: None,
            effective_from: from,
            effective_to: Some(to),
            status: ListingStatus::Active,
            settlement_regime: SettlementRegime::Standard,
            z_category: false,
            settlement_group: None,
            terms: vec![ListingTerms {
                effective_from: from,
                effective_to: Some(to),
                tick_assignment: TickAssignment {
                    tick_paise: 5,
                    reference_close_paise: None,
                    reference_close_date: None,
                    reference_source: None,
                    reference_sha256_hex: None,
                    effective_month: None,
                    schedule_revision: "control-fixture".to_owned(),
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

fn policy() -> Result<RiskPolicy, Box<dyn std::error::Error>> {
    Ok(RiskPolicy {
        version: "risk-controls-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(ACCOUNT)?]),
        allowed_strategies: BTreeSet::from([
            StrategyId::new(STRATEGY)?,
            StrategyId::new(OTHER_STRATEGY)?,
        ]),
        allowed_algos: BTreeSet::from([AlgoId::new(ALGO)?]),
        allowed_instruments: BTreeSet::from([
            InstrumentId::new("nse.cash.synctl-a")?,
            InstrumentId::new("nse.cash.synctl-b")?,
            InstrumentId::new("nse.cash.synderiv")?,
            InstrumentId::new("nse.cash.unlabelled")?,
        ]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(100_000),
        max_position_quantity: Quantity::new(100)?,
        max_gross_exposure_paise: Paise(100_000),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 10,
        max_orders_per_second: 10,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: true,
        max_strategy_exposure_paise: Paise(9_000),
        max_sector_exposure_paise: Some(Paise(9_000)),
        stress_scenario: None,
        max_stress_loss_paise: None,
        max_participation_ppm: None,
        greek_limits: Some(GreekLimits {
            max_abs_delta_notional_paise: 10_000,
            max_abs_gamma: 50,
            max_abs_vega: 100,
            max_abs_theta: 50,
        }),
        max_borrow_quantity: Some(Quantity::new(10)?),
        max_financing_cost_paise: Some(Paise(100)),
    })
}

fn sectors() -> Result<BTreeMap<InstrumentId, String>, Box<dyn std::error::Error>> {
    Ok(BTreeMap::from([
        (
            InstrumentId::new("nse.cash.synctl-a")?,
            "synthetic-bank".to_owned(),
        ),
        (
            InstrumentId::new("nse.cash.synctl-b")?,
            "synthetic-bank".to_owned(),
        ),
        (
            InstrumentId::new("nse.cash.synderiv")?,
            "synthetic-derivative".to_owned(),
        ),
    ]))
}

fn authority(
    raw: RiskPolicy,
    with_sectors: bool,
) -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    let instruments = vec![
        instrument("nse.cash.synctl-a", false)?,
        instrument("nse.cash.synctl-b", false)?,
        instrument("nse.cash.synderiv", true)?,
        instrument("nse.cash.unlabelled", false)?,
    ];
    let mut value = RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        ValidatedRiskPolicy::from_policy(&raw)?,
        instruments,
        date()?,
    );
    if with_sectors {
        value = value.with_sectors(sectors()?);
    }
    value.set_session_open(true)?;
    value.set_snapshot_available(true)?;
    let tokens = value.register_all_owners()?;
    value.set_kill_switch(
        tokens.operator,
        false,
        "fixture-operator".to_owned(),
        "fixture".to_owned(),
        Some("release-fixture".to_owned()),
    )?;
    value.set_reconciliation_blocked(
        tokens.reconciliation,
        false,
        "fixture".to_owned(),
        Some("resolution-fixture".to_owned()),
    )?;
    Ok(value)
}

fn intent(
    id: u64,
    instrument_id: &str,
    strategy: u64,
    quantity: i64,
    side: Side,
) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(id)?,
        account_id: AccountId::new(ACCOUNT)?,
        strategy_id: StrategyId::new(strategy)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(ALGO)?,
        instrument_id: InstrumentId::new(instrument_id)?,
        venue: Venue::Nse,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(PRICE))?,
        reason: "control fixture intent".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(20_000),
    })
}

fn observations(
    value: &RiskAuthority,
    greeks: Option<Greeks>,
    borrow: Option<BorrowAvailability>,
) -> Result<Observations, Box<dyn std::error::Error>> {
    let tokens = value.register_all_owners()?;
    let at = LogicalNanos::ZERO;
    let quote = QuoteSnapshot {
        instrument_id: InstrumentId::new("nse.cash.synctl-a")?,
        venue: Venue::Nse,
        bid_price: Some(PricePaise::new(Paise(995))?),
        bid_quantity: Quantity::new(10)?,
        ask_price: Some(PricePaise::new(Paise(1_005))?),
        ask_quantity: Quantity::new(10)?,
        continuity: Continuity::Snapshot,
    };
    Ok(Observations {
        market: Some(Observation::new(
            tokens.market,
            ObservationSource::MarketDataKernel,
            at,
            at,
            1,
            MarketObservation {
                feed: FeedObservation {
                    quote: Some(quote),
                    received_at: at,
                    gap: None,
                    book_complete: true,
                },
                session_open: true,
                kernel_schema_version: 3,
                source_sequence: Some(1),
                connection_epoch: Some(1),
                depth: Some(LiquidityDepth {
                    displayed_quantity: Quantity::new(10)?,
                    adv_quantity: Quantity::new(100)?,
                }),
                marks_paise: BTreeMap::from([(
                    InstrumentId::new("nse.cash.synctl-a")?,
                    Paise(PRICE),
                )]),
                greeks,
                borrow,
                synthetic: true,
            },
        )),
        oms: Some(Observation::new(
            tokens.oms,
            ObservationSource::Oms,
            at,
            at,
            1,
            OmsObservation {
                fills_digest: "a".repeat(64),
                unresolved_count: 0,
                last_journal_digest: "b".repeat(64),
                is_poisoned: false,
            },
        )),
        account_journal: Some(Observation::new(
            tokens.account_journal,
            ObservationSource::AccountJournal,
            at,
            at,
            1,
            AccountJournalObservation {
                head_hash_hex: "c".repeat(64),
                record_count: 1,
                ledger_state_digest: "d".repeat(64),
                opening_balance_evidence_ref: Some("statement-fixture".to_owned()),
            },
        )),
        reconciliation: Some(Observation::new(
            tokens.reconciliation,
            ObservationSource::Reconciliation,
            at,
            at,
            1,
            ReconciliationObservation {
                report: ReconciliationReport {
                    reconciliation_id: "recon-fixture".to_owned(),
                    status: ReconciliationStatus::Matched,
                    expected_positions: BTreeMap::new(),
                    observed_positions: Some(BTreeMap::new()),
                    expected_active_orders: BTreeSet::new(),
                    observed_active_orders: Some(BTreeSet::new()),
                    differences: Vec::new(),
                },
                replay_digest_matched: true,
            },
        )),
        ops: Some(Observation::new(
            tokens.ops,
            ObservationSource::Ops,
            at,
            at,
            1,
            OpsObservation {
                feed_age_ns: 0,
                sequence_gaps: 0,
                journal_append_latency_ns: 1_000,
                journal_queue_depth: 0,
                disk_available_bytes: 10_000_000,
                unknown_orders: 0,
                reconciliation_differences: 0,
            },
        )),
        operator: Some(Observation::new(
            tokens.operator,
            ObservationSource::OperatorControl,
            at,
            at,
            1,
            OperatorObservation {
                kill_switch_engaged: false,
                actor_id: "fixture-operator".to_owned(),
                reason: "fixture".to_owned(),
                release_evidence_ref: Some("release-fixture".to_owned()),
            },
        )),
    })
}

fn rejected_check(error: &AuthorityError, check: RiskCheck) -> bool {
    match error {
        AuthorityError::Rejected(assessment) => assessment
            .checks
            .iter()
            .any(|result| result.check == check && !result.passed),
        _ => false,
    }
}

#[test]
fn two_orders_under_the_order_cap_breach_the_strategy_cap() -> TestResult {
    let mut raw = policy()?;
    raw.max_sector_exposure_paise = None;
    raw.max_participation_ppm = None;
    raw.greek_limits = None;
    raw.max_borrow_quantity = None;
    raw.max_financing_cost_paise = None;
    let value = authority(raw, false)?;
    let first = value.evaluate_and_reserve(
        intent(1, "nse.cash.synctl-a", STRATEGY, 5, Side::Buy)?,
        observations(&value, None, None)?,
        None,
    )?;
    assert_eq!(
        first.reserved_notional_paise,
        Paise(5_000),
        "the first reservation is 5,000 paise"
    );
    let second = value.evaluate_and_reserve(
        intent(2, "nse.cash.synctl-a", STRATEGY, 5, Side::Buy)?,
        observations(&value, None, None)?,
        None,
    );
    assert!(
        rejected_check(
            second
                .as_ref()
                .expect_err("the 10,000 paise strategy total breaches the 9,000 cap"),
            RiskCheck::StrategyExposure
        ),
        "the second intent is denied by the strategy cap"
    );
    let snapshot = value.snapshot()?;
    assert_eq!(
        snapshot.strategy_exposure.get(&StrategyId::new(STRATEGY)?),
        Some(&Paise(5_000)),
        "only the accepted reservation is aggregated"
    );

    // A distinct strategy uses its own bucket and is not denied by the first.
    let other = value.evaluate_and_reserve(
        intent(3, "nse.cash.synctl-a", OTHER_STRATEGY, 5, Side::Buy)?,
        observations(&value, None, None)?,
        None,
    );
    assert!(other.is_ok(), "a distinct strategy has its own cap");
    Ok(())
}

#[test]
fn sector_cap_and_unlabelled_instrument_fail_closed() -> TestResult {
    let mut raw = policy()?;
    raw.max_strategy_exposure_paise = Paise(100_000);
    raw.max_participation_ppm = None;
    raw.greek_limits = None;
    raw.max_borrow_quantity = None;
    raw.max_financing_cost_paise = None;
    let value = authority(raw, true)?;
    value.evaluate_and_reserve(
        intent(1, "nse.cash.synctl-a", STRATEGY, 5, Side::Buy)?,
        observations(&value, None, None)?,
        None,
    )?;
    let breach = value.evaluate_and_reserve(
        intent(2, "nse.cash.synctl-b", OTHER_STRATEGY, 5, Side::Buy)?,
        observations(&value, None, None)?,
        None,
    );
    assert!(
        rejected_check(
            breach
                .as_ref()
                .expect_err("two instruments in one sector breach the cap"),
            RiskCheck::SectorExposure
        ),
        "a second instrument sharing the synthetic sector is denied"
    );

    let unlabelled = value.evaluate_and_reserve(
        intent(3, "nse.cash.unlabelled", STRATEGY, 1, Side::Buy)?,
        observations(&value, None, None)?,
        None,
    );
    assert!(
        rejected_check(
            unlabelled
                .as_ref()
                .expect_err("an unlabelled instrument cannot pass"),
            RiskCheck::SectorExposure
        ),
        "a missing sector label fails closed"
    );
    Ok(())
}

#[test]
fn authority_rejects_configured_stress_and_participation_breaches() -> TestResult {
    let mut raw = policy()?;
    raw.max_strategy_exposure_paise = Paise(100_000);
    raw.max_sector_exposure_paise = None;
    raw.greek_limits = None;
    raw.max_borrow_quantity = None;
    raw.max_financing_cost_paise = None;
    raw.stress_scenario = Some(StressScenario {
        price_shift_ppm: -100_000,
        liquidity_haircut_ppm: 0,
        correlation_multiplier_ppm: 1_000_000,
    });
    raw.max_stress_loss_paise = Some(Paise(50));
    raw.max_participation_ppm = Some(100_000);
    let value = authority(raw, false)?;
    let stress = value.evaluate_and_reserve(
        intent(1, "nse.cash.synctl-a", STRATEGY, 1, Side::Buy)?,
        observations(&value, None, None)?,
        None,
    );
    assert!(
        rejected_check(
            stress
                .as_ref()
                .expect_err("a 100 paise projected loss breaches the 50 cap"),
            RiskCheck::StressLoss
        ),
        "the stress check denies the intent"
    );

    let mut no_depth = observations(&value, None, None)?;
    no_depth.market.as_mut().expect("market").payload.depth = None;
    let missing = value.evaluate_and_reserve(
        intent(2, "nse.cash.synctl-a", STRATEGY, 1, Side::Buy)?,
        no_depth,
        None,
    );
    assert!(
        rejected_check(
            missing.as_ref().expect_err("missing depth cannot pass"),
            RiskCheck::LiquidityParticipation
        ),
        "missing depth fails closed"
    );

    let mut over = policy()?;
    over.max_strategy_exposure_paise = Paise(100_000);
    over.max_sector_exposure_paise = None;
    over.greek_limits = None;
    over.max_borrow_quantity = None;
    over.max_financing_cost_paise = None;
    over.max_participation_ppm = Some(10_000);
    let strict = authority(over, false)?;
    let wall = strict.evaluate_and_reserve(
        intent(3, "nse.cash.synctl-a", STRATEGY, 5, Side::Buy)?,
        observations(&strict, None, None)?,
        None,
    );
    assert!(
        rejected_check(
            wall.as_ref()
                .expect_err("5,000 paise exceeds 1% of 10,000 paise depth"),
            RiskCheck::LiquidityParticipation
        ),
        "an order above the participation cap is denied"
    );
    Ok(())
}

#[test]
fn derivative_greeks_and_short_borrow_fail_closed() -> TestResult {
    let mut raw = policy()?;
    raw.max_strategy_exposure_paise = Paise(100_000);
    raw.max_sector_exposure_paise = None;
    raw.max_participation_ppm = None;
    let value = authority(raw, false)?;

    let no_greeks = value.evaluate_and_reserve(
        intent(1, "nse.cash.synderiv", STRATEGY, 1, Side::Buy)?,
        observations(&value, None, None)?,
        None,
    );
    assert!(
        rejected_check(
            no_greeks
                .as_ref()
                .expect_err("a derivative without greeks is denied"),
            RiskCheck::Greeks
        ),
        "greeks-less derivative fails closed"
    );

    let over_greeks = value.evaluate_and_reserve(
        intent(2, "nse.cash.synderiv", STRATEGY, 1, Side::Buy)?,
        observations(
            &value,
            Some(Greeks {
                delta_notional_paise: 5_000,
                gamma_scaled: 10,
                vega_scaled: 101,
                theta_scaled: 5,
            }),
            None,
        )?,
        None,
    );
    assert!(
        rejected_check(
            over_greeks
                .as_ref()
                .expect_err("vega 101 exceeds the 100 limit"),
            RiskCheck::Greeks
        ),
        "an over-limit Greek is denied"
    );

    let no_borrow = value.evaluate_and_reserve(
        intent(3, "nse.cash.synctl-a", STRATEGY, 5, Side::Sell)?,
        observations(&value, None, None)?,
        None,
    );
    assert!(
        rejected_check(
            no_borrow
                .as_ref()
                .expect_err("a short without borrow is denied"),
            RiskCheck::BorrowFinancing
        ),
        "a short without borrow evidence fails closed"
    );

    let over_cost = value.evaluate_and_reserve(
        intent(4, "nse.cash.synctl-a", STRATEGY, 5, Side::Sell)?,
        observations(
            &value,
            None,
            Some(BorrowAvailability {
                available_quantity: Quantity::new(5)?,
                financing_cost_paise: Paise(101),
            }),
        )?,
        None,
    );
    assert!(
        rejected_check(
            over_cost
                .as_ref()
                .expect_err("financing cost 101 exceeds the 100 cap"),
            RiskCheck::BorrowFinancing
        ),
        "an over-cost short is denied"
    );
    Ok(())
}

#[test]
fn permit_digest_changes_when_an_authorized_intent_is_tampered() -> TestResult {
    let mut raw = policy()?;
    raw.max_sector_exposure_paise = None;
    raw.max_participation_ppm = None;
    raw.greek_limits = None;
    raw.max_borrow_quantity = None;
    raw.max_financing_cost_paise = None;
    let value = authority(raw, false)?;
    let order = intent(1, "nse.cash.synctl-a", STRATEGY, 2, Side::Buy)?;
    let reservation =
        value.evaluate_and_reserve(order.clone(), observations(&value, None, None)?, None)?;
    let digest = value.permit_digest(&reservation, &order)?;
    assert_eq!(
        digest,
        permit_digest(
            reservation.permit_id,
            &reservation.assessment.policy_version,
            reservation.snapshot_revision,
            &order
        ),
        "the authority exposes the same canonical digest as the OMS"
    );
    let mut tampered = order;
    tampered.quantity = Quantity::new(3)?;
    assert_ne!(
        digest,
        value.permit_digest(&reservation, &tampered)?,
        "tampering with an authorized intent changes its digest"
    );
    Ok(())
}
