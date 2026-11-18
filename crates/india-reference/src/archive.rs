//! Content-addressed archive for original exchange source bytes.
//!
//! Normal open fails closed on a torn manifest; repair is an explicit
//! operation that quarantines damaged bytes and writes a report. Object bytes
//! are verified against their manifest hash and reads and writes are bounded.

use std::collections::{
    BTreeMap, //
    BTreeSet,
};
use std::fs::{
    File, //
    OpenOptions,
};
use std::io::{
    BufRead, //
    BufReader,
    ErrorKind,
    Read,
    Write,
};
use std::path::{
    Path, //
    PathBuf,
};
use std::time::{
    Duration, //
    Instant,
};

use fs2::FileExt;
use india_domain::is_sha256_hex;
use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};
use thiserror::Error;

/// Default bounded wait for the per-archive advisory lock.
pub const DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS: u64 = 10_000;

const COPY_BUFFER: usize = 64 * 1024;
const MAX_MANIFEST_LINE: usize = 64 * 1024;
const MAX_OBJECT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("archive I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive manifest has an incomplete final record; run explicit recovery")]
    TornManifest,
    #[error("archive object {hash} failed its content hash")]
    HashMismatch { hash: String },
    #[error("archived object exceeds the {limit}-byte limit")]
    ObjectTooLarge { limit: usize },
    #[error("archive manifest line exceeds the {limit}-byte limit")]
    ManifestLineTooLarge { limit: usize },
    #[error("archive manifest record is invalid: {0}")]
    InvalidRecord(String),
    #[error("archived source {hash} is not indexed by the manifest")]
    MissingSource { hash: String },
    #[error("archive lock was not acquired within {timeout_ms} ms")]
    LockBusy { timeout_ms: u64 },
    #[error("archive manifest has a conflicting duplicate digest {hash}")]
    ConflictingDigest { hash: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub hash_hex: String,
    pub size: u64,
    pub source_id: String,
    pub source_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub damaged_bytes: u64,
    pub complete_records: usize,
    pub quarantine: String,
}

/// A content-addressed source archive under one directory.
#[derive(Clone, Debug)]
pub struct SourceArchive {
    root: PathBuf,
}

