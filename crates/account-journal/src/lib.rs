//! Segmented, append-only account-event journal with exclusive writer ownership
//! and a BLAKE3 hash chain.
//!
//! Derived from the internal Nirnay `crates/nirnay-journal/src/lib.rs` at source snapshot
//! `cc3a34c317a86a9984401efe90fa3f43d67a1787`.
//!
//! This is the **account/execution** journal. The repository's existing
//! `src/journal.rs` remains the raw market-data frame journal; the two serve
//! separate purposes and remain separate.
//!
//! The on-disk frame format (`NRJ1`, 84-byte header, BLAKE3 chain over the
//! sequence, previous hash and body) is preserved. Bodies are validated
//! [`trading_core::StoredBody`] records with deterministic event identity.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use trading_core::{DomainError, EventId, RunId, StoredBody};

const FILE_SIGNATURE: [u8; 4] = *b"NRJ1";
const FORMAT_VERSION: u16 = 1;
const HEADER_LEN: usize = 84;
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
const HASH_DOMAIN: &[u8] = b"NIRNAY-EVENT-JOURNAL-V1\0";

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hash32([u8; 32]);

impl Hash32 {
    pub const ZERO: Self = Self([0; 32]);

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRecord {
    pub sequence: u64,
    pub previous_hash: Hash32,
    pub hash: Hash32,
    pub body: StoredBody,
}

/// Deterministic append-boundary faults for durability tests. Never set in
/// production wiring.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CommitFault {
    BeforeHeader,
    MidBody,
    AfterBodyBeforeSync,
    AfterSync,
    Rollover,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReport {
    pub record_count: u64,
    pub segment_count: u64,
    pub head_hash_hex: String,
    pub repaired_tail_bytes: u64,
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("journal is owned by another writer: {0}")]
    Locked(PathBuf),
    #[error("journal record is corrupt at {path}, offset {offset}: {reason}")]
    Corrupt {
        path: PathBuf,
        offset: u64,
        reason: String,
    },
    #[error("journal record serialization failed: {0}")]
    Serialize(serde_json::Error),
    #[error("journal body is invalid: {0}")]
    Domain(#[from] DomainError),
    #[error("stream mismatch: expected {expected}, received {actual}")]
    StreamMismatch { expected: String, actual: String },
    #[error("event identity {0} was reused for different content")]
    IdempotencyConflict(String),
    #[error("append outcome is uncertain; close and reopen the journal before issuing writes")]
    CommitOutcomeUnknown,
    #[error("journal cannot accept writes after an uncertain append")]
    Poisoned,
    #[error("journal segment start sequence is invalid in {0}")]
    InvalidSegmentName(PathBuf),
    #[error("journal segment sequence is discontinuous at {0}")]
    SegmentSequence(PathBuf),
    #[error("journal sequence space is exhausted")]
    SequenceExhausted,
    #[error("payload requires journal schema version {required} but the body declares {declared}")]
    SchemaVersionTooOld { required: u16, declared: u16 },
}

impl JournalError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// A journal remains locked for the lifetime of the value. A failed write or sync poisons the
/// writer because durability may have changed even when the caller received an error.
pub struct Journal {
    root: PathBuf,
    stream_id: String,
    _lock_file: File,
    active_file: File,
    active_path: PathBuf,
    active_size: u64,
    segment_limit: u64,
    records: Vec<StoredRecord>,
    event_index: BTreeMap<String, usize>,
    previous_hash: Hash32,
    repaired_tail_bytes: u64,
    segment_count: u64,
    poisoned: bool,
    commit_fault: Option<CommitFault>,
}

impl std::fmt::Debug for Journal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Journal")
            .field("root", &self.root)
            .field("stream_id", &self.stream_id)
            .field("active_path", &self.active_path)
            .field("active_size", &self.active_size)
            .field("records", &self.records.len())
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

impl Journal {
    /// Deterministic identity for the next record in this stream.
    ///
    /// A replay that supplies the same [`RunId`] and reproduces the same event
    /// sequence receives byte-identical identifiers. The journal sequence is
    /// the ordinal, so identity never depends on wall-clock time or randomness.
    pub fn next_event_id(&self, run: RunId) -> Result<EventId, JournalError> {
        Ok(EventId::for_run(
            run,
            next_sequence(self.records.last().map(|record| record.sequence))?,
        ))
    }

