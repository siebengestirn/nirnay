//! Immutable raw-source archive.
//!
//! Hashing a file is not retaining it (reviewer integration requirement):
//! this archive persists raw source bytes write-once, keyed by content
//! (SHA-256 path), alongside an append-only manifest. Properties:
//!
//! * **Write-once / content-addressed.** The file lives at
//!   `{root}/{sha256}.src`. Storing identical bytes again returns the existing reference without another write; it
//!   returns the existing reference — never an overwrite.
//! * **Atomic publication.** Bytes are written to a process-unique
//!   temporary file, fsynced, renamed, and the containing directory fsynced
//!   before the manifest entry is published. A crash mid-store leaves
//!   either the old object (or none) — never a truncated object at the
//!   final path and never a manifest entry pointing at an unpersisted file.
//! * **Idempotent under concurrency.** The duplicate-content check and the
//!   manifest append happen under one per-root cross-process advisory lock,
//!   so concurrent stores of identical bytes from any number of processes
//!   produce exactly one object and one manifest entry.
//! * **Verification.** [`SourceArchive::verify`] recomputes the digest of a
//!   stored object and compares it against the manifest; corruption (or a
//!   replaced file) is reported, never silently accepted.
//! * **Path containment.** Manifest paths are always
//!   `{sha256}.src` under the archive root; traversal outside the root is
//!   rejected, so a hostile manifest cannot escape the archive.

use std::path::{Path, PathBuf};
use std::{fs::OpenOptions, io::Write as _};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(debug_assertions)]
use super::durable::{CrashPoint, maybe_crash};
use crate::util::sha256_hex;

/// Manifest schema version.
pub const ARCHIVE_SCHEMA_VERSION: u32 = 1;

/// Default bounded wait for the archive lock, in milliseconds.
pub const DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS: u64 = 10_000;

/// Poll interval while waiting for the archive lock, in milliseconds.
const ARCHIVE_LOCK_POLL_MS: u64 = 10;

/// A retained source object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivedSource {
    pub schema_version: u32,
    /// Stable source identity (e.g. `nse-cm-master`).
    pub source_id: String,
    /// Lowercase hex SHA-256 of the raw bytes (also the filename stem).
    pub sha256_hex: String,
    /// Retrieval time recorded at ingestion (knowledge time).
    pub retrieved_at: DateTime<Utc>,
    /// Bytes stored.
    pub bytes: u64,
}

/// One line of the archive manifest (one object per line, append-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveEntry {
    pub schema_version: u32,
    pub source_id: String,
    pub sha256_hex: String,
    pub retrieved_at: DateTime<Utc>,
    pub bytes: u64,
}

/// Outcome of a full archive verification pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveVerification {
    /// The verified manifest entries, in manifest order.
    pub entries: Vec<ArchiveEntry>,
    /// Objects present on disk with no manifest entry (crash residue).
    /// Never returned from [`SourceArchive::fetch`]; reported separately.
    pub unindexed_objects: Vec<String>,
}

/// Append-only archive of raw sources on local disk.
#[derive(Debug, Clone)]
pub struct SourceArchive {
    root: PathBuf,
    lock_timeout_ms: u64,
}

/// RAII guard for the per-root advisory lock.
pub(crate) struct LockGuard {
    file: std::fs::File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Archive busy error: the bounded wait for the lock expired.
#[derive(Debug)]
pub struct ArchiveBusy;

impl std::fmt::Display for ArchiveBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "archive lock is busy; try again")
    }
}

impl std::error::Error for ArchiveBusy {}

impl SourceArchive {
    /// Open (creating if needed) the archive at `root`.
    ///
    /// On open, the manifest is examined under the exclusive lock: if it
    /// ends mid-line, the unterminated final fragment is truncated, synced
    /// and discarded (the line was never committed).
    pub fn open(root: &Path) -> Result<Self, anyhow::Error> {
        Self::open_with_timeout(root, DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS)
    }

    /// Open the archive with a bounded lock wait.
    pub fn open_with_timeout(root: &Path, timeout_ms: u64) -> Result<Self, anyhow::Error> {
        std::fs::create_dir_all(root)?;
        let archive = Self {
            root: root.to_owned(),
            lock_timeout_ms: timeout_ms,
        };
        archive.recover_torn_tail()?;
        Ok(archive)
    }

