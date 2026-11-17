//! Identity resolution and snapshot reconciliation.
//!
//! Resolution turns validated venue rows into canonical [`Instrument`]s and
//! reconciles them against the previously accepted universe:
//!
//! * Rows are matched to existing instruments **by ISIN**, so a symbol change
//!   preserves the platform [`InstrumentId`].
//! * A brand-new instrument gets a deterministic bootstrap id
//!   (`in.{isin-lowercase}`). That id is a stored allocation, never a runtime
//!   derivation from a symbol; from then on it is authoritative and survives
//!   symbol, name and token changes.
//! * A fresh `(venue, symbol)` colliding with a listing of a *different*
//!   instrument over an overlapping interval is rejected. Symbol reuse
//!   after the previous occupant's interval ends is allowed.
//! * Listing state changes (symbol, status, regime, Z-category, settlement
//!   group, token, assigned tick, market lot) close the old effective
//!   interval and open a new one; the closing state record retains the
//!   group/lot/tick that were in force before the change. **History is
//!   never overwritten in place** (reviewer finding 3): the state a date
//!   carries is the state of the interval covering it.
//!   In particular a new tick revision updates the terms interval — the
//!   matcher never goes stale on schedule changes.
//! * Every open/close is recorded as a [`ListingStateRecord`] with the
//!   observation's knowledge time (`recorded_at`) and provenance.
//! * Instruments absent from a **full** snapshot master of their venue are
//!   closed on that venue and reported as removed only when no open listing
//!   remains. Full-snapshot reconciliation is per venue, keyed on
//!   `(ISIN, venue)`: an ISIN present on BSE does not protect an absent NSE
//!   listing from closing, and vice versa. Partial ingests leave absent
//!   instruments untouched.

use std::collections::{BTreeMap, BTreeSet};

use india_domain::{
    Instrument, InstrumentId, InstrumentType, Isin, Listing, ListingStatus, ListingTerms, Segment,
    SettlementRegime, TickAssignment, Venue,
};
use jiff::{Timestamp, civil::Date};

use super::archive::SourceArchive;
use super::provenance::SourceProvenance;
use super::tick_schedule::{ReferenceClose, TickScheduleBook};
use super::{bse::BseCashRow, nse::NseCashRow};

/// One venue-scoped listing candidate, before it is merged into an
/// [`Instrument`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedListing {
    pub venue: Venue,
    pub segment: Segment,
    /// Canonical instrument family this venue row belongs to. Only
    /// categories with a domain contract are accepted by the parsers.
    pub instrument_type: InstrumentType,
    pub venue_symbol: String,
    pub token: Option<String>,
    pub isin: Isin,
    pub name: String,
    pub status: ListingStatus,
    pub settlement_regime: SettlementRegime,
    pub z_category: bool,
    /// BSE settlement group (`Group Name`), when the master carries it.
    pub settlement_group: Option<String>,
    /// Tick assigned to this instrument for its term interval, with the
    /// reference-close and schedule lineage that produced it.
    pub tick_assignment: TickAssignment,
    pub market_lot: i64,
    /// Valid-time observation date (the master's `as_of`).
    pub as_of: Date,
}

/// An immutable bitemporal listing-state observation: during
/// `[valid_from, valid_to]` (inclusive) the given instrument had this listing
/// state on this venue, and the system learned it at `recorded_at` from the
/// given source. Closing a state never deletes it; it appends a successor
/// state. Queries (`Reference(t_v, t_r)`) select by both axes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingStateRecord {
    pub instrument_id: InstrumentId,
    pub venue: Venue,
    pub symbol: String,
    /// Venue-assigned token in force for this state (NSE `None`; BSE Scrip
    /// Code).
    pub token: Option<String>,
    pub status: ListingStatus,
    pub settlement_regime: SettlementRegime,
    pub z_category: bool,
    /// Settlement group in force for this state (BSE `Group Name`).
    pub settlement_group: Option<String>,
    /// Market lot in force for this state.
    pub market_lot: i64,
    /// Assigned tick (with its source lineage) in force for this state.
    pub tick_assignment: TickAssignment,
    pub valid_from: Date,
    pub valid_to: Option<Date>,
    pub recorded_at: Timestamp,
    pub provenance: SourceProvenance,
}