    /// Inject an append-boundary fault for deterministic crash tests. The
    /// writer is poisoned on every fault; recovery is always a fresh open.
    pub fn force_commit_fault(&mut self, fault: Option<CommitFault>) {
        self.commit_fault = fault;
    }

    pub fn open(
        root: impl AsRef<Path>,
        stream_id: impl Into<String>,
    ) -> Result<Self, JournalError> {
        Self::open_with_segment_limit(root, stream_id, DEFAULT_SEGMENT_BYTES)
    }

    pub fn open_with_segment_limit(
        root: impl AsRef<Path>,
        stream_id: impl Into<String>,
        segment_limit: u64,
    ) -> Result<Self, JournalError> {
        let root = root.as_ref().to_path_buf();
        let stream_id = stream_id.into();
        if stream_id.trim().is_empty() {
            return Err(JournalError::StreamMismatch {
                expected: "non-empty stream identifier".to_owned(),
                actual: stream_id,
            });
        }
        if segment_limit < (HEADER_LEN + 1024) as u64 {
            return Err(JournalError::io(
                &root,
                io::Error::new(io::ErrorKind::InvalidInput, "segment limit is too small"),
            ));
        }
        fs::create_dir_all(root.join("segments"))
            .map_err(|source| JournalError::io(&root, source))?;
        let lock_path = root.join("LOCK");
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| JournalError::io(&lock_path, source))?;
        FileExt::try_lock_exclusive(&lock_file).map_err(|source| {
            if source.kind() == io::ErrorKind::WouldBlock {
                JournalError::Locked(lock_path.clone())
            } else {
                JournalError::io(&lock_path, source)
            }
        })?;

        let active_file = lock_file
            .try_clone()
            .map_err(|source| JournalError::io(&lock_path, source))?;
        let mut journal = Self {
            root,
            stream_id,
            _lock_file: lock_file,
            active_file,
            active_path: PathBuf::new(),
            active_size: 0,
            segment_limit,
            records: Vec::new(),
            event_index: BTreeMap::new(),
            previous_hash: Hash32::ZERO,
            repaired_tail_bytes: 0,
            segment_count: 0,
            poisoned: false,
            commit_fault: None,
        };
        journal.load_segments()?;
        Ok(journal)
    }