impl SourceArchive {
    /// Create a new archive in an empty directory.
    pub fn create_new(root: impl Into<PathBuf>) -> Result<Self, ArchiveError> {
        let root = root.into();
        std::fs::create_dir(&root)?;
        std::fs::create_dir(root.join("objects"))?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("manifest.jsonl"))?;
        Ok(Self { root })
    }

    /// Open an existing archive. A torn tail is refused, not repaired.
    /// The manifest is scanned in bounded chunks without buffering the file.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ArchiveError> {
        let root = root.into();
        let path = root.join("manifest.jsonl");
        let mut file = File::open(path)?;
        let mut buffer = vec![0_u8; COPY_BUFFER];
        let mut last_byte = 0_u8;
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            last_byte = buffer[read - 1];
        }
        if last_byte != b'\n' {
            return Err(ArchiveError::TornManifest);
        }
        Ok(Self { root })
    }

    /// Open an existing archive, taking a bounded shared lock and repairing a
    /// torn manifest tail by truncating to the last complete record. Normal
    /// `open()` continues to fail closed; repair only happens here, under the
    /// exclusive lock, and is visible to every later reader.
    pub fn open_with_timeout(
        root: impl Into<PathBuf>,
        timeout_ms: u64,
    ) -> Result<Self, ArchiveError> {
        let root = root.into();
        let archive = Self { root };
        let _guard = archive.acquire_lock(true, timeout_ms)?;
        let (complete_len, total, _) = manifest_boundary(&archive.manifest_path())?;
        if total > complete_len {
            let manifest = OpenOptions::new()
                .write(true)
                .open(archive.manifest_path())?;
            manifest.set_len(complete_len)?;
            manifest.sync_all()?;
        }
        Ok(archive)
    }

    /// Open an existing archive for reads under a shared lock. A torn tail is
    /// refused; only a writable `open_with_timeout` may repair it.
    pub fn open_readonly(root: impl Into<PathBuf>, timeout_ms: u64) -> Result<Self, ArchiveError> {
        let root = root.into();
        let archive = Self { root };
        let _guard = archive.acquire_lock(false, timeout_ms)?;
        let (complete_len, total, _) = manifest_boundary(&archive.manifest_path())?;
        if total > complete_len {
            return Err(ArchiveError::TornManifest);
        }
        Ok(archive)
    }

    fn acquire_lock(&self, exclusive: bool, timeout_ms: u64) -> Result<File, ArchiveError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".lock"))?;
        let started = Instant::now();
        loop {
            let attempt = if exclusive {
                FileExt::try_lock_exclusive(&file)
            } else {
                FileExt::try_lock_shared(&file)
            };
            match attempt {
                Ok(()) => return Ok(file),
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock
                        || error.raw_os_error() == Some(11) =>
                {
                    if started.elapsed() >= Duration::from_millis(timeout_ms) {
                        return Err(ArchiveError::LockBusy { timeout_ms });
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(ArchiveError::Io(error)),
            }
        }
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.jsonl")
    }

    fn object_path(&self, hash_hex: &str) -> PathBuf {
        self.root.join("objects").join(hash_hex)
    }

    /// Archive original bytes by content hash. Identical content is not written twice.
    pub fn put_source(
        &mut self,
        bytes: &[u8],
        source_id: &str,
        source_version: &str,
    ) -> Result<SourceRecord, ArchiveError> {
        self.put_source_with_timeout(
            bytes,
            source_id,
            source_version,
            DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS,
        )
    }

    /// Archive bytes under a bounded exclusive lock so no reader can observe a
    /// half-written object or a manifest line without its object.
    pub fn put_source_with_timeout(
        &mut self,
        bytes: &[u8],
        source_id: &str,
        source_version: &str,
        timeout_ms: u64,
    ) -> Result<SourceRecord, ArchiveError> {
        let _guard = self.acquire_lock(true, timeout_ms)?;
        if bytes.len() > MAX_OBJECT_BYTES {
            return Err(ArchiveError::ObjectTooLarge {
                limit: MAX_OBJECT_BYTES,
            });
        }
        let digest = Sha256::digest(bytes);
        let hash_hex = format!("{digest:x}");
        let object_path = self.object_path(&hash_hex);
        if !object_path.exists() {
            let temporary = self.root.join(format!(".{hash_hex}.staging"));
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let mut writer = std::io::BufWriter::new(file);
            writer.write_all(bytes)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            std::fs::rename(&temporary, &object_path)?;
        }
        let record = SourceRecord {
            hash_hex: hash_hex.clone(),
            size: u64::try_from(bytes.len()).map_err(|_| ArchiveError::ObjectTooLarge {
                limit: MAX_OBJECT_BYTES,
            })?,
            source_id: source_id.to_owned(),
            source_version: source_version.to_owned(),
        };
        let line = serde_json::to_vec(&record)
            .map_err(|error| ArchiveError::InvalidRecord(error.to_string()))?;
        let mut manifest = OpenOptions::new().append(true).open(self.manifest_path())?;
        manifest.write_all(&line)?;
        manifest.write_all(b"\n")?;
        manifest.sync_all()?;
        Ok(record)
    }

    fn find_record(&self, hash_hex: &str) -> Result<Option<SourceRecord>, ArchiveError> {
        if !is_sha256_hex(hash_hex) {
            return Err(ArchiveError::InvalidRecord(
                "source lookup key is not a sha256 value".to_owned(),
            ));
        }
        let mut reader = BufReader::new(File::open(self.manifest_path())?);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = (&mut reader)
                .take(MAX_MANIFEST_LINE as u64 + 1)
                .read_until(b'\n', &mut line)?;
            if read == 0 {
                return Ok(None);
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            let record: SourceRecord = serde_json::from_slice(&line)
                .map_err(|error| ArchiveError::InvalidRecord(error.to_string()))?;
            if record.hash_hex == hash_hex {
                return Ok(Some(record));
            }
        }
    }

    /// Assert that a hash resolves to a manifest entry whose object bytes still
    /// verify. Used before a reference revision can claim source lineage.
    pub fn resolve(&self, hash_hex: &str) -> Result<(), ArchiveError> {
        self.resolve_with_timeout(hash_hex, DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS)
    }

    /// Resolve under a bounded shared lock.
    pub fn resolve_with_timeout(
        &self,
        hash_hex: &str,
        timeout_ms: u64,
    ) -> Result<(), ArchiveError> {
        let _guard = self.acquire_lock(false, timeout_ms)?;
        let record = self
            .find_record(hash_hex)?
            .ok_or_else(|| ArchiveError::MissingSource {
                hash: hash_hex.to_owned(),
            })?;
        verify_object(&self.object_path(hash_hex), &record)
    }

    /// Fetch archived bytes by content hash. An unindexed or corrupt object is
    /// never returned, even when a file happens to exist on disk.
    pub fn fetch(&self, hash_hex: &str) -> Result<Vec<u8>, ArchiveError> {
        self.fetch_with_timeout(hash_hex, DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS)
    }

    /// Fetch under a bounded shared lock.
    pub fn fetch_with_timeout(
        &self,
        hash_hex: &str,
        timeout_ms: u64,
    ) -> Result<Vec<u8>, ArchiveError> {
        // Hold the shared lock through the read so verification and byte
        // access observe the same archive state.
        let _guard = self.acquire_lock(false, timeout_ms)?;
        let record = self
            .find_record(hash_hex)?
            .ok_or_else(|| ArchiveError::MissingSource {
                hash: hash_hex.to_owned(),
            })?;
        verify_object(&self.object_path(hash_hex), &record)?;
        std::fs::read(self.object_path(hash_hex)).map_err(ArchiveError::from)
    }

    /// Stream the manifest and verify every referenced object.
    pub fn verify(&self) -> Result<usize, ArchiveError> {
        self.verify_with_timeout(DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS)
    }

    /// Verify under a bounded shared lock, rejecting a complete line that
    /// disagrees with an earlier record for the same content digest.
    pub fn verify_with_timeout(&self, timeout_ms: u64) -> Result<usize, ArchiveError> {
        let _guard = self.acquire_lock(false, timeout_ms)?;
        let file = File::open(self.manifest_path())?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut seen: BTreeMap<String, SourceRecord> = BTreeMap::new();
        let mut count = 0_usize;
        loop {
            line.clear();
            let read = (&mut reader)
                .take(MAX_MANIFEST_LINE as u64 + 1)
                .read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            if read > MAX_MANIFEST_LINE {
                return Err(ArchiveError::ManifestLineTooLarge {
                    limit: MAX_MANIFEST_LINE,
                });
            }
            if !line.ends_with(b"\n") {
                return Err(ArchiveError::TornManifest);
            }
            let record: SourceRecord = serde_json::from_slice(&line)
                .map_err(|error| ArchiveError::InvalidRecord(error.to_string()))?;
            if let Some(earlier) = seen.get(&record.hash_hex)
                && earlier != &record
            {
                return Err(ArchiveError::ConflictingDigest {
                    hash: record.hash_hex.clone(),
                });
            }
            seen.insert(record.hash_hex.clone(), record.clone());
            verify_object(&self.object_path(&record.hash_hex), &record)?;
            count += 1;
        }
        Ok(count)
    }

    /// Object files that exist on disk but have no manifest entry. These are
    /// crash residue or orphaned sources, never served by `fetch`.
    pub fn unindexed_objects(&self) -> Result<Vec<String>, ArchiveError> {
        let _guard = self.acquire_lock(false, DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS)?;
        let mut indexed = BTreeSet::new();
        let file = File::open(self.manifest_path())?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = (&mut reader)
                .take(MAX_MANIFEST_LINE as u64 + 1)
                .read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            let record: SourceRecord = serde_json::from_slice(&line)
                .map_err(|error| ArchiveError::InvalidRecord(error.to_string()))?;
            indexed.insert(record.hash_hex);
        }
        let mut orphans = Vec::new();
        for entry in std::fs::read_dir(self.root.join("objects"))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !indexed.contains(&name) {
                orphans.push(name);
            }
        }
        orphans.sort();
        Ok(orphans)
    }
}

