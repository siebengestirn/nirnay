#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.4 risk authority tests. Fixtures are synthetic; no order submission or
//! broker interaction exists in this crate.

use std::{
    collections::BTreeSet,
    sync::{
        Arc, //
        Barrier,
    },
    thread,
};

use risk_authority::{
    AuthorityError, //
    FeedObservation,
    RiskAuthority,
    SequenceGap,
};
use risk_engine::{
    RiskPolicy, //
    ValidatedRiskPolicy,
};
use trading_core::{
    AccountId, //
    AlgoId,
    Continuity,
    FeedHealth,
    Instrument,
    InstrumentId,
    InstrumentType,
    IntentId,
    JournalRef,
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
    RiskCheck,
    Segment,
    SettlementRegime,
    Side,
    StrategyId,
    TickAssignment,
    TradeDate,
    Venue,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ACCOUNT: u64 = 1;
const STRATEGY: u64 = 2;
const ALGO: u64 = 3;
const MAX_GROSS: i64 = 8_000;
const PRICE: i64 = 1_000;

fn date() -> Result<TradeDate, Box<dyn std::error::Error>> {
    Ok(TradeDate::new(2026, 10, 28)?)
}

fn instrument_id() -> Result<InstrumentId, Box<dyn std::error::Error>> {
    Ok(InstrumentId::new("nse.cash.synidx")?)
}

fn instrument() -> Result<Instrument, Box<dyn std::error::Error>> {
    let from = TradeDate::new(2026, 1, 1)?;
    let to = TradeDate::new(2026, 12, 31)?;
    Ok(Instrument {
        instrument_id: instrument_id()?,
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

fn policy() -> Result<ValidatedRiskPolicy, Box<dyn std::error::Error>> {
    let raw = RiskPolicy {
        version: "risk-v1".to_owned(),
        allowed_accounts: BTreeSet::from([AccountId::new(ACCOUNT)?]),
        allowed_strategies: BTreeSet::from([StrategyId::new(STRATEGY)?]),
        allowed_algos: BTreeSet::from([AlgoId::new(ALGO)?]),
        allowed_instruments: BTreeSet::from([instrument_id()?]),
        max_order_quantity: Quantity::new(10)?,
        max_order_notional_paise: Paise(10_000),
        max_position_quantity: Quantity::new(100)?,
        max_gross_exposure_paise: Paise(MAX_GROSS),
        max_daily_loss_paise: Paise(5_000),
        max_open_orders: 5,
        max_orders_per_second: 5,
        max_price_deviation_ppm: 100_000,
        max_quote_age_ns: 1_000,
        allow_short: false,
    };
    Ok(ValidatedRiskPolicy::from_policy(&raw)?)
}

fn authority() -> Result<RiskAuthority, Box<dyn std::error::Error>> {
    Ok(RiskAuthority::new(
        AccountId::new(ACCOUNT)?,
        policy()?,
        vec![instrument()?],
        date()?,
    ))
}

fn intent(
    id: u64,
    quantity: i64,
    price: i64,
    side: Side,
) -> Result<OrderIntent, Box<dyn std::error::Error>> {
    Ok(OrderIntent {
        intent_id: IntentId::new(id)?,
        account_id: AccountId::new(ACCOUNT)?,
        strategy_id: StrategyId::new(STRATEGY)?,
        strategy_version: "strategy-v1".to_owned(),
        algo_id: AlgoId::new(ALGO)?,
        instrument_id: instrument_id()?,
        venue: Venue::Nse,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(quantity)?,
        limit_price: PricePaise::new(Paise(price))?,
        reason: "test intent".to_owned(),
        evidence: vec![JournalRef {
            sequence: 1,
            hash_hex: "ab".repeat(32),
        }],
        expires_at: LogicalNanos::new(20_000),
    })
}

fn quote() -> Result<QuoteSnapshot, Box<dyn std::error::Error>> {
    Ok(QuoteSnapshot {
        instrument_id: instrument_id()?,
        venue: Venue::Nse,
        bid_price: Some(PricePaise::new(Paise(995))?),
        bid_quantity: Quantity::new(1)?,
        ask_price: Some(PricePaise::new(Paise(1_005))?),
        ask_quantity: Quantity::new(1)?,
        continuity: Continuity::Snapshot,
    })
}

fn observation() -> Result<FeedObservation, Box<dyn std::error::Error>> {
    Ok(FeedObservation {
        quote: Some(quote()?),
        received_at: LogicalNanos::ZERO,
        gap: None,
        book_complete: true,
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
fn baseline_reservation_is_accepted() -> TestResult {
    let authority = authority()?;
    let reservation =
        authority.evaluate_and_reserve(intent(1, 6, PRICE, Side::Buy)?, observation()?, None)?;
    assert!(
        reservation.assessment.accepted,
        "baseline intent is accepted"
    );
    assert_eq!(
        reservation.quantity.get(),
        6,
        "the reserved quantity is bound"
    );
    assert_eq!(
        reservation.reserved_notional_paise,
        Paise(6_000),
        "exact notional is reserved"
    );
    assert_eq!(
        reservation.snapshot_revision, 1,
        "the permit records its state revision"
    );
    Ok(())
}

/// Two simultaneous reservations against one aggregate limit: only one can
/// pass, because evaluation and reservation share one lock and the exposure
/// used by the second call is the authority's freshly updated state.
#[test]
fn concurrent_reservations_cannot_exceed_aggregate_limit() -> TestResult {
    let authority = Arc::new(authority()?);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for id in 1..=2_u64 {
        let authority = Arc::clone(&authority);
        let barrier = Arc::clone(&barrier);
        let target = intent(id, 6, PRICE, Side::Buy)?;
        handles.push(thread::spawn(move || {
            let observation = observation().expect("observation");
            barrier.wait();
            authority.evaluate_and_reserve(target, observation, None)
        }));
    }
    let results: Vec<Result<_, _>> = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread"))
        .collect();
    let accepted = results.iter().filter(|result| result.is_ok()).count();
    let rejected = results.iter().filter(|result| result.is_err()).count();
    assert_eq!(
        accepted, 1,
        "exactly one concurrent reservation fits the aggregate limit"
    );
    assert_eq!(rejected, 1, "the competing reservation is rejected");
    let failure = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("rejection");
    assert!(
        rejected_check(failure, RiskCheck::GrossExposure),
        "the aggregate exposure check is the binding constraint"
    );
    let snapshot = authority.snapshot()?;
    assert_eq!(
        snapshot.pending_gross_exposure_paise,
        Paise(6_000),
        "only one notional is held"
    );
    Ok(())
}

/// A caller holding an older account snapshot cannot spend against it.
#[test]
fn stale_snapshot_revision_is_rejected() -> TestResult {
    let authority = authority()?;
    let observed_revision = authority.revision()?;
    authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation()?, None)?;
    let stale = authority.evaluate_and_reserve(
        intent(2, 2, PRICE, Side::Buy)?,
        observation()?,
        Some(observed_revision),
    );
    assert!(
        matches!(stale, Err(AuthorityError::StaleSnapshot { expected, current }) if expected == observed_revision && current == observed_revision + 1),
        "the stale revision fence is enforced against authority-owned state"
    );
    Ok(())
}

/// Feed health has no caller-supplied boolean: it is derived from the
/// observation. A gapped or stale observation fails the health check even
/// though the caller supplies no health verdict at all.
#[test]
fn caller_cannot_supply_feed_health() -> TestResult {
    let observation = observation()?;
    let wire = serde_json::to_value(&observation)?;
    let fields = wire.as_object().expect("object");
    let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["book_complete", "gap", "quote", "received_at"],
        "the observation carries raw facts, never a health verdict"
    );

    let authority = authority()?;
    let mut gapped = observation.clone();
    gapped.gap = Some(SequenceGap {
        expected: Some(4),
        observed: 9,
    });
    let result = authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, gapped, None);
    assert!(
        rejected_check(
            &result.expect_err("gapped is refused"),
            RiskCheck::MarketFeedHealth
        ),
        "a gap fails the health check"
    );

    authority.set_now(LogicalNanos::new(5_000))?;
    let result = authority.evaluate_and_reserve(intent(2, 2, PRICE, Side::Buy)?, observation, None);
    assert!(
        rejected_check(
            &result.expect_err("stale is refused"),
            RiskCheck::MarketFeedHealth
        ),
        "an aged quote fails the health check"
    );
    Ok(())
}

/// Replaying an accepted intent returns the same permit and does not reserve
/// twice; reusing the id with different terms is refused.
#[test]
fn duplicate_intent_is_idempotent_and_conflicts_are_denied() -> TestResult {
    let authority = authority()?;
    let original =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation()?, None)?;
    let replay =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation()?, None)?;
    assert_eq!(
        replay, original,
        "the same permit is returned for the same intent"
    );
    let snapshot = authority.snapshot()?;
    assert_eq!(
        snapshot.reservation_count, 1,
        "no second reservation was created"
    );
    assert_eq!(
        snapshot.pending_gross_exposure_paise,
        Paise(2_000),
        "exposure was not doubled"
    );

    let conflicting =
        authority.evaluate_and_reserve(intent(1, 3, PRICE, Side::Buy)?, observation()?, None);
    assert!(
        matches!(conflicting, Err(AuthorityError::ConflictingIntent(_))),
        "the same id with different terms is denied"
    );
    Ok(())
}

/// Authority-owned control state overrides any intent: kill switch,
/// reconciliation mismatch and an unavailable account snapshot each block new
/// exposure.
#[test]
fn control_state_blocks_new_exposure() -> TestResult {
    let authority = authority()?;
    authority.set_kill_switch(true)?;
    let killed =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation()?, None);
    assert!(
        rejected_check(&killed.expect_err("kill switch"), RiskCheck::KillSwitch),
        "kill switch blocks"
    );
    authority.set_kill_switch(false)?;

    authority.set_reconciliation_blocked(true)?;
    let blocked =
        authority.evaluate_and_reserve(intent(2, 2, PRICE, Side::Buy)?, observation()?, None);
    assert!(
        rejected_check(
            &blocked.expect_err("reconciliation"),
            RiskCheck::Reconciliation
        ),
        "reconciliation mismatch blocks"
    );
    authority.set_reconciliation_blocked(false)?;

    authority.set_snapshot_available(false)?;
    let missing =
        authority.evaluate_and_reserve(intent(3, 2, PRICE, Side::Buy)?, observation()?, None);
    assert!(
        rejected_check(
            &missing.expect_err("snapshot"),
            RiskCheck::AccountSnapshotAvailable
        ),
        "missing snapshot blocks"
    );
    Ok(())
}

/// A caller cannot assert a different account; the authority's own account
/// identity is used in the context.
#[test]
fn caller_cannot_claim_another_account() -> TestResult {
    let authority = authority()?;
    let mut forged = intent(1, 2, PRICE, Side::Buy)?;
    forged.account_id = AccountId::new(999)?;
    let result = authority.evaluate_and_reserve(forged, observation()?, None);
    assert!(
        rejected_check(
            &result.expect_err("wrong account"),
            RiskCheck::AccountAllowlist
        ),
        "account identity comes from the authority"
    );
    Ok(())
}

/// Exposure accumulates in authority state, so a fill cannot reset it and let
/// a later intent pass against an understated total.
#[test]
fn filled_exposure_is_owned_and_accumulates() -> TestResult {
    let authority = authority()?;
    let first =
        authority.evaluate_and_reserve(intent(1, 6, PRICE, Side::Buy)?, observation()?, None)?;
    authority.apply_fill(first.intent_id)?;
    let snapshot = authority.snapshot()?;
    assert_eq!(
        snapshot.gross_exposure_paise,
        Paise(6_000),
        "filled notional moves to gross exposure"
    );
    assert_eq!(
        snapshot.pending_gross_exposure_paise,
        Paise(0),
        "pending exposure is cleared"
    );

    let second =
        authority.evaluate_and_reserve(intent(2, 6, PRICE, Side::Buy)?, observation()?, None);
    assert!(
        rejected_check(&second.expect_err("aggregate"), RiskCheck::GrossExposure),
        "a later intent sees the accumulated exposure"
    );
    Ok(())
}

/// Releasing an unused reservation returns its exposure and open-order slot.
#[test]
fn releasing_a_reservation_restores_capacity() -> TestResult {
    let authority = authority()?;
    let reservation =
        authority.evaluate_and_reserve(intent(1, 6, PRICE, Side::Buy)?, observation()?, None)?;
    assert_eq!(
        authority.snapshot()?.open_orders,
        1,
        "one order slot is held"
    );
    authority.release(reservation.intent_id)?;
    let snapshot = authority.snapshot()?;
    assert_eq!(snapshot.open_orders, 0, "the order slot is returned");
    assert_eq!(
        snapshot.pending_gross_exposure_paise,
        Paise(0),
        "the notional is returned"
    );
    assert!(
        authority
            .evaluate_and_reserve(intent(2, 6, PRICE, Side::Buy)?, observation()?, None)
            .is_ok(),
        "capacity is available again"
    );
    Ok(())
}

/// The crate has no OMS, broker, venue SDK or order-submission dependency.
#[test]
fn no_oms_broker_or_live_coupling() -> TestResult {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "oms",
        "broker",
        "upstox",
        "fyers",
        "kite",
        "paper-engine",
        "execution",
    ] {
        assert!(
            !manifest.to_ascii_lowercase().contains(forbidden),
            "manifest must not depend on {forbidden}"
        );
    }
    Ok(())
}