    fn load_segments(&mut self) -> Result<(), JournalError> {
        let directory = self.root.join("segments");
        let mut paths = fs::read_dir(&directory)
            .map_err(|source| JournalError::io(&directory, source))?
            .map(|entry| {
                entry
                    .map(|item| item.path())
                    .map_err(|source| JournalError::io(&directory, source))
            })
            .collect::<Result<Vec<_>, _>>()?;
        paths.retain(|path| path.extension().is_some_and(|extension| extension == "nrj"));
        paths.sort();

        self.segment_count = paths.len() as u64;
        for path in &paths {
            let named_start = parse_segment_start(path)?;
            let expected_start = next_sequence(self.records.last().map(|record| record.sequence))?;
            if named_start != expected_start {
                return Err(JournalError::SegmentSequence(path.clone()));
            }
            let is_final = paths.last().is_some_and(|last| path == last);
            self.scan_segment(path, is_final)?;
        }

        if paths.is_empty() {
            self.create_segment(1)?;
        } else {
            let path = paths.last().cloned().ok_or_else(|| {
                JournalError::io(
                    &directory,
                    io::Error::other("segment list lost its final entry"),
                )
            })?;
            self.active_size = fs::metadata(&path)
                .map_err(|source| JournalError::io(&path, source))?
                .len();
            self.active_path = path.clone();
            self.active_file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|source| JournalError::io(&path, source))?;
            self.active_file
                .seek(SeekFrom::End(0))
                .map_err(|source| JournalError::io(&path, source))?;
        }
        self.previous_hash = self
            .records
            .last()
            .map_or(Hash32::ZERO, |record| record.hash);
        Ok(())
    }

    fn scan_segment(&mut self, path: &Path, final_segment: bool) -> Result<(), JournalError> {
        let bytes = fs::read(path).map_err(|source| JournalError::io(path, source))?;
        let mut offset = 0_usize;
        let mut expected_sequence =
            next_sequence(self.records.last().map(|record| record.sequence))?;
        let mut previous_hash = self
            .records
            .last()
            .map_or(Hash32::ZERO, |record| record.hash);

        while offset < bytes.len() {
            let remaining = bytes.len() - offset;
            if remaining < HEADER_LEN {
                if final_segment {
                    self.repair_tail(path, offset, remaining)?;
                    break;
                }
                return Err(corrupt(
                    path,
                    offset,
                    "incomplete header before the final segment",
                ));
            }
            let header = bytes
                .get(offset..offset + HEADER_LEN)
                .ok_or_else(|| corrupt(path, offset, "header range is invalid"))?;
            if header.get(0..4) != Some(FILE_SIGNATURE.as_slice()) {
                return Err(corrupt(path, offset, "file signature does not match"));
            }
            let version = read_u16(header, 4, path, offset)?;
            let flags = read_u16(header, 6, path, offset)?;
            let body_len = read_u32(header, 8, path, offset)? as usize;
            let sequence = read_u64(header, 12, path, offset)?;
            let previous = read_hash(header, 20, path, offset)?;
            let stored_hash = read_hash(header, 52, path, offset)?;
            if version != FORMAT_VERSION || flags != 0 {
                return Err(corrupt(path, offset, "unsupported format version or flags"));
            }
            if body_len > MAX_FRAME_BYTES {
                return Err(corrupt(path, offset, "body length exceeds the frame limit"));
            }
            let frame_len = HEADER_LEN
                .checked_add(body_len)
                .ok_or_else(|| corrupt(path, offset, "frame length overflow"))?;
            if frame_len > remaining {
                if final_segment {
                    self.repair_tail(path, offset, remaining)?;
                    break;
                }
                return Err(corrupt(
                    path,
                    offset,
                    "incomplete frame before the final segment",
                ));
            }
            if sequence != expected_sequence {
                return Err(corrupt(path, offset, "journal sequence is not contiguous"));
            }
            if previous != previous_hash {
                return Err(corrupt(
                    path,
                    offset,
                    "previous hash does not match the chain head",
                ));
            }
            let body_bytes = bytes
                .get(offset + HEADER_LEN..offset + frame_len)
                .ok_or_else(|| corrupt(path, offset, "body range is invalid"))?;
            let actual_hash = hash_record(sequence, previous, body_bytes);
            if stored_hash != actual_hash {
                return Err(corrupt(path, offset, "record hash does not match the body"));
            }
            let body: StoredBody = serde_json::from_slice(body_bytes).map_err(|error| {
                corrupt(path, offset, &format!("body decoding failed: {error}"))
            })?;
            body.validate()
                .map_err(|error| corrupt(path, offset, &error.to_string()))?;
            if body.envelope.stream_id != self.stream_id {
                return Err(JournalError::StreamMismatch {
                    expected: self.stream_id.clone(),
                    actual: body.envelope.stream_id,
                });
            }
            self.index_record(StoredRecord {
                sequence,
                previous_hash: previous,
                hash: stored_hash,
                body,
            })?;
            previous_hash = stored_hash;
            expected_sequence = expected_sequence
                .checked_add(1)
                .ok_or_else(|| corrupt(path, offset, "sequence counter overflow"))?;
            offset += frame_len;
        }
        Ok(())
    }

    fn repair_tail(
        &mut self,
        path: &Path,
        offset: usize,
        tail_len: usize,
    ) -> Result<(), JournalError> {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|source| JournalError::io(path, source))?;
        file.set_len(offset as u64)
            .map_err(|source| JournalError::io(path, source))?;
        file.sync_all()
            .map_err(|source| JournalError::io(path, source))?;
        self.repaired_tail_bytes = self
            .repaired_tail_bytes
            .checked_add(tail_len as u64)
            .ok_or_else(|| {
                JournalError::io(path, io::Error::other("repair byte count overflow"))
            })?;
        Ok(())
    }

    fn index_record(&mut self, record: StoredRecord) -> Result<(), JournalError> {
        let event_key = record.body.envelope.event_id.to_string();
        if self.event_index.contains_key(&event_key) {
            return Err(JournalError::IdempotencyConflict(event_key));
        }
        let index = self.records.len();
        self.event_index.insert(event_key, index);
        self.records.push(record);
        Ok(())
    }

    fn create_segment(&mut self, start_sequence: u64) -> Result<(), JournalError> {
        let directory = self.root.join("segments");
        let path = segment_path(&directory, start_sequence);
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| JournalError::io(&path, source))?;
        file.sync_all()
            .map_err(|source| JournalError::io(&path, source))?;
        sync_directory(&directory)?;
        self.active_size = 0;
        self.segment_count = self.segment_count.saturating_add(1);
        self.active_path = path;
        self.active_file = file;
        Ok(())
    }

    pub fn append(&mut self, body: StoredBody) -> Result<StoredRecord, JournalError> {
        if self.poisoned {
            return Err(JournalError::Poisoned);
        }
        body.validate()?;
        let required = body.payload.required_schema_version();
        if body.envelope.schema_version < required {
            return Err(JournalError::SchemaVersionTooOld {
                required,
                declared: body.envelope.schema_version,
            });
        }
        if body.envelope.stream_id != self.stream_id {
            return Err(JournalError::StreamMismatch {
                expected: self.stream_id.clone(),
                actual: body.envelope.stream_id,
            });
        }
        let event_key = body.envelope.event_id.to_string();
        let body_bytes = serde_json::to_vec(&body).map_err(JournalError::Serialize)?;
        if let Some(index) = self.event_index.get(&event_key).copied() {
            let existing = self.records.get(index).ok_or_else(|| {
                JournalError::io(
                    &self.root,
                    io::Error::other("event index points outside journal records"),
                )
            })?;
            let existing_bytes =
                serde_json::to_vec(&existing.body).map_err(JournalError::Serialize)?;
            if existing_bytes == body_bytes {
                return Ok(existing.clone());
            }
            return Err(JournalError::IdempotencyConflict(event_key));
        }
        if body_bytes.len() > MAX_FRAME_BYTES {
            return Err(JournalError::io(
                &self.active_path,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "event exceeds maximum frame size",
                ),
            ));
        }

        let sequence = next_sequence(self.records.last().map(|record| record.sequence))?;
        let hash = hash_record(sequence, self.previous_hash, &body_bytes);
        let frame_len = HEADER_LEN as u64 + body_bytes.len() as u64;
        let mut rolled = false;
        if self.active_size > 0 && self.active_size.saturating_add(frame_len) > self.segment_limit {
            if self.active_file.sync_all().is_err() {
                self.poisoned = true;
                return Err(JournalError::CommitOutcomeUnknown);
            }
            if self.create_segment(sequence).is_err() {
                self.poisoned = true;
                return Err(JournalError::CommitOutcomeUnknown);
            }
            rolled = true;
        }
        if rolled && self.commit_fault == Some(CommitFault::Rollover) {
            self.poisoned = true;
            return Err(JournalError::CommitOutcomeUnknown);
        }
        if self.commit_fault == Some(CommitFault::BeforeHeader) {
            self.poisoned = true;
            return Err(JournalError::CommitOutcomeUnknown);
        }
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(&FILE_SIGNATURE);
        header.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
        header.extend_from_slice(&0_u16.to_be_bytes());
        let body_len = u32::try_from(body_bytes.len()).map_err(|_| {
            JournalError::io(
                &self.active_path,
                io::Error::new(io::ErrorKind::InvalidInput, "body is too large"),
            )
        })?;
        header.extend_from_slice(&body_len.to_be_bytes());
        header.extend_from_slice(&sequence.to_be_bytes());
        header.extend_from_slice(self.previous_hash.as_bytes());
        header.extend_from_slice(hash.as_bytes());

        if self.active_file.write_all(&header).is_err() {
            self.poisoned = true;
            return Err(JournalError::CommitOutcomeUnknown);
        }
        match self.commit_fault {
            Some(CommitFault::MidBody) => {
                let split = body_bytes.len() / 2;
                if self.active_file.write_all(&body_bytes[..split]).is_ok() {
                    let _ = self.active_file.flush();
                }
                self.poisoned = true;
                return Err(JournalError::CommitOutcomeUnknown);
            }
            Some(CommitFault::AfterBodyBeforeSync) => {
                if self.active_file.write_all(&body_bytes).is_err() {
                    self.poisoned = true;
                    return Err(JournalError::CommitOutcomeUnknown);
                }
                self.poisoned = true;
                return Err(JournalError::CommitOutcomeUnknown);
            }
            Some(CommitFault::AfterSync) => {
                if self.active_file.write_all(&body_bytes).is_err()
                    || self.active_file.sync_data().is_err()
                {
                    self.poisoned = true;
                    return Err(JournalError::CommitOutcomeUnknown);
                }
                self.poisoned = true;
                return Err(JournalError::CommitOutcomeUnknown);
            }
            Some(CommitFault::BeforeHeader | CommitFault::Rollover) | None => {}
        }
        if self.active_file.write_all(&body_bytes).is_err() || self.active_file.sync_data().is_err()
        {
            self.poisoned = true;
            return Err(JournalError::CommitOutcomeUnknown);
        }
        self.active_size = self.active_size.saturating_add(frame_len);
        let stored = StoredRecord {
            sequence,
            previous_hash: self.previous_hash,
            hash,
            body,
        };
        self.previous_hash = hash;
        self.index_record(stored.clone())?;
        Ok(stored)
    }

    #[must_use]
    pub fn records(&self) -> &[StoredRecord] {
        &self.records
    }

    #[must_use]
    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn verification_report(&self) -> VerificationReport {
        VerificationReport {
            record_count: self.records.len() as u64,
            segment_count: self.segment_count,
            head_hash_hex: self.previous_hash.hex(),
            repaired_tail_bytes: self.repaired_tail_bytes,
        }
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._lock_file);
    }
}

