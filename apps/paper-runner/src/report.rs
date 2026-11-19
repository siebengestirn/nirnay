//! Deterministic paper-run report.

use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};

pub const PAPER_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub intent_id: String,
    pub observation_id: String,
    pub risk_accepted: bool,
    pub permit_id: Option<u64>,
    pub oms_client_id: u64,
    pub oms_final_state: String,
    pub broker_order_id: Option<String>,
    pub quantity: i64,
    pub rejection_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperReport {
    pub schema_version: u32,
    pub events_seen: usize,
    pub intents_emitted: usize,
    pub reservations: usize,
    pub health_safe_for_new_risk: bool,
    pub health_reasons: Vec<String>,
    pub decisions: Vec<DecisionRecord>,
    pub input_digest: String,
    pub run_digest: String,
}

impl PaperReport {
    /// Build a report and bind every field into a deterministic digest.
    pub fn new(
        events_seen: usize,
        intents_emitted: usize,
        reservations: usize,
        health_safe_for_new_risk: bool,
        health_reasons: Vec<String>,
        decisions: Vec<DecisionRecord>,
        input_digest: String,
    ) -> Self {
        let preimage = serde_json::to_vec(&(
            PAPER_REPORT_SCHEMA_VERSION,
            events_seen,
            intents_emitted,
            reservations,
            health_safe_for_new_risk,
            &health_reasons,
            &decisions,
            &input_digest,
        ))
        .unwrap_or_default();
        let run_digest = Sha256::digest(&preimage)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            schema_version: PAPER_REPORT_SCHEMA_VERSION,
            events_seen,
            intents_emitted,
            reservations,
            health_safe_for_new_risk,
            health_reasons,
            decisions,
            input_digest,
            run_digest,
        }
    }
}
