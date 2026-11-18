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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailPoint {
    BeforeCommit,
    AfterCommit,
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
        if digest.trim().is_empty() || output_path.as_os_str().is_empty() {
            return Err(StoreError::BadPath(
                "digest and output path are required".to_owned(),
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
    let mut buffer = [0_u8; 1024 * 1024];
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
    if directory.sync_all().is_err() {
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