fn verify_object(path: &Path, record: &SourceRecord) -> Result<(), ArchiveError> {
    if record.size > MAX_OBJECT_BYTES as u64 {
        return Err(ArchiveError::ObjectTooLarge {
            limit: MAX_OBJECT_BYTES,
        });
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .filter(|value| *value <= MAX_OBJECT_BYTES as u64)
            .ok_or(ArchiveError::ObjectTooLarge {
                limit: MAX_OBJECT_BYTES,
            })?;
        hasher.update(&buffer[..read]);
    }
    if total != record.size || format!("{:x}", hasher.finalize()) != record.hash_hex {
        return Err(ArchiveError::HashMismatch {
            hash: record.hash_hex.clone(),
        });
    }
    Ok(())
}

/// Explicit recovery of a torn manifest. Quarantines the damaged tail and
/// keeps every complete record. Never silently truncates.
fn manifest_boundary(path: &Path) -> Result<(u64, u64, usize), ArchiveError> {
    let mut file = File::open(path)?;
    let mut buffer = vec![0_u8; COPY_BUFFER];
    let mut offset = 0_u64;
    let mut last_newline = None;
    let mut records = 0_usize;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for (index, byte) in buffer[..read].iter().enumerate() {
            if *byte == b'\n' {
                last_newline = Some(offset + index as u64 + 1);
                records += 1;
            }
        }
        offset += read as u64;
    }
    Ok((last_newline.unwrap_or(0), offset, records))
}