#[test]
fn feed_health_derivation_is_from_observations() -> TestResult {
    let base = observation()?;
    assert_eq!(
        base.health(LogicalNanos::ZERO, 1_000),
        FeedHealth::Healthy { age_ns: 0 },
        "a fresh two-sided quote is healthy"
    );

    let mut aged = base.clone();
    aged.received_at = LogicalNanos::new(10);
    assert_eq!(
        aged.health(LogicalNanos::new(2_000), 1_000),
        FeedHealth::Stale { age_ns: 1_990 },
        "age is derived from the authority clock"
    );

    let mut crossed = base.clone();
    let mut quote = quote()?;
    quote.bid_price = Some(PricePaise::new(Paise(1_010))?);
    crossed.quote = Some(quote);
    assert_eq!(
        crossed.health(LogicalNanos::ZERO, 1_000),
        FeedHealth::Crossed,
        "a crossed book is detected"
    );

    let mut no_quote = base.clone();
    no_quote.quote = None;
    assert_eq!(
        no_quote.health(LogicalNanos::ZERO, 1_000),
        FeedHealth::NoQuote,
        "no quote is not healthy"
    );
    Ok(())
}

/// A caller cannot claim a receipt time ahead of the authority's clock to
/// defeat the staleness derivation.
#[test]
fn future_observation_receipt_is_rejected() -> TestResult {
    let authority = authority()?;
    let mut observation = observation()?;
    observation.received_at = LogicalNanos::new(1);
    let result = authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation, None);
    assert!(
        matches!(result, Err(AuthorityError::FutureObservation)),
        "a receipt time ahead of the authority clock is refused"
    );
    Ok(())
}

