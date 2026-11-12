#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Order-book representation and mutation invariants.
//! Depends only on `market-types`; no runtime dependencies.

pub mod orderbook;
pub use orderbook::{BookError, OrderBook};