/// Explicit recovery of a torn manifest. Quarantines the damaged tail and
/// keeps every complete record. Never silently truncates.
pub fn recover(root: impl AsRef<Path>) -> Result<RecoveryReport, ArchiveError> {
    let root = root.as_ref();
    let path = root.join("manifest.jsonl");
    let (complete_len, total, complete_records) = manifest_boundary(&path)?;
    let damaged_len = total.saturating_sub(complete_len);
    let mut digest = Sha256::new();
    {
        let mut source = File::open(&path)?;
        std::io::Seek::seek(&mut source, std::io::SeekFrom::Start(complete_len))?;
        let mut buffer = vec![0_u8; COPY_BUFFER];
        let mut remaining = damaged_len;
        while remaining > 0 {
            let wanted = remaining.min(buffer.len() as u64) as usize;
            let read = source.read(&mut buffer[..wanted])?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
            remaining -= read as u64;
        }
    }
    let quarantine = root.join(format!(".manifest-damaged-{:x}", digest.finalize()));
    {
        let mut source = File::open(&path)?;
        let mut destination = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&quarantine)?;
        std::io::Seek::seek(&mut source, std::io::SeekFrom::Start(complete_len))?;
        std::io::copy(&mut source, &mut destination)?;
        destination.sync_all()?;
    }
    let staged = root.join(".manifest.recovered.staging");
    {
        let source = File::open(&path)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        let mut writer = std::io::BufWriter::new(file);
        std::io::copy(&mut source.take(complete_len), &mut writer)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
    }
    std::fs::rename(&staged, &path)?;
    let directory = File::open(root)?;
    directory.sync_all()?;
    Ok(RecoveryReport {
        damaged_bytes: damaged_len,
        complete_records,
        quarantine: quarantine.to_string_lossy().into_owned(),
    })
}
