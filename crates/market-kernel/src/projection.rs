//! Deterministic projection from verified observations to kernel events.

use std::collections::{
    BTreeMap, //
    BTreeSet,
};

use market_event::{
    ContinuityMachine, //
    KernelEvent,
    SequenceScope,
    StreamState,
    capture_frame_id,
    observation_id,
};

use crate::{
    error::KernelError, //
    input::SourceObservation,
};

/// Session input schema version.
pub const KERNEL_SESSION_SCHEMA_VERSION: u32 = 1;

/// A verified session of instrument-local observations.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KernelSession {
    pub schema_version: u32,
    /// Source scope used in observation identity (e.g. venue or feed scope).
    pub source_scope: String,
    /// Exact reference revision asserted for this session, if any.
    pub session_reference_revision: Option<String>,
    pub observations: Vec<SourceObservation>,
}

impl KernelSession {
    pub fn validate(&self) -> Result<(), KernelError> {
        if self.schema_version != KERNEL_SESSION_SCHEMA_VERSION {
            return Err(KernelError::UnsupportedSessionSchema(self.schema_version));
        }
        if self.observations.is_empty() {
            return Err(KernelError::EmptySession);
        }
        for observation in &self.observations {
            observation.validate()?;
        }
        Ok(())
    }
}

/// The scope in which a source sequence value orders observations.
///
/// A local receive counter is never a source sequence and is not a key here.
/// When a provider supplies no sequence scope, no continuity claim is made at
/// all and the event is marked `Unverified`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContinuityKey {
    pub source_id: String,
    pub source_session_id: String,
    pub scope: SequenceScope,
    pub channel_id: Option<String>,
    pub provider_instrument_id: Option<String>,
}

impl ContinuityKey {
    /// Build the key declared by the observation's sequence scope.
    /// Instrument identity is included only for provider-per-instrument
    /// sequences; a channel sequence is shared by all instruments in a frame.
    pub fn for_observation(observation: &SourceObservation) -> Option<Self> {
        if observation.sequence.is_none() || observation.sequence_scope == SequenceScope::None {
            return None;
        }
        let instrument = if observation.sequence_scope == SequenceScope::ProviderInstrument {
            Some(observation.provider_instrument_id.clone())
        } else {
            None
        };
        Some(Self {
            source_id: observation.source_id.clone(),
            source_session_id: observation.source_session_id.clone(),
            scope: observation.sequence_scope,
            channel_id: observation.channel_id.clone(),
            provider_instrument_id: instrument,
        })
    }
}

/// Project one verified observation into a kernel event.
pub fn project_observation(
    session: &KernelSession,
    observation: &SourceObservation,
) -> Result<KernelEvent, KernelError> {
    observation.validate()?;
    let frame = capture_frame_id(&observation.canonical_payload)?;
    let obs = observation_id(
        &observation.source_id,
        &session.source_scope,
        &observation.provider_instrument_id,
        &observation.canonical_payload,
    )
    .map_err(|_| KernelError::EmptyFrame)?;

    let mut event = KernelEvent::observation_only(
        frame,
        obs,
        observation.raw_frame_hash.clone(),
        observation.origin_class,
        observation.source_id.clone(),
        observation.source_session_id.clone(),
        observation.provider_instrument_id.clone(),
        observation.receive_utc_ns,
        observation.receive_monotonic_ns,
    )
    .map_err(|_| KernelError::InvalidReceiveTime)?;

    event = event
        .with_receive_fence(observation.connection_epoch, observation.receive_sequence)
        .map_err(|_| KernelError::InvalidReceiveTime)?;
    event.sequence = observation.sequence;
    event.sequence_scope = observation.sequence_scope;
    if let Some(source_time) = observation.source_time_ns {
        let domain = observation
            .source_clock_domain
            .clone()
            .unwrap_or_else(|| "provider_unspecified".to_owned());
        event = event
            .with_source_time(source_time, domain)
            .map_err(|_| KernelError::InvalidReceiveTime)?;
    }
    if let Some(instrument) = &observation.instrument_id {
        let revision = observation
            .reference_revision
            .as_deref()
            .or(session.session_reference_revision.as_deref())
            .ok_or(KernelError::MissingReferenceRevision)?;
        event = event
            .clone()
            .resolve_instrument(instrument.clone(), None, revision)
            .map_err(|_| KernelError::MissingReferenceRevision)?;
    }
    event.evidence_token = observation.evidence_token.clone();
    Ok(event)
}

/// Project a whole verified session deterministically.
///
/// Continuity is keyed by the declared sequence scope: source session,
/// channel and scope, plus instrument only when the provider sequence is
/// per-instrument. Connection epochs stay inside the tracker so a reconnect
/// cannot silently clear a gap. A source with no sequence scope produces
/// `Unverified` events, never clean ones.
pub fn project_session(session: &KernelSession) -> Result<Vec<KernelEvent>, KernelError> {
    session.validate()?;
    let mut machines: BTreeMap<ContinuityKey, ContinuityMachine> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut events = Vec::with_capacity(session.observations.len());

    for observation in &session.observations {
        let mut event = project_observation(session, observation)?;
        if !seen.insert(event.observation_id.clone()) {
            return Err(KernelError::DuplicateObservation);
        }
        match ContinuityKey::for_observation(observation) {
            None => {
                event.quality = StreamState::Unverified;
                event.instrument_id = None;
            }
            Some(key) => {
                let sequence = observation.sequence.ok_or(KernelError::MissingSequence)?;
                let machine = machines.entry(key).or_default();
                machine
                    .observe(observation.connection_epoch, sequence)
                    .map_err(|_| KernelError::InvalidReceiveTime)?;
                event.quality = machine.state();
                if matches!(
                    event.quality,
                    StreamState::Gap | StreamState::Recovering | StreamState::Unverified
                ) {
                    event.instrument_id = None;
                }
            }
        }
        events.push(event);
    }
    Ok(events)
}
