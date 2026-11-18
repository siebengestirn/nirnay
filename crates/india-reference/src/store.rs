//! Durable reference identity allocation and immutable revision publication.
//!
//! Identity and aliases are persisted in the same transaction. SQLite uses
//! WAL, `synchronous=FULL`, foreign keys and a bounded busy timeout. Revision
//! publication enforces an expected parent and an idempotency key.

use std::fs::File;
use std::io::{
    Read, //
};
use std::path::{
    Path, //
    PathBuf,
};

use rusqlite::{
    Connection, //
    OptionalExtension,
    TransactionBehavior,
    params,
};
use sha2::{
    Digest, //
    Sha256,
};
use thiserror::Error;
use trading_core::{
    InstrumentId, //
    Isin,
    Venue,
};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("reference store database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("reference store I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("expected parent {expected} but store head is {actual:?}")]
    ParentMismatch {
        expected: String,
        actual: Option<String>,
    },
    #[error("alias {alias} is already assigned to another instrument")]
    AliasConflict { alias: String },
    #[error("backup or restore path is not a regular directory: {0}")]
    BadPath(String),
    #[error("reference store corruption: {0}")]
    Corruption(String),
    #[error("unsupported reference store schema version {actual}; expected {expected}")]
    UnsupportedStoreSchema { actual: u32, expected: u32 },
    #[error(transparent)]
    Snapshot(#[from] crate::snapshot::SnapshotError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailPoint {
    BeforeCommit,
    AfterCommit,
    RestoreSync,
}