/// One explicit change produced by reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentChange {
    pub instrument_id: InstrumentId,
    /// Listing closed by this reconciliation, e.g. `"NSE:OLD-SYMBOL"`.
    pub closed_listing: Option<String>,
    /// Listing opened or status-changed by this reconciliation, e.g.
    /// `"NSE:NEW-SYMBOL"`. `None` when nothing changed for this instrument.
    pub opened_listing: Option<String>,
}

/// Explicit reconciliation result: additions, removals and in-place changes.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reconciliation {
    pub added: Vec<InstrumentId>,
    pub removed: Vec<InstrumentId>,
    pub changed: Vec<InstrumentChange>,
}

/// The reconciled universe plus its reconciliation and fresh bitemporal
/// listing-state records (accumulated by the caller into the snapshot).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciledUniverse {
    /// Instruments sorted by [`InstrumentId`] (canonical order).
    pub instruments: Vec<Instrument>,
    pub reconciliation: Reconciliation,
    /// State records created by this reconciliation (closes and opens).
    pub listing_states: Vec<ListingStateRecord>,
}

fn venue_name(venue: Venue) -> &'static str {
    match venue {
        Venue::Nse => "NSE",
        Venue::Bse => "BSE",
    }
}

fn listing_label(venue: Venue, symbol: &str) -> String {
    format!("{}:{symbol}", venue_name(venue))
}

fn bootstrap_id(isin: &Isin) -> Result<InstrumentId, anyhow::Error> {
    InstrumentId::new(format!("in.{}", isin.as_str().to_lowercase()))
        .map_err(|err| anyhow::anyhow!("cannot allocate id for {isin}: {err}"))
}

fn previous_day(as_of: Date) -> Result<Date, anyhow::Error> {
    as_of
        .checked_add(jiff::Span::new().days(-1))
        .map_err(|_| anyhow::anyhow!("as_of {as_of} has no previous day"))
}

/// Quantity step for cash-equity listings (post-reform unit step).
pub const QUANTITY_INCREMENT: i64 = 1;

/// Convert a validated NSE CM row into a normalized candidate.
///
/// The tick assignment is the NSE monthly assignment for `as_of` fixed by
/// the recorded `reference` close (the previous month's last available
/// close) through the given schedule book. The close's date, dataset and
/// hash travel into the assignment, as do the provenanced calendar's source
/// hash and revision; a bare calendar is rejected.
pub fn nse_carry(
    row: &NseCashRow,
    as_of: Date,
    reference: &ReferenceClose,
    book: &TickScheduleBook,
    archive: &SourceArchive,
    calendar: &super::calendar::BuiltCalendar,
) -> Result<NormalizedListing, anyhow::Error> {
    anyhow::ensure!(
        reference.isin == row.isin,
        "reference close for {} cannot fix the tick of {}",
        reference.isin.as_str(),
        row.isin.as_str()
    );
    let tick_assignment = book.monthly_assignment(as_of, reference, archive, calendar)?;
    Ok(NormalizedListing {
        venue: Venue::Nse,
        segment: Segment::Cash,
        instrument_type: InstrumentType::Equity,
        venue_symbol: row.symbol.clone(),
        token: None,
        isin: row.isin.clone(),
        name: row.name.clone(),
        status: row.status,
        settlement_regime: row.settlement_regime,
        z_category: row.z_category,
        settlement_group: None,
        tick_assignment,
        market_lot: row.market_lot,
        as_of,
    })
}

/// Convert a validated BSE scrip row into a normalized candidate.
///
/// BSE's per-scrip `New Tick Size` is the assigned tick directly (no
/// monthly computation); `Group Name` is classified into the settlement
/// regime (trade-to-trade groups carry no netting and their `Z*` variants
/// carry the Z-category designation) and preserved as the settlement group.
/// The master's `Market Lot` is carried through unchanged (never defaulted
/// to one). The master's own provenance supplies the schedule revision and
/// hash, so the tick can be reproduced from the archive.
#[must_use]
pub fn bse_carry(
    row: &BseCashRow,
    as_of: Date,
    provenance: &SourceProvenance,
) -> NormalizedListing {
    NormalizedListing {
        venue: Venue::Bse,
        segment: Segment::Cash,
        instrument_type: InstrumentType::Equity,
        venue_symbol: row.symbol.clone(),
        token: Some(row.scrip_code.clone()),
        isin: row.isin.clone(),
        name: row.name.clone(),
        status: row.status,
        settlement_regime: row.group_class.settlement_regime,
        z_category: row.group_class.z_category,
        settlement_group: Some(row.settlement_group.clone()),
        tick_assignment: TickAssignment {
            tick_paise: row.tick_paise,
            reference_close_paise: None,
            reference_close_date: None,
            reference_source: None,
            reference_sha256_hex: None,
            effective_month: None,
            schedule_revision: format!(
                "{}:{}",
                provenance.source_id, provenance.source_format_version
            ),
            schedule_effective_from: Some(as_of),
            schedule_sha256_hex: Some(provenance.sha256_hex.clone()),
            calendar_revision: None,
            calendar_sha256_hex: None,
        },
        market_lot: row.market_lot,
        as_of,
    }
}

