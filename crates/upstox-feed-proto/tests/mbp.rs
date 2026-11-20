#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.1-C typed five-level MBP projection tests. Synthetic V3 structures; no
//! real captured session is claimed.

use market_event::{
    MarketByPriceSnapshot, //
    MarketDataEvent,
};
use upstox_feed_proto::{
    FULL_DEPTH_LEVELS, //
    MbpDecodeError,
    market_full_to_mbp,
    millis_to_ns,
    proto::{
        Ltpc, //
        MarketFullFeed,
        MarketLevel,
        Quote,
    },
    rupees_to_paise,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn full(levels: Vec<Quote>) -> MarketFullFeed {
    MarketFullFeed {
        ltpc: Some(Ltpc {
            ltp: 100.0,
            ltt: 1_700_000_000_000,
            ..Default::default()
        }),
        market_level: Some(MarketLevel {
            bid_ask_quote: levels,
        }),
        ..Default::default()
    }
}

fn five_levels() -> Vec<Quote> {
    (0..5)
        .map(|index| Quote {
            bid_q: 100 + index,
            bid_p: 100.0 - (index as f64) * 0.05,
            ask_q: 200 + index,
            ask_p: 100.05 + (index as f64) * 0.05,
        })
        .collect()
}

fn project(feed: &MarketFullFeed) -> Result<MarketByPriceSnapshot, Box<dyn std::error::Error>> {
    match market_full_to_mbp(feed, "nse.cash.test", "NSE", "upstox", "full", 42)? {
        MarketDataEvent::MarketByPriceSnapshot(snapshot) => Ok(snapshot),
        other => Err(format!("unexpected event variant: {other:?}").into()),
    }
}

#[test]
fn five_level_full_projects_to_exact_paise_mbp() -> TestResult {
    let feed = full(five_levels());
    let snapshot = project(&feed)?;
    snapshot.validate()?;
    assert_eq!(
        snapshot.depth_levels, FULL_DEPTH_LEVELS,
        "five levels are declared"
    );
    assert_eq!(snapshot.bids.len(), 5, "five bid levels are projected");
    assert_eq!(snapshot.asks.len(), 5, "five ask levels are projected");
    assert_eq!(
        snapshot.best_bid_paise(),
        Some(10_000),
        "best bid is exact paise"
    );
    assert_eq!(
        snapshot.best_ask_paise(),
        Some(10_005),
        "best ask is exact paise"
    );
    assert_eq!(snapshot.received_utc_ns, 42, "receive time is carried");
    assert_eq!(
        snapshot.source_time_ns, None,
        "full feed supplies no source observation time"
    );
    assert_eq!(
        snapshot.last_trade_time_ns,
        Some(1_700_000_000_000_000_000),
        "last-trade milliseconds are converted to nanoseconds"
    );
    assert_eq!(snapshot.source_id, "upstox", "source identity is carried");
    if let Some(pair) = snapshot.bids.windows(2).next() {
        assert!(
            pair[0].price_paise > pair[1].price_paise,
            "bids stay descending"
        );
    }
    Ok(())
}

#[test]
fn rupee_double_conversion_is_explicit_and_half_away_from_zero() -> TestResult {
    assert_eq!(
        rupees_to_paise(100.005)?,
        10_001,
        "half paise rounds away from zero"
    );
    assert_eq!(
        rupees_to_paise(100.0)?,
        10_000,
        "whole rupees convert exactly"
    );
    assert_eq!(
        millis_to_ns(1_700_000_000_000)?,
        1_700_000_000_000_000_000,
        "epoch milliseconds convert to nanoseconds"
    );
    assert!(
        millis_to_ns(i64::MAX).is_err(),
        "millisecond overflow is refused"
    );
    Ok(())
}

#[test]
fn non_finite_and_negative_provider_prices_are_refused() -> TestResult {
    assert_eq!(
        rupees_to_paise(f64::NAN),
        Err(MbpDecodeError::NonFinitePrice),
        "NaN is refused"
    );
    assert_eq!(
        rupees_to_paise(f64::INFINITY),
        Err(MbpDecodeError::NonFinitePrice),
        "infinity is refused"
    );
    assert_eq!(
        rupees_to_paise(-1.0),
        Err(MbpDecodeError::NegativePrice),
        "negative prices are refused"
    );
    Ok(())
}

#[test]
fn more_than_five_levels_is_rejected_as_a_different_product() -> TestResult {
    let mut levels = five_levels();
    levels.push(Quote {
        bid_q: 1,
        bid_p: 99.9,
        ask_q: 1,
        ask_p: 100.3,
    });
    let result = project(&full(levels));
    assert!(
        matches!(result, Err(error) if error.to_string().contains("full supports at most 5")),
        "D30 depth is not silently accepted by the five-level decoder"
    );
    Ok(())
}

#[test]
fn unordered_levels_are_rejected() -> TestResult {
    let levels = vec![
        Quote {
            bid_q: 1,
            bid_p: 100.0,
            ask_q: 1,
            ask_p: 100.05,
        },
        Quote {
            bid_q: 1,
            bid_p: 101.0,
            ask_q: 1,
            ask_p: 101.05,
        },
    ];
    let result = project(&full(levels));
    assert!(
        result.is_err(),
        "ascending bids are rejected by MBP validation"
    );
    Ok(())
}

#[test]
fn missing_market_level_is_rejected() -> TestResult {
    let feed = MarketFullFeed {
        ltpc: Some(Ltpc::default()),
        market_level: None,
        ..Default::default()
    };
    assert_eq!(
        project(&feed).err().map(|error| error.to_string()),
        Some(MbpDecodeError::MissingMarketLevel.to_string()),
        "a full frame without depth cannot produce MBP"
    );
    Ok(())
}

#[test]
fn negative_level_quantity_is_refused() -> TestResult {
    let mut levels = five_levels();
    levels[0].bid_q = -1;
    assert!(
        project(&full(levels)).is_err(),
        "negative depth quantity is refused"
    );
    Ok(())
}
