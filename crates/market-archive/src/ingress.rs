//! Shared ingress port for independently sourced raw market data.
//!
//! A source adapter supplies original payload bytes plus capture metadata; the
//! ingestor routes them through the durable raw-frame journal. Provider
//! decoding and canonical instrument resolution are deliberately separate
//! stages: an unknown instrument stays unresolved and no identity is invented.

use std::collections::{
    BTreeMap, //
    HashSet,
    VecDeque,
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
    JournalWriter,
    MAX_PAYLOAD_BYTES,
    Observation,
    RawFrame,
    scan_journal,
};

/// Source identity used by the Upstox V3 raw protobuf adapter.
pub const UPSTOX_SOURCE_ID: &str = "upstox.v3";

/// Number of recent payload hashes retained for in-process deduplication and
/// rebuilt from the journal on restart. The most recent hash additionally
/// fences an exact replay of the last committed frame.
const RECENT_HASHES: usize = 4096;

/// Maximum number of per-instrument continuity trackers retained in memory.
/// Least-recently-added streams are evicted; a returning stream is treated as
/// a fresh one rather than allowing source-supplied ids to grow memory freely.
const MAX_CONTINUITY_STREAMS: usize = 1024;

/// An original provider frame plus the capture metadata needed to audit it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFrame {
    pub source_id: String,
    pub provider_instrument_id: String,
    pub entitlement_ref: String,
    pub received_utc_nanos: i64,
    pub received_monotonic_nanos: u64,
    pub exchange_utc_nanos: Option<i64>,
    pub provider_sequence: Option<u64>,
    pub payload: Vec<u8>,
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

/// One independent source implementation. The adapter never creates canonical
/// identities or interprets the payload; it only preserves bytes and metadata.
pub trait IngressSource {
    fn source_id(&self) -> &str;
    fn next_frame(&mut self) -> Result<Option<SourceFrame>, IngressError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    Committed { capture_sequence: u64 },
    DuplicateSkipped,
    MalformedRejected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngressCheckpoint {
    pub committed_frames: u64,
    pub last_capture_sequence: Option<u64>,
    pub last_payload_sha256: Option<String>,
    pub duplicates_skipped: u64,
    pub gaps_observed: u64,
    pub out_of_order: u64,
    pub malformed_rejected: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    pub committed: u64,
    pub duplicates: u64,
    pub malformed: u64,
    pub disconnected: bool,
}

/// Durable ingress for one bound source. Every committed frame is written to
/// the shared journal before the checkpoint advances.
pub struct DurableIngestor {
    source_id: String,
    writer: JournalWriter,
    checkpoint: IngressCheckpoint,
    continuity: BTreeMap<String, ContinuityTracker>,
    stream_order: VecDeque<String>,
    recent: VecDeque<String>,
    seen: HashSet<String>,
}

impl DurableIngestor {
    /// Start a fresh journal bound to one source identity.
    pub fn create_new(
        path: impl AsRef<std::path::Path>,
        source_id: &str,
    ) -> Result<Self, IngressError> {
        Ok(Self {
            source_id: source_id.to_owned(),
            writer: JournalWriter::create_new(path)?,
            checkpoint: IngressCheckpoint::default(),
            continuity: BTreeMap::new(),
            stream_order: VecDeque::new(),
            recent: VecDeque::new(),
            seen: HashSet::new(),
        })
    }

