//! Typed market-by-price payloads.
//!
//! The common envelope in [`crate::KernelEvent`] carries identity, provenance
//! and quality. Financial meaning lives in these typed payloads. A five-level
//! MBP snapshot is not a market-by-order book and never claims order identity.

use serde::{
    Deserialize, //
    Serialize,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MbpError {
    #[error("levels are not in price order")]
    UnorderedLevels,
    #[error("duplicate price level {0}")]
    DuplicatePrice(i64),
    #[error("level quantity must be non-negative")]
    NegativeQuantity,
    #[error("empty price level")]
    EmptyLevel,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MbpSide {
    Bid,
    Ask,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MbpLevel {
    /// Exact integer paise. Provider doubles are converted before entering
    /// this type, with the conversion rule recorded by the decoder.
    pub price_paise: i64,
    pub quantity: i64,
    /// Displayed order count, when the product supplies one.
    pub order_count: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketByPriceSnapshot {
    pub instrument_id: String,
    pub venue: String,
    pub source_id: String,
    pub product: String,
    pub depth_levels: u32,
    pub bids: Vec<MbpLevel>,
    pub asks: Vec<MbpLevel>,
    /// Source observation time, when the product supplies one. Upstox Full
    /// does not, so this stays None rather than substituting last-trade time.
    pub source_time_ns: Option<i64>,
    /// Last-trade time from the feed, in nanoseconds. Kept separate from
    /// source observation time and from local receive time.
    pub last_trade_time_ns: Option<i64>,
    pub received_utc_ns: i64,
}

impl MarketByPriceSnapshot {
    pub fn validate(&self) -> Result<(), MbpError> {
        if self.bids.is_empty() || self.asks.is_empty() {
            return Err(MbpError::EmptyLevel);
        }
        validate_side(&self.bids, true)?;
        validate_side(&self.asks, false)?;
        let declared = usize::try_from(self.depth_levels).unwrap_or(usize::MAX);
        if self.bids.len() > declared || self.asks.len() > declared {
            return Err(MbpError::UnorderedLevels);
        }
        Ok(())
    }

    #[must_use]
    pub fn best_bid_paise(&self) -> Option<i64> {
        self.bids.first().map(|level| level.price_paise)
    }

    #[must_use]
    pub fn best_ask_paise(&self) -> Option<i64> {
        self.asks.first().map(|level| level.price_paise)
    }
}

fn validate_side(levels: &[MbpLevel], descending: bool) -> Result<(), MbpError> {
    let mut previous: Option<i64> = None;
    for level in levels {
        if level.quantity < 0 || level.price_paise <= 0 {
            return Err(MbpError::NegativeQuantity);
        }
        if let Some(last) = previous {
            if level.price_paise == last {
                return Err(MbpError::DuplicatePrice(level.price_paise));
            }
            if descending && level.price_paise > last {
                return Err(MbpError::UnorderedLevels);
            }
            if !descending && level.price_paise < last {
                return Err(MbpError::UnorderedLevels);
            }
        }
        previous = Some(level.price_paise);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketByPriceDelta {
    pub instrument_id: String,
    pub venue: String,
    pub source_id: String,
    pub product: String,
    pub sequence: Option<u64>,
    pub side: MbpSide,
    pub price_paise: i64,
    pub quantity_change: i64,
    pub order_count: Option<u32>,
    pub received_utc_ns: i64,
}

/// One typed market-data payload. Variants are added when a real product and
/// decoder exist, never to pre-announce unsupported depth or MBO semantics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketDataEvent {
    MarketByPriceSnapshot(MarketByPriceSnapshot),
    MarketByPriceDelta(MarketByPriceDelta),
}
