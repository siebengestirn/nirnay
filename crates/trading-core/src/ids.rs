//! Identifiers. Nirnay's random UUID event identity is replaced with a
//! deterministic run + ordinal identity.
//!
//! Derived from the internal Nirnay `crates/nirnay-core/src/types.rs` at snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.

use std::{fmt, num::NonZeroU64};

use serde::{Deserialize, Serialize};

use crate::DomainError;

macro_rules! positive_id {
    ($name:ident) => {
        #[derive(
            Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(NonZeroU64);

        impl $name {
            pub fn new(value: u64) -> Result<Self, DomainError> {
                NonZeroU64::new(value)
                    .map(Self)
                    .ok_or(DomainError::ZeroIdentifier)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }

        impl TryFrom<u64> for $name {
            type Error = DomainError;

            fn try_from(value: u64) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.get())
            }
        }
    };
}

positive_id!(AccountId);
positive_id!(AlgoId);
positive_id!(FillId);
positive_id!(IntentId);
positive_id!(OrderId);
positive_id!(StrategyId);

/// Identity of one simulated or live run. A replay must supply the recorded
/// run identity so event IDs reproduce exactly.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RunId([u8; 16]);

impl RunId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self, DomainError> {
        let value = value.as_ref();
        if value.len() != 32 || !value.bytes().all(is_lower_hex) {
            return Err(DomainError::InvalidIdentifier(format!(
                "run id `{value}` must be 32 lowercase hexadecimal characters"
            )));
        }
        let mut bytes = [0_u8; 16];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let pair = value
                .get(index * 2..index * 2 + 2)
                .ok_or_else(|| DomainError::InvalidIdentifier("run id is truncated".to_owned()))?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| {
                DomainError::InvalidIdentifier("run id is not hexadecimal".to_owned())
            })?;
        }
        Ok(Self(bytes))
    }
}

impl TryFrom<String> for RunId {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for RunId {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RunId> for String {
    fn from(value: RunId) -> Self {
        value.to_hex()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// Deterministic event identity: run identity plus a monotonic ordinal.
///
/// Serialized as 48 lowercase hexadecimal characters. Parsing validates the
/// shape, so a journal or broker payload cannot inject an arbitrary opaque
/// event key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EventId(String);

impl EventId {
    pub const HEX_LEN: usize = 48;

    #[must_use]
    pub fn for_run(run: RunId, ordinal: u64) -> Self {
        let mut value = run.to_hex();
        value.push_str(&format!("{ordinal:016x}"));
        Self(value)
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self, DomainError> {
        let value = value.as_ref();
        if value.len() != Self::HEX_LEN || !value.bytes().all(is_lower_hex) {
            return Err(DomainError::InvalidIdentifier(format!(
                "event id `{value}` must be 48 lowercase hexadecimal characters"
            )));
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EventId {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for EventId {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<EventId> for String {
    fn from(value: EventId) -> Self {
        value.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}