fn next_sequence(last_sequence: Option<u64>) -> Result<u64, JournalError> {
    last_sequence
        .map_or(Some(1), |sequence| sequence.checked_add(1))
        .ok_or(JournalError::SequenceExhausted)
}

fn segment_path(directory: &Path, start_sequence: u64) -> PathBuf {
    directory.join(format!("segment-{start_sequence:020}.nrj"))
}

fn parse_segment_start(path: &Path) -> Result<u64, JournalError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| JournalError::InvalidSegmentName(path.to_path_buf()))?;
    let digits = name
        .strip_prefix("segment-")
        .and_then(|name| name.strip_suffix(".nrj"))
        .ok_or_else(|| JournalError::InvalidSegmentName(path.to_path_buf()))?;
    digits
        .parse()
        .map_err(|_| JournalError::InvalidSegmentName(path.to_path_buf()))
}

fn hash_record(sequence: u64, previous_hash: Hash32, body: &[u8]) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(HASH_DOMAIN);
    hasher.update(&sequence.to_be_bytes());
    hasher.update(previous_hash.as_bytes());
    hasher.update(body);
    Hash32::from_bytes(*hasher.finalize().as_bytes())
}

fn corrupt(path: &Path, offset: usize, reason: &str) -> JournalError {
    JournalError::Corrupt {
        path: path.to_path_buf(),
        offset: offset as u64,
        reason: reason.to_owned(),
    }
}

