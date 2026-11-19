#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.3 deterministic Rust strategy runtime.
//!
//! Executes only frozen, validated artifacts. Strategies emit intents through a
//! narrow interface; they never access cash, OMS, risk, broker state or live
//! routing. Python remains the research environment and supplies golden-vector
//! parity fixtures for the rational arithmetic.

pub mod artifact;
pub mod ema;
pub mod error;
pub mod rational;

pub use artifact::{
    ArtifactStatus, //
    ArtifactValidation,
    FrozenArtifact,
};
pub use ema::{
    RationalEmaCross,
    RuntimeIntent, //
    SUPPORTED_RULE,
    Strategy,
    StrategyEvent,
};
pub use error::{
    RuntimeArtifactError, //
    StrategyError,
};
pub use rational::Rational;
