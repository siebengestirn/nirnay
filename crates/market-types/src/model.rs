//! Normalized market-data domain types shared by exchanges, the collector core,
//! replay, and inspection.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Side;
/// Stable identifier of one websocket source (a shard replica).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct SourceId {
    pub shard: u16,
    pub replica: u16,
}

impl SourceId {
    #[must_use]
    pub const fn new(shard: u16, replica: u16) -> Self {
        Self { shard, replica }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "s{}r{}", self.shard, self.replica)
    }
}

/// Exact price coefficient.
///
/// The coefficient is interpreted using the scale and unit declared by the
/// owning instrument's effective venue terms. Those live in instrument
/// metadata, not in every book level, so a level stays one machine word. Prices
/// are signed: negative prices are valid for yield, spread, and other
/// non-probability instruments.
///
/// `Price` and [`Quantity`] are not interchangeable:
///
/// ```compile_fail
/// use market_types::model::{Price, Quantity};
/// let _: Quantity = Price::from_raw(1);
/// ```
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Price(i64);

impl Price {
    #[must_use]
    pub const fn from_raw(value: i64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    #[must_use]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    #[must_use]
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }
}

impl From<i64> for Price {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<Price> for i64 {
    fn from(value: Price) -> Self {
        value.0
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Exact non-negative quantity coefficient.
///
/// Positive book quantities are non-negative by construction. Deltas use
/// [`QuantityDelta`] instead, so a signed delta can never be stored as an
/// absolute quantity by accident.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(try_from = "i64", into = "i64")]
pub struct Quantity(i64);

impl Quantity {
    pub const ZERO: Self = Self(0);

    pub fn new(value: i64) -> anyhow::Result<Self> {
        anyhow::ensure!(value >= 0, "negative quantity {value}");
        Ok(Self(value))
    }

    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl TryFrom<i64> for Quantity {
    type Error = anyhow::Error;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Quantity> for i64 {
    fn from(value: Quantity) -> Self {
        value.0
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Signed change to a non-negative quantity.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct QuantityDelta(i64);

impl QuantityDelta {
    #[must_use]
    pub const fn from_raw(value: i64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl From<i64> for QuantityDelta {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<QuantityDelta> for i64 {
    fn from(value: QuantityDelta) -> Self {
        value.0
    }
}

impl fmt::Display for QuantityDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One aggregated price level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level {
    pub price: Price,
    pub quantity: Quantity,
}

impl Level {
    pub fn new(price: i64, quantity: i64) -> anyhow::Result<Self> {
        Ok(Self {
            price: Price::from_raw(price),
            quantity: Quantity::new(quantity)?,
        })
    }
}

/// A complete two-sided book state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookSnapshot {
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

impl BookSnapshot {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bids.is_empty() && self.asks.is_empty()
    }
}

/// Metadata carried by a decoded exchange action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventMeta {
    /// Local time captured immediately after the websocket frame was read.
    pub received_at_us: i64,
    /// Best exchange-provided timestamp, if the feed supplied one.
    pub exchange_ts_us: Option<i64>,
    /// Per-subscription sequence number, if the feed supplies one.
    pub seq: Option<u64>,
    /// Exchange-wide ordering clock, if the feed supplies one.
    pub clock: Option<i64>,
    /// Locally assigned connection generation. It increases on every successful
    /// reconnect and is stamped by the supervisor before the coordinator sees
    /// the event. Together with `receive_seq` it provides a monotonic receive
    /// fence that does not depend on wall-clock timestamps.
    pub connection_epoch: u64,
    /// Locally assigned, monotonically increasing frame number within
    /// `connection_epoch`. Frames are numbered in read order before decoding.
    pub receive_seq: u64,
}

impl EventMeta {
    #[must_use]
    pub const fn new(received_at_us: i64) -> Self {
        Self {
            received_at_us,
            exchange_ts_us: None,
            seq: None,
            clock: None,
            connection_epoch: 0,
            receive_seq: 0,
        }
    }

    /// Whether this event carries a supervisor-assigned receive fence.
    #[must_use]
    pub const fn has_receive_fence(&self) -> bool {
        self.connection_epoch != 0 || self.receive_seq != 0
    }

    #[must_use]
    pub fn exchange_ts_or_received(&self) -> i64 {
        self.exchange_ts_us.unwrap_or(self.received_at_us)
    }
}

/// How a level's quantity changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityChange {
    /// Replace the level with this non-negative absolute quantity.
    Absolute(Quantity),
    /// Add this signed quantity to the current level.
    Delta(QuantityDelta),
}

/// Exchange lifecycle notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleKind {
    Created,
    Activated,
    Deactivated,
    CloseDateUpdated,
    Determined,
    Settled,
    PriceStructureUpdated,
    MetadataUpdated,
    NewMarket,
    Resolved,
    Other(String),
}

impl LifecycleKind {
    /// Whether the market is no longer tradable and should be removed from the
    /// live collection until discovery/lifecycle says otherwise.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Determined | Self::Settled | Self::Resolved)
    }
}

/// Continuity state of a symbol's order book.
///
/// This tracks whether a book is seeded, live, stale, resyncing after a
/// discontinuity, or terminal. It is separate from source connectivity: a
/// connected socket does not imply a live book, and a self-consistent SBE
/// capture does not prove complete event receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuityState {
    /// No accepted exchange snapshot has established the book.
    Unseeded,
    /// An accepted snapshot anchors the active source; no known gap is
    /// currently unresolved.
    Live,
    /// Freshness policy has fired; the book must not be considered eligible
    /// for downstream risk-increasing use.
    Stale,
    /// Continuity uncertain: active-source loss, reset, disconnect, malformed
    /// frame, or stale-probe escalation. Deltas cannot restore state by
    /// themselves. A fresh authoritative book is required.
    Resyncing,
    /// Venue lifecycle marks the instrument resolved/terminal. Do not
    /// silently re-enable from a late delta.
    Terminal,
}

impl ContinuityState {
    /// Whether the book may be used for downstream risk-increasing decisions.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live)
    }

    /// Whether the book is in a state that blocks risk-increasing actions.
    #[must_use]
    pub fn blocks_risk(&self) -> bool {
        matches!(
            self,
            Self::Unseeded | Self::Stale | Self::Resyncing | Self::Terminal
        )
    }
}

/// Reason for a continuity gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscontinuityReason {
    /// Active source lost (disconnect, protocol error)
    ActiveSourceLost,
    /// Sequence/clock gap detected
    SequenceGap,
    /// Malformed frame rejected
    MalformedFrame,
    /// Stale probe escalation (max probes exceeded)
    StaleProbeEscalation,
    /// Active source switched (failover)
    SourceSwitched,
    /// Stream reset
    StreamReset,
    /// Protocol error on the wire
    ProtocolError,
    /// Lifecycle terminal event
    Terminal,
}

impl fmt::Display for DiscontinuityReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ActiveSourceLost => "active_source_lost",
            Self::SequenceGap => "sequence_gap",
            Self::MalformedFrame => "malformed_frame",
            Self::StaleProbeEscalation => "stale_probe_escalation",
            Self::SourceSwitched => "source_switched",
            Self::StreamReset => "stream_reset",
            Self::ProtocolError => "protocol_error",
            Self::Terminal => "terminal",
        })
    }
}

