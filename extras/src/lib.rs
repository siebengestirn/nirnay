//! Nirnay extras: predecessor India reference-data ingestion and durable
//! point-in-time store.
//!
//! This crate is a **separate reference workspace**. It is not a member of the
//! active Nirnay workspace and no active code depends on it. The active
//! canonical instrument domain lives in `crates/india-domain`; the extras here
//! own the exchange-master parsers, reconciliation, durable snapshot store and
//! backup/restore verification that have not been promoted.
//!
//! Run the retained tests with:
//!
//! ```text
//! cargo test --manifest-path extras/Cargo.toml
//! ```

pub mod util;
pub mod venue;
