//! Bitemporal reference snapshots and the revision store.
//!
//! A `ReferenceSnapshot` answers the full query
//! `Reference(t_valid, t_recorded)`: the listing state that was *valid* at
//! `t_valid`, *as known* at `t_recorded` (reviewer finding 7).
//!
//! Two structures make that possible:
//!
//! * **Revisions.** [`ReferenceStore`] keeps every published [`ReferenceSnapshot`]
//!   immutably, in publication order. Revisions are never edited; a new
//!   observation publishes a new revision. Retrieval by revision works after
//!   interruption (see [`ReferenceStore::publish`]).
//! * **Listing-state history.** Each snapshot carries `listing_states`:
//!   immutable [`ListingStateRecord`]s accumulated from every reconciliation
//!   since the first (recorded on open/close in `resolution.rs`). Every
//!   state has its own knowledge time (`recorded_at`) and valid interval —
//!   the snapshot's `recorded_at` is never stamped across history.
//!
//! Selecting the latest state per `(instrument, venue)` among records with
//! `recorded_at <= t_recorded` and a valid interval covering `t_valid` gives
//! the historical universe as the system knew it at that moment.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use india_domain::{Instrument, InstrumentId, TradingCalendar, Venue};
use serde::{Deserialize, Serialize};

use crate::util::sha256_hex;

use super::provenance::SourceProvenance;
use super::resolution::{ListingStateRecord, ReconciledUniverse};

/// Canonical ordering for accumulated listing-state history:
/// ascending by `(recorded_at, instrument_id, venue discriminant, symbol,
/// valid_from, valid_to)`, with a null `valid_to` before a present date.
pub fn listing_state_ordering(
    a: &ListingStateRecord,
    b: &ListingStateRecord,
) -> std::cmp::Ordering {
    a.recorded_at
        .cmp(&b.recorded_at)
        .then_with(|| a.instrument_id.cmp(&b.instrument_id))
        .then_with(|| (a.venue as u8).cmp(&(b.venue as u8)))
        .then_with(|| a.symbol.cmp(&b.symbol))
        .then_with(|| a.valid_from.cmp(&b.valid_from))
        .then_with(|| a.valid_to.cmp(&b.valid_to))
}

/// Whether two snapshot records carry the same duplicate observation.
fn is_duplicate_observation(a: &ListingStateRecord, b: &ListingStateRecord) -> bool {
    a.instrument_id == b.instrument_id
        && a.venue == b.venue
        && a.valid_from == b.valid_from
        && a.recorded_at == b.recorded_at
}

fn venue_label(venue: Venue) -> String {
    match venue {
        Venue::Nse => "NSE".to_owned(),
        Venue::Bse => "BSE".to_owned(),
    }
}

/// An immutable, deterministically ordered point-in-time universe plus the
/// listing-state history that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSnapshot {
    /// Valid time: the trading date this snapshot is *about*.
    pub valid_at: NaiveDate,
    /// Knowledge time: when this snapshot was ingested.
    pub recorded_at: DateTime<Utc>,
    /// Raw-file provenance that produced the universe.
    pub provenance: SourceProvenance,
    /// Instruments sorted by [`InstrumentId`].
    pub instruments: Vec<Instrument>,
    /// Trading calendar carried with the snapshot, when the ingest had one.
    pub calendar: Option<TradingCalendar>,
    /// Reconciliation observed while building this snapshot.
    pub reconciliation: Option<super::resolution::Reconciliation>,
    /// Every listing-state observed up to and including this snapshot,
    /// each with its own `recorded_at` and valid interval.
    pub listing_states: Vec<ListingStateRecord>,
}