/// Why a source's books must be considered invalid until fresh snapshots
/// arrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetReason {
    Connected,
    SequenceGap,
    ProtocolError,
    SubscriptionRefresh,
    RequestedRefresh,
}

impl fmt::Display for ResetReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected => f.write_str("connected"),
            Self::SequenceGap => f.write_str("sequence-gap"),
            Self::ProtocolError => f.write_str("protocol-error"),
            Self::SubscriptionRefresh => f.write_str("subscription-refresh"),
            Self::RequestedRefresh => f.write_str("requested-refresh"),
        }
    }
}

/// Normalized event emitted by an exchange adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExchangeEvent {
    Snapshot {
        native_symbol: String,
        book: BookSnapshot,
        meta: EventMeta,
    },
    LevelChange {
        native_symbol: String,
        side: Side,
        price: Price,
        change: QuantityChange,
        meta: EventMeta,
    },
    Lifecycle {
        native_symbol: String,
        kind: LifecycleKind,
        close_time_us: Option<i64>,
        title: Option<String>,
        /// Updated finest tick size when the exchange publishes one.
        tick_size_us: Option<i64>,
        meta: EventMeta,
    },
    /// Invalidate some or all source-local books and wait for snapshots.
    StreamReset {
        native_symbols: Vec<String>,
        reason: ResetReason,
        /// Local time at which the reset was observed.
        received_at_us: i64,
    },
}

