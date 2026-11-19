//! Shared ingress port for independently sourced raw market data.
//!
//! Delivery identity is distinct from a payload digest: identical bytes can be
//! two observations. Sequence provenance and scope are explicit; a capture
//! session sequence is never treated as a provider sequence. Resume and replay
//! stream the journal through `JournalReader`, so memory is bounded by record
//! size and explicit state budgets rather than by archive size.

use std::{
    collections::{
        BTreeMap, //
        VecDeque,
    },
    fs::File,
    io::BufReader,
};

use base64::{
    Engine as _, //
    engine::general_purpose::STANDARD,
};
use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};
use thiserror::Error;

use crate::{
    ArchiveError, //
    ContinuityTracker,
    JournalReader,
    JournalWriter,
    MAX_PAYLOAD_BYTES,
    Observation,
    RawFrame,
};

/// The on-disk source identity is `upstox`, as required by the established
/// raw-frame journal contract and its decoder adapters. Changing this value
/// breaks previously captured sessions, so v3 protocol identity is recorded in
/// manifest fields, not in the frame source string.
pub const UPSTOX_SOURCE_ID: &str = "upstox";
pub const FYERS_SOURCE_ID: &str = "fyers.data_socket";

const RECENT_DELIVERIES: usize = 4096;
const MAX_CONTINUITY_STREAMS: usize = 1024;

/// Where a sequence value came from and what it orders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SequenceScope {
    /// Recorder-local counter for one capture session, across all symbols.
    CaptureSession,
    /// Sequence supplied by the provider for one instrument stream.
    Provider,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    pub scope: SequenceScope,
    pub value: u64,
}

impl Sequence {
    #[must_use]
    pub const fn capture_session(value: u64) -> Self {
        Self {
            scope: SequenceScope::CaptureSession,
            value,
        }
    }

    #[must_use]
    pub const fn provider(value: u64) -> Self {
        Self {
            scope: SequenceScope::Provider,
            value,
        }
    }
}

/// Original provider bytes plus capture metadata. No payload interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFrame {
    pub source_id: String,
    pub provider_instrument_id: String,
    pub entitlement_ref: String,
    pub received_utc_nanos: i64,
    pub received_monotonic_nanos: u64,
    pub exchange_utc_nanos: Option<i64>,
    pub sequence: Option<Sequence>,
    pub payload: Vec<u8>,
}

/// Identity of one delivery, never derived from payload content alone.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeliveryKey {
    Sequenced {
        source: String,
        scope: SequenceScope,
        value: u64,
        instrument: String,
    },
    Timed {
        source: String,
        instrument: String,
        received_utc: i64,
        received_monotonic: u64,
    },
}

impl SourceFrame {
    #[must_use]
    pub fn delivery_key(&self) -> DeliveryKey {
        match self.sequence {
            Some(Sequence { scope, value }) => DeliveryKey::Sequenced {
                source: self.source_id.clone(),
                scope,
                value,
                instrument: self.provider_instrument_id.clone(),
            },
            None => DeliveryKey::Timed {
                source: self.source_id.clone(),
                instrument: self.provider_instrument_id.clone(),
                received_utc: self.received_utc_nanos,
                received_monotonic: self.received_monotonic_nanos,
            },
        }
    }
}

/// How a source's sequence values may be used for continuity accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequencePolicy {
    CaptureSession,
    ProviderPerInstrument,
    None,
}

#[derive(Debug, Error)]
pub enum IngressError {
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error("frame source `{actual}` does not match bound source `{expected}`")]
    SourceMismatch { expected: String, actual: String },
    #[error("an empty payload is not a valid raw frame")]
    EmptyPayload,
    #[error("source reported a disconnect")]
    Disconnected,
    #[error("adapter reported a malformed frame: {0}")]
    Malformed(String),
}

