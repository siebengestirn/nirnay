#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.8 bounded end-to-end paper/shadow certification harness.
//!
//! Wires the M2.1–M2.7 components into one deterministic pass over a synthetic
//! verified session. This is a systems integration test, not a claim of real
//! source capture, broker interaction, validated alpha or live readiness.

pub mod report;

use std::path::Path;

use authoritative_oms::{
    BrokerOrderId, //
    ClientOrderId,
    OmsStore,
    PermitRef,
};
use broker_adapter::{
    BrokerAdapter, //
    BrokerEvent,
    BrokerOrderRequest, //
    SimulatedBroker,
    SimulatorConfig, //
    SubmissionOutcome,
    SubmitBehavior,
};
use market_event::{
    OriginClass, //
    SequenceScope,
    StreamState,
    capture_frame_id,
};
use market_kernel::{
    KernelSession, //
    SourceObservation,
    project_session,
};
use ops_runtime::{
    HealthSnapshot, //
    RuntimeThresholds,
};
use report::{
    DecisionRecord, //
    PaperReport,
};
use risk_authority::{
    FeedObservation, //
    RiskAuthority,
    SequenceGap,
};
use risk_engine::{
    RiskPolicy, //
    ValidatedRiskPolicy,
};
use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};
use strategy_runtime::{
    FrozenArtifact, //
    Rational,
    RationalEmaCross, //
    RuntimeIntent,
    SUPPORTED_RULE,
    Strategy, //
    StrategyEvent,
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
    OrderIntent,
    Paise,
    PricePaise,
    Product,
    Quantity,
    QuoteSnapshot,
    Segment,
    SettlementRegime,
    Side,
    StrategyId,
    TickAssignment,
    TradeDate,
    Venue,
};

/// Serializable input for one paper run. All market data is synthetic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperInput {
    pub schema_version: u32,
    pub instrument_id: String,
    pub account_id: u64,
    pub strategy_id: u64,
    pub algo_id: u64,
    pub trade_date: TradeDate,
    pub policy: RiskPolicy,
    pub fast: i64,
    pub slow: i64,
    pub quantity: i64,
    pub min_observations: u32,
    pub aggressive_limit_bps: i64,
    /// Each entry is a synthetic (bid, ask) in paise and a receive age in ns.
    pub quotes: Vec<(i64, i64, u64)>,
    pub feed_age_ns: u64,
    pub disk_available_bytes: u64,
    /// Scripted broker behaviour: `ack`, `timeout` or `fill`.
    pub broker_behavior: String,
}

/// Outcome of one pipeline pass, including the report and the OMS store so
/// tests can inspect persisted lifecycle state.
pub struct PaperRun {
    pub report: PaperReport,
    pub oms: OmsStore,
}

