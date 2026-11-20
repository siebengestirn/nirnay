#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Order-book representation and mutation invariants.
//! Depends only on `market-types`; no runtime dependencies.

pub mod mbp;
pub mod orderbook;
pub use mbp::{
    BookPhase, //
    MbpBook,
    MbpBookError,
    SnapshotEvidence,
};
pub use orderbook::{BookError, OrderBook};
