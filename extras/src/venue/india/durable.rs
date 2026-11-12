//! Durable on-disk reference store (task P1-04).
//!
//! `DurableReferenceStore` persists immutable [`ReferenceSnapshot`] revisions
//! under one configurable application data directory outside the Git working
//! tree:
//!
//! ```text
//! <root>/
//!   reference.sqlite3           revision catalogue, immutable payloads, head
//!   source-archive/             content-addressed raw source bytes + manifest
//!   backups/                    verified database-plus-archive copies
//! ```
//!
//! Durable rules (see `engineering/reference-store-contract.json`, DEC-011):
//!
//! * Embedded SQLite only; `journal_mode=WAL`, `synchronous=FULL`,
//!   `foreign_keys=ON`, finite busy timeout; all writes use `BEGIN IMMEDIATE`.
//! * Immutable append-only revisions (triggers abort `UPDATE`/`DELETE`),
//!   expected-parent publication, publication-key resolution after uncertain
//!   commits, zero-based contiguous ids.
//! * The semantic [`snapshot_hash`] meaning is unchanged; the payload
//!   integrity checksum (`payload_sha256`) is a separate storage-integrity
//!   checksum covering the exact stored bytes.
//! * Accumulated history is queried from the head snapshot only.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::util::sha256_hex;

use super::archive::SourceArchive;
use super::resolution::ListingStateRecord;
use super::store::{ReferenceSnapshot, latest_states_at};

/// Durable store schema version (`PRAGMA user_version`).
pub const STORE_SCHEMA_VERSION: u32 = 1;
/// Versioned payload envelope schema version.
pub const PAYLOAD_SCHEMA_VERSION: u32 = 1;
/// Default busy timeout for SQLite, in milliseconds (finite and bounded).
pub const DEFAULT_BUSY_TIMEOUT_MS: u64 = 5_000;

const PUBLICATION_KEY_MARKER: &str = "p1-04-publication-v1";

/// Crash-injection points, used by the chaos harness only. Zero effect on the
/// production path beyond an `Option` comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashPoint {
    /// Crash before the archive object write (before archive publication).
    BeforeArchiveObjectWrite,
    /// Crash after the archive object rename + directory sync, before the
    /// manifest append.
    AfterArchiveObjectRename,
    /// Crash after the manifest append + sync (archive fully committed),
    /// before the revision transaction begins.
    AfterArchiveManifestAppend,
    /// Crash inside the publication transaction, before insert.
    InTransaction,
    /// Crash after insert and head update, before commit.
    AfterInsertBeforeCommit,
    /// Crash after commit, before returning the revision.
    AfterCommitBeforeReturn,
}

impl std::str::FromStr for CrashPoint {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "before_archive_object_write" => Ok(Self::BeforeArchiveObjectWrite),
            "after_archive_object_rename" => Ok(Self::AfterArchiveObjectRename),
            "after_archive_manifest_append" => Ok(Self::AfterArchiveManifestAppend),
            "in_transaction" => Ok(Self::InTransaction),
            "after_insert_before_commit" => Ok(Self::AfterInsertBeforeCommit),
            "after_commit_before_return" => Ok(Self::AfterCommitBeforeReturn),
            other => anyhow::bail!("unknown crash point `{other}`"),
        }
    }
}

#[cfg(debug_assertions)]
static CRASH_POINT: std::sync::Mutex<Option<CrashPoint>> = std::sync::Mutex::new(None);

/// Set the crash-injection point. No-op outside debug builds.
#[cfg(debug_assertions)]
pub fn debug_set_crash_point(point: Option<CrashPoint>) {
    *CRASH_POINT.lock().unwrap_or_else(|err| err.into_inner()) = point;
}

#[cfg(not(debug_assertions))]
pub fn debug_set_crash_point(_point: Option<CrashPoint>) {}

#[cfg(debug_assertions)]
pub(crate) fn maybe_crash(point: CrashPoint) {
    if let Ok(current) = CRASH_POINT.lock()
        && current.is_some_and(|wanted| wanted == point)
    {
        std::process::abort();
    }
}

