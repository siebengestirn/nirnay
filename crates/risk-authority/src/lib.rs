#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.4 stateful risk authority with atomic exposure reservation.
//!
//! Sits in the engines layer and depends only on the risk engine and domain
//! types. It owns account-global state and derives feed health from raw
//! observations, so a caller cannot pass a `Healthy` flag or a stale snapshot.
//! It produces reservations (permits) only; it has no OMS, broker, order
//! submission or live-routing capability.

pub mod authority;
pub mod error;
pub mod observation;

pub use authority::{
    AuthoritySnapshot, //
    Reservation,
    RiskAuthority,
};
pub use error::AuthorityError;
pub use observation::{
    FeedObservation, //
    SequenceGap,
};
