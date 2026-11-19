//! Input contract: a verified source observation supplied to the kernel.
//!
//! Raw bytes remain the evidence authority upstream. The kernel receives the
//! exact verified frame bytes and capture metadata, plus an instrument-local
//! canonical payload for identity derivation. A multi-instrument frame produces
//! several observations sharing one raw-frame commitment.

use serde::{
    Deserialize, //
    Serialize,
};

use market_event::{
    ContentId, //
    OriginClass,
    SequenceScope,
};

/// One verified, instrument-local source observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceObservation {
    /// Identity of the original raw frame (shared across instruments).
    pub raw_frame_hash: ContentId,
    /// Provider or venue source identity.
    pub source_id: String,
    /// Capture session identity from the verified session manifest.
    pub source_session_id: String,
    /// Provider-specific instrument identity from the frame.
    pub provider_instrument_id: String,
    /// Canonical bytes of this instrument's portion of the payload.
    pub canonical_payload: Vec<u8>,
    /// Origin of the observation. Exchange origin requires evidence.
    pub origin_class: OriginClass,
    /// Local UTC receive time, ns since epoch.
    pub receive_utc_ns: i64,
    /// Local monotonic receive time.
    pub receive_monotonic_ns: u64,
    /// Source-provided timestamp and clock domain, if any.
    pub source_time_ns: Option<i64>,
    pub source_clock_domain: Option<String>,
    /// Connection generation and receive fence.
    pub connection_epoch: u64,
    pub receive_sequence: u64,
    /// Source sequence and its declared scope.
    pub sequence: Option<u64>,
    pub sequence_scope: SequenceScope,
    /// Exact reference revision used for resolution, if resolved.
    pub reference_revision: Option<String>,
    /// Opaque Nirnay instrument identity, or `None` when unresolved.
    pub instrument_id: Option<String>,
    /// Optional evidence token (e.g. exchange-origin evidence).
    pub evidence_token: Option<String>,
}

impl SourceObservation {
    /// Validate the input before any identity is derived.
    pub fn validate(&self) -> Result<(), crate::KernelError> {
        if self.source_id.is_empty() {
            return Err(crate::KernelError::EmptySource);
        }
        if self.provider_instrument_id.is_empty() {
            return Err(crate::KernelError::EmptyProviderInstrument);
        }
        if self.canonical_payload.is_empty() {
            return Err(crate::KernelError::EmptyFrame);
        }
        if self.receive_utc_ns <= 0 {
            return Err(crate::KernelError::InvalidReceiveTime);
        }
        // A resolved instrument needs an exact revision. The revision may be
        // supplied on the observation or inherited from the session, so the
        // resolution check happens in `project_observation` where both are
        // visible.
        Ok(())
    }
}