/// Command sent from the collector core/coordinator to a source worker.
#[derive(Debug, Clone)]
pub enum Control {
    /// Replace this source's desired native symbol set.
    SetSubscriptions(BTreeSet<String>),
    /// Ask the source to emit fresh snapshots for these symbols without a
    /// recovery trigger. Used for non-incident probes.
    RequestSnapshot(Vec<String>),
    /// Explicit continuity recovery. The core creates the request before this
    /// control is sent and only accepts a snapshot whose supervisor-assigned
    /// receive fence is strictly newer than the acknowledgement sent by the
    /// worker for `generation`.
    Recover {
        generation: u64,
        symbols: Vec<String>,
        /// Drop the connection first. The replacement snapshot must then come
        /// from a later connection epoch.
        reconnect: bool,
    },
    /// Drop the connection and reconnect with exponential backoff.
    Reconnect(&'static str),
    /// Close the websocket and stop the worker.
    Shutdown,
}

/// Per-source connectivity and protocol counters surfaced by a worker.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceStatus {
    pub source: SourceId,
    pub connected: bool,
    pub reconnects: u64,
    pub messages: u64,
    pub decode_errors: u64,
    pub protocol_errors: u64,
    pub bytes_received: u64,
    pub last_frame_us: Option<i64>,
    pub last_keepalive_us: Option<i64>,
    pub latency_ewma_us: Option<i64>,
    pub subscriptions: usize,
    pub last_error: Option<String>,
}

impl SourceStatus {
    #[must_use]
    pub fn new(source: SourceId) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod numeric_tests {
    use super::*;

    #[test]
    fn price_and_quantity_have_distinct_units_and_ranges() {
        assert_eq!(Price::from_raw(-2_500_000).raw(), -2_500_000);
        assert!(Price::from_raw(-1).is_negative());
        assert_eq!(Quantity::new(0).unwrap().raw(), 0);
        assert!(Quantity::new(-1).is_err());
        assert!(Quantity::try_from(-1).is_err());
        assert_eq!(Quantity::try_from(3).unwrap().raw(), 3);
        // Deltas are signed and separate from absolute quantities.
        assert_eq!(QuantityDelta::from_raw(-4).raw(), -4);
        assert!(QuantityDelta::from_raw(-4).raw() < 0);
    }

    #[test]
    fn negative_and_large_prices_round_trip_through_serde() {
        for price in [
            Price::from_raw(i64::MIN),
            Price::from_raw(-2_500_000),
            Price::from_raw(-1),
            Price::from_raw(0),
            Price::from_raw(i64::MAX),
        ] {
            let encoded = serde_json::to_string(&price).unwrap();
            let decoded: Price = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, price);
        }
        let level = Level::new(-2_500_000, 7).unwrap();
        let encoded = serde_json::to_string(&level).unwrap();
        assert_eq!(encoded, r#"{"price":-2500000,"quantity":7}"#);
        assert_eq!(serde_json::from_str::<Level>(&encoded).unwrap(), level);
        assert!(serde_json::from_str::<Level>(r#"{"price":1,"quantity":-1}"#).is_err());
    }
}
