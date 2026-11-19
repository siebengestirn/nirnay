//! Raw feed observations supplied to the authority.
//!
//! Callers submit what was observed, never a health verdict. The authority
//! derives [`FeedHealth`] from the quote, its continuity evidence, book
//! completeness and its age against the authority's own logical clock. A
//! caller-supplied `healthy: bool` has no place in this contract.

use serde::Serialize;

use trading_core::{
    FeedHealth, //
    LogicalNanos,
    QuoteSnapshot,
};

/// Sequence evidence carried by an observation, used to detect a gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SequenceGap {
    pub expected: Option<u64>,
    pub observed: u64,
}

/// One observed market fact for a single instrument and venue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeedObservation {
    /// The displayed quote as observed. `None` means no quote was received.
    pub quote: Option<QuoteSnapshot>,
    /// Logical time at which the observation was received.
    pub received_at: LogicalNanos,
    /// Gap evidence from continuity tracking, if a gap was detected.
    pub gap: Option<SequenceGap>,
    /// Book completeness reported by the decoder (a raw observation
    /// attribute, not a health judgement).
    pub book_complete: bool,
}

impl FeedObservation {
    /// Derive feed health from observation facts and the authority's clock.
    pub fn health(&self, now: LogicalNanos, max_quote_age_ns: u64) -> FeedHealth {
        let Some(quote) = self.quote.as_ref() else {
            return FeedHealth::NoQuote;
        };
        let (Some(bid), Some(ask)) = (quote.bid_price.as_ref(), quote.ask_price.as_ref()) else {
            return FeedHealth::OneSided;
        };
        if bid.get().0 > ask.get().0 {
            return FeedHealth::Crossed;
        }
        if let Some(gap) = self.gap {
            return FeedHealth::Gapped {
                expected_sequence: gap.expected,
                observed_sequence: gap.observed,
            };
        }
        if !self.book_complete {
            return FeedHealth::Incomplete;
        }
        let age_ns = now.get().saturating_sub(self.received_at.get());
        if age_ns > max_quote_age_ns {
            return FeedHealth::Stale { age_ns };
        }
        FeedHealth::Healthy { age_ns }
    }
}
