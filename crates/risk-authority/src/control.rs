//! Kill-switch control state with owner-local append-only persistence.
//!
//! No new service or database is introduced: the file-backed implementation
//! is a single fsynced JSON-lines file under the owner's runtime directory.
//! Fail-closed default: with no readable event the switch is engaged.

use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One authored kill-switch decision. Append-only; never rewritten.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillSwitchState {
    pub enabled: bool,
    pub actor_id: String,
    pub reason: String,
    pub revision: u64,
    pub at_ns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ControlError {
    #[error("control store io failure: {0}")]
    Io(String),
    #[error("control store serialization failure: {0}")]
    Serialize(String),
    #[error("control store record is corrupt: {0}")]
    Corrupt(String),
}

/// Minimal persistence contract. Implementations must make `append` durable
/// before returning, and `load` must fail closed on unreadable data.
pub trait ControlStore {
    fn load_kill_switch(&self) -> Result<Option<KillSwitchState>, ControlError>;
    fn append_kill_switch(&self, state: &KillSwitchState) -> Result<(), ControlError>;
}

/// In-memory store for callers that do not own a control directory.
#[derive(Clone, Debug, Default)]
pub struct NoopControlStore;

impl ControlStore for NoopControlStore {
    fn load_kill_switch(&self) -> Result<Option<KillSwitchState>, ControlError> {
        Ok(None)
    }

    fn append_kill_switch(&self, _state: &KillSwitchState) -> Result<(), ControlError> {
        Ok(())
    }
}

/// Append-only file store. Every record is one JSON line followed by fsync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileControlStore {
    path: PathBuf,
}

impl FileControlStore {
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ControlStore for FileControlStore {
    fn load_kill_switch(&self) -> Result<Option<KillSwitchState>, ControlError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let file = File::open(&self.path).map_err(|e| ControlError::Io(e.to_string()))?;
        let mut latest = None;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|e| ControlError::Io(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let state: KillSwitchState = serde_json::from_str(&line)
                .map_err(|e| ControlError::Corrupt(format!("line {}: {e}", index + 1)))?;
            if latest
                .as_ref()
                .is_none_or(|prior: &KillSwitchState| state.revision >= prior.revision)
            {
                latest = Some(state);
            }
        }
        Ok(latest)
    }

    fn append_kill_switch(&self, state: &KillSwitchState) -> Result<(), ControlError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| ControlError::Io(e.to_string()))?;
        let line =
            serde_json::to_string(state).map_err(|e| ControlError::Serialize(e.to_string()))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|e| ControlError::Io(e.to_string()))
    }
}
