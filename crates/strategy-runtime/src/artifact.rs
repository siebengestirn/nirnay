//! Frozen strategy artifact loading and runtime eligibility.
//!
//! Only a validated or paper-approved artifact may execute. A research
//! candidate, a rejected or retired artifact, or one missing its validation
//! lineage is refused before any strategy is constructed.

use std::path::Path;

use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};

use crate::error::RuntimeArtifactError;

/// Runtime-acceptable artifact statuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Validated,
    ApprovedForPaper,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactValidation {
    pub walk_forward_windows: u32,
    pub holdout_locked: bool,
    pub multiple_testing_correction: String,
    /// Decimal string of the reported statistic; the runtime stores and
    /// compares contract text, it does not recompute statistics.
    pub deflated_sharpe: String,
    pub cost_model: String,
    pub probability_of_backtest_overfit: Option<String>,
}

/// Frozen artifact v1 as consumed by the runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenArtifact {
    pub candidate_id: String,
    pub hypothesis: String,
    pub return_source: String,
    pub universe: Vec<String>,
    pub entry_rule: String,
    pub exit_rule: String,
    pub rebalance_frequency: Option<String>,
    pub experiment_ids: Vec<String>,
    pub validation: ArtifactValidation,
    pub status: ArtifactStatus,
    /// Digest of the exact artifact bytes, computed at load time.
    #[serde(skip)]
    pub artifact_sha256: String,
}

impl FrozenArtifact {
    /// Load and gate an artifact. No code is executed; the rule string must
    /// match the one strategy family this runtime implements.
    pub fn load(path: &Path, supported_rule: &str) -> Result<Self, RuntimeArtifactError> {
        let bytes = std::fs::read(path).map_err(|_| RuntimeArtifactError::UnreadableArtifact)?;
        let artifact_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let mut artifact: Self =
            serde_json::from_slice(&bytes).map_err(|_| RuntimeArtifactError::InvalidArtifact)?;
        artifact.artifact_sha256 = artifact_sha256;
        artifact.validate_runtime(supported_rule)?;
        Ok(artifact)
    }

    /// Runtime gate: a frozen, validated contract with non-empty lineage.
    pub fn validate_runtime(&self, supported_rule: &str) -> Result<(), RuntimeArtifactError> {
        if self.candidate_id.is_empty() || self.hypothesis.is_empty() {
            return Err(RuntimeArtifactError::IncompleteArtifact);
        }
        if self.experiment_ids.is_empty() {
            return Err(RuntimeArtifactError::MissingExperimentLineage);
        }
        if self.validation.walk_forward_windows == 0 || !self.validation.holdout_locked {
            return Err(RuntimeArtifactError::IncompleteValidation);
        }
        if self.validation.multiple_testing_correction.is_empty()
            || self.validation.cost_model.is_empty()
        {
            return Err(RuntimeArtifactError::IncompleteValidation);
        }
        if self.validation.deflated_sharpe.trim().is_empty() {
            return Err(RuntimeArtifactError::IncompleteValidation);
        }
        if self.entry_rule != supported_rule {
            return Err(RuntimeArtifactError::UnsupportedRule(
                self.entry_rule.clone(),
            ));
        }
        Ok(())
    }
}
