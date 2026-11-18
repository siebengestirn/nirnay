//! Durable reference identity allocation and immutable revision publication.
//!
//! Identity and aliases are persisted in the same transaction. SQLite uses
//! WAL, `synchronous=FULL`, foreign keys and a bounded busy timeout. Revision
//! publication enforces an expected parent and an idempotency key.

use std::path::{
    Path, //
    PathBuf,
};

use rusqlite::{
    Connection, //
    OptionalExtension,
    params,
};
use thiserror::Error;
use trading_core::{InstrumentId, Isin, Venue};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("reference store database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
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
static FAIL_POINT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[cfg(debug_assertions)]
fn inject(point: FailPoint) -> Result<(), StoreError> {
    let code = match point {
        FailPoint::BeforeCommit => 1,
        FailPoint::AfterCommit => 2,
    };
    if FAIL_POINT.load(std::sync::atomic::Ordering::SeqCst) == code {
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
    FAIL_POINT.store(code, std::sync::atomic::Ordering::SeqCst);
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
             );",
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
        let existing: Option<(i64, String)> = self
            .connection
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
        let head: Option<String> = self
            .connection
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
        let transaction = self.connection.transaction()?;
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
        inject(FailPoint::BeforeCommit)?;
        transaction.commit()?;
        inject(FailPoint::AfterCommit)?;
        let sequence = self.connection.last_insert_rowid();
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

    /// Path of the database, for backup and verification.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Back up the database with SQLite `VACUUM INTO` into a fresh directory.
    pub fn backup(&self, destination: &Path) -> Result<Vec<PathBuf>, StoreError> {
        if destination.is_symlink() {
            return Err(StoreError::BadPath(
                "backup destination is a symlink".to_owned(),
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
        Ok(vec![database])
    }
}