#[cfg(debug_assertions)]
std::thread_local! {
    static FAIL_POINT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[cfg(debug_assertions)]
fn inject(point: FailPoint) -> Result<(), StoreError> {
    let code = match point {
        FailPoint::BeforeCommit => 1,
        FailPoint::AfterCommit => 2,
        FailPoint::RestoreSync => 3,
    };
    if FAIL_POINT.with(|cell| cell.get()) == code {
        return Err(StoreError::BadPath(format!(
            "injected failure at {point:?}"
        )));
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn inject(_point: FailPoint) -> Result<(), StoreError> {
    Ok(())
}

#[cfg(debug_assertions)]
pub fn set_fail_point(point: Option<&str>) {
    let code = match point {
        Some("before_commit") => 1,
        Some("after_commit") => 2,
        Some("restore_sync") => 3,
        _ => 0,
    };
    FAIL_POINT.with(|cell| cell.set(code));
}

/// One publication result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publication {
    pub sequence: i64,
    pub deduplicated: bool,
}

/// A durable local reference store.
pub struct ReferenceStore {
    connection: Connection,
    path: PathBuf,
}

impl ReferenceStore {
    /// Open or create a store with the required durability settings.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS instruments (
                instrument_id TEXT PRIMARY KEY
             );
             CREATE TABLE IF NOT EXISTS aliases (
                alias TEXT PRIMARY KEY,
                instrument_id TEXT NOT NULL REFERENCES instruments(instrument_id)
             );
             CREATE TABLE IF NOT EXISTS identity_counter (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                next_value INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO identity_counter (id, next_value) VALUES (1, 0);
             CREATE TABLE IF NOT EXISTS revisions (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                parent TEXT,
                digest TEXT NOT NULL,
                output_path TEXT NOT NULL,
                idempotency_key TEXT NOT NULL UNIQUE
             );
             CREATE TRIGGER IF NOT EXISTS revisions_immutable_update
                BEFORE UPDATE ON revisions
                BEGIN SELECT RAISE(ABORT, 'revisions are immutable'); END;
             CREATE TRIGGER IF NOT EXISTS revisions_immutable_delete
                BEFORE DELETE ON revisions
                BEGIN SELECT RAISE(ABORT, 'revisions are immutable'); END;",
        )?;
        Ok(Self { connection, path })
    }

    /// Resolve or allocate a stable opaque identity in one transaction.
    pub fn resolve_identity(
        &mut self,
        isin: &Isin,
        venue: Venue,
        symbol: &str,
        prefix: &str,
    ) -> Result<InstrumentId, StoreError> {
        self.resolve_identity_with_provider(isin, venue, symbol, prefix, None)
    }

    /// Resolve an identity including an optional provider-specific identifier.
    /// Provider identities are aliases with the same uniqueness guarantee.
    pub fn resolve_identity_with_provider(
        &mut self,
        isin: &Isin,
        venue: Venue,
        symbol: &str,
        prefix: &str,
        provider_id: Option<&str>,
    ) -> Result<InstrumentId, StoreError> {
        if prefix.trim().is_empty() {
            return Err(StoreError::BadPath("empty identity prefix".to_owned()));
        }
        let isin_alias = format!("ISIN:{}", isin.as_str());
        let symbol_alias = format!("{venue:?}:{symbol}");
        let transaction = self.connection.transaction()?;
        let by_isin: Option<String> = transaction
            .query_row(
                "SELECT instrument_id FROM aliases WHERE alias = ?1",
                params![isin_alias],
                |row| row.get(0),
            )
            .optional()?;
        let by_symbol: Option<String> = transaction
            .query_row(
                "SELECT instrument_id FROM aliases WHERE alias = ?1",
                params![symbol_alias],
                |row| row.get(0),
            )
            .optional()?;
        let id_text = match (by_isin, by_symbol) {
            (Some(a), Some(b)) if a == b => a,
            (Some(_), Some(_)) => {
                return Err(StoreError::AliasConflict {
                    alias: symbol_alias,
                });
            }
            (Some(id), None) => id,
            (None, Some(id)) => {
                let existing_isin: Option<String> = transaction
                    .query_row(
                        "SELECT alias FROM aliases
                         WHERE instrument_id = ?1 AND alias LIKE 'ISIN:%' LIMIT 1",
                        params![id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(other) = existing_isin
                    && other != isin_alias
                {
                    return Err(StoreError::AliasConflict {
                        alias: symbol_alias,
                    });
                }
                id
            }
            (None, None) => {
                let sequence: i64 = transaction.query_row(
                    "SELECT next_value FROM identity_counter WHERE id = 1",
                    [],
                    |row| row.get(0),
                )?;
                let id_text = format!("{prefix}{sequence:08x}");
                transaction.execute(
                    "UPDATE identity_counter SET next_value = ?1 WHERE id = 1",
                    params![sequence + 1],
                )?;
                transaction.execute(
                    "INSERT INTO instruments (instrument_id) VALUES (?1)",
                    params![id_text],
                )?;
                id_text
            }
        };
        let instrument_id =
            InstrumentId::new(&id_text).map_err(|error| StoreError::BadPath(error.to_string()))?;
        if let Some(provider) = provider_id {
            if provider.trim().is_empty() {
                return Err(StoreError::AliasConflict {
                    alias: "empty provider id".to_owned(),
                });
            }
            let provider_alias = format!("PROVIDER:{provider}");
            let owner: Option<String> = transaction
                .query_row(
                    "SELECT instrument_id FROM aliases WHERE alias = ?1",
                    params![provider_alias],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(owner) = owner
                && owner != id_text
            {
                return Err(StoreError::AliasConflict {
                    alias: provider_alias,
                });
            }
            transaction.execute(
                "INSERT OR IGNORE INTO aliases (alias, instrument_id) VALUES (?1, ?2)",
                params![provider_alias, id_text],
            )?;
        }
        for alias in [isin_alias, symbol_alias] {
            match transaction.execute(
                "INSERT OR IGNORE INTO aliases (alias, instrument_id) VALUES (?1, ?2)",
                params![alias, id_text],
            ) {
                Ok(_) => {}
                Err(error) => return Err(StoreError::Sqlite(error)),
            }
        }
        inject(FailPoint::BeforeCommit)?;
        transaction.commit()?;
        inject(FailPoint::AfterCommit)?;
        Ok(instrument_id)
    }

    /// Publish an immutable revision with an expected parent and idempotency key.
    pub fn publish_revision(
        &mut self,
        parent: Option<&str>,
        digest: &str,
        output_path: &Path,
        idempotency_key: &str,
    ) -> Result<Publication, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(i64, String)> = transaction
            .query_row(
                "SELECT sequence, digest FROM revisions WHERE idempotency_key = ?1",
                params![idempotency_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((sequence, stored_digest)) = existing {
            if stored_digest != digest {
                return Err(StoreError::BadPath(
                    "idempotency key reused with a different digest".to_owned(),
                ));
            }
            return Ok(Publication {
                sequence,
                deduplicated: true,
            });
        }
        let head: Option<String> = transaction
            .query_row(
                "SELECT digest FROM revisions ORDER BY sequence DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if head.as_deref() != parent {
            return Err(StoreError::ParentMismatch {
                expected: parent.unwrap_or("none").to_owned(),
                actual: head,
            });
        }
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(StoreError::BadPath(
                "digest must be 64 lowercase hex characters".to_owned(),
            ));
        }
        if output_path.as_os_str().is_empty()
            || output_path.is_absolute()
            || output_path.components().count() != 1
        {
            return Err(StoreError::BadPath(
                "output path must be a relative file name".to_owned(),
            ));
        }
        transaction.execute(
            "INSERT INTO revisions (parent, digest, output_path, idempotency_key)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                parent,
                digest,
                output_path.to_string_lossy(),
                idempotency_key
            ],
        )?;
        let sequence = transaction.last_insert_rowid();
        inject(FailPoint::BeforeCommit)?;
        transaction.commit()?;
        inject(FailPoint::AfterCommit)?;
        Ok(Publication {
            sequence,
            deduplicated: false,
        })
    }

    /// Current revision head digest.
    pub fn head(&self) -> Result<Option<String>, StoreError> {
        self.connection
            .query_row(
                "SELECT digest FROM revisions ORDER BY sequence DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::Sqlite)
    }

    #[cfg(debug_assertions)]
    pub fn connection_for_test(&mut self) -> &mut Connection {
        &mut self.connection
    }

    /// Path of the database, for backup and verification.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Back up the database and source archive as one verified directory.
    /// Every regular file is hashed incrementally; symlinks are refused.
    pub fn backup_with_source(
        &self,
        destination: &Path,
        source_root: &Path,
    ) -> Result<Vec<PathBuf>, StoreError> {
        if destination.is_symlink() || source_root.is_symlink() {
            return Err(StoreError::BadPath(
                "backup inputs contain a symlink".to_owned(),
            ));
        }
        std::fs::create_dir(destination).map_err(|_| {
            StoreError::BadPath(
                "backup destination already exists or is not a directory".to_owned(),
            )
        })?;
        let database = destination.join("reference.sqlite3");
        self.connection.execute(
            "VACUUM INTO ?1",
            params![database.to_string_lossy().into_owned()],
        )?;
        let archive = destination.join("source-archive");
        std::fs::create_dir(&archive)?;
        copy_tree(source_root, &archive)?;
        let mut files = collect_files(destination)?;
        files.sort();
        let mut sums = String::new();
        for file in &files {
            let digest = hash_file(file)?;
            let relative = file.strip_prefix(destination).map_err(|_| {
                StoreError::BadPath("backup file is outside the destination".to_owned())
            })?;
            sums.push_str(&format!("{}  {}\n", digest, relative.to_string_lossy()));
        }
        let sums_path = destination.join("SHA256SUMS");
        std::fs::write(&sums_path, sums)?;
        files.push(sums_path);
        Ok(files)
    }
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let mut files = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(StoreError::BadPath(format!(
                    "symlink in backup tree: {}",
                    path.display()
                )));
            }
            if kind.is_dir() {
                directories.push(path);
            } else if kind.is_file() {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn hash_file(path: &Path) -> Result<String, StoreError> {
    use sha2::{
        Digest, //
        Sha256,
    };
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), StoreError> {
    let mut directories = vec![source.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(StoreError::BadPath(format!(
                    "symlink in source tree: {}",
                    entry.path().display()
                )));
            }
            let relative = entry
                .path()
                .strip_prefix(source)
                .map_err(|_| {
                    StoreError::BadPath("source entry is outside the source tree".to_owned())
                })?
                .to_path_buf();
            let target = destination.join(relative);
            if kind.is_dir() {
                std::fs::create_dir_all(&target)?;
                directories.push(entry.path());
            } else if kind.is_file() {
                std::fs::copy(entry.path(), &target)?;
            }
        }
    }
    Ok(())
}

/// Verify a backup and restore it into a fresh directory. A sync failure after
/// the rename is reported as an uncertain outcome rather than a clean rollback.
pub fn restore_verified(backup: &Path, target: &Path) -> Result<RestoreOutcome, StoreError> {
    if backup.is_symlink() || target.is_symlink() {
        return Err(StoreError::BadPath(
            "restore inputs contain a symlink".to_owned(),
        ));
    }
    let sums = std::fs::read_to_string(backup.join("SHA256SUMS"))?;
    for line in sums.lines() {
        let (expected, relative) = line
            .split_once("  ")
            .ok_or_else(|| StoreError::BadPath("malformed backup checksum line".to_owned()))?;
        let actual = hash_file(&backup.join(relative))?;
        if actual != expected {
            return Err(StoreError::BadPath(format!(
                "backup file {relative} failed its checksum"
            )));
        }
    }
    std::fs::create_dir(target).map_err(|_| {
        StoreError::BadPath("restore target already exists or is not a directory".to_owned())
    })?;
    copy_tree(backup, target)?;
    let staging = target
        .parent()
        .ok_or_else(|| StoreError::BadPath("restore target has no parent directory".to_owned()))?;
    let directory = File::open(staging)?;
    if inject(FailPoint::RestoreSync).is_err() || directory.sync_all().is_err() {
        return Ok(RestoreOutcome::Uncertain {
            message: "parent directory sync failed after restore; target may have changed"
                .to_owned(),
        });
    }
    Ok(RestoreOutcome::Verified)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    Verified,
    Uncertain { message: String },
}

// ---------------------------------------------------------------------------
// Promoted P1-04 reference-store contract (RSV-01/02/03 foundation).
// ---------------------------------------------------------------------------

/// Schema version of the contract-shaped reference tables.
pub const STORE_SCHEMA_VERSION: u32 = 1;

type RawReplayRow = (i64, Option<i64>, String, String, Vec<u8>);

type ReplayRow = (u64, Option<u64>, String, String, Vec<u8>);

type RawContractRevision = (
    i64,
    Option<i64>,
    String,
    String,
    String,
    String,
    String,
    Vec<u8>,
);

type ContractRevision = (
    u64,
    Option<u64>,
    String,
    String,
    String,
    String,
    String,
    Vec<u8>,
);

const REFERENCE_DDL: &str = concat!(
    "CREATE TABLE IF NOT EXISTS reference_revision (",
    "revision_id INTEGER PRIMARY KEY CHECK (revision_id >= 0), ",
    "parent_revision_id INTEGER UNIQUE REFERENCES reference_revision(revision_id), ",
    "payload_schema_version INTEGER NOT NULL CHECK (payload_schema_version = 1), ",
    "valid_at TEXT NOT NULL CHECK (length(valid_at) = 10), ",
    "recorded_at TEXT NOT NULL, ",
    "snapshot_hash TEXT NOT NULL CHECK (length(snapshot_hash) = 64 AND snapshot_hash NOT GLOB '*[^0-9a-f]*'), ",
    "payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64 AND payload_sha256 NOT GLOB '*[^0-9a-f]*'), ",
    "publication_key TEXT NOT NULL UNIQUE CHECK (length(publication_key) = 64 AND publication_key NOT GLOB '*[^0-9a-f]*'), ",
    "payload BLOB NOT NULL, ",
    "CHECK ((revision_id = 0 AND parent_revision_id IS NULL) ",
    "OR (revision_id > 0 AND parent_revision_id = revision_id - 1))); ",
    "CREATE TABLE IF NOT EXISTS reference_head (",
    "singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1), ",
    "revision_id INTEGER NOT NULL REFERENCES reference_revision(revision_id)); ",
    "CREATE TRIGGER IF NOT EXISTS reference_revision_no_update ",
    "BEFORE UPDATE ON reference_revision ",
    "BEGIN SELECT RAISE(ABORT, 'reference revisions are immutable'); END; ",
    "CREATE TRIGGER IF NOT EXISTS reference_revision_no_delete ",
    "BEFORE DELETE ON reference_revision ",
    "BEGIN SELECT RAISE(ABORT, 'reference revisions are immutable'); END; ",
    "CREATE TRIGGER IF NOT EXISTS reference_head_no_delete ",
    "BEFORE DELETE ON reference_head ",
    "BEGIN SELECT RAISE(ABORT, 'the reference head cannot be deleted'); END; ",
);

/// One immutable, fully verified stored revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRevision {
    pub revision_id: u64,
    pub parent_revision_id: Option<u64>,
    pub valid_at: String,
    pub recorded_at: String,
    pub snapshot_hash: String,
    pub payload_sha256: String,
    pub publication_key: String,
    pub snapshot: crate::snapshot::ReferenceSnapshot,
}

/// Result of publishing a snapshot under an expected parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishOutcome {
    pub revision_id: u64,
    pub replayed: bool,
}

