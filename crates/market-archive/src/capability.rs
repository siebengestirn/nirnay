//! Machine-readable source-product capability contract.
//!
//! A product declares what it actually supplies. Missing capabilities are
//! rejections, never inferences. Rights and entitlements are recorded as
//! verification state only; this module never grants or upgrades them.

use std::path::Path;

use serde::{
    Deserialize, //
    Serialize,
};
use thiserror::Error;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityError {
    #[error("registry has schema version {0}, expected 1")]
    UnsupportedSchema(u32),
    #[error("registry could not be read")]
    Unreadable,
    #[error("registry is invalid: {0}")]
    InvalidRegistry(String),
    #[error("no declared product for provider `{0}`, product `{1}`, venue `{2}`, segment `{3}`")]
    UnknownProduct(String, String, String, String),
    #[error("product supplies {declared} levels; requirement needs {required}")]
    InsufficientDepth { declared: u32, required: u32 },
    #[error("product depth model is {declared:?}; requirement needs {required:?}")]
    DepthModelMismatch {
        declared: DepthModel,
        required: DepthModel,
    },
    #[error("product does not supply trade prints")]
    NoTradePrints,
    #[error("product does not supply a clean snapshot")]
    NoSnapshot,
    #[error("product declares no source sequence continuity")]
    NoSourceContinuity,
    #[error("product does not grant the verified right to {0}")]
    MissingRight(String),
    #[error("product entitlement `{0}` has not been verified")]
    EntitlementUnverified(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepthModel {
    /// Aggregated price levels with a displayed order count.
    Mbp,
    /// Individual orders with identities and lifecycle events.
    Mbo,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    SnapshotOnly,
    Delta,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSequence {
    Unavailable,
    Connection,
    ProviderInstrument,
    Channel,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    Reconnect,
    FreshSnapshot,
    Retransmission,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolRef {
    pub name: String,
    pub schema_digest: Option<String>,
    pub schema_reference: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitlementRef {
    pub reference: String,
    pub verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRights {
    pub capture: bool,
    pub store: bool,
    pub replay: bool,
    pub redistribute: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLimits {
    pub max_symbols: u32,
    pub connections: u32,
    pub requests_per_second: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProduct {
    pub provider: String,
    pub product: String,
    pub venue: String,
    pub segment: String,
    pub instrument_scope: String,
    pub depth_model: DepthModel,
    pub depth_levels: u32,
    pub trade_prints: bool,
    pub order_level_identity: bool,
    pub snapshot: bool,
    pub incremental_updates: UpdateMode,
    pub source_sequence: SourceSequence,
    pub recovery: Vec<RecoveryMode>,
    pub protocol: ProtocolRef,
    pub entitlement: EntitlementRef,
    pub rights: DataRights,
    pub limits: SourceLimits,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProductRegistry {
    pub schema_version: u32,
    pub registry_id: String,
    pub products: Vec<SourceProduct>,
}

/// What a consumer requires before it may activate on a feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketDataRequirement {
    pub provider: String,
    pub product: String,
    pub venue: String,
    pub segment: String,
    pub depth_model: DepthModel,
    pub min_depth: u32,
    pub requires_trade_prints: bool,
    pub requires_clean_snapshot: bool,
    pub requires_source_continuity: bool,
    pub requires_store: bool,
    pub requires_replay: bool,
}

impl SourceProductRegistry {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CapabilityError> {
        let bytes = std::fs::read(path).map_err(|_| CapabilityError::Unreadable)?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CapabilityError> {
        let registry: Self = serde_json::from_slice(bytes)
            .map_err(|error| CapabilityError::InvalidRegistry(error.to_string()))?;
        if registry.schema_version != SCHEMA_VERSION {
            return Err(CapabilityError::UnsupportedSchema(registry.schema_version));
        }
        if registry.registry_id.trim().is_empty() || registry.products.is_empty() {
            return Err(CapabilityError::InvalidRegistry(
                "registry id and at least one product are required".to_owned(),
            ));
        }
        Ok(registry)
    }

    /// Find the exact declared product. No venue or segment inference.
    pub fn product(
        &self,
        provider: &str,
        product: &str,
        venue: &str,
        segment: &str,
    ) -> Result<&SourceProduct, CapabilityError> {
        self.products
            .iter()
            .find(|candidate| {
                candidate.provider == provider
                    && candidate.product == product
                    && candidate.venue == venue
                    && candidate.segment == segment
            })
            .ok_or_else(|| {
                CapabilityError::UnknownProduct(
                    provider.to_owned(),
                    product.to_owned(),
                    venue.to_owned(),
                    segment.to_owned(),
                )
            })
    }

    /// Reject the requirement unless every field is satisfied by the declared
    /// product. A five-level feed never satisfies a twenty-level requirement.
    pub fn evaluate(&self, requirement: &MarketDataRequirement) -> Result<(), CapabilityError> {
        let declared = self.product(
            &requirement.provider,
            &requirement.product,
            &requirement.venue,
            &requirement.segment,
        )?;
        if declared.depth_model != requirement.depth_model {
            return Err(CapabilityError::DepthModelMismatch {
                declared: declared.depth_model,
                required: requirement.depth_model,
            });
        }
        if declared.depth_levels < requirement.min_depth {
            return Err(CapabilityError::InsufficientDepth {
                declared: declared.depth_levels,
                required: requirement.min_depth,
            });
        }
        if requirement.requires_trade_prints && !declared.trade_prints {
            return Err(CapabilityError::NoTradePrints);
        }
        if requirement.requires_clean_snapshot && !declared.snapshot {
            return Err(CapabilityError::NoSnapshot);
        }
        if requirement.requires_source_continuity
            && declared.source_sequence == SourceSequence::Unavailable
        {
            return Err(CapabilityError::NoSourceContinuity);
        }
        if requirement.requires_store && !declared.rights.store {
            return Err(CapabilityError::MissingRight("store".to_owned()));
        }
        if requirement.requires_replay && !declared.rights.replay {
            return Err(CapabilityError::MissingRight("replay".to_owned()));
        }
        if (requirement.requires_store || requirement.requires_replay)
            && !declared.entitlement.verified
        {
            return Err(CapabilityError::EntitlementUnverified(
                declared.entitlement.reference.clone(),
            ));
        }
        Ok(())
    }
}
