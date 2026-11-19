#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.2 market-data kernel projection.
//!
//! Consumes verified source observations and produces canonical kernel events
//! through the M2.1 identity contract. The kernel applies per-stream continuity
//! and is deterministic: the same verified session yields the same events in a
//! fresh process. No provider decoding, broker, order or trading behavior lives
//! here; the raw journal remains the evidence authority upstream.

pub mod error;
pub mod input;
pub mod projection;

pub use error::KernelError;
pub use input::SourceObservation;
pub use projection::{
    KERNEL_SESSION_SCHEMA_VERSION, //
    KernelSession,
    project_observation,
    project_session,
};
