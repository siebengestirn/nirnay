//! Content-addressed archive for original exchange source bytes.
//!
//! Normal open fails closed on a torn manifest; repair is an explicit
//! operation that quarantines damaged bytes and writes a report. Object bytes
//! are verified against their manifest hash and reads and writes are bounded.

use std::fs::{
    File, //
    OpenOptions,
};
use std::io::{
    BufRead, //
    BufReader,
    Read,
    Write,
};
use std::path::{
    Path, //
    PathBuf,
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

const COPY_BUFFER: usize = 1024 * 1024;
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
        let mut buffer = [0_u8; COPY_BUFFER];
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

    /// Stream the manifest and verify every referenced object.
    pub fn verify(&self) -> Result<usize, ArchiveError> {
        let file = File::open(self.manifest_path())?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut count = 0_usize;
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
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
            verify_object(&self.object_path(&record.hash_hex), &record)?;
            count += 1;
        }
        Ok(count)
    }
}

fn verify_object(path: &Path, record: &SourceRecord) -> Result<(), ArchiveError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; COPY_BUFFER];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
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
    let mut buffer = [0_u8; COPY_BUFFER];
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
        let mut buffer = [0_u8; COPY_BUFFER];
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