    /// Open the archive in read-only mode.
    ///
    /// Unlike [`SourceArchive::open`], read-only storage never repairs: a
    /// manifest that ends mid-line is a verification failure, not an
    /// invitation to truncate it.
    pub fn open_readonly(root: &Path, timeout_ms: u64) -> Result<Self, anyhow::Error> {
        let archive = Self {
            root: root.to_owned(),
            lock_timeout_ms: timeout_ms,
        };
        let _guard = archive.lock_exclusive(timeout_ms)?;
        let manifest = root.join("manifest.jsonl");
        if manifest.exists() {
            let text = std::fs::read_to_string(&manifest)?;
            anyhow::ensure!(
                text.is_empty() || text.ends_with('\n'),
                "archive manifest does not end with a newline; refusing to repair read-only storage"
            );
        }
        Ok(archive)
    }

    /// The archive root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Expected path for a content hash; rejects non-hash names so callers
    /// cannot escape the root via crafted manifests.
    fn path_for(&self, sha256_hex: &str) -> Result<PathBuf, anyhow::Error> {
        anyhow::ensure!(
            sha256_hex.len() == 64 && sha256_hex.chars().all(|c| c.is_ascii_hexdigit()),
            "invalid content hash in archive path"
        );
        Ok(self.root.join(format!("{sha256_hex}.src")))
    }