/// Build a state record for `listing` observed at `recorded_at` via
/// `provenance`.
fn state_record(
    instrument_id: &InstrumentId,
    listing: &Listing,
    recorded_at: Timestamp,
    provenance: &SourceProvenance,
) -> Result<ListingStateRecord, anyhow::Error> {
    // Record the trading terms that were actually in force at the end of the
    // state's valid interval (the close date when closing, the start date
    // when opening) — never "the first term ever", which would restate an
    // old tick/lot into a later interval.
    let at = listing.effective_to.unwrap_or(listing.effective_from);
    let term = listing
        .terms
        .iter()
        .rfind(|term| term.effective_from <= at)
        .or_else(|| listing.terms.first())
        .ok_or_else(|| anyhow::anyhow!("listing has no trading terms to record"))?;
    Ok(ListingStateRecord {
        instrument_id: instrument_id.clone(),
        venue: listing.venue,
        symbol: listing.venue_symbol.clone(),
        token: listing.token.clone(),
        status: listing.status,
        settlement_regime: listing.settlement_regime,
        z_category: listing.z_category,
        settlement_group: listing.settlement_group.clone(),
        market_lot: term.market_lot,
        tick_assignment: term.tick_assignment.clone(),
        valid_from: listing.effective_from,
        valid_to: listing.effective_to,
        recorded_at,
        provenance: provenance.clone(),
    })
}

/// Whether two effective intervals [from, to] (open-ended when None)
/// intersect (inclusive both ends).
fn intervals_overlap(a_from: Date, a_to: Option<Date>, b_from: Date, b_to: Option<Date>) -> bool {
    let a_starts_before_b_ends = b_to.is_none_or(|b_end| a_from <= b_end);
    let b_starts_before_a_ends = a_to.is_none_or(|a_end| b_from <= a_end);
    a_starts_before_b_ends && b_starts_before_a_ends
}

