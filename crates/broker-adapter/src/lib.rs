#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.6 broker-neutral adapter contract and deterministic simulator.
//!
//! The adapter binds the complete typed order request from the authoritative
//! OMS. Strategies and the risk engine never see this interface. No broker
//! SDK, network endpoint, live routing or order submission to a real venue
//! exists here.

pub mod contract;
pub mod error;
pub mod simulator;

pub use contract::{
    BrokerAdapter, //
    BrokerEvent,
    BrokerOrderRequest,
    BrokerOrderStatus,
    CancelRequest,
    SubmissionAck,
    SubmissionOutcome,
};
pub use error::AdapterError;
pub use simulator::{
    CancelBehavior, //
    SimulatedBroker,
    SimulatorConfig,
    SubmitBehavior,
};