pub trait IngressSource {
    fn source_id(&self) -> &str;
    fn next_frame(&mut self) -> Result<Option<SourceFrame>, IngressError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    Committed { sequence: Option<u64> },
    DuplicateSkipped,
    ConflictRejected,
    MalformedRejected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngressCheckpoint {
    pub committed_frames: u64,
    pub last_capture_sequence: Option<u64>,
    pub last_payload_sha256: Option<String>,
    pub duplicates_skipped: u64,
    pub conflicts_rejected: u64,
    pub gaps_observed: u64,
    pub out_of_order: u64,
    pub malformed_rejected: u64,
    pub unknown_continuity_streams: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    pub committed: u64,
    pub duplicates: u64,
    pub conflicts: u64,
    pub malformed: u64,
    pub disconnected: bool,
}

pub struct DurableIngestor {
    source_id: String,
    policy: SequencePolicy,
    writer: JournalWriter,
    checkpoint: IngressCheckpoint,
    continuity: BTreeMap<String, ContinuityTracker>,
    session_tracker: ContinuityTracker,
    stream_order: VecDeque<String>,
    deliveries: BTreeMap<DeliveryKey, String>,
    delivery_order: VecDeque<DeliveryKey>,
    continuity_degraded: bool,
    committed_ordinal: u64,
}

fn payload_digest(payload: &[u8]) -> String {
    format!("{:x}", Sha256::digest(payload))
}

impl DurableIngestor {
    pub fn create_new(
        path: impl AsRef<std::path::Path>,
        source_id: &str,
        policy: SequencePolicy,
    ) -> Result<Self, IngressError> {
        Ok(Self {
            source_id: source_id.to_owned(),
            policy,
            writer: JournalWriter::create_new(path)?,
            checkpoint: IngressCheckpoint::default(),
            continuity: BTreeMap::new(),
            session_tracker: ContinuityTracker::new(),
            stream_order: VecDeque::new(),
            deliveries: BTreeMap::new(),
            delivery_order: VecDeque::new(),
            continuity_degraded: false,
            committed_ordinal: 0,
        })
    }

    /// Reopen a journal and rebuild bounded state by streaming it. Memory does
    /// not grow with the total number of committed frames.
    pub fn resume(
        path: impl AsRef<std::path::Path>,
        source_id: &str,
        policy: SequencePolicy,
    ) -> Result<Self, IngressError> {
        let mut ingestor = Self {
            source_id: source_id.to_owned(),
            policy,
            writer: JournalWriter::open_append(&path)?,
            checkpoint: IngressCheckpoint::default(),
            continuity: BTreeMap::new(),
            session_tracker: ContinuityTracker::new(),
            stream_order: VecDeque::new(),
            deliveries: BTreeMap::new(),
            delivery_order: VecDeque::new(),
            continuity_degraded: false,
            committed_ordinal: 0,
        };
        let file = File::open(path).map_err(ArchiveError::from)?;
        let mut reader = JournalReader::new(BufReader::new(file));
        while let Some(frame) = reader.next_frame()? {
            if frame.source != source_id {
                continue;
            }
            let raw_sequence = frame.capture_sequence;
            ingestor.checkpoint.committed_frames += 1;
            ingestor.committed_ordinal += 1;
            ingestor.checkpoint.last_payload_sha256 = Some(frame.payload_sha256.clone());
            let key = match raw_sequence {
                Some(value) => DeliveryKey::Sequenced {
                    source: frame.source.clone(),
                    scope: match policy {
                        SequencePolicy::ProviderPerInstrument => SequenceScope::Provider,
                        _ => SequenceScope::CaptureSession,
                    },
                    value,
                    instrument: frame.provider_instrument_id.clone(),
                },
                None => DeliveryKey::Timed {
                    source: frame.source.clone(),
                    instrument: frame.provider_instrument_id.clone(),
                    received_utc: frame.received_utc_nanos,
                    received_monotonic: frame.received_monotonic_nanos,
                },
            };
            ingestor.remember(key, frame.payload_sha256.clone());
            if let Some(sequence) = raw_sequence {
                ingestor.checkpoint.last_capture_sequence = Some(
                    ingestor
                        .checkpoint
                        .last_capture_sequence
                        .map_or(sequence, |last| last.max(sequence)),
                );
                ingestor.observe_continuity(&frame.provider_instrument_id, sequence);
            }
        }
        Ok(ingestor)
    }