/// Reconcile `incoming` against `existing` as of `as_of`, observed at
/// `recorded_at` from `provenance`.
///
/// Lineage is enforced: the observation cannot claim to predate the source
/// it was read from (backdating is rejected, as for corporate actions).
///
/// `full_snapshot` must be `true` only when `incoming` is the complete master
/// for its venue(s); then open listings on covered venues of instruments
/// absent from `incoming` are closed, and an instrument is reported removed
/// when no open listing remains. Seen-tracking is per `(ISIN, venue)`.
/// When `false`, absent instruments are left untouched.
pub fn reconcile(
    existing: &[Instrument],
    incoming: Vec<NormalizedListing>,
    as_of: Date,
    full_snapshot: bool,
    recorded_at: Timestamp,
    provenance: &SourceProvenance,
) -> Result<ReconciledUniverse, anyhow::Error> {
    provenance.validate()?;
    anyhow::ensure!(
        recorded_at >= provenance.retrieved_at,
        "reconciliation knowledge time precedes the source retrieval time"
    );
    for instrument in existing {
        instrument.validate()?;
    }

    let covered_venues: BTreeSet<String> = incoming
        .iter()
        .map(|row| venue_name(row.venue).to_owned())
        .collect();

    // One candidate per (ISIN, venue): the same ISIN on both NSE and BSE
    // is the normal multi-venue case, never a duplicate.
    let mut by_key: BTreeMap<(String, String), NormalizedListing> = BTreeMap::new();
    for row in incoming {
        anyhow::ensure!(
            row.market_lot > 0,
            "market lot must be positive for {}",
            row.venue_symbol
        );
        row.tick_assignment.validate()?;
        let key = (
            row.isin.as_str().to_owned(),
            venue_name(row.venue).to_owned(),
        );
        anyhow::ensure!(
            !by_key.contains_key(&key),
            "duplicate ISIN {} on {} in one ingest",
            row.isin,
            key.1
        );
        by_key.insert(key, row);
    }

    // Index existing instruments by ISIN (cross-venue identity) and by
    // (venue, symbol) listings (collision detection).
    let mut isin_index: BTreeMap<String, usize> = BTreeMap::new();
    for (index, instrument) in existing.iter().enumerate() {
        if let Some(isin) = instrument.isin.as_ref() {
            isin_index.insert(isin.as_str().to_owned(), index);
        }
    }

    let mut instruments: Vec<Instrument> = existing.to_vec();
    let mut reconciliation = Reconciliation::default();
    let mut states: Vec<ListingStateRecord> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

    for row in by_key.into_values() {
        seen.insert((
            row.isin.as_str().to_owned(),
            venue_name(row.venue).to_owned(),
        ));
        if let Some(&index) = isin_index.get(row.isin.as_str()) {
            let instrument = &mut instruments[index];
            let id = instrument.instrument_id.clone();
            if let Some((closed_listing, opened_listing)) =
                merge_listing(instrument, &row, recorded_at, provenance, &mut states)?
            {
                reconciliation.changed.push(InstrumentChange {
                    instrument_id: id,
                    closed_listing,
                    opened_listing,
                });
            }
        } else {
            // Fresh instrument: the (venue, symbol) may be reused only when
            // it does not overlap an existing listing of the same venue of
            // another instrument (historical reuse after an ended listing
            // is allowed).
            let venue_label = venue_name(row.venue).to_owned();
            for (index, instrument) in instruments.iter().enumerate() {
                let _ = index;
                for listing in &instrument.listings {
                    if listing.venue == row.venue
                        && listing.venue_symbol == row.venue_symbol
                        && intervals_overlap(
                            listing.effective_from,
                            listing.effective_to,
                            row.as_of,
                            None,
                        )
                    {
                        anyhow::bail!(
                            "symbol {venue_label}:{} collides with a listing of another instrument; refusing to merge",
                            row.venue_symbol
                        );
                    }
                }
            }
            let id = bootstrap_id(&row.isin)?;
            // A bootstrap id that lands on a different instrument is a
            // contradiction in the prior catalogue, not a new instrument.
            anyhow::ensure!(
                !instruments
                    .iter()
                    .any(|instrument| instrument.instrument_id == id),
                "bootstrap id {id} collides with an existing instrument"
            );
            let listing = new_listing(&row);
            states.push(state_record(&id, &listing, recorded_at, provenance)?);
            instruments.push(Instrument {
                instrument_type: row.instrument_type,
                instrument_id: id.clone(),
                isin: Some(row.isin.clone()),
                name: row.name.clone(),
                underlying_instrument_id: None,
                expiry_date: None,
                strike_price_paise: None,
                option_kind: None,
                listings: vec![listing],
                corporate_actions: vec![],
            });
            isin_index.insert(row.isin.as_str().to_owned(), instruments.len() - 1);
            reconciliation.added.push(id);
        }
    }

    if full_snapshot {
        for instrument in &mut instruments {
            let Some(isin) = instrument.isin.clone() else {
                continue;
            };
            let id = instrument.instrument_id.clone();
            for listing in &mut instrument.listings {
                if listing.effective_to.is_none()
                    && covered_venues.contains(venue_name(listing.venue))
                    && !seen.contains(&(
                        isin.as_str().to_owned(),
                        venue_name(listing.venue).to_owned(),
                    ))
                {
                    let close_date = previous_day(as_of)?;
                    anyhow::ensure!(
                        close_date >= listing.effective_from,
                        "as_of {as_of} predates the open listing of {id}; refusing to restate"
                    );
                    listing.effective_to = Some(close_date);
                    states.push(state_record(&id, listing, recorded_at, provenance)?);
                }
            }
            let still_open = instrument
                .listings
                .iter()
                .any(|listing| listing.effective_to.is_none());
            if !still_open {
                reconciliation
                    .removed
                    .push(instrument.instrument_id.clone());
            }
        }
    }

    for instrument in &instruments {
        instrument.validate()?;
    }
    instruments.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
    reconciliation.added.sort();
    reconciliation.removed.sort();
    reconciliation
        .changed
        .sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
    states.sort_by(|a, b| {
        a.instrument_id
            .cmp(&b.instrument_id)
            .then_with(|| (a.venue as u8).cmp(&(b.venue as u8)))
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then_with(|| a.valid_from.cmp(&b.valid_from))
    });
    Ok(ReconciledUniverse {
        instruments,
        reconciliation,
        listing_states: states,
    })
}