fn read_u16(bytes: &[u8], offset: usize, path: &Path, base: usize) -> Result<u16, JournalError> {
    let raw: [u8; 2] = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| corrupt(path, base, "short integer field"))?
        .try_into()
        .map_err(|_| corrupt(path, base, "integer field length mismatch"))?;
    Ok(u16::from_be_bytes(raw))
}

fn read_u32(bytes: &[u8], offset: usize, path: &Path, base: usize) -> Result<u32, JournalError> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| corrupt(path, base, "short integer field"))?
        .try_into()
        .map_err(|_| corrupt(path, base, "integer field length mismatch"))?;
    Ok(u32::from_be_bytes(raw))
}

fn read_u64(bytes: &[u8], offset: usize, path: &Path, base: usize) -> Result<u64, JournalError> {
    let raw: [u8; 8] = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| corrupt(path, base, "short integer field"))?
        .try_into()
        .map_err(|_| corrupt(path, base, "integer field length mismatch"))?;
    Ok(u64::from_be_bytes(raw))
}

fn read_hash(
    bytes: &[u8],
    offset: usize,
    path: &Path,
    base: usize,
) -> Result<Hash32, JournalError> {
    let raw: [u8; 32] = bytes
        .get(offset..offset + 32)
        .ok_or_else(|| corrupt(path, base, "short digest field"))?
        .try_into()
        .map_err(|_| corrupt(path, base, "digest field length mismatch"))?;
    Ok(Hash32::from_bytes(raw))
}

fn sync_directory(path: &Path) -> Result<(), JournalError> {
    let directory = File::open(path).map_err(|source| JournalError::io(path, source))?;
    directory
        .sync_all()
        .map_err(|source| JournalError::io(path, source))
}
