#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.5 authoritative OMS.
//!
//! One append-only durable commit journal owns the client order identity, the complete
//! typed order request, submission-attempt/outbox state, broker identities,
//! fills and the full order lifecycle. A timeout after dispatch yields
//! `Unknown`; it is never auto-retried and can only leave through explicit
//! reconciliation with broker evidence. No strategy, risk, broker SDK, live
//! routing or automatic deployment dependency exists here.

pub mod error;
pub mod lifecycle;
pub mod store;

pub use error::OmsError;
pub use lifecycle::{
    OmsOrderState, //
    OutboxState,
};
pub use store::{
    BrokerOrderId, //
    ClientOrderId,
    DispatchClaim,
    OmsStore,
    OrderView,
    OutboxView,
    PermitRef,
    Resolution,
};
