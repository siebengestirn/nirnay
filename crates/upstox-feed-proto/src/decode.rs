//! Structural decoding helpers for `FeedResponse` frames.

use crate::proto::{FeedResponse, feed::FeedUnion};
use prost::Message;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("protobuf decode failed: {0}")]
    Decode(String),
}

pub type DecodeResult<T> = Result<T, DecodeError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedResponseType {
    InitialFeed,
    LiveFeed,
    MarketInfo,
    Unknown(i32),
}

pub fn response_type_from_raw(raw: i32) -> FeedResponseType {
    match raw {
        0 => FeedResponseType::InitialFeed,
        1 => FeedResponseType::LiveFeed,
        2 => FeedResponseType::MarketInfo,
        value => FeedResponseType::Unknown(value),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    Ltpc,
    FullFeed,
    FirstLevelWithGreeks,
    Unset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedSummary {
    pub key: String,
    pub kind: FeedKind,
    pub request_mode_raw: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedResponseDiagnostic {
    pub response_type_raw: i32,
    pub response_type: FeedResponseType,
    pub current_ts: i64,
    pub has_market_info: bool,
    pub feed_keys: Vec<String>,
    pub feeds: Vec<FeedSummary>,
}

/// Decode raw bytes into the generated `FeedResponse`.
///
/// Unknown fields are skipped by prost as usual; unknown enum values are
/// preserved in the raw `i32` fields.
pub fn decode_feed_response(bytes: &[u8]) -> DecodeResult<FeedResponse> {
    FeedResponse::decode(bytes).map_err(|error| DecodeError::Decode(error.to_string()))
}

/// Build a deterministic diagnostic projection: keys sorted, no ordering
/// implied by arrival position.
pub fn decode_feed_response_diagnostic(bytes: &[u8]) -> DecodeResult<FeedResponseDiagnostic> {
    let response = decode_feed_response(bytes)?;
    let mut feeds: Vec<FeedSummary> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for (key, feed) in &response.feeds {
        keys.push(key.clone());
        let kind = match &feed.feed_union {
            Some(FeedUnion::Ltpc(_)) => FeedKind::Ltpc,
            Some(FeedUnion::FullFeed(_)) => FeedKind::FullFeed,
            Some(FeedUnion::FirstLevelWithGreeks(_)) => FeedKind::FirstLevelWithGreeks,
            None => FeedKind::Unset,
        };
        feeds.push(FeedSummary {
            key: key.clone(),
            kind,
            request_mode_raw: feed.request_mode,
        });
    }
    keys.sort();
    feeds.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(FeedResponseDiagnostic {
        response_type_raw: response.r#type,
        response_type: response_type_from_raw(response.r#type),
        current_ts: response.current_ts,
        has_market_info: response.market_info.is_some(),
        feed_keys: keys,
        feeds,
    })
}