#[cfg(not(debug_assertions))]
pub(crate) fn maybe_crash(_point: CrashPoint) {}

/// Encoded payload envelope: schema version plus the full snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadEnvelope {
    pub schema_version: u32,
    pub snapshot: ReferenceSnapshot,
}

/// Encode a snapshot into its exact stored bytes.
fn encode_payload(snapshot: &ReferenceSnapshot) -> Result<Vec<u8>, anyhow::Error> {
    let envelope = PayloadEnvelope {
        schema_version: PAYLOAD_SCHEMA_VERSION,
        snapshot: snapshot.clone(),
    };
    let bytes = serde_json::to_vec(&envelope)?;
    Ok(bytes)
}

/// Decode stored bytes and validate them before a caller can observe the
/// result.
fn decode_payload(bytes: &[u8]) -> Result<PayloadEnvelope, anyhow::Error> {
    let envelope: PayloadEnvelope = serde_json::from_slice(bytes)
        .map_err(|err| anyhow::anyhow!("payload does not decode: {err}"))?;
    anyhow::ensure!(
        envelope.schema_version == PAYLOAD_SCHEMA_VERSION,
        "unsupported payload schema_version {} (expected {PAYLOAD_SCHEMA_VERSION})",
        envelope.schema_version
    );
    Ok(envelope)
}

/// Compute a SHA-256 hex digest exactly as the archive does. Test-only.
#[doc(hidden)]
#[must_use]
pub fn payload_digest_for_test(bytes: &[u8]) -> String {
    crate::util::sha256_hex(bytes)
}

/// Publication key: deterministic SHA-256 over the publication inputs.
fn publication_key(
    expected_parent: Option<u64>,
    payload_schema_version: u32,
    payload_sha256: &str,
) -> Result<String, anyhow::Error> {
    let input = serde_json::to_vec(&serde_json::json!([
        PUBLICATION_KEY_MARKER,
        expected_parent,
        payload_schema_version,
        payload_sha256,
    ]))?;
    Ok(sha256_hex(&input))
}

/// Outcome of a publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishOutcome {
    pub revision_id: u64,
    /// True when the publication resolved to an already-committed revision
    /// with the same publication key (idempotent retry).
    pub replayed: bool,
}

/// A revision as stored durably: the decoded snapshot plus its storage
/// metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRevision {
    pub revision_id: u64,
    pub parent_revision_id: Option<u64>,
    pub payload_schema_version: u32,
    pub valid_at: NaiveDate,
    pub recorded_at: DateTime<Utc>,
    pub snapshot_hash: String,
    pub payload_sha256: String,
    pub publication_key: String,
    pub snapshot: ReferenceSnapshot,
}

/// Summary of a full store verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    pub revision_count: u64,
    pub head_revision_id: Option<u64>,
    pub orphans: Vec<String>,
}

/// A durable reference store over one data directory.
#[derive(Debug)]
pub struct DurableReferenceStore {
    root: PathBuf,
    archive: SourceArchive,
    connection: Connection,
    poisoned: bool,
    busy_timeout_ms: u64,
}

impl DurableReferenceStore {
    /// Open (creating if needed) the durable store at `root`.
    ///
    /// # Errors
    ///
    /// Fails without mutation when the store schema version is newer than
    /// this build supports, when the payload schema version is newer, or
    /// when the open fails to verify the archive's committed state.
    pub fn open(root: &Path) -> Result<Self, anyhow::Error> {
        Self::open_with_busy_timeout(root, DEFAULT_BUSY_TIMEOUT_MS)
    }

    /// Open the durable store with a bounded SQLite busy timeout.
    pub fn open_with_busy_timeout(
        root: &Path,
        busy_timeout_ms: u64,
    ) -> Result<Self, anyhow::Error> {
        Self::open_with_options(
            root,
            busy_timeout_ms,
            super::archive::DEFAULT_ARCHIVE_LOCK_TIMEOUT_MS,
        )
    }