    #[must_use]
    pub const fn checkpoint(&self) -> &IngressCheckpoint {
        &self.checkpoint
    }

    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    #[must_use]
    pub const fn policy(&self) -> SequencePolicy {
        self.policy
    }

    #[must_use]
    pub const fn continuity_degraded(&self) -> bool {
        self.continuity_degraded
    }

    #[must_use]
    pub fn tracked_streams(&self) -> usize {
        self.continuity.len()
    }

    #[must_use]
    pub fn known_deliveries(&self) -> usize {
        self.deliveries.len()
    }

    fn remember(&mut self, key: DeliveryKey, digest: String) {
        if self.delivery_order.len() == RECENT_DELIVERIES
            && let Some(oldest) = self.delivery_order.pop_front()
        {
            self.deliveries.remove(&oldest);
        }
        self.deliveries.insert(key.clone(), digest);
        self.delivery_order.push_back(key);
    }

    fn observe_continuity(&mut self, instrument: &str, sequence: u64) {
        let observation = match self.policy {
            SequencePolicy::CaptureSession => self.session_tracker.observe(sequence),
            SequencePolicy::ProviderPerInstrument => {
                if self.continuity_degraded {
                    return;
                }
                if !self.continuity.contains_key(instrument)
                    && self.continuity.len() >= MAX_CONTINUITY_STREAMS
                    && let Some(oldest) = self.stream_order.pop_front()
                {
                    self.continuity.remove(&oldest);
                    // One bounded flag records that continuity is no longer
                    // authoritative; remembering every evicted id would grow
                    // memory with archive size.
                    self.continuity_degraded = true;
                    self.checkpoint.unknown_continuity_streams += 1;
                }
                if !self.continuity.contains_key(instrument) {
                    self.stream_order.push_back(instrument.to_owned());
                }
                self.continuity
                    .entry(instrument.to_owned())
                    .or_default()
                    .observe(sequence)
            }
            SequencePolicy::None => return,
        };
        match observation {
            Observation::Gap { .. } => self.checkpoint.gaps_observed += 1,
            Observation::OutOfOrder { .. } => self.checkpoint.out_of_order += 1,
            Observation::Duplicate => self.checkpoint.duplicates_skipped += 1,
            Observation::Initial | Observation::InOrder => {}
        }
    }

    fn preview_continuity(&self, instrument: &str, sequence: u64) -> Observation {
        match self.policy {
            // A capture-session sequence is session-wide: distinct delivery
            // identities sharing a value are not continuity duplicates.
            SequencePolicy::CaptureSession => Observation::InOrder,
            SequencePolicy::ProviderPerInstrument => self
                .continuity
                .get(instrument)
                .cloned()
                .unwrap_or_default()
                .observe(sequence),
            SequencePolicy::None => Observation::Initial,
        }
    }

    pub fn ingest_one(&mut self, frame: SourceFrame) -> Result<IngestOutcome, IngressError> {
        if frame.source_id != self.source_id {
            return Err(IngressError::SourceMismatch {
                expected: self.source_id.clone(),
                actual: frame.source_id,
            });
        }
        if frame.payload.is_empty() || frame.payload.len() > MAX_PAYLOAD_BYTES {
            self.checkpoint.malformed_rejected += 1;
            return Ok(IngestOutcome::MalformedRejected);
        }
        let digest = payload_digest(&frame.payload);
        let key = frame.delivery_key();
        if let Some(committed_digest) = self.deliveries.get(&key) {
            if committed_digest == &digest {
                self.checkpoint.duplicates_skipped += 1;
                return Ok(IngestOutcome::DuplicateSkipped);
            }
            self.checkpoint.conflicts_rejected += 1;
            return Ok(IngestOutcome::ConflictRejected);
        }
        if let Some(sequence) = frame.sequence {
            let observation =
                self.preview_continuity(&frame.provider_instrument_id, sequence.value);
            if matches!(observation, Observation::Duplicate) {
                self.checkpoint.duplicates_skipped += 1;
                return Ok(IngestOutcome::DuplicateSkipped);
            }
        }
        let raw = match RawFrame::new(
            &frame.source_id,
            &frame.provider_instrument_id,
            frame.received_utc_nanos,
            frame.received_monotonic_nanos,
            frame.exchange_utc_nanos,
            frame.sequence.map(|item| item.value),
            &frame.entitlement_ref,
            &frame.payload,
        ) {
            Ok(raw) => raw,
            Err(_) => {
                self.checkpoint.malformed_rejected += 1;
                return Ok(IngestOutcome::MalformedRejected);
            }
        };
        self.writer.append(&raw)?;
        if let Some(sequence) = frame.sequence {
            self.observe_continuity(&frame.provider_instrument_id, sequence.value);
        }
        self.remember(key, digest.clone());
        self.committed_ordinal += 1;
        self.checkpoint.committed_frames += 1;
        self.checkpoint.last_payload_sha256 = Some(digest);
        if let Some(sequence) = frame.sequence {
            self.checkpoint.last_capture_sequence = Some(
                self.checkpoint
                    .last_capture_sequence
                    .map_or(sequence.value, |last| last.max(sequence.value)),
            );
        }
        Ok(IngestOutcome::Committed {
            sequence: frame.sequence.map(|item| item.value),
        })
    }