/// Result of verifying every revision, the head and the parent chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    pub revision_count: u64,
    pub head_revision_id: Option<u64>,
}

/// A durable store for versioned reference snapshots. This is additive: the
/// original opaque-identity ledger and its API remain unchanged.
pub struct DurableReferenceStore {
    connection: Connection,
    path: PathBuf,
}

fn contract_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut text, format_args!("{byte:02x}"));
    }
    text
}

fn read_head(connection: &Connection) -> Result<Option<u64>, StoreError> {
    let value: Option<i64> = connection
        .query_row(
            "SELECT revision_id FROM reference_head WHERE singleton_id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    value.map(read_revision_id_from_value).transpose()
}

fn read_revision_id_from_value(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Corruption("negative revision id".to_owned()))
}

fn to_db_id(id: u64) -> Result<i64, StoreError> {
    i64::try_from(id)
        .map_err(|_| StoreError::Corruption("revision id exceeds SQLite INTEGER range".to_owned()))
}

fn publication_key(
    expected_parent: Option<u64>,
    payload_sha256: &str,
) -> Result<String, StoreError> {
    let parent =
        expected_parent.map_or_else(|| serde_json::Value::Null, |value| serde_json::json!(value));
    let bytes = serde_json::to_vec(&serde_json::json!([
        "p1-04-publication-v1",
        parent,
        crate::snapshot::PAYLOAD_SCHEMA_VERSION,
        payload_sha256
    ]))
    .map_err(|error| StoreError::Corruption(format!("publication key encoding failed: {error}")))?;
    Ok(contract_hex(&bytes))
}

