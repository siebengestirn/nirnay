//! Order lifecycle and outbox states for the authoritative OMS.
//!
//! The full lifecycle from the target architecture is represented explicitly.
//! Transitions are guarded: terminal states never move, and `Unknown` can only
//! leave through reconciliation, never through an automatic retry.

use serde::{
    Deserialize, //
    Serialize,
};

/// Authoritative order lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmsOrderState {
    Created,
    RiskAccepted,
    PendingSubmission,
    Rejected,
    Unknown,
    Reconciling,
    Resolved,
    Working,
    PartiallyFilled,
    Filled,
    CancelPending,
    Cancelled,
}

impl OmsOrderState {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Filled | Self::Cancelled | Self::Rejected)
    }

    #[must_use]
    pub const fn permits(self, next: Self) -> bool {
        match self {
            Self::Created => matches!(next, Self::RiskAccepted),
            Self::RiskAccepted => matches!(next, Self::PendingSubmission),
            Self::PendingSubmission => {
                matches!(next, Self::Working | Self::Rejected | Self::Unknown)
            }
            Self::Working => matches!(
                next,
                Self::PartiallyFilled
                    | Self::Filled
                    | Self::CancelPending
                    | Self::Rejected
                    | Self::Unknown
            ),
            Self::PartiallyFilled => matches!(
                next,
                Self::PartiallyFilled | Self::Filled | Self::CancelPending | Self::Unknown
            ),
            Self::CancelPending => matches!(
                next,
                Self::PartiallyFilled | Self::Cancelled | Self::Filled | Self::Unknown
            ),
            Self::Unknown => matches!(next, Self::Reconciling),
            Self::Reconciling => matches!(next, Self::Resolved),
            Self::Resolved => matches!(
                next,
                Self::Working
                    | Self::PartiallyFilled
                    | Self::Filled
                    | Self::Rejected
                    | Self::Cancelled
            ),
            Self::Filled | Self::Cancelled | Self::Rejected => false,
        }
    }
}

/// Outbox / submission-attempt state. Each row is one submission attempt.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
    Pending,
    Claimed,
    Dispatching,
    Acknowledged,
    Unknown,
    Rejected,
}

impl OutboxState {
    #[must_use]
    pub const fn permits(self, next: Self) -> bool {
        match self {
            Self::Pending => matches!(next, Self::Claimed),
            Self::Claimed => matches!(next, Self::Dispatching | Self::Pending),
            Self::Dispatching => {
                matches!(next, Self::Acknowledged | Self::Rejected | Self::Unknown)
            }
            Self::Unknown | Self::Acknowledged | Self::Rejected => false,
        }
    }
}