    /// Replay committed frames with an ordinal fence so older frames are never
    /// re-committed, independent of the bounded delivery window.
    pub fn replay(&mut self, source: &mut JournalReplaySource) -> Result<RunSummary, IngressError> {
        if source.source_id() != self.source_id {
            return Err(IngressError::SourceMismatch {
                expected: self.source_id.clone(),
                actual: source.source_id().to_owned(),
            });
        }
        let mut summary = RunSummary::default();
        while let Some((ordinal, frame)) = source.next_indexed_frame()? {
            if ordinal < self.committed_ordinal {
                summary.duplicates += 1;
                continue;
            }
            match self.ingest_one(frame)? {
                IngestOutcome::Committed { .. } => summary.committed += 1,
                IngestOutcome::DuplicateSkipped => summary.duplicates += 1,
                IngestOutcome::ConflictRejected => summary.conflicts += 1,
                IngestOutcome::MalformedRejected => summary.malformed += 1,
            }
        }
        Ok(summary)
    }

    pub fn run(
        &mut self,
        source: &mut impl IngressSource,
        max_frames: usize,
    ) -> Result<RunSummary, IngressError> {
        if source.source_id() != self.source_id {
            return Err(IngressError::SourceMismatch {
                expected: self.source_id.clone(),
                actual: source.source_id().to_owned(),
            });
        }
        let mut summary = RunSummary::default();
        for _ in 0..max_frames {
            match source.next_frame() {
                Ok(None) => break,
                Ok(Some(frame)) => match self.ingest_one(frame)? {
                    IngestOutcome::Committed { .. } => summary.committed += 1,
                    IngestOutcome::DuplicateSkipped => summary.duplicates += 1,
                    IngestOutcome::ConflictRejected => summary.conflicts += 1,
                    IngestOutcome::MalformedRejected => summary.malformed += 1,
                },
                Err(IngressError::Disconnected) => {
                    summary.disconnected = true;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(summary)
    }
}

/// Upstox adapter: original protobuf bytes are never decoded here.
pub struct UpstoxV3Adapter {
    frames: VecDeque<Result<SourceFrame, IngressError>>,
}

impl UpstoxV3Adapter {
    #[must_use]
    pub fn new(frames: Vec<SourceFrame>) -> Self {
        Self {
            frames: frames.into_iter().map(Ok).collect(),
        }
    }

    pub fn push(&mut self, frame: SourceFrame) {
        self.frames.push_back(Ok(frame));
    }

    pub fn disconnect(&mut self) {
        self.frames.push_back(Err(IngressError::Disconnected));
    }
}

impl IngressSource for UpstoxV3Adapter {
    fn source_id(&self) -> &str {
        UPSTOX_SOURCE_ID
    }

    fn next_frame(&mut self) -> Result<Option<SourceFrame>, IngressError> {
        match self.frames.pop_front() {
            None => Ok(None),
            Some(Ok(frame)) => Ok(Some(frame)),
            Some(Err(error)) => Err(error),
        }
    }
}

/// FYERS callback observation. The SDK supplies decoded JSON; its bytes are
/// preserved exactly and the recorder's local sequence is a capture-session
/// sequence, never a provider sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FyersCallback {
    pub provider_instrument_id: String,
    pub entitlement_ref: String,
    pub received_utc_nanos: i64,
    pub received_monotonic_nanos: u64,
    pub exchange_utc_nanos: Option<i64>,
    pub local_sequence: Option<u64>,
    pub callback_json: Vec<u8>,
}

impl FyersCallback {
    #[must_use]
    pub fn into_source_frame(self) -> SourceFrame {
        SourceFrame {
            source_id: FYERS_SOURCE_ID.to_owned(),
            provider_instrument_id: self.provider_instrument_id,
            entitlement_ref: self.entitlement_ref,
            received_utc_nanos: self.received_utc_nanos,
            received_monotonic_nanos: self.received_monotonic_nanos,
            exchange_utc_nanos: self.exchange_utc_nanos,
            sequence: self.local_sequence.map(Sequence::capture_session),
            payload: self.callback_json,
        }
    }
}

pub struct FyersCallbackAdapter {
    callbacks: VecDeque<Result<FyersCallback, IngressError>>,
}

impl FyersCallbackAdapter {
    #[must_use]
    pub fn new(callbacks: Vec<FyersCallback>) -> Self {
        Self {
            callbacks: callbacks.into_iter().map(Ok).collect(),
        }
    }

    pub fn push(&mut self, callback: FyersCallback) {
        self.callbacks.push_back(Ok(callback));
    }

    pub fn disconnect(&mut self) {
        self.callbacks.push_back(Err(IngressError::Disconnected));
    }
}

impl IngressSource for FyersCallbackAdapter {
    fn source_id(&self) -> &str {
        FYERS_SOURCE_ID
    }

    fn next_frame(&mut self) -> Result<Option<SourceFrame>, IngressError> {
        match self.callbacks.pop_front() {
            None => Ok(None),
            Some(Ok(callback)) => Ok(Some(callback.into_source_frame())),
            Some(Err(error)) => Err(error),
        }
    }
}

/// Streaming replay source. It holds one `JournalReader`, not a journal-sized
/// collection, so memory is bounded by the record limit.
pub struct JournalReplaySource {
    source_id: String,
    reader: JournalReader<BufReader<File>>,
    ordinal: u64,
}

impl JournalReplaySource {
    pub fn open(path: impl AsRef<std::path::Path>, source_id: &str) -> Result<Self, IngressError> {
        let file = File::open(path).map_err(ArchiveError::from)?;
        Ok(Self {
            source_id: source_id.to_owned(),
            reader: JournalReader::new(BufReader::new(file)),
            ordinal: 0,
        })
    }

    /// Next frame and its ordinal among committed frames of this source.
    pub fn next_indexed_frame(&mut self) -> Result<Option<(u64, SourceFrame)>, IngressError> {
        while let Some(raw) = self.reader.next_frame()? {
            if raw.source != self.source_id {
                continue;
            }
            let payload = STANDARD.decode(&raw.payload_base64).map_err(|_| {
                IngressError::Malformed("replayed payload is not valid base64".to_owned())
            })?;
            let ordinal = self.ordinal;
            self.ordinal += 1;
            return Ok(Some((
                ordinal,
                SourceFrame {
                    source_id: raw.source,
                    provider_instrument_id: raw.provider_instrument_id,
                    entitlement_ref: raw.entitlement_ref,
                    received_utc_nanos: raw.received_utc_nanos,
                    received_monotonic_nanos: raw.received_monotonic_nanos,
                    exchange_utc_nanos: raw.exchange_utc_nanos,
                    sequence: raw.capture_sequence.map(Sequence::capture_session),
                    payload,
                },
            )));
        }
        Ok(None)
    }
}

impl IngressSource for JournalReplaySource {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    fn next_frame(&mut self) -> Result<Option<SourceFrame>, IngressError> {
        Ok(self.next_indexed_frame()?.map(|(_, frame)| frame))
    }
}