/// A permit returned on replay stays bound to the authority's account, not a
/// field the caller can set.
#[test]
fn permit_account_is_authority_owned() -> TestResult {
    let authority = authority()?;
    let reservation =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation()?, None)?;
    assert_eq!(
        reservation.account_id,
        AccountId::new(ACCOUNT)?,
        "the permit records the authority account"
    );
    Ok(())
}

/// A consumed intent is distinctly reported on replay and cannot be replayed
/// into a second reservation.
#[test]
fn consumed_intent_replay_is_distinguished() -> TestResult {
    let authority = authority()?;
    let reservation =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation()?, None)?;
    authority.apply_fill(reservation.intent_id)?;
    let replay =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation()?, None);
    assert!(
        matches!(replay, Err(AuthorityError::AlreadyConsumed(_))),
        "a filled intent is reported as consumed, not merely rejected"
    );
    assert_eq!(
        authority.snapshot()?.reservation_count,
        0,
        "no second reservation exists"
    );
    Ok(())
}

/// A fabricated or deserialized permit must fail self-validation before a
/// downstream consumer could act on it.
#[test]
fn forged_permit_fails_validation() -> TestResult {
    let authority = authority()?;
    let valid =
        authority.evaluate_and_reserve(intent(1, 2, PRICE, Side::Buy)?, observation()?, None)?;
    assert!(
        valid.validate_invariants().is_ok(),
        "an authority permit validates"
    );

    let mut forged = valid.clone();
    forged.assessment.accepted = false;
    assert!(
        matches!(
            forged.validate_invariants(),
            Err(AuthorityError::InvalidReservation(_))
        ),
        "a permit with a rejected assessment is invalid"
    );

    let mut forged = valid;
    forged.assessment.intent_id = IntentId::new(999)?;
    assert!(
        matches!(
            forged.validate_invariants(),
            Err(AuthorityError::InvalidReservation(_))
        ),
        "a permit whose intent id does not match is invalid"
    );
    Ok(())
}
