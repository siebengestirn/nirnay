//! Immutable backup bundles with digest manifests and verified restore.

use std::{
    collections::BTreeSet, //
    fs,
    path::{Path, PathBuf},
};

use serde::{
    Deserialize, //
    Serialize,
};
use sha2::{
    Digest, //
    Sha256,
};

use crate::{
    error::OpsError, //
    layout::{RuntimeDir, RuntimeLayout},
};

const MANIFEST_FILE: &str = "manifest.json";
const MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupEntry {
    /// Path relative to the backup root. Never absolute or parent-escaping.
    pub relative_path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: u32,
    pub created_at_ns: u64,
    pub entries: Vec<BackupEntry>,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_backup_name(name: &str) -> Result<(), OpsError> {
    if name.is_empty()
        || name.len() > 64
        || name == "."
        || name == ".."
        || !name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'.'
        })
    {
        return Err(OpsError::InvalidManifest("unsafe backup name".to_owned()));
    }
    Ok(())
}

fn safe_relative(relative: &str) -> Result<(), OpsError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        || relative.is_empty()
    {
        return Err(OpsError::InvalidManifest("unsafe relative path".to_owned()));
    }
    Ok(())
}

fn collect_files(
    source: &Path,
    prefix: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), OpsError> {
    let metadata = fs::metadata(source).map_err(|e| OpsError::Io(e.to_string()))?;
    if metadata.is_file() {
        let name = prefix.join(
            source
                .file_name()
                .ok_or_else(|| OpsError::Io("no file name".to_owned()))?,
        );
        out.push((name.to_string_lossy().into_owned(), source.to_path_buf()));
        return Ok(());
    }
    if metadata.is_dir() {
        let directory = fs::read_dir(source).map_err(|e| OpsError::Io(e.to_string()))?;
        for entry in directory {
            let entry = entry.map_err(|e| OpsError::Io(e.to_string()))?;
            let name = entry.file_name();
            collect_files(
                &entry.path(),
                &prefix.join(name.to_string_lossy().as_ref()),
                out,
            )?;
        }
    }
    Ok(())
}

/// Create a backup bundle under the runtime's backup directory. Publication
/// is atomic: files are staged, then renamed into place only after success.
pub fn create_backup(
    layout: &RuntimeLayout,
    sources: &[PathBuf],
    name: &str,
    created_at_ns: u64,
) -> Result<PathBuf, OpsError> {
    validate_backup_name(name)?;
    let root = layout.path(RuntimeDir::Backups);
    let destination = root.join(name);
    if destination.exists() {
        return Err(OpsError::TargetExists(name.to_owned()));
    }
    let staging = root.join(format!(".{name}.staging"));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| OpsError::Io(e.to_string()))?;
    }
    fs::create_dir_all(&staging).map_err(|e| OpsError::Io(e.to_string()))?;

    let mut files = Vec::new();
    for source in sources {
        collect_files(source, Path::new(""), &mut files)?;
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for (relative, source) in files {
        safe_relative(&relative)?;
        if !seen.insert(relative.clone()) {
            return Err(OpsError::InvalidManifest(format!(
                "duplicate backup entry `{relative}`"
            )));
        }
        let bytes = fs::read(&source).map_err(|e| OpsError::Io(e.to_string()))?;
        let target = staging.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| OpsError::Io(e.to_string()))?;
        }
        fs::write(&target, &bytes).map_err(|e| OpsError::Io(e.to_string()))?;
        entries.push(BackupEntry {
            relative_path: relative,
            size: u64::try_from(bytes.len())
                .map_err(|_| OpsError::InvalidManifest("size overflow".to_owned()))?,
            sha256: digest(&bytes),
        });
    }

    let manifest = BackupManifest {
        version: MANIFEST_VERSION,
        created_at_ns,
        entries,
    };
    let encoded = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| OpsError::InvalidManifest(e.to_string()))?;
    fs::write(staging.join(MANIFEST_FILE), encoded).map_err(|e| OpsError::Io(e.to_string()))?;
    fs::rename(&staging, &destination).map_err(|e| OpsError::Io(e.to_string()))?;
    Ok(destination)
}

/// Re-verify every entry against its digest.
pub fn verify_backup(backup_dir: &Path) -> Result<BackupManifest, OpsError> {
    let manifest_path = backup_dir.join(MANIFEST_FILE);
    let bytes = fs::read(&manifest_path).map_err(|e| OpsError::Io(e.to_string()))?;
    let manifest: BackupManifest =
        serde_json::from_slice(&bytes).map_err(|e| OpsError::InvalidManifest(e.to_string()))?;
    if manifest.version != MANIFEST_VERSION {
        return Err(OpsError::InvalidManifest(
            "unsupported manifest version".to_owned(),
        ));
    }
    for entry in &manifest.entries {
        safe_relative(&entry.relative_path)?;
        let content = fs::read(backup_dir.join(&entry.relative_path))
            .map_err(|_| OpsError::MissingEntry(entry.relative_path.clone()))?;
        if u64::try_from(content.len())
            .map_err(|_| OpsError::InvalidManifest("size overflow".to_owned()))?
            != entry.size
            || digest(&content) != entry.sha256
        {
            return Err(OpsError::DigestMismatch(entry.relative_path.clone()));
        }
    }
    Ok(manifest)
}

/// Restore a verified bundle into a fresh path, then verify the copy.
pub fn restore_backup(backup_dir: &Path, target_root: &Path) -> Result<(), OpsError> {
    let manifest = verify_backup(backup_dir)?;
    if target_root.exists() {
        return Err(OpsError::TargetExists(
            target_root.to_string_lossy().into_owned(),
        ));
    }
    fs::create_dir_all(target_root).map_err(|e| OpsError::Io(e.to_string()))?;
    for entry in &manifest.entries {
        let source = backup_dir.join(&entry.relative_path);
        let destination = target_root.join(&entry.relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|e| OpsError::Io(e.to_string()))?;
        }
        let bytes = fs::read(&source).map_err(|e| OpsError::Io(e.to_string()))?;
        fs::write(&destination, bytes).map_err(|e| OpsError::Io(e.to_string()))?;
    }
    for entry in &manifest.entries {
        let bytes = fs::read(target_root.join(&entry.relative_path))
            .map_err(|_| OpsError::MissingEntry(entry.relative_path.clone()))?;
        if digest(&bytes) != entry.sha256 {
            return Err(OpsError::DigestMismatch(entry.relative_path.clone()));
        }
    }
    Ok(())
}