impl DurableReferenceStore {
    /// Open or create a contract-shaped store at `<root>/reference.sqlite3`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref();
        std::fs::create_dir_all(root)?;
        let path = root.join("reference.sqlite3");
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )?;
        let journal: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
        let foreign_keys: i64 =
            connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        let busy_timeout: i64 =
            connection.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
        if journal != "wal" || synchronous != 2 || foreign_keys != 1 || busy_timeout <= 0 {
            return Err(StoreError::Corruption(
                "reference store durability pragmas were not applied".to_owned(),
            ));
        }
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                connection.execute_batch(REFERENCE_DDL)?;
                connection.execute_batch("PRAGMA user_version = 1;")?;
            }
            STORE_SCHEMA_VERSION => {}
            other => {
                return Err(StoreError::UnsupportedStoreSchema {
                    actual: other,
                    expected: STORE_SCHEMA_VERSION,
                });
            }
        }
        let after: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if after != STORE_SCHEMA_VERSION {
            return Err(StoreError::Corruption(
                "reference store schema version could not be established".to_owned(),
            ));
        }
        Ok(Self { connection, path })
    }

    /// Publish a snapshot. An exact key replay is idempotent; a stale expected
    /// parent is rejected before any row is written.
    pub fn publish(
        &mut self,
        snapshot: &crate::snapshot::ReferenceSnapshot,
        expected_parent: Option<u64>,
    ) -> Result<PublishOutcome, StoreError> {
        snapshot.validate()?;
        let payload = crate::snapshot::encode_payload(snapshot)?;
        let payload_sha256 = contract_hex(&payload);
        let snapshot_hash = snapshot.snapshot_hash()?;
        let key = publication_key(expected_parent, &payload_sha256)?;
        let valid_at = snapshot.valid_at.to_string();
        let recorded_at = snapshot.recorded_at.to_string();

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // An exact key replay is resolved before the parent check: a retry of
        // an acknowledged delivery must not be turned into a stale-parent
        // error by its own earlier success.
        let raw: Option<RawReplayRow> = transaction
            .query_row(
                "SELECT revision_id, parent_revision_id, snapshot_hash, payload_sha256, payload FROM reference_revision WHERE publication_key = ?1",
                params![key],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let existing: Option<ReplayRow> = raw
            .map(|(id, parent, hash, checksum, bytes)| {
                Ok::<_, StoreError>((
                    read_revision_id_from_value(id)?,
                    match parent {
                        Some(value) => Some(read_revision_id_from_value(value)?),
                        None => None,
                    },
                    hash,
                    checksum,
                    bytes,
                ))
            })
            .transpose()?;
        if let Some((revision_id, stored_parent, stored_hash, stored_checksum, stored_bytes)) =
            existing
        {
            // Replay is only safe when the committed row still matches every
            // dimension of the key, including its parent and exact bytes.
            if stored_parent != expected_parent
                || stored_hash != snapshot_hash
                || stored_checksum != payload_sha256
                || contract_hex(&stored_bytes) != stored_checksum
            {
                return Err(StoreError::Corruption(
                    "publication key was reused with a different parent or snapshot content"
                        .to_owned(),
                ));
            }
            transaction.commit()?;
            return Ok(PublishOutcome {
                revision_id,
                replayed: true,
            });
        }
        let head: Option<u64> = read_head(&transaction)?;
        if head != expected_parent {
            return Err(StoreError::ParentMismatch {
                expected: format!("{expected_parent:?}"),
                actual: head.map(|value| value.to_string()),
            });
        }
        let revision_id = head.map_or(0, |value| value + 1);
        transaction.execute(
            "INSERT INTO reference_revision (revision_id, parent_revision_id, payload_schema_version, valid_at, recorded_at, snapshot_hash, payload_sha256, publication_key, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                to_db_id(revision_id)?,
                head.map(to_db_id).transpose()?,
                STORE_SCHEMA_VERSION,
                valid_at,
                recorded_at,
                snapshot_hash,
                payload_sha256,
                key,
                payload
            ],
        )?;
        if head.is_none() {
            transaction.execute(
                "INSERT INTO reference_head (singleton_id, revision_id) VALUES (1, ?1)",
                params![to_db_id(revision_id)?],
            )?;
        } else {
            transaction.execute(
                "UPDATE reference_head SET revision_id = ?1 WHERE singleton_id = 1",
                params![to_db_id(revision_id)?],
            )?;
        }
        transaction.commit()?;
        Ok(PublishOutcome {
            revision_id,
            replayed: false,
        })
    }

    /// The committed head revision id, if any.
    pub fn head(&self) -> Result<Option<u64>, StoreError> {
        read_head(&self.connection)
    }

    fn decode_row(raw: ContractRevision) -> Result<StoredRevision, StoreError> {
        let (
            revision_id,
            parent_revision_id,
            valid_at,
            recorded_at,
            snapshot_hash,
            payload_sha256,
            publication_key,
            payload,
        ) = raw;
        let recomputed_payload = contract_hex(&payload);
        if recomputed_payload != payload_sha256 {
            return Err(StoreError::Corruption(format!(
                "revision {revision_id} payload checksum mismatch"
            )));
        }
        let snapshot = crate::snapshot::decode_payload(&payload)?;
        let recomputed_snapshot = snapshot.snapshot_hash()?;
        if recomputed_snapshot != snapshot_hash {
            return Err(StoreError::Corruption(format!(
                "revision {revision_id} semantic hash mismatch"
            )));
        }
        if snapshot.valid_at.to_string() != valid_at
            || snapshot.recorded_at.to_string() != recorded_at
        {
            return Err(StoreError::Corruption(format!(
                "revision {revision_id} metadata does not match its payload"
            )));
        }
        Ok(StoredRevision {
            revision_id,
            parent_revision_id,
            valid_at,
            recorded_at,
            snapshot_hash,
            payload_sha256,
            publication_key,
            snapshot,
        })
    }

    /// Decode and fully verify one revision. Never returns a partial snapshot.
    pub fn revision(
        &self,
        id: u64,
    ) -> Result<Option<crate::snapshot::ReferenceSnapshot>, StoreError> {
        Ok(self.revision_with_meta(id)?.map(|row| row.snapshot))
    }

    /// Decode and fully verify one revision with its metadata.
    pub fn revision_with_meta(&self, id: u64) -> Result<Option<StoredRevision>, StoreError> {
        let raw: Option<RawContractRevision> = self
            .connection
            .query_row(
                "SELECT revision_id, parent_revision_id, valid_at, recorded_at, snapshot_hash, payload_sha256, publication_key, payload FROM reference_revision WHERE revision_id = ?1",
                params![to_db_id(id)?],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((revision_id, parent, valid_at, recorded_at, hash, checksum, key, payload)) = raw
        else {
            return Ok(None);
        };
        let converted = (
            read_revision_id_from_value(revision_id)?,
            match parent {
                Some(value) => Some(read_revision_id_from_value(value)?),
                None => None,
            },
            valid_at,
            recorded_at,
            hash,
            checksum,
            key,
            payload,
        );
        Self::decode_row(converted).map(Some)
    }

    /// Verify counts, contiguous ids, parent links, head and every payload.
    pub fn verify_full(&self) -> Result<VerificationReport, StoreError> {
        let revision_count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM reference_revision", [], |row| {
                    row.get(0)
                })?;
        let head_revision_id: Option<u64> = self.head()?;
        if revision_count == 0 && head_revision_id.is_some() {
            return Err(StoreError::Corruption(
                "head exists without any revision".to_owned(),
            ));
        }
        if revision_count > 0 && head_revision_id != Some((revision_count - 1) as u64) {
            return Err(StoreError::Corruption(
                "head is not the last contiguous revision".to_owned(),
            ));
        }
        for id in 0..revision_count {
            let row = self
                .revision_with_meta(id as u64)?
                .ok_or_else(|| StoreError::Corruption(format!("missing revision {id}")))?;
            let expected_parent = if id == 0 { None } else { Some(id as u64 - 1) };
            if row.parent_revision_id != expected_parent {
                return Err(StoreError::Corruption(format!(
                    "revision {id} has an unexpected parent"
                )));
            }
        }
        Ok(VerificationReport {
            revision_count: revision_count as u64,
            head_revision_id,
        })
    }
}

/// Test-only handle to the underlying connection, used for immutability and
/// corruption negative controls. Not available in release builds.
#[cfg(debug_assertions)]
#[must_use]
pub fn durable_connection_for_test(store: &DurableReferenceStore) -> &Connection {
    &store.connection
}

/// The database path used by a contract-shaped store.
#[must_use]
pub fn durable_store_path(store: &DurableReferenceStore) -> &Path {
    &store.path
}
