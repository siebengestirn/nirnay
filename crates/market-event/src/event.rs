//! The versioned kernel event envelope.

use serde::{
    Deserialize, //
    Serialize,
};

use crate::{
    error::ContractError,
    identity::{
        ContentId, //
        OriginClass,
        SequenceScope,
    },
};

/// Contract schema version. Bump only with a reviewed decision.
pub const KERNEL_EVENT_SCHEMA_VERSION: u32 = 1;

/// Evidence that a recovery boundary was crossed with an authoritative event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryEvidence {
    /// Identity of the authoritative snapshot or reset that cleared the gap.
    pub authority_id: ContentId,
    /// The connection epoch at which the authority was accepted.
    pub connection_epoch: u64,
}

/// One kernel market event. Every field required to reconstruct origin and
/// interpretation is explicit; payload content stays in the research domain
/// model, not in this envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KernelEvent {
    pub schema_version: u32,
    /// Identity of the original captured frame (all instruments in it).
    pub capture_frame_id: ContentId,
    /// Stable identity of one source delivery.
    pub observation_id: ContentId,
    /// Source-backed business identity, present only when the source supplies
    /// a distinct business event. `None` for a bare state observation.
    pub business_event_id: Option<ContentId>,
    /// Back-reference for a derived event.
    pub source_observation_id: Option<ContentId>,
    pub origin_class: OriginClass,
    pub source_id: String,
    pub source_session_id: String,
    pub connection_epoch: u64,
    pub receive_sequence: u64,
    pub sequence: Option<u64>,
    pub sequence_scope: SequenceScope,
    pub provider_instrument_id: String,
    /// Nirnay opaque instrument identity. `None` until resolved.
    pub instrument_id: Option<String>,
    pub venue_id: Option<String>,
    /// Source-provided timestamp in its own clock domain.
    pub source_time_ns: Option<i64>,
    pub source_clock_domain: Option<String>,
    /// Local UTC receive time, ns since epoch.
    pub receive_utc_ns: i64,
    /// Local monotonic receive time.
    pub receive_monotonic_ns: u64,
    pub raw_frame_hash: ContentId,
    /// Hash of the exact reference revision used for resolution.
    pub reference_revision: Option<String>,
    pub quality: super::continuity::StreamState,
    pub evidence_token: Option<String>,
}

impl KernelEvent {
    /// Build an observation-only event. No business identity is invented.
    #[allow(clippy::too_many_arguments)] // full identity and time surface
    pub fn observation_only(
        capture_frame_id: ContentId,
        observation_id: ContentId,
        raw_frame_hash: ContentId,
        origin_class: OriginClass,
        source_id: impl Into<String>,
        source_session_id: impl Into<String>,
        provider_instrument_id: impl Into<String>,
        receive_utc_ns: i64,
        receive_monotonic_ns: u64,
    ) -> Result<Self, ContractError> {
        if origin_class.can_be_exchange_event() {
            return Err(ContractError::ExchangeEventWithoutEvidence);
        }
        if receive_utc_ns <= 0 {
            return Err(ContractError::TimeDomainCollision);
        }
        Ok(Self {
            schema_version: KERNEL_EVENT_SCHEMA_VERSION,
            capture_frame_id,
            observation_id,
            business_event_id: None,
            source_observation_id: None,
            origin_class,
            source_id: source_id.into(),
            source_session_id: source_session_id.into(),
            connection_epoch: 0,
            receive_sequence: 0,
            sequence: None,
            sequence_scope: SequenceScope::None,
            provider_instrument_id: provider_instrument_id.into(),
            instrument_id: None,
            venue_id: None,
            source_time_ns: None,
            source_clock_domain: None,
            receive_utc_ns,
            receive_monotonic_ns,
            raw_frame_hash,
            reference_revision: None,
            quality: super::continuity::StreamState::Clean,
            evidence_token: None,
        })
    }

    /// Attach a receive fence. Epoch zero means no fence; sequence must
    /// advance with the epoch so no half-fence is accepted.
    pub fn with_receive_fence(
        mut self,
        connection_epoch: u64,
        receive_sequence: u64,
    ) -> Result<Self, ContractError> {
        if (connection_epoch == 0) != (receive_sequence == 0) {
            return Err(ContractError::InvalidReceiveFence);
        }
        self.connection_epoch = connection_epoch;
        self.receive_sequence = receive_sequence;
        Ok(self)
    }

    /// Attach a source sequence with its declared scope.
    pub fn with_sequence(mut self, sequence: u64, scope: SequenceScope) -> Self {
        self.sequence = Some(sequence);
        self.sequence_scope = scope;
        self
    }

    /// Attach source time with an explicit clock domain. The clock domain must
    /// not claim receive semantics.
    pub fn with_source_time(
        mut self,
        source_time_ns: i64,
        source_clock_domain: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let domain = source_clock_domain.into();
        if domain == "receive" || domain == "receive_monotonic" {
            return Err(ContractError::TimeDomainCollision);
        }
        self.source_time_ns = Some(source_time_ns);
        self.source_clock_domain = Some(domain);
        Ok(self)
    }

