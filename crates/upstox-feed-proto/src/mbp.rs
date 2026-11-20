//! Typed five-level MBP projection from Upstox V3 Full frames.
//!
//! Upstox publishes `Quote.bidP`/`askP` as IEEE-754 doubles. Nirnay enters its
//! financial model only in exact integer paise. The conversion rule here is
//! explicit: rupees × 100, rounded half away from zero, with non-finite,
//! negative and out-of-range values rejected. Provider double precision is the
//! source limitation; this module does not add a second one.

use market_event::{
    MarketByPriceSnapshot,
    MarketDataEvent,
    MbpLevel, //
};
use thiserror::Error;

use crate::proto::{
    MarketFullFeed, //
    Quote,
};

/// Maximum levels for the `full` (full_d5) product. D30 is a separate product
/// and is not accepted by this decoder.
pub const FULL_DEPTH_LEVELS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MbpDecodeError {
    #[error("price is not finite")]
    NonFinitePrice,
    #[error("price is negative")]
    NegativePrice,
    #[error("price exceeds the paise range")]
    PriceOverflow,
    #[error("product declares {declared} levels; full supports at most {limit}")]
    DepthMismatch { declared: usize, limit: u32 },
    #[error("market level is absent from the full frame")]
    MissingMarketLevel,
    #[error("snapshot validation failed: {0}")]
    InvalidSnapshot(String),
    #[error("provider millisecond timestamp overflows nanoseconds")]
    TimestampOverflow,
}

/// Convert a provider rupee double into exact paise.
pub fn rupees_to_paise(value: f64) -> Result<i64, MbpDecodeError> {
    if !value.is_finite() {
        return Err(MbpDecodeError::NonFinitePrice);
    }
    if value < 0.0 {
        return Err(MbpDecodeError::NegativePrice);
    }
    let scaled = value * 100.0;
    if scaled >= i64::MAX as f64 {
        return Err(MbpDecodeError::PriceOverflow);
    }
    Ok(scaled.round() as i64)
}

/// Upstox `LTPC.ltt` is Unix epoch milliseconds. Convert to nanoseconds with
/// checked arithmetic; do not store milliseconds in a nanosecond field.
pub fn millis_to_ns(milliseconds: i64) -> Result<i64, MbpDecodeError> {
    i64::try_from(
        i128::from(milliseconds)
            .checked_mul(1_000_000)
            .ok_or(MbpDecodeError::TimestampOverflow)?,
    )
    .map_err(|_| MbpDecodeError::TimestampOverflow)
}

fn level(quote: &Quote) -> Result<(MbpLevel, MbpLevel), MbpDecodeError> {
    if quote.bid_q < 0 || quote.ask_q < 0 {
        return Err(MbpDecodeError::InvalidSnapshot(
            "negative level quantity".to_owned(),
        ));
    }
    Ok((
        MbpLevel {
            price_paise: rupees_to_paise(quote.bid_p)?,
            quantity: quote.bid_q,
            order_count: None,
        },
        MbpLevel {
            price_paise: rupees_to_paise(quote.ask_p)?,
            quantity: quote.ask_q,
            order_count: None,
        },
    ))
}

/// Project a decoded V3 Full frame into a typed five-level MBP snapshot.
pub fn market_full_to_mbp(
    feed: &MarketFullFeed,
    instrument_id: impl Into<String>,
    venue: impl Into<String>,
    source_id: impl Into<String>,
    product: impl Into<String>,
    received_utc_ns: i64,
) -> Result<MarketDataEvent, MbpDecodeError> {
    let market_level = feed
        .market_level
        .as_ref()
        .ok_or(MbpDecodeError::MissingMarketLevel)?;
    let quotes = &market_level.bid_ask_quote;
    if quotes.len() > FULL_DEPTH_LEVELS as usize {
        return Err(MbpDecodeError::DepthMismatch {
            declared: quotes.len(),
            limit: FULL_DEPTH_LEVELS,
        });
    }
    let mut bids = Vec::with_capacity(quotes.len());
    let mut asks = Vec::with_capacity(quotes.len());
    for quote in quotes {
        let (bid, ask) = level(quote)?;
        bids.push(bid);
        asks.push(ask);
    }
    let snapshot = MarketByPriceSnapshot {
        instrument_id: instrument_id.into(),
        venue: venue.into(),
        source_id: source_id.into(),
        product: product.into(),
        depth_levels: FULL_DEPTH_LEVELS,
        bids,
        asks,
        source_time_ns: None,
        last_trade_time_ns: feed
            .ltpc
            .as_ref()
            .map(|ltpc| millis_to_ns(ltpc.ltt))
            .transpose()?,
        received_utc_ns,
    };
    snapshot
        .validate()
        .map_err(|error| MbpDecodeError::InvalidSnapshot(error.to_string()))?;
    Ok(MarketDataEvent::MarketByPriceSnapshot(snapshot))
}