    /// Wait for the cross-process lock, bounded by `timeout_ms`.
    fn lock(&self, shared: bool, timeout_ms: u64) -> Result<LockGuard, anyhow::Error> {
        let path = self.root.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let attempt = if shared {
                file.try_lock_shared()
            } else {
                file.try_lock()
            };
            match attempt {
                Ok(()) => return Ok(LockGuard { file }),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(anyhow::anyhow!(ArchiveBusy));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(ARCHIVE_LOCK_POLL_MS));
                }
                Err(std::fs::TryLockError::Error(err)) => return Err(err.into()),
            }
        }
    }

    /// Crate-visible lock for backup operations.
    pub(crate) fn lock_exclusive_for_backup(&self) -> Result<LockGuard, anyhow::Error> {
        self.lock(false, self.lock_timeout_ms)
    }

    fn lock_exclusive(&self, timeout_ms: u64) -> Result<LockGuard, anyhow::Error> {
        self.lock(false, timeout_ms)
    }

    fn lock_shared(&self) -> Result<LockGuard, anyhow::Error> {
        self.lock(true, self.lock_timeout_ms)
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.jsonl")
    }

    /// Torn-tail recovery under the exclusive lock. On open only; readers
    /// under the shared lock never repair anything.
    fn recover_torn_tail(&self) -> Result<(), anyhow::Error> {
        let _guard = self.lock_exclusive(self.lock_timeout_ms)?;
        let manifest = self.manifest_path();
        if !manifest.exists() {
            return Ok(());
        }
        let bytes = std::fs::read(&manifest)?;
        if bytes.is_empty() || bytes.ends_with(b"\n") {
            return Ok(());
        }
        let last_newline = bytes
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |index| index + 1);
        // Truncate in place — never rewrite the file with `fs::write`,
        // which would destroy the committed prefix if the rewrite itself
        // were interrupted.
        let file = OpenOptions::new().write(true).open(&manifest)?;
        file.set_len(last_newline as u64)?;
        file.sync_all()?;
        std::fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    /// Store `bytes` under `source_id`, returning its content reference.
    ///
    /// Content-addressed and idempotent: storing identical bytes again
    /// returns the *original* reference (same object, same earliest
    /// retrieval time) without rewriting the object.
    pub fn store(
        &self,
        source_id: &str,
        bytes: &[u8],
        retrieved_at: DateTime<Utc>,
    ) -> Result<ArchivedSource, anyhow::Error> {
        // The whole check-and-publish operation is one critical section:
        // looking up duplicate content outside the lock would let two
        // concurrent stores both miss and both append.
        let _guard = self.lock_exclusive(self.lock_timeout_ms)?;
        self.store_locked(source_id, bytes, retrieved_at)
    }

    fn store_locked(
        &self,
        source_id: &str,
        bytes: &[u8],
        retrieved_at: DateTime<Utc>,
    ) -> Result<ArchivedSource, anyhow::Error> {
        anyhow::ensure!(!source_id.is_empty(), "source_id must not be empty");
        let digest = sha256_hex(bytes);
        let path = self.path_for(&digest)?;
        if let Some(existing) = self
            .entries_locked()?
            .iter()
            .find(|entry| entry.sha256_hex == digest)
        {
            // At most one entry per digest; a conflicting duplicate
            // digest is corruption. An identical duplicate is not appendable
            // either — at most one entry is ever permitted.
            return Ok(ArchivedSource {
                schema_version: ARCHIVE_SCHEMA_VERSION,
                source_id: existing.source_id.clone(),
                sha256_hex: existing.sha256_hex.clone(),
                retrieved_at: existing.retrieved_at,
                bytes: existing.bytes,
            });
        }
        if path.exists() {
            // A path existing on disk is not proof of content: verify the
            // existing object's bytes against the expected SHA-256 before
            // appending a manifest entry.
            let existing_bytes = std::fs::read(&path)?;
            anyhow::ensure!(
                sha256_hex(&existing_bytes) == digest,
                "archived source {digest} failed its hash check before manifest append"
            );
            anyhow::ensure!(
                existing_bytes.len() as u64 == bytes.len() as u64,
                "archived source {digest} size mismatch before manifest append"
            );
        } else {
            // Publish atomically with durability: a process-unique temp
            // file, fsync the object, rename, then fsync the *directory*
            // so the rename itself survives a crash. Only then is the
            // object eligible for the manifest.
            let tmp = self.root.join(format!(
                "{digest}.{}.{}tmp",
                std::process::id(),
                TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            #[cfg(debug_assertions)]
            maybe_crash(CrashPoint::BeforeArchiveObjectWrite);
            {
                let mut file = OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .open(&tmp)?;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            std::fs::rename(&tmp, &path)?;
            std::fs::File::open(&self.root)?.sync_all()?;
            #[cfg(debug_assertions)]
            maybe_crash(CrashPoint::AfterArchiveObjectRename);
        }
        self.append_manifest_locked(&ArchiveEntry {
            schema_version: ARCHIVE_SCHEMA_VERSION,
            source_id: source_id.to_owned(),
            sha256_hex: digest.clone(),
            retrieved_at,
            bytes: bytes.len() as u64,
        })?;
        #[cfg(debug_assertions)]
        maybe_crash(CrashPoint::AfterArchiveManifestAppend);
        Ok(ArchivedSource {
            schema_version: ARCHIVE_SCHEMA_VERSION,
            source_id: source_id.to_owned(),
            sha256_hex: digest,
            retrieved_at,
            bytes: bytes.len() as u64,
        })
    }

    /// Append one manifest entry. Caller must hold the exclusive lock.
    fn append_manifest_locked(&self, entry: &ArchiveEntry) -> Result<(), anyhow::Error> {
        let manifest = self.manifest_path();
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&manifest)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        // Persist the manifest entry's directory entry too, so the file
        // itself survives a crash after creation.
        std::fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    /// Read all manifest entries (serialized against appends).
    pub fn entries(&self) -> Result<Vec<ArchiveEntry>, anyhow::Error> {
        let _guard = self.lock_shared()?;
        self.entries_locked()
    }

    /// Read all manifest entries. Caller must hold a lock.
    ///
    /// Every newline-terminated line must be a valid version-1 entry. A
    /// malformed complete line is corruption and fails closed; it is never
    /// skipped or guessed.
    pub(crate) fn entries_locked(&self) -> Result<Vec<ArchiveEntry>, anyhow::Error> {
        let manifest = self.manifest_path();
        if !manifest.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&manifest)?;
        anyhow::ensure!(
            text.is_empty() || text.ends_with('\n'),
            "archive manifest does not end with a newline; torn tail not recovered"
        );
        // Parse and validate each newline-terminated line. Store the raw
        // lines so digest comparison borrows no local.
        let parsed: Vec<ArchiveEntry> = text
            .lines()
            .enumerate()
            .map(|(index, line)| -> Result<ArchiveEntry, anyhow::Error> {
                anyhow::ensure!(
                    !line.trim().is_empty(),
                    "archive manifest line {} is blank",
                    index + 1
                );
                let entry: ArchiveEntry = serde_json::from_str(line).map_err(|err| {
                    anyhow::anyhow!("archive manifest line {} is corrupt: {err}", index + 1)
                })?;
                anyhow::ensure!(
                    entry.schema_version == ARCHIVE_SCHEMA_VERSION,
                    "archive manifest line {} has unsupported schema_version {}",
                    index + 1,
                    entry.schema_version
                );
                anyhow::ensure!(
                    entry
                        .source_id
                        .as_bytes()
                        .iter()
                        .any(|b| !b.is_ascii_whitespace()),
                    "archive manifest line {} has an empty source_id",
                    index + 1
                );
                anyhow::ensure!(
                    entry.sha256_hex.len() == 64
                        && entry.sha256_hex.chars().all(|c| c.is_ascii_hexdigit()),
                    "archive manifest line {} has an invalid sha256_hex",
                    index + 1
                );
                Ok(entry)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut by_digest: std::collections::BTreeMap<String, &ArchiveEntry> =
            std::collections::BTreeMap::new();
        for entry in &parsed {
            if let Some(existing) = by_digest.get(&entry.sha256_hex) {
                anyhow::ensure!(
                    existing.source_id == entry.source_id
                        && existing.retrieved_at == entry.retrieved_at
                        && existing.bytes == entry.bytes,
                    "archive manifest contains a conflicting duplicate entry for {}",
                    entry.sha256_hex
                );
                anyhow::bail!(
                    "archive manifest contains a duplicate entry for {}",
                    entry.sha256_hex
                );
            }
            by_digest.insert(entry.sha256_hex.clone(), entry);
        }
        Ok(parsed)
    }

    /// Retrieve and hash-check an object by content digest. Refuses when
    /// the object, its manifest entry, or either hash check fails
    /// (corruption). An unindexed object is never committed source data.
    pub fn fetch(&self, content_hash: &str) -> Result<Vec<u8>, anyhow::Error> {
        let _guard = self.lock_shared()?;
        let path = self.path_for(content_hash)?;
        let entries = self.entries_locked()?;
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.sha256_hex == content_hash)
        {
            let bytes = std::fs::read(&path)?;
            anyhow::ensure!(
                sha256_hex(&bytes) == content_hash,
                "archived source {content_hash} failed its hash check"
            );
            anyhow::ensure!(
                bytes.len() as u64 == entry.bytes,
                "archived source {content_hash} size mismatch"
            );
            Ok(bytes)
        } else {
            anyhow::bail!(
                "archived source {content_hash} has no manifest entry; unindexed objects are not committed source data"
            )
        }
    }

    /// Verify every manifest entry: object present, hash-correct and size
    /// matching. Returns the verified entries; any failure is an error
    /// (never a warning).
    pub fn verify(&self) -> Result<Vec<ArchiveEntry>, anyhow::Error> {
        Ok(self.verify_full()?.entries)
    }

    /// Full verification, reporting unindexed objects separately.
    pub fn verify_full(&self) -> Result<ArchiveVerification, anyhow::Error> {
        let _guard = self.lock_shared()?;
        let entries = self.entries_locked()?;
        let mut by_digest = std::collections::BTreeMap::new();
        for entry in &entries {
            let path = self.path_for(&entry.sha256_hex)?;
            anyhow::ensure!(
                path.exists(),
                "archived source {} is missing",
                entry.sha256_hex
            );
            let bytes = std::fs::read(&path)?;
            anyhow::ensure!(
                sha256_hex(&bytes) == entry.sha256_hex,
                "archived source {} failed its hash check",
                entry.sha256_hex
            );
            anyhow::ensure!(
                bytes.len() as u64 == entry.bytes,
                "archived source {} size mismatch",
                entry.sha256_hex
            );
            by_digest.insert(entry.sha256_hex.clone(), ());
        }
        // Objects present on disk with no manifest entry are crash
        // residue: report separately, never return from fetch().
        let mut unindexed = Vec::new();
        for item in std::fs::read_dir(&self.root)? {
            let name = item?.file_name().into_string().unwrap_or_default();
            if let Some(stem) = name.strip_suffix(".src")
                && stem.len() == 64
                && stem.chars().all(|c| c.is_ascii_hexdigit())
                && !by_digest.contains_key(stem)
            {
                unindexed.push(stem.to_owned());
            }
        }
        unindexed.sort();
        Ok(ArchiveVerification {
            entries,
            unindexed_objects: unindexed,
        })
    }

    /// Verify a single hash resolves to retained, hash-correct bytes.
    pub fn resolve(&self, sha256_hex: &str) -> Result<(), anyhow::Error> {
        self.fetch(sha256_hex).map(|_| ())?;
        Ok(())
    }
}

/// Process-unique counter for temp-object filenames.
static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