impl ReferenceSnapshot {
    /// Build and validate a snapshot from a reconciled universe.
    ///
    /// `previous_states` is the accumulated history from earlier revisions;
    /// this reconciliation's fresh states are appended to it.
    pub fn from_reconciled(
        reconciled: ReconciledUniverse,
        valid_at: NaiveDate,
        recorded_at: DateTime<Utc>,
        provenance: SourceProvenance,
        previous_states: &[ListingStateRecord],
    ) -> Result<Self, anyhow::Error> {
        provenance.validate()?;
        let mut listing_states = previous_states.to_vec();
        listing_states.extend(reconciled.listing_states);
        listing_states.sort_by(listing_state_ordering);
        let reconciliation = reconciled.reconciliation;
        let mut instruments = reconciled.instruments;
        instruments.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
        let snapshot = Self {
            valid_at,
            recorded_at,
            provenance,
            instruments,
            calendar: None,
            reconciliation: Some(reconciliation),
            listing_states,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Validate the whole universe.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        // Envelope inputs that cannot be reconstructed after decode.
        self.provenance.validate()?;
        anyhow::ensure!(
            self.recorded_at.timestamp_nanos_opt().is_some(),
            "snapshot recorded_at is out of representable range"
        );
        // Instruments must already be in canonical strict order.
        for pair in self.instruments.windows(2) {
            anyhow::ensure!(
                pair[0].instrument_id < pair[1].instrument_id,
                "snapshot instruments are not in canonical ascending order: {} then {}",
                pair[0].instrument_id,
                pair[1].instrument_id
            );
        }
        // Listing intervals must be well formed and already in canonical
        // order. Listing-state history is *not* sorted silently: a stored
        // vector that is out of order is rejected.
        for state in &self.listing_states {
            if let Some(end) = state.valid_to {
                anyhow::ensure!(
                    state.valid_from <= end,
                    "listing state for {} has valid_from {} after valid_to {}",
                    state.instrument_id,
                    state.valid_from,
                    end
                );
            }
        }
        for pair in self.listing_states.windows(2) {
            anyhow::ensure!(
                listing_state_ordering(&pair[0], &pair[1]) != std::cmp::Ordering::Greater,
                "listing-state history is not in canonical order"
            );
        }
        // Duplicate listing observations are rejected outright.
        for window in self.listing_states.windows(2) {
            if is_duplicate_observation(&window[0], &window[1]) && window[0] != window[1] {
                anyhow::bail!(
                    "conflicting duplicate listing observation for {} on {:?} at {}",
                    window[0].instrument_id,
                    window[0].venue,
                    window[0].recorded_at
                );
            }
        }
        // O(n log n) duplicate detection keyed on the observation identity.
        let mut seen = std::collections::BTreeSet::new();
        for state in &self.listing_states {
            let key = (
                state.instrument_id.clone(),
                state.venue as u8,
                state.valid_from,
                state.recorded_at,
            );
            anyhow::ensure!(
                seen.insert(key),
                "duplicate listing observation for {} on {:?} at {}",
                state.instrument_id,
                state.venue,
                state.recorded_at
            );
        }
        // No (venue, symbol) may be claimed by two *different* instruments
        // over an overlapping interval. The same instrument may repeat a
        // symbol across closed+reopened intervals (symbol/status changes);
        // `Instrument::validate` already forbids overlaps within one.
        let mut claimed: Vec<(String, String, NaiveDate, Option<NaiveDate>, &InstrumentId)> =
            Vec::new();
        for instrument in &self.instruments {
            instrument.validate()?;
            for listing in &instrument.listings {
                claimed.push((
                    venue_label(listing.venue),
                    listing.venue_symbol.clone(),
                    listing.effective_from,
                    listing.effective_to,
                    &instrument.instrument_id,
                ));
            }
        }
        for (index, (venue, symbol, from, to, id)) in claimed.iter().enumerate() {
            if claimed[index + 1..].iter().any(
                |(other_venue, other_symbol, other_from, other_to, other_id)| {
                    other_venue == venue
                        && other_symbol == symbol
                        && *other_id != *id
                        && intervals_overlap(*from, *to, *other_from, *other_to)
                },
            ) {
                anyhow::bail!(
                    "snapshot: {venue}:{symbol} is claimed by both {} and another instrument",
                    id
                );
            }
        }
        // State history: ordered by knowledge time (validated by sort in
        // construction, but re-check for externally built snapshots).
        for window in self.listing_states.windows(2) {
            anyhow::ensure!(
                window[0].recorded_at <= window[1].recorded_at,
                "listing-state history is not knowledge-ordered"
            );
        }
        Ok(())
    }

    /// Ids of instruments with an active listing on `date` (valid time).
    #[must_use]
    pub fn universe_on(&self, date: NaiveDate) -> Vec<InstrumentId> {
        self.instruments
            .iter()
            .filter(|instrument| {
                instrument.listings.iter().any(|listing| {
                    listing.effective_from <= date
                        && listing.effective_to.is_none_or(|end| date <= end)
                })
            })
            .map(|instrument| instrument.instrument_id.clone())
            .collect()
    }

    /// Trading symbol for `instrument_id` on `venue` at `date`, if listed.
    #[must_use]
    pub fn symbol_on(
        &self,
        instrument_id: &InstrumentId,
        venue: Venue,
        date: NaiveDate,
    ) -> Option<String> {
        self.instruments
            .iter()
            .find(|instrument| &instrument.instrument_id == instrument_id)
            .and_then(|instrument| {
                instrument
                    .listings
                    .iter()
                    .find(|listing| {
                        listing.venue == venue
                            && listing.effective_from <= date
                            && listing.effective_to.is_none_or(|end| date <= end)
                    })
                    .map(|listing| listing.venue_symbol.clone())
            })
    }

    /// The full bitemporal listing-state history (valid intervals with
    /// their own knowledge times) carried by this snapshot.
    #[must_use]
    pub fn bitemporal_listings(&self) -> &[ListingStateRecord] {
        &self.listing_states
    }

    /// `Reference(t_valid, t_recorded)`: the latest knowledge of each
    /// listing state that was valid at `t_valid`, considering only records
    /// learned by `t_recorded`.
    ///
    /// Returns one [`ListingStateRecord`] per `(instrument, venue)` that
    /// was listed at `t_valid` according to the latest knowledge available
    /// at `t_recorded`. Each record carries the full reference contract in
    /// force for that state (symbol, token, settlement regime/group,
    /// Z-category, market lot and the assigned tick with its lineage), so a
    /// historical consumer never has to fall back to today's master.
    /// Ordered deterministically.
    #[must_use]
    pub fn reference(
        &self,
        t_valid: NaiveDate,
        t_recorded: DateTime<Utc>,
    ) -> Vec<ListingStateRecord> {
        latest_states_at(&self.listing_states, t_valid, t_recorded)
    }

    /// Content hash of the snapshot: SHA-256 over the canonical JSON of
    /// `(valid_at, instruments, calendar, listing_states)`. Top-level
    /// provenance and reconciliation are excluded, so the same universe
    /// rebuilds to the same hash across restatements; callers keep
    /// provenance alongside the hash. Listing-state provenance records ARE
    /// included because they are embedded in each `ListingStateRecord`.
    ///
    /// # Errors
    ///
    /// Serialization failure aborts the call; there is no empty-bytes
    /// fallback.
    pub fn snapshot_hash(&self) -> Result<String, anyhow::Error> {
        snapshot_hash_of(
            &self.instruments,
            self.valid_at,
            self.calendar.as_ref(),
            &self.listing_states,
        )
    }
}

/// Select, per `(instrument, venue)`, the listing state that was valid at
/// `t_valid` according to the latest knowledge available at `t_recorded`.
///
/// A listing interval is identified by `(instrument, venue, valid_from)`.
/// Every revision of that interval — opening it, closing it, restating its
/// group — appends an immutable record under a new `recorded_at`; the
/// latest version within `t_recorded` is the one believed. A closed
/// interval's revised version therefore outranks the earlier open-ended
/// record: looking merely for "some record covering `t_valid`" would
/// resurrect a stale open-ended belief and report a delisted security as
/// listed.
#[must_use]
pub fn latest_states_at(
    states: &[ListingStateRecord],
    t_valid: NaiveDate,
    t_recorded: DateTime<Utc>,
) -> Vec<ListingStateRecord> {
    type IntervalKey = (InstrumentId, u8, NaiveDate);
    let mut versions: BTreeMap<IntervalKey, &ListingStateRecord> = BTreeMap::new();
    for state in states {
        if state.recorded_at > t_recorded {
            continue;
        }
        let key = (
            state.instrument_id.clone(),
            state.venue as u8,
            state.valid_from,
        );
        match versions.get(&key) {
            Some(existing) if existing.recorded_at >= state.recorded_at => {}
            _ => {
                versions.insert(key, state);
            }
        }
    }
    let mut best: BTreeMap<(InstrumentId, u8), &ListingStateRecord> = BTreeMap::new();
    for state in versions.into_values() {
        let valid = state.valid_from <= t_valid && state.valid_to.is_none_or(|end| t_valid <= end);
        if !valid {
            continue;
        }
        let key = (state.instrument_id.clone(), state.venue as u8);
        match best.get(&key) {
            Some(existing) if existing.valid_from >= state.valid_from => {}
            _ => {
                best.insert(key, state);
            }
        }
    }
    let mut out: Vec<ListingStateRecord> = best.into_values().cloned().collect();
    out.sort_by(|a, b| {
        a.instrument_id
            .cmp(&b.instrument_id)
            .then_with(|| (a.venue as u8).cmp(&(b.venue as u8)))
    });
    out
}

/// Whether two effective intervals [from, to] (open-ended when None)
/// intersect. Intervals are inclusive at both ends (matches
/// `india_domain`'s listing overlap rule).
fn intervals_overlap(
    a_from: NaiveDate,
    a_to: Option<NaiveDate>,
    b_from: NaiveDate,
    b_to: Option<NaiveDate>,
) -> bool {
    let a_starts_before_b_ends = b_to.is_none_or(|b_end| a_from <= b_end);
    let b_starts_before_a_ends = a_to.is_none_or(|a_end| b_from <= a_end);
    a_starts_before_b_ends && b_starts_before_a_ends
}

/// Canonical snapshot hash used by [`ReferenceSnapshot::snapshot_hash`].
///
/// # Errors
///
/// Returns an error when the canonical encoding fails. A serialization
/// failure must never silently fall back to hashing empty bytes.
pub fn snapshot_hash_of(
    instruments: &[Instrument],
    valid_at: NaiveDate,
    calendar: Option<&TradingCalendar>,
    listing_states: &[ListingStateRecord],
) -> Result<String, anyhow::Error> {
    let serializable = serde_json::json!({
        "valid_at": valid_at.format("%Y-%m-%d").to_string(),
        "instruments": instruments,
        "calendar": calendar,
        "listing_states": listing_states,
    });
    let bytes = serde_json::to_vec(&serializable)
        .map_err(|err| anyhow::anyhow!("snapshot hash input serialization failed: {err}"))?;
    Ok(sha256_hex(&bytes))
}

/// Immutable revision store: append-only sequence of snapshots.
///
/// Publication is atomic in-process: a snapshot is validated and only then
/// appended (a failed validation publishes nothing, so an interrupted
/// ingestion leaves no partial revision). Retrieval by revision id is exact.
#[derive(Debug, Default)]
pub struct ReferenceStore {
    revisions: Vec<ReferenceSnapshot>,
}

impl ReferenceStore {
    /// Publish `snapshot` as the next revision; returns its revision id.
    pub fn publish(&mut self, snapshot: ReferenceSnapshot) -> Result<u64, anyhow::Error> {
        snapshot.validate()?;
        let id = self.revisions.len() as u64;
        self.revisions.push(snapshot);
        Ok(id)
    }

