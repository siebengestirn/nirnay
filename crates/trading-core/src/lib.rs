#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Trading domain and event contracts for the India platform.
//!
//! Derived from the internal Nirnay (`crates/nirnay-core/src/types.rs`,
//! `model.rs`, `risk.rs`) at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! The port does **not** keep Nirnay's instrument identity,
//! money, quantity or side types. Those are mapped onto the canonical
//! `india-domain` instrument model and `market-types` side semantics:
//!
//! - [`InstrumentId`] is `india_domain::InstrumentId`: one canonical identity.
//! - [`Paise`] is `india_domain::Paise`: exact integer cash.
//! - [`Lots`] is `india_domain::Lots`: quantity is measured in lots.
//! - [`Side`] is `market_types::Side`: Buy = 0, Sell = 1.
//! - [`PricePaise`] is a validated positive price with explicit, lossless
//!   conversions to and from canonical paise.
//!
//! Event identity is deterministic for a run ([`EventId::for_run`]) so the
//! same simulated input produces the same journal bytes and hashes. A live
//! caller supplies a distinct [`RunId`]; a replay supplies the recorded one.

mod error;
mod ids;
mod instrument;
mod model;
mod money;
mod time;

pub use error::DomainError;
pub use ids::{AccountId, AlgoId, EventId, FillId, IntentId, OrderId, RunId, StrategyId};
pub use instrument::{
    CalendarDay, Instrument, InstrumentCatalog, InstrumentId, InstrumentType, Isin, Listing,
    ListingStatus, ListingTerms, OptionKind, Segment, SessionDay, SettlementRegime, TickAssignment,
    TradingCalendar, Venue, is_active_on,
};
pub use model::{
    ChargeComponents, Continuity, DataQualityFlag, DecisionRecord, EventEnvelope, EventPayload,
    EventTimes, FeedHealth, Fill, JournalRef, LedgerChargeStatus, LedgerPostingKind,
    OpeningBalanceEvidence, Order, OrderIntent, OrderStatus, Product, QuoteSnapshot,
    ReconciliationReport, ReconciliationStatus, RiskAssessment, RiskCheck, RiskCheckResult,
    SessionPhase, StoredBody,
};
pub use money::{CheckedPaise, DoubledPaise, Lots, Paise, PricePaise, Quantity};
pub use time::{LogicalNanos, TradeDate, UtcNanos};

pub use market_types::Side;
