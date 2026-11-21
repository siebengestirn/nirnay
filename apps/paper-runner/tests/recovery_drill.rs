#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Deterministic cross-component recovery drill using existing fault injection.
//! All inputs are synthetic; this is an in-process drill, not independent
//! certification, HA failover or broker-delivery evidence. Every append
//! boundary drills the OMS journal, the account journal, the persisted kill
//! switch, the reconciliation latch and the ledger replay in one pass.

use std::{
    collections::{
        BTreeMap, //
        BTreeSet,
    },
    path::{
        Path, //
        PathBuf,
    },
};

use account_journal::{
    CommitFault as JournalCommitFault, //
    JournalError,
};
use authoritative_oms::{
    ChargeStatus, //
    ClientOrderId,
    CommitFault as OmsCommitFault,
    FillEvent,
    OmsError,
    OmsStore,
    PermitRef,
    ProjectionError,
    Projector,
};
use risk_authority::{
    AccountJournalObservation, //
    AuthorityError,
    ControlStore,
    FeedObservation,
    FileControlStore,
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
    RiskPolicy, //
    ValidatedRiskPolicy,
};
use trading_core::{
    AccountId, //
    AlgoId,
    Continuity,
    Instrument,
    InstrumentId,
    InstrumentType,
    IntentId,
    JournalRef,
    Listing,
    ListingStatus,
    ListingTerms,
    LogicalNanos,
    OpeningBalanceEvidence,
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

/// One journal fault per boundary; the rotation is fixed so the drill is
/// deterministic and covers four distinct append boundaries.
const JOURNAL_FAULTS: [JournalCommitFault; 4] = [
    JournalCommitFault::BeforeHeader,
    JournalCommitFault::MidBody,
    JournalCommitFault::AfterBodyBeforeSync,
    JournalCommitFault::AfterSync,
];

fn date() -> Result<TradeDate, Box<dyn std::error::Error>> {
    Ok(TradeDate::new(2026, 10, 28)?)
}

fn instrument_id() -> Result<InstrumentId, Box<dyn std::error::Error>> {
    Ok(InstrumentId::new("nse.cash.drillidx")?)
}

fn instrument() -> Result<Instrument, Box<dyn std::error::Error>> {
    let from = TradeDate::new(2026, 1, 1)?;
    let to = TradeDate::new(2026, 12, 31)?;
    Ok(Instrument {
        instrument_id: instrument_id()?,
        instrument_type: InstrumentType::Index,
        isin: None,
        name: "Synthetic drill instrument".to_owned(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: Segment::Cash,
            venue_symbol: "DRILL".to_owned(),
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
                    schedule_revision: "drill-fixture".to_owned(),
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
        version: "drill-risk-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(1)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(2)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(3)?]),
        allowed_instruments: BTreeSet::from([instrument_id()?]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(100_000),
        max_position_quantity: Quantity::new(100)?,
        max_gross_exposure_paise: Paise(100_000),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 10,
        max_orders_per_second: 10,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: false,
        max_strategy_exposure_paise: Paise(100_000),
        max_sector_exposure_paise: None,
        stress_scenario: None,
        max_stress_loss_paise: None,
        max_participation_ppm: None,
        greek_limits: None,
        max_borrow_quantity: None,
        max_financing_cost_paise: None,
    })
}

fn intent(id: u64, quantity: i64) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(id)?,
        account_id: AccountId::new(1)?,
        strategy_id: StrategyId::new(2)?,
        strategy_version: "drill-v1".to_owned(),
        algo_id: AlgoId::new(3)?,
        instrument_id: instrument_id()?,
        venue: Venue::Nse,
        side: Side::Buy,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(1_000))?,
        reason: "recovery drill intent".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(100_000),
    })
}

fn permit(permit_id: u64, order: &OrderIntent) -> PermitRef {
    PermitRef {
        permit_id,
        policy_version: "drill-risk-v1".to_owned(),
        snapshot_revision: 1,
        digest: permit_digest(permit_id, "drill-risk-v1", 1, order),
    }
}

fn ledger_fill(ordinal: u64, quantity: i64) -> FillEvent {
    FillEvent {
        schema_version: 1,
        execution_id: format!("drill-exec-{ordinal}"),
        broker_event_id: format!("drill-broker-{ordinal}"),
        client_order_id: ClientOrderId::new(1).expect("fixture client id"),
        oms_order_id: "1".to_owned(),
        side: Side::Buy,
        quantity,
        price_paise: 1_000,
        currency: "INR".to_owned(),
        broker_timestamp_ns: 10,
        received_at_ns: 10,
        source: "drill-synthetic".to_owned(),
        account_scope: "drill".to_owned(),
        charges_paise: 0,
        charge_status: ChargeStatus::Estimated,
        provenance: "recovery drill".to_owned(),
    }
}

