//! Runtime filesystem layout. Evidence and secrets never live in the source
//! tree; each class of state has its own directory.

use std::{
    fs, //
    path::{Path, PathBuf},
};

use crate::error::OpsError;

/// Separate state classes for the paper runtime.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RuntimeDir {
    Binaries,
    Config,
    Secrets,
    RawFrames,
    Journals,
    Reference,
    Research,
    Backups,
    Logs,
}

impl RuntimeDir {
    pub const ALL: [Self; 9] = [
        Self::Binaries,
        Self::Config,
        Self::Secrets,
        Self::RawFrames,
        Self::Journals,
        Self::Reference,
        Self::Research,
        Self::Backups,
        Self::Logs,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Binaries => "bin",
            Self::Config => "config",
            Self::Secrets => "secrets",
            Self::RawFrames => "raw-frames",
            Self::Journals => "journals",
            Self::Reference => "reference-revisions",
            Self::Research => "research-datasets",
            Self::Backups => "backups",
            Self::Logs => "logs",
        }
    }
}

/// A runtime root and its validated subdirectories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeLayout {
    root: PathBuf,
}

impl RuntimeLayout {
    /// Construct a layout rooted at `root`. The root must not be inside the
    /// source tree that contains this crate.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, OpsError> {
        let root = root.as_ref().to_path_buf();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .ok_or(OpsError::PathEscape)?;
        let source_canonical = source
            .canonicalize()
            .map_err(|e| OpsError::Io(e.to_string()))?;
        let root_canonical = if root.exists() {
            root.canonicalize()
                .map_err(|e| OpsError::Io(e.to_string()))?
        } else {
            resolve_existing_ancestor(&root)?
        };
        if root_canonical.starts_with(&source_canonical)
            || source_canonical.starts_with(&root_canonical)
        {
            return Err(OpsError::RootInsideSourceTree);
        }
        Ok(Self { root })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn path(&self, directory: RuntimeDir) -> PathBuf {
        self.root.join(directory.name())
    }

    /// Create every state directory. Secrets are created with owner-only
    /// permissions on Unix.
    pub fn create_all(&self) -> Result<(), OpsError> {
        for directory in RuntimeDir::ALL {
            let path = self.path(directory);
            fs::create_dir_all(&path).map_err(|e| OpsError::Io(e.to_string()))?;
            if directory == RuntimeDir::Secrets {
                restrict_owner_only(&path)?;
            }
        }
        Ok(())
    }

    /// Verify the layout is complete and every path stays under the root.
    pub fn validate(&self) -> Result<(), OpsError> {
        for directory in RuntimeDir::ALL {
            let path = self.path(directory);
            if !path.is_dir() {
                return Err(OpsError::IncompleteLayout(directory.name().to_owned()));
            }
            let target = fs::canonicalize(&path).map_err(|e| OpsError::Io(e.to_string()))?;
            let root = fs::canonicalize(&self.root).map_err(|e| OpsError::Io(e.to_string()))?;
            if !target.starts_with(&root) {
                return Err(OpsError::PathEscape);
            }
        }
        Ok(())
    }
}

fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf, OpsError> {
    let mut current = path;
    while !current.exists() {
        current = current.parent().ok_or(OpsError::PathEscape)?;
    }
    let base = fs::canonicalize(current).map_err(|e| OpsError::Io(e.to_string()))?;
    let missing = path
        .strip_prefix(current)
        .map_err(|_| OpsError::PathEscape)?;
    Ok(base.join(missing))
}

#[cfg(unix)]
fn restrict_owner_only(path: &Path) -> Result<(), OpsError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| OpsError::Io(e.to_string()))
}

#[cfg(not(unix))]
fn restrict_owner_only(_path: &Path) -> Result<(), OpsError> {
    Ok(())
}