    /// Open the durable store with both bounds configurable. Used by tests.
    pub fn open_with_options(
        root: &Path,
        busy_timeout_ms: u64,
        archive_timeout_ms: u64,
    ) -> Result<Self, anyhow::Error> {
        // Schema preflight FIRST, before any directory creation, archive
        // recovery or write-mode work: an unsupported newer schema must fail
        // without modifying the database, the archive or the backups, and
        // without creating missing directories. The version is read with a
        // bounded header read (bytes 60..64, big-endian) directly from the
        // file, so the check cannot create WAL sidecar files or change the
        // journal mode.
        let database_path = root.join("reference.sqlite3");
        if database_path.exists() {
            let user_version = read_user_version_from_header(&database_path)?;
            if user_version != 0 && user_version != STORE_SCHEMA_VERSION {
                anyhow::bail!(
                    "unsupported store schema version {user_version}; expected {STORE_SCHEMA_VERSION}"
                );
            }
        }
        std::fs::create_dir_all(root)?;
        std::fs::create_dir_all(root.join("backups"))?;
        std::fs::create_dir_all(root.join("source-archive"))?;
        let archive =
            SourceArchive::open_with_timeout(&root.join("source-archive"), archive_timeout_ms)?;
        let path = root.join("reference.sqlite3");
        let connection = Connection::open(&path)?;
        configure_connection(&connection, busy_timeout_ms)?;
        let user_version: u32 =
            connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if user_version == 0 {
            create_schema(&connection)?;
        } else if user_version == STORE_SCHEMA_VERSION {
            verify_schema_compatibility(&connection)?;
        } else {
            anyhow::bail!(
                "unsupported store schema version {user_version}; expected {STORE_SCHEMA_VERSION}"
            );
        }
        let store = Self {
            root: root.to_owned(),
            archive,
            connection,
            poisoned: false,
            busy_timeout_ms,
        };
        store.verify_head_chain()?;
        Ok(store)
    }

    /// The store root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The archive at this store's root.
    #[must_use]
    pub fn archive(&self) -> &SourceArchive {
        &self.archive
    }