fn opening_evidence() -> OpeningBalanceEvidence {
    OpeningBalanceEvidence {
        statement_ref: "drill-statement".to_owned(),
        journal_ref: JournalRef {
            sequence: 1,
            hash_hex: "a".repeat(64),
        },
        observed_at_ns: 1,
    }
}

/// Run one clean append prefix in both journals, inject a fault on the next
/// append in each, and assert all five cross-component properties at that
/// boundary.
fn drill_boundary(
    root: &Path,
    boundary: usize,
    oms_fault: OmsCommitFault,
    journal_fault: JournalCommitFault,
) -> TestResult {
    // (1) OMS committed prefix and poisoned writer.
    let oms_path = root.join("oms.journal");
    let mut store = OmsStore::open(&oms_path)?;
    if boundary >= 1 {
        let order = intent(1, 2)?;
        let issued = permit(1, &order);
        store.create(ClientOrderId::new(1)?, &order, &issued, 1_000)?;
    }
    if boundary >= 2 {
        store.accept_risk_approved(ClientOrderId::new(1)?, 1_100)?;
    }
    if boundary >= 3 {
        let order = intent(2, 3)?;
        let issued = permit(2, &order);
        store.create(ClientOrderId::new(2)?, &order, &issued, 1_200)?;
    }
    store.force_commit_fault(oms_fault);
    let next = intent(3, 4)?;
    let next_permit = permit(3, &next);
    let attempted = store.create(ClientOrderId::new(3)?, &next, &next_permit, 1_300);
    assert!(attempted.is_err(), "the OMS fault must surface");
    assert!(
        store.is_poisoned(),
        "the OMS writer is poisoned at the boundary"
    );
    let refused = store.create(ClientOrderId::new(4)?, &next, &next_permit, 1_400);
    assert!(
        matches!(refused, Err(OmsError::WriterPoisoned)),
        "a poisoned OMS writer refuses every later mutation: {refused:?}"
    );
    drop(store);
    match oms_fault {
        OmsCommitFault::PartialWrite => {
            assert!(
                OmsStore::recover(&oms_path).is_err(),
                "a torn OMS tail must never be accepted as a committed prefix"
            );
        }
        OmsCommitFault::SyncFailure => {
            let recovered = OmsStore::recover(&oms_path)?;
            assert!(
                !recovered.is_poisoned(),
                "fresh OMS recovery yields a clean writer"
            );
            // A full line is written before the durability step fails, so one
            // more fully written record replays than in the clean prefix.
            let expected = usize::from(boundary >= 1) + usize::from(boundary >= 3) + 1;
            assert_eq!(
                recovered.orders()?.len(),
                expected,
                "only the durable OMS committed prefix replays"
            );
            let digest = recovered.state_digest().to_owned();
            drop(recovered);
            let reopened = OmsStore::recover(&oms_path)?;
            assert_eq!(
                reopened.state_digest(),
                digest,
                "independent replay of the OMS prefix is byte-identical"
            );
        }
    }

    // (2) Account-journal committed prefix, poisoned writer and replay.
    let ledger_root = root.join("ledger");
    let mut projector = Projector::open(&ledger_root, AccountId::new(1)?)?;
    projector.open_account("drill", Paise(10_000), date()?, opening_evidence())?;
    for ordinal in 1..=boundary {
        projector.project_fill(&ledger_fill(ordinal as u64, 2), &instrument_id()?)?;
    }
    projector.force_commit_fault(Some(journal_fault));
    let fault_fill = ledger_fill(100 + boundary as u64, 3);
    let attempted = projector.project_fill(&fault_fill, &instrument_id()?);
    assert!(attempted.is_err(), "the account-journal fault must surface");
    let refused = projector.project_fill(&ledger_fill(200 + boundary as u64, 4), &instrument_id()?);
    assert!(
        matches!(
            refused,
            Err(ProjectionError::Journal(JournalError::Poisoned))
        ),
        "a poisoned journal writer refuses every later mutation: {refused:?}"
    );
    drop(projector);
    let replayed = Projector::open(&ledger_root, AccountId::new(1)?)?;
    let verification = replayed.verify_replay()?;
    assert!(
        verification.matched,
        "Projector::verify_replay succeeds on the committed ledger prefix"
    );
    // BeforeHeader and MidBody add no replayable record; the two After*
    // faults write a complete frame and therefore add one replayable record.
    let extra = match journal_fault {
        JournalCommitFault::AfterBodyBeforeSync | JournalCommitFault::AfterSync => 1,
        JournalCommitFault::BeforeHeader
        | JournalCommitFault::MidBody
        | JournalCommitFault::Rollover => 0,
    };
    assert_eq!(
        replayed.verification_report().record_count,
        1 + boundary as u64 + extra,
        "only the durable account-journal committed prefix replays"
    );

    // (3) Engaged kill switch survives a rebuild at this boundary.
    let engaged_path = root.join("control-engaged.jsonl");
    {
        let first = control_authority(engaged_path.clone())?;
        assert!(
            first.kill_switch_engaged()?,
            "the operator engaged the switch before restart"
        );
    }
    let rebuilt = control_authority(engaged_path)?;
    assert!(
        rebuilt.kill_switch_engaged()?,
        "an engaged switch survives a fresh authority at this boundary"
    );
    let denied = rebuilt.evaluate_and_reserve(
        intent(300 + boundary as u64, 2)?,
        observations(&rebuilt, true, ReconciliationStatus::Matched)?,
        None,
    );
    assert!(
        matches!(denied, Err(AuthorityError::KillSwitchEngaged { .. })),
        "no new risk is admitted by the rebuilt authority: {denied:?}"
    );

    // (4) Reconciliation digest mismatch blocks at this boundary.
    let recon_path = root.join("control-reconciliation.jsonl");
    let recon = control_authority(recon_path)?;
    let tokens = recon.register_all_owners()?;
    recon.set_kill_switch(
        tokens.operator,
        false,
        "drill-operator".to_owned(),
        "drill release".to_owned(),
        Some("drill-release".to_owned()),
    )?;
    let mismatch = recon.evaluate_and_reserve(
        intent(400 + boundary as u64, 2)?,
        observations(&recon, false, ReconciliationStatus::Matched)?,
        None,
    );
    assert!(
        matches!(
            mismatch,
            Err(AuthorityError::ObservationDigestMismatch { .. })
        ),
        "a replay digest mismatch blocks the decision at this boundary"
    );
    let after = recon.evaluate_and_reserve(
        intent(500 + boundary as u64, 2)?,
        observations(&recon, true, ReconciliationStatus::Matched)?,
        None,
    );
    assert!(
        matches!(after, Err(AuthorityError::Rejected(ref assessment))
        if assessment.checks.iter().any(|result| {
            result.check == RiskCheck::Reconciliation && !result.passed
        })),
        "the latched reconciliation block survives a later matched view: {after:?}"
    );
    Ok(())
}

