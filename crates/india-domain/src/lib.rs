//! Canonical Indian financial instrument and listing domain (task P1-01/P1-02).

use serde::{Deserialize, Serialize};

pub mod corporate_actions;
pub mod instrument;
pub mod provenance;
pub mod reference_history;

pub use corporate_actions::{CorporateActionEvent, CorporateActionLedger};
pub use provenance::{PROVENANCE_SCHEMA_VERSION, SourceProvenance, provenance_for_source};
pub use reference_history::{ProviderListingHistory, ProviderListingMapping};

pub use instrument::{
    CorporateAction, CorporateActionKind, CorporateActionWire, Instrument, InstrumentId,
    InstrumentType, InstrumentWire, InstrumentWireType, Isin, Listing, ListingStatus, ListingTerms,
    ListingTermsWire, ListingWire, Lots, OptionKind, Paise, Segment, SettlementRegime,
    TickAssignment, TickAssignmentWire, TickBand, TickBandWire, TickSchedule, TickScheduleWire,
    Venue, adjusted_price_paise, is_sha256_hex, month_start,
};

/// Trading-calendar day classification for an exchange (task P1: calendars).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionDay {
    /// Regular trading session.
    Trading,
    /// Holiday: no session.
    Closed,
    /// Special session (e.g. live-testing hour announced by the exchange).
    Special,
}

/// One calendar day for a venue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarDay {
    pub venue: Venue,
    pub date: chrono::NaiveDate,
    pub session: SessionDay,
    /// Optional description (holiday name, special-session purpose).
    pub note: Option<String>,
}

/// A venue trading calendar. Point-in-time: lookups are by date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingCalendar {
    pub venue: Venue,
    pub days: Vec<CalendarDay>,
}

impl TradingCalendar {
    #[must_use]
    pub fn is_trading_day(&self, date: chrono::NaiveDate) -> bool {
        self.days
            .iter()
            .any(|day| day.date == date && day.session != SessionDay::Closed)
    }

    /// The next trading day strictly after `date`, bounded by the calendar's
    /// known entries. Returns None when the calendar has no later known
    /// session; callers must fetch more calendar data rather than guess.
    #[must_use]
    pub fn next_trading_day(&self, date: chrono::NaiveDate) -> Option<chrono::NaiveDate> {
        self.days
            .iter()
            .filter(|day| day.date > date && day.session != SessionDay::Closed)
            .map(|day| day.date)
            .min()
    }

    /// Whether the calendar has an entry for **every** date of the month
    /// starting at `month_start`.
    ///
    /// A holiday file only lists exceptions, so its absence of a date is not
    /// evidence that the date is a session *or* a holiday. Callers that need
    /// "the last trading session of the month" must require this coverage
    /// proof: a sparse calendar can otherwise make an early session look
    /// like the last one.
    #[must_use]
    pub fn covers_month(&self, month_start: chrono::NaiveDate) -> bool {
        use chrono::Datelike;

        let mut expected = month_start;
        loop {
            if !self.days.iter().any(|day| day.date == expected) {
                return false;
            }
            let Some(next) = expected.succ_opt() else {
                return true;
            };
            if next.month() != month_start.month() || next.year() != month_start.year() {
                return true;
            }
            expected = next;
        }
    }

    /// Validate: unique dates per venue, consistent venue, chronologically
    /// ordered entries. Rejects malformed calendars at construction/ingest.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        let mut previous: Option<chrono::NaiveDate> = None;
        let mut seen: std::collections::BTreeSet<chrono::NaiveDate> =
            std::collections::BTreeSet::new();
        for day in &self.days {
            anyhow::ensure!(
                day.venue == self.venue,
                "calendar for {:?} contains a day for {:?}",
                self.venue,
                day.venue
            );
            anyhow::ensure!(
                seen.insert(day.date),
                "calendar for {:?} contains duplicate date {}",
                self.venue,
                day.date
            );
            if let Some(last) = previous {
                anyhow::ensure!(
                    day.date >= last,
                    "calendar for {:?} is not chronologically ordered",
                    self.venue
                );
            }
            previous = Some(day.date);
        }
        Ok(())
    }
}
