//! Backup and restore for the durable reference store (task P1-04).
//!
//! A backup is a verified database-plus-archive copy with a `backup.json`
//! manifest. `restore` verifies it in a temporary location and only swaps it
//! into place after every check passes, leaving any existing store untouched
//! on failure.

use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::util::sha256_hex;

use super::durable::{DurableReferenceStore, PAYLOAD_SCHEMA_VERSION, STORE_SCHEMA_VERSION};

/// Backup format version.
pub const BACKUP_FORMAT_VERSION: u32 = 1;

/// Machine-readable backup manifest (`backup.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub format_version: u32,
    pub created_at: String,
    pub store_schema_version: u32,
    pub payload_schema_version: u32,
    pub head_revision_id: Option<u64>,
    pub database_sha256: String,
    pub manifest_sha256: String,
}

/// A completed backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupResult {
    pub path: PathBuf,
    pub manifest: BackupManifest,
}

/// Result of a restore verification pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreVerification {
    pub revision_count: u64,
    pub head_revision_id: Option<u64>,
    pub unindexed_objects: Vec<String>,
}

/// Create a verified backup of `store` under `<root>/backups/<backup_id>/`.
///
/// The store is verified before the copy, so a backup never silently
/// blesses an unhealthy store. The archive lock is held while the manifest
/// and its objects are copied so they agree with each other.
pub fn backup(store: &mut DurableReferenceStore) -> Result<BackupResult, anyhow::Error> {
    // Fail closed on an unhealthy store before snapshotting it.
    let verification = store.verify_full()?;

    let backup_id = format!(
        "{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S%9fZ"),
        std::process::id()
    );
    let root = store.root().to_owned();
    let backups = root.join("backups");
    std::fs::create_dir_all(&backups)?;
    let tmp = backups.join(format!(".tmp-{backup_id}"));
    let final_path = backups.join(&backup_id);
    std::fs::create_dir_all(tmp.join("source-archive"))?;

    // Consistent snapshot via SQLite's online backup path (VACUUM INTO);
    // a live WAL is never copied byte-for-byte by us.
    let database = tmp.join("reference.sqlite3");
    let database_path = database.to_string_lossy().replace('\'', "''");
    store
        .connection()
        .execute_batch(&format!("VACUUM INTO '{database_path}'"))?;

    // Copy the manifest and every object it names, under the archive lock.
    let archive_root = store.archive().root().to_owned();
    let _archive_guard = store.archive().lock_exclusive_for_backup()?;
    let manifest_src = archive_root.join("manifest.jsonl");
    if manifest_src.exists() {
        let bytes = std::fs::read(&manifest_src)?;
        std::fs::write(tmp.join("source-archive").join("manifest.jsonl"), &bytes)?;
        std::fs::File::open(tmp.join("source-archive"))?.sync_all()?;
        for entry in store.archive().entries_locked()? {
            let object = archive_root.join(format!("{}.src", entry.sha256_hex));
            anyhow::ensure!(
                object.exists(),
                "archived source {} is missing; refusing to write an incomplete backup",
                entry.sha256_hex
            );
            std::fs::copy(
                &object,
                tmp.join("source-archive")
                    .join(format!("{}.src", entry.sha256_hex)),
            )?;
        }
    }
    drop(_archive_guard);

    let database_hash = sha256_hex(&std::fs::read(&database)?);
    let manifest_hash = if tmp.join("source-archive").join("manifest.jsonl").exists() {
        sha256_hex(&std::fs::read(
            tmp.join("source-archive").join("manifest.jsonl"),
        )?)
    } else {
        sha256_hex(b"")
    };

    // Derive the head from the *copied* database so the manifest always
    // matches the backed-up bytes, even if another process published after
    // the pre-copy verification ran.
    let copied = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let copied_head: Option<i64> = copied
        .query_row(
            "SELECT revision_id FROM reference_head WHERE singleton_id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let copied_head = copied_head.map(|value| value as u64);
    if copied_head != verification.head_revision_id {
        anyhow::bail!(
            "store changed during backup: copied head is {copied_head:?}, verified head was {:?}; refusing to write a manifest that disagrees with its database",
            verification.head_revision_id
        );
    }

    let manifest = BackupManifest {
        format_version: BACKUP_FORMAT_VERSION,
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true),
        store_schema_version: STORE_SCHEMA_VERSION,
        payload_schema_version: PAYLOAD_SCHEMA_VERSION,
        head_revision_id: copied_head,
        database_sha256: database_hash.clone(),
        manifest_sha256: manifest_hash.clone(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(tmp.join("backup.json"), &manifest_bytes)?;

    // Sync everything, then rename into place.
    for entry in std::fs::read_dir(tmp.join("source-archive"))? {
        let path = entry?.path();
        if path.is_file() {
            std::fs::File::open(&path)?.sync_all()?;
        }
    }
    std::fs::File::open(&database)?.sync_all()?;
    std::fs::File::open(tmp.join("backup.json"))?.sync_all()?;
    std::fs::File::open(tmp.join("source-archive"))?.sync_all()?;
    std::fs::File::open(&tmp)?.sync_all()?;
    std::fs::rename(&tmp, &final_path)?;
    std::fs::File::open(&backups)?.sync_all()?;

    Ok(BackupResult {
        path: final_path,
        manifest,
    })
}

/// Verify a backup and restore it into `target_root`.
///
/// On any failure the existing store at `target_root` is left untouched.
pub fn restore(
    backup_dir: &Path,
    target_root: &Path,
) -> Result<RestoreVerification, anyhow::Error> {
    if target_root.exists() {
        anyhow::ensure!(target_root.is_dir(), "restore target is not a directory");
    }

    // 1. Verify backup.json before touching anything.
    let manifest_path = backup_dir.join("backup.json");
    let manifest_bytes = std::fs::read(&manifest_path)?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    anyhow::ensure!(
        manifest.format_version == BACKUP_FORMAT_VERSION,
        "unsupported backup format version {}",
        manifest.format_version
    );
    anyhow::ensure!(
        manifest.store_schema_version == STORE_SCHEMA_VERSION,
        "backup store schema version {} is unsupported",
        manifest.store_schema_version
    );
    anyhow::ensure!(
        manifest.payload_schema_version == PAYLOAD_SCHEMA_VERSION,
        "backup payload schema version {} is unsupported",
        manifest.payload_schema_version
    );

    let database_src = backup_dir.join("reference.sqlite3");
    let database_bytes = std::fs::read(&database_src)?;
    anyhow::ensure!(
        sha256_hex(&database_bytes) == manifest.database_sha256,
        "backup database hash mismatch"
    );
    let manifest_src = backup_dir.join("source-archive").join("manifest.jsonl");
    let manifest_hash = if manifest_src.exists() {
        sha256_hex(&std::fs::read(&manifest_src)?)
    } else {
        sha256_hex(b"")
    };
    anyhow::ensure!(
        manifest_hash == manifest.manifest_sha256,
        "backup manifest hash mismatch"
    );

    // 2. Restore into a temporary location, never directly over a live
    //    store.
    let parent = target_root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("restore target has no parent"))?;
    let name = target_root
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("restore target has no file name"))?
        .to_string_lossy()
        .to_string();
    let temp = parent.join(format!(
        ".{name}.restore-tmp-{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S%9fZ"),
        std::process::id()
    ));
    std::fs::create_dir_all(temp.join("source-archive"))?;

    let copy_result = (|| -> Result<(), anyhow::Error> {
        std::fs::copy(&database_src, temp.join("reference.sqlite3"))?;
        let archive_src = backup_dir.join("source-archive");
        if archive_src.is_dir() {
            for entry in std::fs::read_dir(&archive_src)? {
                let item = entry?.path();
                if item.is_file() {
                    let destination = temp
                        .join("source-archive")
                        .join(item.file_name().unwrap_or_default());
                    std::fs::copy(&item, &destination)?;
                    std::fs::File::open(&destination)?.sync_all()?;
                }
            }
        }
        std::fs::File::open(temp.join("source-archive"))?.sync_all()?;
        std::fs::File::open(temp.join("reference.sqlite3"))?.sync_all()?;
        std::fs::File::open(&temp)?.sync_all()?;
        Ok(())
    })();
    if copy_result.is_err() {
        let _ = std::fs::remove_dir_all(&temp);
        return copy_result.map(|()| unreachable!());
    }

    // 3. Open the restored store and run every check.
    let verify_result = (|| -> Result<RestoreVerification, anyhow::Error> {
        let mut restored = DurableReferenceStore::open(&temp)?;
        let verification = restored.verify_full()?;
        let head = restored.head()?;
        Ok(RestoreVerification {
            revision_count: verification.revision_count,
            head_revision_id: head,
            unindexed_objects: verification.orphans,
        })
    })();
    let verification = match verify_result {
        Ok(verification) => verification,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&temp);
            return Err(err);
        }
    };
    // The restored database head must match the backup manifest.
    if verification.head_revision_id != manifest.head_revision_id {
        let _ = std::fs::remove_dir_all(&temp);
        anyhow::bail!(
            "restored head {:?} does not match backup manifest head {:?}",
            verification.head_revision_id,
            manifest.head_revision_id
        );
    }

    // 4. Swap into place only after all checks pass. Move the existing
    //    store aside first; if the swap fails, move it back.
    let retired = parent.join(format!(
        ".{name}.retired-{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S%9fZ"),
        std::process::id()
    ));
    let had_existing = target_root.exists();
    if had_existing {
        std::fs::rename(target_root, &retired)?;
    }
    if let Err(err) = std::fs::rename(&temp, target_root) {
        if had_existing {
            let _ = std::fs::rename(&retired, target_root);
        }
        let _ = std::fs::remove_dir_all(&temp);
        return Err(err.into());
    }
    std::fs::File::open(parent)?.sync_all()?;
    Ok(verification)
}