/// `(closed, opened)` labels when `merge_listing` changed anything.
pub type ListingMerge = Option<(Option<String>, Option<String>)>;

fn new_listing(row: &NormalizedListing) -> Listing {
    Listing {
        venue: row.venue,
        segment: row.segment,
        venue_symbol: row.venue_symbol.clone(),
        token: row.token.clone(),
        effective_from: row.as_of,
        effective_to: None,
        status: row.status,
        settlement_regime: row.settlement_regime,
        z_category: row.z_category,
        settlement_group: row.settlement_group.clone(),
        terms: vec![ListingTerms {
            effective_from: row.as_of,
            effective_to: None,
            tick_assignment: row.tick_assignment.clone(),
            market_lot: row.market_lot,
            quantity_increment: QUANTITY_INCREMENT,
        }],
    }
}

/// Whether the open listing `existing` carries the same listing *state* as
/// `row` (symbol, status, regime, Z-category, settlement group, token, tick
/// assignment, market lot). Anything differing closes and reopens the
/// interval. The comparison uses the terms in force on `row.as_of`, so an
/// old term interval with a coincidentally equal tick cannot mask a change.
fn same_state(existing: &Listing, row: &NormalizedListing) -> bool {
    let terms = existing
        .terms
        .iter()
        .find(|term| term.is_valid_on(row.as_of));
    existing.venue_symbol == row.venue_symbol
        && existing.status == row.status
        && existing.settlement_regime == row.settlement_regime
        && existing.z_category == row.z_category
        && existing.settlement_group == row.settlement_group
        && existing.token == row.token
        && terms.is_some_and(|term| {
            term.tick_assignment == row.tick_assignment && term.market_lot == row.market_lot
        })
}

/// Close every open listing of `instrument` on `venue` at `as_of - 1`,
/// appending the closed state records it creates. Returns the closed
/// listing's label when anything closed.
fn close_open_listings(
    instrument: &mut Instrument,
    venue: Venue,
    as_of: Date,
    recorded_at: Timestamp,
    provenance: &SourceProvenance,
    states: &mut Vec<ListingStateRecord>,
) -> Result<Option<String>, anyhow::Error> {
    let mut closed: Option<String> = None;
    for listing in &mut instrument.listings {
        if listing.venue == venue && listing.effective_to.is_none() {
            let close_date = previous_day(as_of)?;
            anyhow::ensure!(
                close_date >= listing.effective_from,
                "as_of {as_of} predates the open {}:{} listing; refusing to restate",
                venue_name(venue),
                listing.venue_symbol
            );
            let symbol = listing.venue_symbol.clone();
            listing.effective_to = Some(close_date);
            states.push(state_record(
                &instrument.instrument_id,
                listing,
                recorded_at,
                provenance,
            )?);
            closed = Some(listing_label(venue, &symbol));
        }
    }
    Ok(closed)
}

/// Merge one candidate into `instrument`. History is preserved: any state
/// change closes the old interval and opens a new one — never rewriting
/// history in place (including tick reassignments: a new tick revision
/// applies from its own effective date, never retroactively).
fn merge_listing(
    instrument: &mut Instrument,
    row: &NormalizedListing,
    recorded_at: Timestamp,
    provenance: &SourceProvenance,
    states: &mut Vec<ListingStateRecord>,
) -> Result<ListingMerge, anyhow::Error> {
    // Unchanged open listing state on this venue: nothing to do.
    if instrument.listings.iter().any(|listing| {
        listing.venue == row.venue && listing.effective_to.is_none() && same_state(listing, row)
    }) {
        return Ok(None);
    }

    // Copy the new listing out, then close and open with its states.
    let closed = close_open_listings(
        instrument,
        row.venue,
        row.as_of,
        recorded_at,
        provenance,
        states,
    )?;
    let listing = new_listing(row);
    let opened = listing_label(row.venue, &row.venue_symbol);
    states.push(state_record(
        &instrument.instrument_id,
        &listing,
        recorded_at,
        provenance,
    )?);
    instrument.listings.push(listing);
    Ok(Some((closed, Some(opened))))
}
