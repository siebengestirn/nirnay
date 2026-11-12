//! Instrument lookup over the canonical India domain.
//!
//! There is exactly one instrument identity: `india_domain::InstrumentId`.
//! Symbols, tokens, ISINs and broker ids are lookup attributes, never
//! identity. A catalog rejects duplicate identities and invalid instruments
//! at construction.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::DomainError;

pub use india_domain::{
    CalendarDay, Instrument, InstrumentId, InstrumentType, Isin, Listing, ListingStatus,
    ListingTerms, OptionKind, Segment, SessionDay, SettlementRegime, TickAssignment,
    TradingCalendar, Venue,
};

/// Whether at least one listing is active on `date`.
#[must_use]
pub fn is_active_on(instrument: &Instrument, date: NaiveDate) -> bool {
    instrument.listings.iter().any(|listing| {
        listing.status == ListingStatus::Active
            && listing.effective_from <= date
            && listing.effective_to.is_none_or(|end| date <= end)
    })
}

/// Point-in-time instrument catalog keyed by canonical identity.
#[derive(Debug, Clone, Default)]
pub struct InstrumentCatalog {
    instruments: BTreeMap<InstrumentId, Instrument>,
}

impl InstrumentCatalog {
    pub fn new(instruments: Vec<Instrument>) -> Result<Self, DomainError> {
        let mut catalog = BTreeMap::new();
        for instrument in instruments {
            instrument
                .validate()
                .map_err(|error| DomainError::InvalidInstrument(error.to_string()))?;
            let identity = instrument.instrument_id.clone();
            if catalog.insert(identity.clone(), instrument).is_some() {
                return Err(DomainError::DuplicateInstrument(identity.to_string()));
            }
        }
        Ok(Self {
            instruments: catalog,
        })
    }

    /// The instrument only when it is active on `date`.
    #[must_use]
    pub fn at(&self, identity: &InstrumentId, date: NaiveDate) -> Option<&Instrument> {
        self.instruments
            .get(identity)
            .filter(|instrument| is_active_on(instrument, date))
    }

    /// The instrument regardless of listing dates. Use for reference data,
    /// not for execution eligibility.
    #[must_use]
    pub fn get(&self, identity: &InstrumentId) -> Option<&Instrument> {
        self.instruments.get(identity)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Instrument> {
        self.instruments.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.instruments.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instruments.is_empty()
    }
}