    /// Resolve the instrument against an exact reference revision.
    pub fn resolve_instrument(
        mut self,
        instrument_id: impl Into<String>,
        venue_id: Option<String>,
        reference_revision: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let revision = reference_revision.into();
        if revision.is_empty() {
            return Err(ContractError::MissingReferenceRevision);
        }
        self.instrument_id = Some(instrument_id.into());
        self.venue_id = venue_id;
        self.reference_revision = Some(revision);
        Ok(self)
    }

    /// Derive a business event from this observation. The discriminator is
    /// caller supplied and must name a domain event kind.
    pub fn derived_business_event(
        &self,
        event_kind: &str,
        discriminator: &str,
    ) -> Result<Self, ContractError> {
        if discriminator.is_empty() {
            return Err(ContractError::MissingDomainDiscriminator);
        }
        let business =
            crate::identity::business_event_id(&self.observation_id, event_kind, discriminator)?;
        let mut derived = self.clone();
        derived.business_event_id = Some(business);
        derived.source_observation_id = Some(self.observation_id.clone());
        derived.origin_class = OriginClass::Reconstructed;
        Ok(derived)
    }

    /// Validate structural invariants. Public fields permit construction; this
    /// is the authoritative check and is invoked on deserialization.
    pub fn validate_invariants(&self) -> Result<(), ContractError> {
        if self.schema_version != KERNEL_EVENT_SCHEMA_VERSION {
            return Err(ContractError::UnsupportedSchemaVersion(self.schema_version));
        }
        if self.business_event_id.is_some() && self.source_observation_id.is_none() {
            return Err(ContractError::MissingSourceObservation);
        }
        if self.origin_class.can_be_exchange_event() {
            let token = self
                .evidence_token
                .as_deref()
                .ok_or(ContractError::ExchangeEventWithoutEvidence)?;
            ContentId::parse(token).map_err(|_| ContractError::ExchangeEventWithoutEvidence)?;
        }
        if (self.connection_epoch == 0) != (self.receive_sequence == 0) {
            return Err(ContractError::InvalidReceiveFence);
        }
        if self.quality == super::continuity::StreamState::Recovered {
            let token = self
                .evidence_token
                .as_deref()
                .ok_or(ContractError::RecoveryWithoutEvidence)?;
            ContentId::parse(token).map_err(|_| ContractError::RecoveryWithoutEvidence)?;
        }
        Ok(())
    }

    /// Whether this event may be used for a risk-increasing decision.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        self.validate_invariants().is_ok()
            && self.instrument_id.is_some()
            && self.reference_revision.is_some()
            && self.quality.is_clean()
    }
}

/// Deserialization validates invariants so a forged wire object cannot pass as
/// a kernel event.
impl<'de> Deserialize<'de> for KernelEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            schema_version: u32,
            capture_frame_id: ContentId,
            observation_id: ContentId,
            business_event_id: Option<ContentId>,
            source_observation_id: Option<ContentId>,
            origin_class: OriginClass,
            source_id: String,
            source_session_id: String,
            connection_epoch: u64,
            receive_sequence: u64,
            sequence: Option<u64>,
            sequence_scope: SequenceScope,
            provider_instrument_id: String,
            instrument_id: Option<String>,
            venue_id: Option<String>,
            source_time_ns: Option<i64>,
            source_clock_domain: Option<String>,
            receive_utc_ns: i64,
            receive_monotonic_ns: u64,
            raw_frame_hash: ContentId,
            reference_revision: Option<String>,
            quality: super::continuity::StreamState,
            evidence_token: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let event = Self {
            schema_version: wire.schema_version,
            capture_frame_id: wire.capture_frame_id,
            observation_id: wire.observation_id,
            business_event_id: wire.business_event_id,
            source_observation_id: wire.source_observation_id,
            origin_class: wire.origin_class,
            source_id: wire.source_id,
            source_session_id: wire.source_session_id,
            connection_epoch: wire.connection_epoch,
            receive_sequence: wire.receive_sequence,
            sequence: wire.sequence,
            sequence_scope: wire.sequence_scope,
            provider_instrument_id: wire.provider_instrument_id,
            instrument_id: wire.instrument_id,
            venue_id: wire.venue_id,
            source_time_ns: wire.source_time_ns,
            source_clock_domain: wire.source_clock_domain,
            receive_utc_ns: wire.receive_utc_ns,
            receive_monotonic_ns: wire.receive_monotonic_ns,
            raw_frame_hash: wire.raw_frame_hash,
            reference_revision: wire.reference_revision,
            quality: wire.quality,
            evidence_token: wire.evidence_token,
        };
        event
            .validate_invariants()
            .map_err(serde::de::Error::custom)?;
        Ok(event)
    }
}