    /// Crate-visible connection for backup snapshotting.
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    fn ensure_usable(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            !self.poisoned,
            "writer handle is poisoned; reopen the store to resolve the publication key"
        );
        Ok(())
    }

    /// Publish `snapshot` with the expected parent.
    ///
    /// `expected_parent` is `None` for an empty store, `Some(id)` when the
    /// caller reconciled against revision `id`. A mismatch rejects the
    /// publication with a stale-parent error unless the same publication
    /// key is already committed (idempotent retry).
    pub fn publish(
        &mut self,
        snapshot: &ReferenceSnapshot,
        expected_parent: Option<u64>,
    ) -> Result<PublishOutcome, anyhow::Error> {
        self.ensure_usable()?;
        for attempt in 0..5 {
            match self.try_publish(snapshot, expected_parent) {
                Ok(outcome) => return Ok(outcome),
                Err(err) if is_known_rollback(&err) && attempt < 4 => {
                    std::thread::sleep(std::time::Duration::from_millis(25 * (1 << attempt)));
                    continue;
                }
                Err(err) if is_known_rollback(&err) => return Err(err),
                Err(err) => return Err(err),
            }
        }
        unreachable!("retry loop always returns")
    }

    fn try_publish(
        &mut self,
        snapshot: &ReferenceSnapshot,
        expected_parent: Option<u64>,
    ) -> Result<PublishOutcome, anyhow::Error> {
        snapshot.validate()?;
        self.validate_lineages(snapshot)?;

        let payload = encode_payload(snapshot)?;
        let payload_sha256 = sha256_hex(&payload);
        let snapshot_hash = snapshot.snapshot_hash()?;
        anyhow::ensure!(
            snapshot_hash.len() == 64 && snapshot_hash.chars().all(|c| c.is_ascii_hexdigit()),
            "snapshot hash is not 64 lowercase hex digits"
        );
        let payload_sha256_hex = payload_sha256;
        let key = publication_key(expected_parent, PAYLOAD_SCHEMA_VERSION, &payload_sha256_hex)?;

        maybe_crash(CrashPoint::AfterArchiveManifestAppend);

        self.connection
            .busy_timeout(std::time::Duration::from_millis(self.busy_timeout_ms))?;

        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let head: Option<u64> = transaction
            .query_row(
                "SELECT revision_id FROM reference_head WHERE singleton_id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?
            .map(|value: i64| value as u64);

        if head != expected_parent {
            // Head moved since the caller reconciled. An idempotent retry
            // of this exact publication resolves to the committed revision;
            // anything else is stale-parent.
            match resolve_by_key(&transaction, &key, expected_parent, &payload_sha256_hex)? {
                Some(revision_id) => {
                    transaction.commit()?;
                    return Ok(PublishOutcome {
                        revision_id,
                        replayed: true,
                    });
                }
                None => {
                    transaction.rollback().ok();
                    anyhow::bail!(
                        "stale expected parent: store head is {head:?}, caller expected {expected_parent:?}"
                    );
                }
            }
        }

        // Resolve by publication key before allocating a new revision.
        if let Some(revision_id) =
            resolve_by_key(&transaction, &key, expected_parent, &payload_sha256_hex)?
        {
            transaction.commit()?;
            return Ok(PublishOutcome {
                revision_id,
                replayed: true,
            });
        }

        let next_id: i64 = match head {
            Some(current) => current
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("revision id space exhausted"))?
                as i64,
            None => 0,
        };
        let parent: Option<i64> = head.map(|value| value as i64);

        maybe_crash(CrashPoint::InTransaction);

        transaction.execute(
            "INSERT INTO reference_revision (
                revision_id, parent_revision_id, payload_schema_version,
                valid_at, recorded_at, snapshot_hash, payload_sha256,
                publication_key, payload
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                next_id,
                parent,
                PAYLOAD_SCHEMA_VERSION as i64,
                snapshot.valid_at.format("%Y-%m-%d").to_string(),
                format_rfc3339(snapshot.recorded_at),
                snapshot_hash,
                payload_sha256_hex,
                key,
                payload,
            ],
        )?;
        transaction.execute(
            "INSERT INTO reference_head (singleton_id, revision_id) VALUES (1, ?1)
             ON CONFLICT(singleton_id) DO UPDATE SET revision_id = excluded.revision_id",
            rusqlite::params![next_id],
        )?;

        maybe_crash(CrashPoint::AfterInsertBeforeCommit);

        // COMMIT is the durability boundary for the revision and head pair.
        match transaction.commit() {
            Ok(()) => {}
            Err(err) => {
                if is_busy_error(&err) {
                    // Known rollback: the transaction did not commit. The
                    // caller may retry with bounded backoff.
                    return Err(err.into());
                }
                // Outcome unknown: the handle is poisoned. The caller must
                // reopen and resolve by publication key.
                self.poisoned = true;
                return Err(err.into());
            }
        }

        maybe_crash(CrashPoint::AfterCommitBeforeReturn);
        Ok(PublishOutcome {
            revision_id: next_id as u64,
            replayed: false,
        })
    }

    /// The current head revision id, if any.
    pub fn head(&mut self) -> Result<Option<u64>, anyhow::Error> {
        self.ensure_usable()?;
        let head: Option<i64> = self
            .connection
            .query_row(
                "SELECT revision_id FROM reference_head WHERE singleton_id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(head.map(|value| value as u64))
    }

    /// The number of committed revisions.
    pub fn revision_count(&mut self) -> Result<u64, anyhow::Error> {
        self.ensure_usable()?;
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM reference_revision", [], |row| {
                    row.get(0)
                })?;
        Ok(count as u64)
    }

    /// Retrieve and verify a stored revision by id.
    pub fn revision(&mut self, id: u64) -> Result<Option<ReferenceSnapshot>, anyhow::Error> {
        Ok(self.revision_with_meta(id)?.map(|stored| stored.snapshot))
    }

    /// Retrieve and verify a stored revision, including its storage metadata.
    pub fn revision_with_meta(&mut self, id: u64) -> Result<Option<StoredRevision>, anyhow::Error> {
        self.ensure_usable()?;
        let mut statement = self.connection.prepare(
            "SELECT revision_id, parent_revision_id, payload_schema_version,
                    valid_at, recorded_at, snapshot_hash, payload_sha256,
                    publication_key, payload
             FROM reference_revision WHERE revision_id = ?1",
        )?;
        let row = statement
            .query_row(rusqlite::params![id as i64], |row| {
                Ok(StoredRevisionRow {
                    revision_id: row.get::<_, i64>(0)? as u64,
                    parent_revision_id: row.get::<_, Option<i64>>(1)?.map(|value| value as u64),
                    payload_schema_version: row.get::<_, i64>(2)? as u32,
                    valid_at: row.get::<_, String>(3)?,
                    recorded_at: row.get::<_, String>(4)?,
                    snapshot_hash: row.get(5)?,
                    payload_sha256: row.get(6)?,
                    publication_key: row.get(7)?,
                    payload: row.get::<_, Vec<u8>>(8)?,
                })
            })
            .optional()?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(decode_row(row)?))
    }

    /// `Reference(t_valid, t_recorded)` over the head snapshot.
    pub fn reference(
        &mut self,
        t_valid: NaiveDate,
        t_recorded: DateTime<Utc>,
    ) -> Result<Vec<ListingStateRecord>, anyhow::Error> {
        let head_snapshot = self.head_snapshot()?;
        match head_snapshot {
            Some(snapshot) => Ok(latest_states_at(
                &snapshot.listing_states,
                t_valid,
                t_recorded,
            )),
            None => Ok(Vec::new()),
        }
    }

    /// The accumulated head snapshot, if the store is not empty.
    pub fn head_snapshot(&mut self) -> Result<Option<ReferenceSnapshot>, anyhow::Error> {
        self.ensure_usable()?;
        let head: Option<i64> = self
            .connection
            .query_row(
                "SELECT revision_id FROM reference_head WHERE singleton_id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match head {
            Some(id) => Ok(self.revision(id as u64)?),
            None => Ok(None),
        }
    }

    fn verify_head_chain(&self) -> Result<(), anyhow::Error> {
        let user_version: u32 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if user_version == 0 {
            return Ok(());
        }
        if user_version != STORE_SCHEMA_VERSION {
            anyhow::bail!("unsupported store schema version {user_version}");
        }
        let rows = {
            let mut statement = self.connection.prepare(
                "SELECT revision_id, parent_revision_id FROM reference_revision ORDER BY revision_id ASC",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u64,
                        row.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        for (index, (id, parent)) in rows.iter().enumerate() {
            if *id != index as u64 {
                anyhow::bail!("revision id {id} is not contiguous at position {index}");
            }
            if index == 0 {
                anyhow::ensure!(parent.is_none(), "revision 0 has a parent");
            } else {
                anyhow::ensure!(
                    *parent == Some(index as u64 - 1),
                    "revision {id} parent is {parent:?}, expected {}",
                    index as u64 - 1
                );
            }
        }
        let head: Option<i64> = self
            .connection
            .query_row(
                "SELECT revision_id FROM reference_head WHERE singleton_id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match (rows.is_empty(), head) {
            (true, None) => Ok(()),
            (false, Some(head)) => {
                let head = head as u64;
                anyhow::ensure!(
                    head == rows.len() as u64 - 1,
                    "head revision {head} is not the highest committed revision {}",
                    rows.len() as u64 - 1
                );
                Ok(())
            }
            _ => anyhow::bail!("head row does not match revision table"),
        }
    }

    fn validate_lineages(&self, snapshot: &ReferenceSnapshot) -> Result<(), anyhow::Error> {
        self.validate_lineage_shallow(snapshot)?;
        let hashes = collect_lineage_hashes(snapshot);
        for hash in hashes {
            self.archive.resolve(&hash)?;
        }
        Ok(())
    }

    fn validate_lineage_shallow(&self, snapshot: &ReferenceSnapshot) -> Result<(), anyhow::Error> {
        snapshot.validate()?;
        Ok(())
    }

    /// Verify the complete store: schema versions, head chain, every payload
    /// checksum and semantic hash, and every source reference across every
    /// revision. Archive objects without a revision reference are reported
    /// separately and are never an error on their own.
    pub fn verify_full(&mut self) -> Result<VerificationReport, anyhow::Error> {
        self.ensure_usable()?;
        let verification = self.archive.verify_full()?;
        let rows: Vec<StoredRevisionRow> = {
            let mut statement = self.connection.prepare(
                "SELECT revision_id, parent_revision_id, payload_schema_version,
                        valid_at, recorded_at, snapshot_hash, payload_sha256,
                        publication_key, payload
                 FROM reference_revision ORDER BY revision_id ASC",
            )?;
            statement
                .query_map([], |row| {
                    Ok(StoredRevisionRow {
                        revision_id: row.get::<_, i64>(0)? as u64,
                        parent_revision_id: row.get::<_, Option<i64>>(1)?.map(|value| value as u64),
                        payload_schema_version: row.get::<_, i64>(2)? as u32,
                        valid_at: row.get(3)?,
                        recorded_at: row.get(4)?,
                        snapshot_hash: row.get(5)?,
                        payload_sha256: row.get(6)?,
                        publication_key: row.get(7)?,
                        payload: row.get(8)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        for row in &rows {
            decode_row(StoredRevisionRow {
                revision_id: row.revision_id,
                parent_revision_id: row.parent_revision_id,
                payload_schema_version: row.payload_schema_version,
                valid_at: row.valid_at.clone(),
                recorded_at: row.recorded_at.clone(),
                snapshot_hash: row.snapshot_hash.clone(),
                payload_sha256: row.payload_sha256.clone(),
                publication_key: row.publication_key.clone(),
                payload: row.payload.clone(),
            })?;
        }
        // Resolve every source reference of every revision.
        for row in &rows {
            let envelope = decode_payload(&row.payload)?;
            let snapshot = envelope.snapshot;
            for hash in collect_lineage_hashes(&snapshot) {
                self.archive.resolve(&hash).map_err(|err| {
                    anyhow::anyhow!(
                        "revision {} references missing or hash-wrong source: {err}",
                        row.revision_id
                    )
                })?;
            }
        }
        let head = self.head()?;
        Ok(VerificationReport {
            revision_count: rows.len() as u64,
            head_revision_id: head,
            orphans: verification.unindexed_objects,
        })
    }
}

/// Collect every archive-resolvable hash referenced by a snapshot.
pub fn collect_lineage_hashes(snapshot: &ReferenceSnapshot) -> Vec<String> {
    let mut hashes: Vec<String> = Vec::new();
    hashes.push(snapshot.provenance.sha256_hex.clone());
    for state in &snapshot.listing_states {
        hashes.push(state.provenance.sha256_hex.clone());
        for hash in collect_tick_hashes(&state.tick_assignment) {
            hashes.push(hash);
        }
    }
    for instrument in &snapshot.instruments {
        for listing in &instrument.listings {
            for terms in &listing.terms {
                for hash in collect_tick_hashes(&terms.tick_assignment) {
                    hashes.push(hash);
                }
            }
        }
    }
    hashes.sort();
    hashes.dedup();
    hashes
}

fn collect_tick_hashes(assignment: &india_domain::TickAssignment) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(hash) = &assignment.reference_sha256_hex {
        out.push(hash.clone());
    }
    if let Some(hash) = &assignment.schedule_sha256_hex {
        out.push(hash.clone());
    }
    if let Some(hash) = &assignment.calendar_sha256_hex {
        out.push(hash.clone());
    }
    out
}

struct StoredRevisionRow {
    revision_id: u64,
    parent_revision_id: Option<u64>,
    payload_schema_version: u32,
    valid_at: String,
    recorded_at: String,
    snapshot_hash: String,
    payload_sha256: String,
    publication_key: String,
    payload: Vec<u8>,
}

fn decode_row(row: StoredRevisionRow) -> Result<StoredRevision, anyhow::Error> {
    let recomputed = sha256_hex(&row.payload);
    anyhow::ensure!(
        recomputed == row.payload_sha256,
        "revision {} payload checksum mismatch",
        row.revision_id
    );
    let envelope = decode_payload(&row.payload)?;
    anyhow::ensure!(
        envelope.schema_version == row.payload_schema_version,
        "revision {} envelope schema version does not match its metadata",
        row.revision_id
    );
    let snapshot = envelope.snapshot;
    snapshot.validate()?;
    let semantic = snapshot.snapshot_hash()?;
    anyhow::ensure!(
        semantic == row.snapshot_hash,
        "revision {} semantic snapshot hash mismatch",
        row.revision_id
    );
    let decoded_valid_at = snapshot.valid_at;
    let stored_valid_at = NaiveDate::parse_from_str(&row.valid_at, "%Y-%m-%d")?;
    anyhow::ensure!(
        decoded_valid_at == stored_valid_at,
        "revision {} valid_at metadata does not match payload",
        row.revision_id
    );
    let stored_recorded_at = parse_rfc3339(&row.recorded_at)?;
    anyhow::ensure!(
        snapshot.recorded_at == stored_recorded_at,
        "revision {} recorded_at metadata does not match payload",
        row.revision_id
    );
    Ok(StoredRevision {
        revision_id: row.revision_id,
        parent_revision_id: row.parent_revision_id,
        payload_schema_version: row.payload_schema_version,
        valid_at: decoded_valid_at,
        recorded_at: snapshot.recorded_at,
        snapshot_hash: row.snapshot_hash,
        payload_sha256: row.payload_sha256,
        publication_key: row.publication_key,
        snapshot,
    })
}

fn publication_key_lookup(
    transaction: &Transaction<'_>,
    key: &str,
) -> Result<Option<StoredRevisionRow>, anyhow::Error> {
    let row = transaction
        .query_row(
            "SELECT revision_id, parent_revision_id, payload_schema_version,
                    valid_at, recorded_at, snapshot_hash, payload_sha256,
                    publication_key, payload
             FROM reference_revision WHERE publication_key = ?1",
            rusqlite::params![key],
            |row| {
                Ok(StoredRevisionRow {
                    revision_id: row.get::<_, i64>(0)? as u64,
                    parent_revision_id: row.get::<_, Option<i64>>(1)?.map(|value| value as u64),
                    payload_schema_version: row.get::<_, i64>(2)? as u32,
                    valid_at: row.get(3)?,
                    recorded_at: row.get(4)?,
                    snapshot_hash: row.get(5)?,
                    payload_sha256: row.get(6)?,
                    publication_key: row.get(7)?,
                    payload: row.get(8)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Resolve a publication by its key against committed durable state.
///
/// `Some(id)` means the publication already committed with matching
/// expected parent *and* a fully verified stored row: payload bytes,
/// checksum, semantic hash and metadata. `None` means the key is absent.
/// Any disagreement fails closed as corruption.
fn resolve_by_key(
    transaction: &Transaction<'_>,
    key: &str,
    expected_parent: Option<u64>,
    expected_checksum: &str,
) -> Result<Option<u64>, anyhow::Error> {
    let Some(existing) = publication_key_lookup(transaction, key)? else {
        return Ok(None);
    };
    // Full verification of the committed row through the same path used for
    // retrieval: payload checksum, envelope version, domain validation,
    // semantic hash against the stored `snapshot_hash` column, and
    // valid_at/recorded_at metadata agreement. A corrupted BLOB, checksum or
    // semantic-hash column can never be reported as a successful replay.
    let verified = decode_row(existing)?;
    if verified.payload_sha256 != expected_checksum {
        anyhow::bail!(
            "publication key names a revision whose checksum disagrees with the attempted publication; failing closed as corruption"
        );
    }
    if verified.parent_revision_id != expected_parent {
        anyhow::bail!("publication key exists with different parent; failing closed as corruption");
    }
    Ok(Some(verified.revision_id))
}

fn is_busy_error(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::DatabaseBusy
    )
}

fn is_known_rollback(err: &anyhow::Error) -> bool {
    err.downcast_ref::<rusqlite::Error>()
        .is_some_and(is_busy_error)
}

/// Canonical UTC RFC 3339 for stored metadata.
fn format_rfc3339(recorded_at: DateTime<Utc>) -> String {
    // AutoSi matches chrono's default DateTime serde output (0/3/6/9
    // fractional digits), so the stored metadata and the payload timestamp
    // round-trip at full nanosecond precision.
    recorded_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

fn parse_rfc3339(value: &str) -> Result<DateTime<Utc>, anyhow::Error> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

/// Read the SQLite `user_version` with a bounded header read: open the file,
/// read at most the first 100 bytes, and decode bytes 60..64 (big-endian).
///
/// A zero-length file is a valid empty SQLite database (version 0). A short
/// non-empty file is not a SQLite database and fails.
fn read_user_version_from_header(path: &Path) -> Result<u32, anyhow::Error> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(0);
    }
    let mut header = [0u8; 100];
    let mut read = 0usize;
    while read < header.len() {
        let bytes_read = file.read(&mut header[read..])?;
        if bytes_read == 0 {
            break;
        }
        read += bytes_read;
    }
    anyhow::ensure!(
        read == header.len() && &header[..16] == b"SQLite format 3\x00",
        "reference database is not a SQLite database"
    );
    Ok(u32::from_be_bytes([
        header[60], header[61], header[62], header[63],
    ]))
}

fn configure_connection(
    connection: &Connection,
    busy_timeout_ms: u64,
) -> Result<(), anyhow::Error> {
    connection.busy_timeout(std::time::Duration::from_millis(busy_timeout_ms))?;
    let mode: String = connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    anyhow::ensure!(
        mode.eq_ignore_ascii_case("wal"),
        "journal_mode must be WAL, got {mode}"
    );
    connection.execute_batch("PRAGMA synchronous = FULL;\nPRAGMA foreign_keys = ON;\n")?;
    let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
    anyhow::ensure!(synchronous == 2, "synchronous must be FULL");
    let foreign_keys: i64 = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    anyhow::ensure!(foreign_keys == 1, "foreign_keys must be ON");
    Ok(())
}

fn create_schema(connection: &Connection) -> Result<(), anyhow::Error> {
    connection.execute_batch(
        "CREATE TABLE reference_revision (
             revision_id INTEGER PRIMARY KEY CHECK (revision_id >= 0),
             parent_revision_id INTEGER UNIQUE REFERENCES reference_revision(revision_id),
             payload_schema_version INTEGER NOT NULL CHECK (payload_schema_version = 1),
             valid_at TEXT NOT NULL CHECK (length(valid_at) = 10),
             recorded_at TEXT NOT NULL,
             snapshot_hash TEXT NOT NULL CHECK (length(snapshot_hash) = 64),
             payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64),
             publication_key TEXT NOT NULL UNIQUE CHECK (length(publication_key) = 64),
             payload BLOB NOT NULL,
             CHECK (
                 (revision_id = 0 AND parent_revision_id IS NULL)
                 OR (revision_id > 0 AND parent_revision_id = revision_id - 1)
             ),
             CHECK (snapshot_hash NOT GLOB '*[^0-9a-f]*'),
             CHECK (payload_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK (publication_key NOT GLOB '*[^0-9a-f]*')
         );
         CREATE TABLE reference_head (
             singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
             revision_id INTEGER NOT NULL REFERENCES reference_revision(revision_id)
         );
         CREATE TRIGGER reference_revision_no_update
         BEFORE UPDATE ON reference_revision
         BEGIN
             SELECT RAISE(ABORT, 'reference revisions are immutable');
         END;
         CREATE TRIGGER reference_revision_no_delete
         BEFORE DELETE ON reference_revision
         BEGIN
             SELECT RAISE(ABORT, 'reference revisions are immutable');
         END;
         CREATE TRIGGER reference_head_no_delete
         BEFORE DELETE ON reference_head
         BEGIN
             SELECT RAISE(ABORT, 'the reference head cannot be deleted');
         END;
         PRAGMA user_version = 1;",
    )?;
    Ok(())
}

fn verify_schema_compatibility(connection: &Connection) -> Result<(), anyhow::Error> {
    let payload_versions: Vec<i64> = connection
        .prepare("SELECT DISTINCT payload_schema_version FROM reference_revision")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for version in payload_versions {
        anyhow::ensure!(
            version == PAYLOAD_SCHEMA_VERSION as i64,
            "unsupported payload schema version {version}"
        );
    }
    Ok(())
}