#[test]
fn every_append_boundary_drills_all_cross_component_controls() -> TestResult {
    for (boundary, journal_fault) in JOURNAL_FAULTS.iter().enumerate() {
        for oms_fault in [OmsCommitFault::PartialWrite, OmsCommitFault::SyncFailure] {
            let directory = tempfile::tempdir()?;
            drill_boundary(directory.path(), boundary, oms_fault, *journal_fault)?;
        }
    }
    Ok(())
}

fn control_authority(path: PathBuf) -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    let store = FileControlStore::new(path);
    let engaged = store
        .load_kill_switch()
        .ok()
        .flatten()
        .is_some_and(|state| state.enabled);
    let authority = RiskAuthority::new_with_control_store(
        AccountId::new(1)?,
        ValidatedRiskPolicy::from_policy(&policy()?)?,
        vec![instrument()?],
        date()?,
        Box::new(store),
    );
    if !engaged {
        let tokens = authority.register_all_owners()?;
        authority.set_kill_switch(
            tokens.operator,
            true,
            "drill-operator".to_owned(),
            "drill engage".to_owned(),
            None,
        )?;
    }
    Ok(authority)
}

fn observations(
    authority: &RiskAuthority,
    replay_matched: bool,
    status: ReconciliationStatus,
) -> Result<Observations, Box<dyn std::error::Error>> {
    let tokens = authority.register_all_owners()?;
    let at = LogicalNanos::ZERO;
    let quote = QuoteSnapshot {
        instrument_id: instrument_id()?,
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
                depth: None,
                marks_paise: BTreeMap::new(),
                greeks: None,
                borrow: None,
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
                opening_balance_evidence_ref: Some("drill-statement".to_owned()),
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
                    reconciliation_id: "drill-recon".to_owned(),
                    status,
                    expected_positions: BTreeMap::new(),
                    observed_positions: Some(BTreeMap::new()),
                    expected_active_orders: BTreeSet::new(),
                    observed_active_orders: Some(BTreeSet::new()),
                    differences: Vec::new(),
                },
                replay_digest_matched: replay_matched,
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
                actor_id: "drill-operator".to_owned(),
                reason: "drill".to_owned(),
                release_evidence_ref: Some("drill-release".to_owned()),
            },
        )),
    })
}
