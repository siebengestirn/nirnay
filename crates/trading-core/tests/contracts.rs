//! Public contract tests for the ported trading domain.
//!
//! These pin the type-boundary decisions: canonical identity, explicit
//! money/quantity conversion, deterministic event identity and the
//! event-time/source-time split.

use std::collections::BTreeSet;

use trading_core::{
    AccountId, Continuity, DataQualityFlag, DomainError, DoubledPaise, EventEnvelope, EventId,
    EventPayload, EventTimes, Instrument, InstrumentCatalog, InstrumentId, InstrumentType, Listing,
    ListingStatus, ListingTerms, LogicalNanos, Lots, OrderStatus, Paise, PricePaise, Quantity,
    QuoteSnapshot, RunId, Segment, SettlementRegime, StoredBody, TickAssignment, TradeDate,
    UtcNanos, Venue, is_active_on,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn run() -> RunId {
    RunId::from_bytes([
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ])
}

fn envelope() -> EventEnvelope {
    EventEnvelope {
        event_id: EventId::for_run(run(), 1),
        schema_version: 1,
        stream_id: "paper-account".to_owned(),
        source_id: None,
        source_clock_domain: None,
        session_id: None,
        source_sequence: None,
        correlation_id: None,
        causation_id: None,
        times: EventTimes {
            source_time_utc: None,
            received_time_utc: UtcNanos::new(1_000),
            available_time: LogicalNanos::new(1_000),
            recorded_time_utc: UtcNanos::new(1_001),
        },
        quality: BTreeSet::new(),
    }
}

fn date(year: i32, month: u32, day: u32) -> Result<TradeDate, Box<dyn std::error::Error>> {
    TradeDate::new(year as i16, month as i8, day as i8)
        .ok()
        .ok_or_else(|| "invalid calendar date".into())
}

fn identity(value: &str) -> Result<InstrumentId, Box<dyn std::error::Error>> {
    Ok(InstrumentId::new(value)?)
}

fn index_instrument(identity_value: &str) -> Result<Instrument, Box<dyn std::error::Error>> {
    Ok(Instrument {
        instrument_id: identity(identity_value)?,
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

#[test]
fn price_rejects_zero_and_round_trips_through_serde() -> TestResult {
    assert_eq!(PricePaise::new(Paise(0)), Err(DomainError::ZeroPrice));
    assert!(PricePaise::new(Paise(-1)).is_err());

    let price = PricePaise::new(Paise(15_000))?;
    let encoded = serde_json::to_string(&price)?;
    assert_eq!(encoded, "15000");
    let decoded: PricePaise = serde_json::from_str(&encoded)?;
    assert_eq!(decoded, price);

    assert!(serde_json::from_str::<PricePaise>("0").is_err());
    Ok(())
}

#[test]
fn price_notional_is_exact_and_overflow_is_rejected() -> TestResult {
    let price = PricePaise::new(Paise(1_500))?;
    assert_eq!(price.notional(Quantity::new(3)?), Ok(Paise(4_500)));
    assert_eq!(
        price.notional(Quantity::new(i64::MAX)?),
        Err(DomainError::Overflow)
    );
    Ok(())
}

#[test]
fn quantity_is_non_negative_lots_and_conversion_is_lossless() -> TestResult {
    let quantity = Quantity::new(7)?;
    assert_eq!(quantity.get(), 7);
    assert_eq!(quantity.as_lots(), Lots(7));
    assert_eq!(Lots::from(quantity), Lots(7));

    assert_eq!(Quantity::new(-1), Err(DomainError::NegativeQuantity));
    assert_eq!(Quantity::new(0), Ok(Quantity::ZERO));
    assert!(serde_json::from_str::<Quantity>("-1").is_err());
    assert_eq!(serde_json::from_str::<Quantity>("7")?, quantity);
    Ok(())
}

#[test]
fn event_identity_is_deterministic_for_the_same_run_and_ordinal() {
    let first = EventId::for_run(run(), 42);
    let second = EventId::for_run(run(), 42);
    assert_eq!(first, second);
    assert_ne!(first, EventId::for_run(run(), 43));
    assert_ne!(first, EventId::for_run(RunId::from_bytes([0; 16]), 42));

    assert_eq!(first.as_str().len(), EventId::HEX_LEN);
    assert!(
        first
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert!(EventId::parse(first.as_str()).is_ok());
    assert!(EventId::parse("A".repeat(EventId::HEX_LEN)).is_err());
    assert!(EventId::parse("abc").is_err());
}

#[test]
fn source_time_requires_a_source_clock_domain() {
    let mut value = envelope();
    value.times.source_time_utc = Some(UtcNanos::new(5));
    assert!(value.validate().is_err());

    value.source_id = Some("nse-feed".to_owned());
    value.source_clock_domain = Some("nse-exchange".to_owned());
    assert!(value.validate().is_ok());
}

#[test]
fn quote_quantity_without_a_price_is_rejected() -> TestResult {
    let quote = QuoteSnapshot {
        instrument_id: identity("nse.cash.synidx")?,
        venue: Venue::Nse,
        bid_price: None,
        bid_quantity: Quantity::new(5)?,
        ask_price: Some(PricePaise::new(Paise(1_005))?),
        ask_quantity: Quantity::new(5)?,
        continuity: Continuity::Snapshot,
    };
    assert!(quote.validate().is_err());

    let two_sided = QuoteSnapshot {
        bid_price: Some(PricePaise::new(Paise(995))?),
        ..quote
    };
    assert!(two_sided.validate().is_ok());
    assert_eq!(
        two_sided.midpoint_doubled(),
        Some(DoubledPaise::from_paise_pair(Paise(995), Paise(1_005)))
    );
    Ok(())
}

#[test]
fn terminal_order_states_never_transition() {
    assert!(OrderStatus::Working.permits(OrderStatus::PartiallyFilled));
    assert!(OrderStatus::CancelPending.permits(OrderStatus::Filled));
    assert!(OrderStatus::OutcomeUnknown.permits(OrderStatus::Rejected));
    assert!(!OrderStatus::Filled.permits(OrderStatus::Cancelled));
    assert!(!OrderStatus::Cancelled.permits(OrderStatus::Working));
    assert!(!OrderStatus::Rejected.permits(OrderStatus::OutcomeUnknown));
}

#[test]
fn quoted_event_without_source_identity_is_rejected_before_journaling() -> TestResult {
    let body = StoredBody {
        envelope: envelope(),
        payload: EventPayload::QuoteReceived {
            quote: QuoteSnapshot {
                instrument_id: identity("nse.cash.synidx")?,
                venue: Venue::Nse,
                bid_price: Some(PricePaise::new(Paise(995))?),
                bid_quantity: Quantity::new(1)?,
                ask_price: Some(PricePaise::new(Paise(1_005))?),
                ask_quantity: Quantity::new(1)?,
                continuity: Continuity::Snapshot,
            },
        },
    };
    assert!(body.validate().is_err());

    let mut sourced = body;
    sourced.envelope.source_id = Some("nse-feed".to_owned());
    sourced.envelope.source_clock_domain = Some("nse-exchange".to_owned());
    sourced.envelope.source_sequence = Some(10);
    assert!(sourced.validate().is_ok());
    Ok(())
}

#[test]
fn account_opening_round_trips_and_requires_positive_cash() -> TestResult {
    let body = StoredBody {
        envelope: envelope(),
        payload: EventPayload::AccountOpened {
            account_id: AccountId::new(1)?,
            starting_cash: Paise(1_000_000),
            trade_date: date(2026, 10, 28)?,
        },
    };
    assert!(body.validate().is_ok());

    let encoded = serde_json::to_string(&body)?;
    let decoded: StoredBody = serde_json::from_str(&encoded)?;
    assert_eq!(decoded, body);

    let mut invalid = body;
    invalid.payload = EventPayload::AccountOpened {
        account_id: AccountId::new(1)?,
        starting_cash: Paise(0),
        trade_date: date(2026, 10, 28)?,
    };
    assert!(invalid.validate().is_err());
    Ok(())
}

#[test]
fn instrument_catalog_rejects_duplicate_identity() -> TestResult {
    let instrument = index_instrument("nse.cash.synidx")?;
    assert!(instrument.validate().is_ok());
    assert!(is_active_on(&instrument, date(2026, 6, 1)?));
    assert!(!is_active_on(&instrument, date(2027, 1, 1)?));

    let catalog = InstrumentCatalog::new(vec![instrument.clone()])?;
    assert_eq!(catalog.len(), 1);
    assert!(
        catalog
            .at(&instrument.instrument_id, date(2026, 6, 1)?)
            .is_some()
    );
    assert!(
        catalog
            .at(&instrument.instrument_id, date(2027, 1, 1)?)
            .is_none()
    );

    assert!(matches!(
        InstrumentCatalog::new(vec![instrument.clone(), instrument]),
        Err(DomainError::DuplicateInstrument(_))
    ));
    Ok(())
}

#[test]
fn event_quality_flags_are_ordered_and_serializable() -> TestResult {
    let mut envelope = envelope();
    envelope.quality.insert(DataQualityFlag::SequenceGap);
    envelope.quality.insert(DataQualityFlag::LateArrival);
    let encoded = serde_json::to_string(&envelope)?;
    let decoded: EventEnvelope = serde_json::from_str(&encoded)?;
    assert_eq!(decoded, envelope);
    Ok(())
}

#[test]
fn exact_midpoint_round_trip_sweep() -> TestResult {
    // Deterministic pseudo-random sweep: the doubled value must equal the
    // reference sum exactly, and rounded_paise must respect the documented
    // half-even policy|no-floors (ties to even).
    let mut state = 0x1234_5678_9abc_def0_u64;
    let mut next = move || {
        state = state
            .wrapping_mul(0x2545_f491_4f6c_dd1d)
            .wrapping_add(0x9e37_79b9_7f4a_7c15);
        (state >> 11) & 0x1_FFFF_FFFF
    };
    for _ in 0..4096 {
        // Prices span both parities and broad magnitudes, including large
        // extremes that would overflow i64 arithmetic (they don't here).
        let bid = i64::try_from(next() % 1_800_000_000_000).unwrap_or(1) + 1;
        let ask = i64::try_from(next() % 1_800_000_000_000).unwrap_or(1) + 1;
        let midpoint = DoubledPaise::from_paise_pair(Paise(bid), Paise(ask));
        assert_eq!(midpoint.value(), i128::from(bid) + i128::from(ask));
        assert_eq!(
            midpoint,
            DoubledPaise::from_paise_pair(Paise(ask), Paise(bid))
        );
        let rounded = midpoint.rounded_paise()?;
        let float_mid = (bid as f64 + ask as f64) / 2.0;
        let float_rounded = float_mid.round_ties_even() as i64;
        assert_eq!(
            rounded.0, float_rounded,
            "bid {bid} ask {ask} (f64 ties-to-even oracle)"
        );
    }
    Ok(())
}

#[test]
fn doubled_paise_rounding_policy_is_half_even() -> TestResult {
    // Ties go to the even neighbour; non-ties go to nearest.
    assert_eq!(
        DoubledPaise::from_paise_pair(Paise(100), Paise(101)).rounded_paise()?,
        Paise(100)
    ); // 100.5 -> 100 (even)
    assert_eq!(
        DoubledPaise::from_paise_pair(Paise(100), Paise(103)).rounded_paise()?,
        Paise(102)
    ); // 101.5 -> 102 (even)
    assert_eq!(
        DoubledPaise::from_paise_pair(Paise(101), Paise(102)).rounded_paise()?,
        Paise(102)
    ); // 101.5 -> 102
    assert_eq!(
        DoubledPaise::from_paise_pair(Paise(102), Paise(103)).rounded_paise()?,
        Paise(102)
    ); // 102.5 -> 102
    // Larger magnitudes, same policy.
    assert_eq!(
        DoubledPaise::from_paise_pair(Paise(999_999), Paise(1_000_000)).rounded_paise()?,
        Paise(1_000_000)
    );
    Ok(())
}