    /// Retrieve a revision by id.
    #[must_use]
    pub fn revision(&self, id: u64) -> Option<&ReferenceSnapshot> {
        self.revisions.get(id as usize)
    }

    /// Number of published revisions.
    #[must_use]
    pub fn revision_count(&self) -> usize {
        self.revisions.len()
    }

    /// All revisions in publication order.
    #[must_use]
    pub fn revisions(&self) -> &[ReferenceSnapshot] {
        &self.revisions
    }

    /// `Reference(t_valid, t_recorded)` across the store: the latest state
    /// per `(instrument, venue)` learned by `t_recorded` and valid at
    /// `t_valid`. Queries the head snapshot only; the head already carries
    /// the accumulated listing-state history, so enumerating earlier
    /// revisions would duplicate or resurrect stale records.
    #[must_use]
    pub fn reference(
        &self,
        t_valid: NaiveDate,
        t_recorded: DateTime<Utc>,
    ) -> Vec<ListingStateRecord> {
        self.revisions
            .last()
            .map(|head| latest_states_at(&head.listing_states, t_valid, t_recorded))
            .unwrap_or_default()
    }

    /// Ids of instruments with an `Active` listing state valid at `t_valid`
    /// and known by `t_recorded`.
    #[must_use]
    pub fn known_universe(
        &self,
        t_valid: NaiveDate,
        t_recorded: DateTime<Utc>,
    ) -> Vec<InstrumentId> {
        let mut ids: Vec<InstrumentId> = self
            .reference(t_valid, t_recorded)
            .into_iter()
            .filter(|state| state.status == india_domain::ListingStatus::Active)
            .map(|state| state.instrument_id)
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }
}