fn instrument(input: &PaperInput) -> Result<Instrument, String> {
    let from = TradeDate::new(2026, 1, 1).map_err(|e| e.to_string())?;
    let to = TradeDate::new(2026, 12, 31).map_err(|e| e.to_string())?;
    Ok(Instrument {
        instrument_id: InstrumentId::new(input.instrument_id.clone()).map_err(|e| e.to_string())?,
        instrument_type: InstrumentType::Index,
        isin: None,
        name: "Synthetic paper index".to_owned(),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: Segment::Cash,
            venue_symbol: "PAPER".to_owned(),
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
                    tick_paise: 1,
                    reference_close_paise: None,
                    reference_close_date: None,
                    reference_source: None,
                    reference_sha256_hex: None,
                    effective_month: None,
                    schedule_revision: "paper-fixture".to_owned(),
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

fn synthetic_session(input: &PaperInput) -> Result<(KernelSession, Vec<QuoteSnapshot>), String> {
    let mut observations = Vec::new();
    let mut quotes = Vec::new();
    for (index, (bid, ask, age_ns)) in input.quotes.iter().enumerate() {
        let payload = format!("paper:{bid}:{ask}:{index}");
        let bytes = payload.as_bytes();
        let frame = capture_frame_id(bytes).map_err(|e| e.to_string())?;
        let sequence = (index as u64) + 1;
        let receive_utc_ns = 1_700_000_000_000_000_000_i64 + sequence as i64;
        observations.push(SourceObservation {
            raw_frame_hash: frame.clone(),
            source_id: "upstox".to_owned(),
            source_session_id: "paper-session".to_owned(),
            provider_instrument_id: input.instrument_id.clone(),
            canonical_payload: bytes.to_vec(),
            origin_class: OriginClass::SdkCallback,
            receive_utc_ns,
            receive_monotonic_ns: sequence + *age_ns,
            source_time_ns: Some(1_699_999_999_000_000_000 + sequence as i64),
            source_clock_domain: Some("provider_feed_clock".to_owned()),
            channel_id: Some("paper-channel".to_owned()),
            connection_epoch: 1,
            receive_sequence: sequence,
            sequence: Some(sequence),
            sequence_scope: SequenceScope::Connection,
            reference_revision: Some("paper-revision".to_owned()),
            instrument_id: Some(input.instrument_id.clone()),
            evidence_token: None,
        });
        quotes.push(QuoteSnapshot {
            instrument_id: InstrumentId::new(input.instrument_id.clone())
                .map_err(|e| e.to_string())?,
            venue: Venue::Nse,
            bid_price: Some(PricePaise::new(Paise(*bid)).map_err(|e| e.to_string())?),
            bid_quantity: Quantity::new(1).map_err(|e| e.to_string())?,
            ask_price: Some(PricePaise::new(Paise(*ask)).map_err(|e| e.to_string())?),
            ask_quantity: Quantity::new(1).map_err(|e| e.to_string())?,
            continuity: Continuity::Snapshot,
        });
    }
    Ok((
        KernelSession {
            schema_version: market_kernel::KERNEL_SESSION_SCHEMA_VERSION,
            source_scope: "NSE".to_owned(),
            session_reference_revision: Some("paper-revision".to_owned()),
            observations,
        },
        quotes,
    ))
}

fn frozen_artifact(input: &PaperInput) -> FrozenArtifact {
    FrozenArtifact {
        candidate_id: "paper.rational-ema.v1".to_owned(),
        hypothesis: "Synthetic systems fixture for end-to-end paper wiring".to_owned(),
        return_source: "momentum".to_owned(),
        universe: vec![input.instrument_id.clone()],
        entry_rule: SUPPORTED_RULE.to_owned(),
        exit_rule: "opposite_cross".to_owned(),
        rebalance_frequency: Some("intraday".to_owned()),
        experiment_ids: vec!["paper-exp-0001".to_owned()],
        validation: strategy_runtime::ArtifactValidation {
            walk_forward_windows: 2,
            holdout_locked: true,
            multiple_testing_correction: "bonferroni".to_owned(),
            deflated_sharpe: "0.000000".to_owned(),
            cost_model: "paper-fixture-no-costs".to_owned(),
            probability_of_backtest_overfit: Some("1.000000".to_owned()),
        },
        status: strategy_runtime::ArtifactStatus::ApprovedForPaper,
        artifact_sha256: String::new(),
    }
}

fn order_intent(
    input: &PaperInput,
    intent: &RuntimeIntent,
    observation_id_text: &str,
    ordinal: u64,
) -> Result<OrderIntent, String> {
    let side = match intent.side.as_str() {
        "buy" => Side::Buy,
        "sell" => Side::Sell,
        other => return Err(format!("unsupported side `{other}`")),
    };
    Ok(OrderIntent {
        intent_id: IntentId::new(ordinal).map_err(|e| e.to_string())?,
        account_id: AccountId::new(input.account_id).map_err(|e| e.to_string())?,
        strategy_id: StrategyId::new(input.strategy_id).map_err(|e| e.to_string())?,
        strategy_version: "paper-strategy-v1".to_owned(),
        algo_id: AlgoId::new(input.algo_id).map_err(|e| e.to_string())?,
        instrument_id: InstrumentId::new(input.instrument_id.clone()).map_err(|e| e.to_string())?,
        venue: Venue::Nse,
        side,
        product: Product::Intraday,
        quantity: Quantity::new(intent.quantity).map_err(|e| e.to_string())?,
        limit_price: PricePaise::new(Paise(intent.limit_price_paise)).map_err(|e| e.to_string())?,
        reason: format!(
            "paper intent {} from {observation_id_text}",
            intent.intent_id
        ),
        evidence: vec![JournalRef {
            sequence: ordinal,
            hash_hex: observation_id_text.to_owned(),
        }],
        expires_at: LogicalNanos::new(1_000_000),
    })
}

fn thresholds() -> RuntimeThresholds {
    RuntimeThresholds {
        max_feed_age_ns: 1_500_000_000,
        max_journal_append_latency_ns: 10_000_000,
        max_journal_queue_depth: 100,
        min_disk_available_bytes: 1_000_000,
        max_unknown_orders: 0,
        max_reconciliation_differences: 0,
        max_sequence_gaps: 0,
    }
}

/// Execute one bounded paper pass over a synthetic session.
pub fn run_paper(input: &PaperInput, oms_path: &Path) -> Result<PaperRun, String> {
    let (session, quotes) = synthetic_session(input)?;
    let events = project_session(&session).map_err(|e| e.to_string())?;
    let instrument = instrument(input)?;
    let policy = ValidatedRiskPolicy::from_policy(&input.policy).map_err(|e| e.to_string())?;
    let authority = RiskAuthority::new(
        AccountId::new(input.account_id).map_err(|e| e.to_string())?,
        policy,
        vec![instrument],
        input.trade_date,
    );
    authority
        .set_now(LogicalNanos::new(input.feed_age_ns))
        .map_err(|e| e.to_string())?;
    let mut store = OmsStore::open(oms_path).map_err(|e| e.to_string())?;
    let mut strategy = RationalEmaCross::from_artifact(
        &frozen_artifact(input),
        input.fast,
        input.slow,
        input.quantity,
        input.min_observations,
        input.aggressive_limit_bps,
    )
    .map_err(|e| e.to_string())?;

    let behavior = match input.broker_behavior.as_str() {
        "ack" => SubmitBehavior::ImmediateAck,
        "timeout" => SubmitBehavior::Timeout,
        "fill" => SubmitBehavior::AckThenFills {
            fills: vec![(input.quantity, 1_000)],
        },
        other => return Err(format!("unsupported broker behavior `{other}`")),
    };
    let mut config = SimulatorConfig::new("paper-sim", behavior);
    config.duplicate_events = true;
    let mut broker = SimulatedBroker::new(config).map_err(|e| e.to_string())?;
    broker.authenticate();

    let mut health = HealthSnapshot {
        feed_age_ns: input.feed_age_ns,
        sequence_gaps: events
            .iter()
            .filter(|event| matches!(event.quality, StreamState::Gap | StreamState::Recovering))
            .count() as u32,
        reconnect_count: 0,
        journal_append_latency_ns: 1_000,
        journal_queue_depth: 0,
        disk_available_bytes: input.disk_available_bytes,
        open_orders: 0,
        unknown_orders: 0,
        reconciliation_differences: 0,
        kill_switch_engaged: false,
        account_snapshot_available: true,
    };

    let mut decisions = Vec::new();
    let mut intents_emitted = 0_usize;
    let mut reservations = 0_usize;
    let mut ordinal = 0_u64;
    let mut position = 0_i64;

    for (index, event) in events.iter().enumerate() {
        let quote = &quotes[index];
        let bid = quote.bid_price.map(|price| price.get().0).unwrap_or(0);
        let ask = quote.ask_price.map(|price| price.get().0).unwrap_or(0);
        let midpoint = Rational::fraction(bid + ask, 2).map_err(|e| e.to_string())?;
        let strategy_event = StrategyEvent {
            observation_sequence: event.sequence.unwrap_or((index as u64) + 1),
            instrument_id: input.instrument_id.clone(),
            midpoint,
            best_bid: bid,
            best_ask: ask,
            position_quantity: position,
            has_open_order: false,
        };
        let Some(intent) = strategy
            .on_event(&strategy_event)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        intents_emitted += 1;
        ordinal += 1;
        let order = order_intent(input, &intent, event.observation_id.as_str(), ordinal)?;
        let mut record = DecisionRecord {
            intent_id: intent.intent_id.clone(),
            observation_id: event.observation_id.to_string(),
            risk_accepted: false,
            permit_id: None,
            oms_client_id: ordinal,
            oms_final_state: "not_created".to_owned(),
            broker_order_id: None,
            quantity: intent.quantity,
            rejection_reason: None,
        };

        // Refresh operational health from live OMS state before each gate.
        let mut open_orders = 0_u32;
        for id in 1..=ordinal {
            if let Some(order) = store
                .order(ClientOrderId::new(id).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                && !order.state.terminal()
            {
                open_orders = open_orders.saturating_add(1);
            }
        }
        health.open_orders = open_orders;
        health.unknown_orders =
            u32::try_from(store.unresolved().map_err(|e| e.to_string())?.len()).unwrap_or(u32::MAX);
        let verdict = health.evaluate(&thresholds());
        if !verdict.safe_for_new_risk {
            record.rejection_reason = Some(format!(
                "health gate blocked new risk: {}",
                verdict.reasons.join(", ")
            ));
            decisions.push(record);
            continue;
        }

        let gap = if matches!(
            event.quality,
            StreamState::Gap | StreamState::Recovering | StreamState::Unverified
        ) {
            Some(SequenceGap {
                expected: None,
                observed: event.sequence.unwrap_or(0),
            })
        } else {
            None
        };
        let feed_observation = FeedObservation {
            quote: Some(quote.clone()),
            received_at: LogicalNanos::new(input.feed_age_ns),
            gap,
            book_complete: true,
        };
        let reservation =
            match authority.evaluate_and_reserve(order.clone(), feed_observation, None) {
                Ok(value) => value,
                Err(error) => {
                    record.rejection_reason = Some(error.to_string());
                    decisions.push(record);
                    continue;
                }
            };
        record.risk_accepted = true;
        record.permit_id = Some(reservation.permit_id);
        reservations += 1;

        let client = ClientOrderId::new(ordinal).map_err(|e| e.to_string())?;
        let permit = PermitRef {
            permit_id: reservation.permit_id,
            policy_version: reservation.assessment.policy_version.clone(),
            snapshot_revision: reservation.snapshot_revision,
        };
        store
            .create(client, &order, &permit, ordinal as i64)
            .map_err(|e| e.to_string())?;
        store
            .accept_risk_approved(client, ordinal as i64 + 1)
            .map_err(|e| e.to_string())?;
        let claim = store
            .claim("paper-dispatcher", ordinal as i64 + 2, 1_000)
            .map_err(|e| e.to_string())?
            .ok_or("no submission claim")?;
        store
            .mark_dispatching(claim.attempt_id, ordinal as i64 + 3)
            .map_err(|e| e.to_string())?;
        let request = BrokerOrderRequest::from_claim(&claim, &permit, broker.adapter_id())
            .map_err(|e| e.to_string())?;
        match broker.submit(&request).map_err(|e| e.to_string())? {
            SubmissionOutcome::Acknowledged(ack) => {
                let broker_id =
                    BrokerOrderId::new(ack.broker_order_id.clone()).map_err(|e| e.to_string())?;
                store
                    .acknowledge(
                        claim.attempt_id,
                        &broker_id,
                        &ack.broker_event_id,
                        ordinal as i64 + 4,
                    )
                    .map_err(|e| e.to_string())?;
                record.broker_order_id = Some(ack.broker_order_id);
                for event in broker.poll_events().map_err(|e| e.to_string())? {
                    if let BrokerEvent::Filled {
                        execution_id,
                        broker_event_id,
                        quantity,
                        price_paise,
                        ..
                    } = event
                    {
                        let applied = store
                            .apply_fill(
                                client,
                                &execution_id,
                                &broker_event_id,
                                quantity,
                                price_paise,
                                ordinal as i64 + 5,
                            )
                            .map_err(|e| e.to_string())?;
                        if applied {
                            position = position.saturating_add(quantity);
                            authority
                                .apply_fill(order.intent_id)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
            SubmissionOutcome::TimedOut { .. } => {
                store
                    .expire_dispatched(ordinal as i64 + 10, 1)
                    .map_err(|e| e.to_string())?;
            }
            SubmissionOutcome::Rejected {
                reason,
                broker_event_id,
                ..
            } => {
                store
                    .reject_submission(
                        claim.attempt_id,
                        &reason,
                        &broker_event_id,
                        ordinal as i64 + 4,
                    )
                    .map_err(|e| e.to_string())?;
                record.rejection_reason = Some(reason);
            }
        }
        let final_state = store
            .order(client)
            .map_err(|e| e.to_string())?
            .map(|order| order.state);
        record.oms_final_state = final_state
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|| "unknown_order".to_owned());
        decisions.push(record);
    }

    // Final verdict reflects state after the last decision, including any
    // UNKNOWN orders left by a dispatch timeout.
    health.unknown_orders =
        u32::try_from(store.unresolved().map_err(|e| e.to_string())?.len()).unwrap_or(u32::MAX);
    let final_verdict = health.evaluate(&thresholds());
    let input_bytes = serde_json::to_vec(input).map_err(|e| e.to_string())?;
    let input_digest = Sha256::digest(&input_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let report = PaperReport::new(
        events.len(),
        intents_emitted,
        reservations,
        final_verdict.safe_for_new_risk,
        final_verdict.reasons,
        decisions,
        input_digest,
    );
    Ok(PaperRun { report, oms: store })
}
