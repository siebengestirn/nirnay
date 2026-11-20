//! Market-by-price book construction with declared depth and evidence-gated
//! snapshot recovery.
//!
//! One book belongs to one source. Books from independent providers are never
//! summed or merged; reconciliation compares them without combining liquidity.

use market_types::{
    Side,
    model::{BookSnapshot, Price, QuantityChange},
};
use thiserror::Error;

use crate::orderbook::{
    BookError, //
    OrderBook,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookPhase {
    Uninitialized,
    SnapshotReceiving,
    SnapshotComplete,
    LiveUpdates,
    Gap,
    Recovering,
}

impl BookPhase {
    #[must_use]
    pub const fn accepts_delta(&self) -> bool {
        matches!(self, Self::LiveUpdates)
    }
}

/// Evidence that a baseline snapshot was established from a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEvidence {
    pub source_id: String,
    pub snapshot_id: String,
    pub source_digest: String,
    pub received_utc_ns: i64,
}

impl SnapshotEvidence {
    pub fn validate(&self) -> Result<(), MbpBookError> {
        if self.source_id.trim().is_empty()
            || self.snapshot_id.trim().is_empty()
            || self.source_digest.len() != 64
            || !self
                .source_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.received_utc_ns <= 0
        {
            return Err(MbpBookError::MissingEvidence);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum MbpBookError {
    #[error("declared depth must be positive")]
    InvalidDepth,
    #[error("book phase {current:?} does not permit {operation}")]
    InvalidPhase {
        current: BookPhase,
        operation: &'static str,
    },
    #[error("snapshot has {actual} levels, declared depth is {declared}")]
    DepthExceeded { actual: usize, declared: u32 },
    #[error("a snapshot baseline requires valid evidence")]
    MissingEvidence,
    #[error("evidence source `{evidence}` does not match book source `{book}`")]
    SourceMismatch { evidence: String, book: String },
    #[error("snapshot is crossed")]
    CrossedSnapshot,
    #[error("order book error: {0}")]
    Book(#[from] BookError),
}

/// One source's aggregated price-level book.
#[derive(Debug, Clone)]
pub struct MbpBook {
    inner: OrderBook,
    source_id: String,
    declared_depth: u32,
    phase: BookPhase,
    baseline: Option<SnapshotEvidence>,
    gap_sequence: Option<u64>,
}

impl MbpBook {
    pub fn new(source_id: impl Into<String>, declared_depth: u32) -> Result<Self, MbpBookError> {
        if declared_depth == 0 {
            return Err(MbpBookError::InvalidDepth);
        }
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(MbpBookError::MissingEvidence);
        }
        Ok(Self {
            inner: OrderBook::new(),
            source_id,
            declared_depth,
            phase: BookPhase::Uninitialized,
            baseline: None,
            gap_sequence: None,
        })
    }

    #[must_use]
    pub const fn phase(&self) -> BookPhase {
        self.phase
    }

    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    #[must_use]
    pub const fn declared_depth(&self) -> u32 {
        self.declared_depth
    }

    #[must_use]
    pub fn baseline(&self) -> Option<&SnapshotEvidence> {
        self.baseline.as_ref()
    }

    #[must_use]
    pub const fn gap_sequence(&self) -> Option<u64> {
        self.gap_sequence
    }

    /// Begin a snapshot baseline after initialization or a detected gap.
    pub fn begin_snapshot(&mut self, evidence: &SnapshotEvidence) -> Result<(), MbpBookError> {
        self.check_evidence(evidence)?;
        if !matches!(self.phase, BookPhase::Uninitialized | BookPhase::Recovering) {
            return Err(MbpBookError::InvalidPhase {
                current: self.phase,
                operation: "begin_snapshot",
            });
        }
        self.phase = BookPhase::SnapshotReceiving;
        Ok(())
    }

    /// Install an evidenced baseline. A fresh snapshot establishes current
    /// state but does not reconstruct missing historical updates.
    pub fn apply_snapshot(
        &mut self,
        snapshot: &BookSnapshot,
        evidence: &SnapshotEvidence,
    ) -> Result<(), MbpBookError> {
        self.check_evidence(evidence)?;
        if self.phase != BookPhase::SnapshotReceiving {
            return Err(MbpBookError::InvalidPhase {
                current: self.phase,
                operation: "apply_snapshot",
            });
        }
        let depth = usize::try_from(self.declared_depth).unwrap_or(usize::MAX);
        if snapshot.bids.len() > depth || snapshot.asks.len() > depth {
            return Err(MbpBookError::DepthExceeded {
                actual: snapshot.bids.len().max(snapshot.asks.len()),
                declared: self.declared_depth,
            });
        }
        self.inner.replace(snapshot)?;
        if self.inner.is_crossed() {
            return Err(MbpBookError::CrossedSnapshot);
        }
        self.baseline = Some(evidence.clone());
        self.phase = BookPhase::SnapshotComplete;
        Ok(())
    }

    /// Move a complete baseline into live update mode.
    pub fn mark_live(&mut self) -> Result<(), MbpBookError> {
        if self.phase != BookPhase::SnapshotComplete {
            return Err(MbpBookError::InvalidPhase {
                current: self.phase,
                operation: "mark_live",
            });
        }
        self.phase = BookPhase::LiveUpdates;
        Ok(())
    }

    /// Apply one level delta only while live.
    pub fn apply_delta(
        &mut self,
        side: Side,
        price: Price,
        change: QuantityChange,
    ) -> Result<(), MbpBookError> {
        if !self.phase.accepts_delta() {
            return Err(MbpBookError::InvalidPhase {
                current: self.phase,
                operation: "apply_delta",
            });
        }
        self.inner.apply(side, price, change)?;
        if self.inner.is_crossed() {
            self.phase = BookPhase::Gap;
        }
        Ok(())
    }

    /// Record a sequence gap. Deltas stop until a new evidenced baseline.
    pub fn mark_gap(&mut self, sequence: u64) -> Result<(), MbpBookError> {
        if self.phase != BookPhase::LiveUpdates {
            return Err(MbpBookError::InvalidPhase {
                current: self.phase,
                operation: "mark_gap",
            });
        }
        self.phase = BookPhase::Gap;
        self.gap_sequence = Some(sequence);
        Ok(())
    }

    /// Move from gap to recovery before taking a fresh snapshot.
    pub fn begin_recovery(&mut self) -> Result<(), MbpBookError> {
        if self.phase != BookPhase::Gap {
            return Err(MbpBookError::InvalidPhase {
                current: self.phase,
                operation: "begin_recovery",
            });
        }
        self.phase = BookPhase::Recovering;
        Ok(())
    }

    #[must_use]
    pub fn to_snapshot(&self) -> BookSnapshot {
        self.inner.to_snapshot()
    }

    #[must_use]
    pub fn best_bid(&self) -> Option<market_types::model::Level> {
        self.inner.best_bid()
    }

    #[must_use]
    pub fn best_ask(&self) -> Option<market_types::model::Level> {
        self.inner.best_ask()
    }

    #[must_use]
    pub fn is_crossed(&self) -> bool {
        self.inner.is_crossed()
    }

    fn check_evidence(&self, evidence: &SnapshotEvidence) -> Result<(), MbpBookError> {
        evidence.validate()?;
        if evidence.source_id != self.source_id {
            return Err(MbpBookError::SourceMismatch {
                evidence: evidence.source_id.clone(),
                book: self.source_id.clone(),
            });
        }
        Ok(())
    }
}
