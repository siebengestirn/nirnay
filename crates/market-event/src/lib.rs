#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.1 kernel event identity and continuity contract.
//!
//! This crate defines one versioned Rust contract. Python research deserializes
//! it; it does not define a second authoritative event model. No order, broker,
//! risk or trading behavior lives here.

pub mod continuity;
pub mod error;
pub mod event;
pub mod identity;

pub use continuity::{
    ContinuityMachine, //
    ObservationOutcome,
    StreamState,
};
pub use error::ContractError;
pub use event::{
    KERNEL_EVENT_SCHEMA_VERSION, //
    KernelEvent,
    RecoveryEvidence,
};
pub use identity::{
    ContentId, //
    OriginClass,
    SequenceScope,
    business_event_id,
    capture_frame_id,
    observation_id,
};