    /// Reopen a journal and rebuild the checkpoint from its committed frames.
    /// Only frames of the bound source contribute to the checkpoint.
    pub fn resume(
        path: impl AsRef<std::path::Path>,
        source_id: &str,
    ) -> Result<Self, IngressError> {
        let mut ingestor = Self {
            source_id: source_id.to_owned(),
            writer: JournalWriter::open_append(&path)?,
            checkpoint: IngressCheckpoint::default(),
            continuity: BTreeMap::new(),
            stream_order: VecDeque::new(),
            recent: VecDeque::new(),
            seen: HashSet::new(),
        };
        for frame in scan_journal(path).map_err(IngressError::Archive)?.frames {
            if frame.source != source_id {
                continue;
            }
            ingestor.checkpoint.committed_frames += 1;
            ingestor.checkpoint.last_payload_sha256 = Some(frame.payload_sha256.clone());
            // Rebuild the bounded hash window so a replay of a recently
            // committed frame is still recognised after restart.
            ingestor.remember(frame.payload_sha256.clone());
            if let Some(sequence) = frame.capture_sequence {
                ingestor.checkpoint.last_capture_sequence = Some(
                    ingestor
                        .checkpoint
                        .last_capture_sequence
                        .map_or(sequence, |last| last.max(sequence)),
                );
                let observation = ingestor
                    .stream_tracker(&frame.provider_instrument_id)
                    .observe(sequence);
                match observation {
                    Observation::Gap { .. } => ingestor.checkpoint.gaps_observed += 1,
                    Observation::OutOfOrder { .. } => ingestor.checkpoint.out_of_order += 1,
                    Observation::Duplicate => ingestor.checkpoint.duplicates_skipped += 1,
                    Observation::Initial | Observation::InOrder => {}
                }
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
    pub fn tracked_streams(&self) -> usize {
        self.continuity.len()
    }

    fn stream_tracker(&mut self, instrument: &str) -> &mut ContinuityTracker {
        if !self.continuity.contains_key(instrument) {
            if self.continuity.len() >= MAX_CONTINUITY_STREAMS
                && let Some(oldest) = self.stream_order.pop_front()
            {
                self.continuity.remove(&oldest);
            }
            self.stream_order.push_back(instrument.to_owned());
        }
        self.continuity.entry(instrument.to_owned()).or_default()
    }

    fn preview(&self, instrument: &str, sequence: u64) -> Observation {
        self.continuity
            .get(instrument)
            .cloned()
            .unwrap_or_default()
            .observe(sequence)
    }

    /// Validate and durably append one source frame.
    pub fn ingest_one(&mut self, frame: SourceFrame) -> Result<IngestOutcome, IngressError> {
        if frame.source_id != self.source_id {
            return Err(IngressError::SourceMismatch {
                expected: self.source_id.clone(),
                actual: frame.source_id,
            });
        }
        if frame.payload.is_empty() {
            self.checkpoint.malformed_rejected += 1;
            return Ok(IngestOutcome::MalformedRejected);
        }
        if frame.payload.len() > MAX_PAYLOAD_BYTES {
            self.checkpoint.malformed_rejected += 1;
            return Ok(IngestOutcome::MalformedRejected);
        }
        let digest = format!("{:x}", Sha256::digest(&frame.payload));
        // The recovered last hash fences an exact replay of the most recent
        // committed frame even when its hash has aged out of the window.
        if self.checkpoint.last_payload_sha256.as_deref() == Some(digest.as_str())
            || self.seen.contains(&digest)
        {
            self.checkpoint.duplicates_skipped += 1;
            return Ok(IngestOutcome::DuplicateSkipped);
        }
        let preview = frame.provider_sequence.map(|sequence| {
            (
                sequence,
                self.preview(&frame.provider_instrument_id, sequence),
            )
        });
        if let Some((_, Observation::Duplicate)) = preview {
            self.checkpoint.duplicates_skipped += 1;
            return Ok(IngestOutcome::DuplicateSkipped);
        }
        let raw = match RawFrame::new(
            &frame.source_id,
            &frame.provider_instrument_id,
            frame.received_utc_nanos,
            frame.received_monotonic_nanos,
            frame.exchange_utc_nanos,
            frame.provider_sequence,
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
        // Continuity advances only after the frame is durably committed, so a
        // validation or write failure cannot create a phantom observation.
        if let Some((sequence, observation)) = preview {
            let applied = self
                .stream_tracker(&frame.provider_instrument_id)
                .observe(sequence);
            debug_assert_eq!(
                applied, observation,
                "the previewed continuity observation must match the applied one"
            );
            match observation {
                Observation::Gap { .. } => self.checkpoint.gaps_observed += 1,
                Observation::OutOfOrder { .. } => self.checkpoint.out_of_order += 1,
                Observation::Duplicate | Observation::Initial | Observation::InOrder => {}
            }
        }
        self.remember(digest.clone());
        self.checkpoint.committed_frames += 1;
        self.checkpoint.last_payload_sha256 = Some(digest);
        if let Some(sequence) = frame.provider_sequence {
            self.checkpoint.last_capture_sequence = Some(
                self.checkpoint
                    .last_capture_sequence
                    .map_or(sequence, |last| last.max(sequence)),
            );
        }
        Ok(IngestOutcome::Committed {
            capture_sequence: frame.provider_sequence.unwrap_or(0),
        })
    }

    fn remember(&mut self, digest: String) {
        if self.recent.len() == RECENT_HASHES
            && let Some(evicted) = self.recent.pop_front()
        {
            self.seen.remove(&evicted);
        }
        self.seen.insert(digest.clone());
        self.recent.push_back(digest);
    }

    /// Drain a source into the journal, preserving the checkpoint at the last
    /// durable frame. A disconnect is a clean stop, not a failure.
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

/// Upstox V3 adapter over a supplied frame queue. `payload` is the original
/// binary WebSocket message; no protobuf decoding happens in the ingress path.
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
