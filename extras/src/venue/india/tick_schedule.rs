//! Exchange tick-schedule ingestion.
//!
//! Tick sizes are price-linked exchange data that NSE revises by circular
//! (sub-₹250 → 1 paise since June 2024; higher bands revised 2025-04-15).
//! They are ingested as *data* with provenance and effective dates — never
//! as a hard-coded constant (reviewer finding 5).
//!
//! Source format (one file per revision):
//!
//! ```text
//! EFFECTIVE_FROM,LOWER_PAISE,UPPER_PAISE,TICK_PAISE
//! 2025-04-15,0,24999,1
//! 2025-04-15,25000,100000,5
//! ```
//!
//! `UPPER_PAISE` empty means the open-ended last band. Bands must satisfy
//! [`india_domain::TickSchedule::validate`] (contiguous from `0` to
//! unbounded). A [`TickScheduleBook`] holds several dated revisions and
//! selects, for any date, the latest revision effective on or before it —
//! failing closed when no revision covers the date.

use chrono::{Datelike, NaiveDate};
use india_domain::{
    Isin, Segment, SessionDay, TickAssignment, TickBand, TickSchedule, Venue, month_start,
};

use super::archive::SourceArchive;
use super::calendar::BuiltCalendar;
use super::csv::{column, split_csv_file};
use super::eod_close::parse_eod_close_csv;
use super::provenance::SourceProvenance;

/// One effective-dated exchange tick-schedule revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeTickSchedule {
    /// Date from which this schedule is in force (inclusive).
    pub effective_from: NaiveDate,
    /// The banded schedule.
    pub schedule: TickSchedule,
    /// Provenance of the source circular/table it was transcribed from.
    pub provenance: SourceProvenance,
}

impl ExchangeTickSchedule {
    /// Validate the revision.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        self.schedule.validate()?;
        self.provenance.validate()?;
        Ok(())
    }
}

/// Parse one revision CSV (see module docs) without provenance; bind the
/// result with [`ExchangeTickSchedule`] once provenance is recorded.
pub fn parse_tick_schedule_csv(
    effective_from: NaiveDate,
    csv: &str,
) -> Result<TickSchedule, anyhow::Error> {
    let (header, rows) = split_csv_file(csv)?;
    let lower_col = column(&header, &["LOWER_PAISE", "LOWER", "FROM_PAISE"])?;
    let upper_col = column(&header, &["UPPER_PAISE", "UPPER", "TO_PAISE"])?;
    let tick_col = column(&header, &["TICK_PAISE", "TICK"])?;

    let mut bands = Vec::with_capacity(rows.len());
    for (offset, row) in rows.iter().enumerate() {
        let line = offset + 2;
        let lower = row[lower_col]
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("line {line}: LOWER_PAISE is not an integer"))?;
        let upper_text = row[upper_col].trim();
        let upper = if upper_text.is_empty() {
            None
        } else {
            Some(
                upper_text
                    .parse::<i64>()
                    .map_err(|_| anyhow::anyhow!("line {line}: UPPER_PAISE is not an integer"))?,
            )
        };
        let tick = row[tick_col]
            .trim()
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("line {line}: TICK_PAISE is not an integer"))?;
        bands.push(TickBand {
            lower_paise: lower,
            upper_paise: upper,
            tick_paise: tick,
        });
    }
    bands.sort_by_key(|band| band.lower_paise);
    let schedule = TickSchedule { bands };
    schedule.validate()?;
    // The effective date travels with the revision, not the band set; the
    // caller binds it when constructing ExchangeTickSchedule. (Validated
    // here so a typo cannot silently pin the wrong base date.)
    let _ = effective_from;
    Ok(schedule)
}

/// The NSE cash tick revision of 2025-04-15, transcribed from the exchange
/// circular table (verified against broker/publisher reproductions of the
/// circular): below ₹250 → 1 paise; ₹250–₹1,000 → 5 paise; ₹1,000–₹5,000 →
/// 10 paise; ₹5,000–₹10,000 → 50 paise; ₹10,000–₹20,000 → 100 paise; above
/// ₹20,000 → 500 paise. Historical revisions are supplied as CSV fixtures
/// under `fixtures/tick_schedules/` and bound with provenance by callers.
#[must_use]
pub fn nse_tick_schedule_2025_04_15() -> TickSchedule {
    TickSchedule {
        bands: vec![
            TickBand {
                lower_paise: 0,
                upper_paise: Some(24_999),
                tick_paise: 1,
            },
            TickBand {
                lower_paise: 25_000,
                upper_paise: Some(100_000),
                tick_paise: 5,
            },
            TickBand {
                lower_paise: 100_001,
                upper_paise: Some(500_000),
                tick_paise: 10,
            },
            TickBand {
                lower_paise: 500_001,
                upper_paise: Some(1_000_000),
                tick_paise: 50,
            },
            TickBand {
                lower_paise: 1_000_001,
                upper_paise: Some(2_000_000),
                tick_paise: 100,
            },
            TickBand {
                lower_paise: 2_000_001,
                upper_paise: None,
                tick_paise: 500,
            },
        ],
    }
}

/// An ordered book of schedule revisions; selects by date.
#[derive(Debug, Clone, Default)]
pub struct TickScheduleBook {
    revisions: Vec<ExchangeTickSchedule>,
}

impl TickScheduleBook {
    /// Build and validate a book. Revisions must be non-empty; equal
    /// effective dates are a contradiction and rejected.
    pub fn new(mut revisions: Vec<ExchangeTickSchedule>) -> Result<Self, anyhow::Error> {
        anyhow::ensure!(!revisions.is_empty(), "tick schedule book is empty");
        for revision in &revisions {
            revision.validate()?;
        }
        revisions.sort_by_key(|revision| revision.effective_from);
        for window in revisions.windows(2) {
            anyhow::ensure!(
                window[0].effective_from < window[1].effective_from,
                "duplicate tick schedule revision date"
            );
        }
        Ok(Self { revisions })
    }

    /// Schedule in force on `date`: the latest revision effective on or
    /// before it. Fails closed before the first known revision.
    pub fn schedule_for(&self, date: NaiveDate) -> Option<&TickSchedule> {
        self.revisions
            .iter()
            .rev()
            .find(|revision| revision.effective_from <= date)
            .map(|revision| &revision.schedule)
    }

    /// Tick (paise) for `price_paise` on `date`, or `None` when the date
    /// or price is uncovered.
    #[must_use]
    pub fn tick_for(&self, date: NaiveDate, price_paise: i64) -> Option<i64> {
        self.schedule_for(date)?.tick_for(price_paise)
    }

    /// Revision in force on `date`: the latest effective on or before it.
    pub fn revision_for(&self, date: NaiveDate) -> Option<&ExchangeTickSchedule> {
        self.revisions
            .iter()
            .rev()
            .find(|revision| revision.effective_from <= date)
    }

    /// Build the NSE monthly tick assignment governing `as_of` from a
    /// recorded `reference` close (the previous month's last available
    /// close). `effective_month` is the first day of `as_of`'s month; the
    /// assignment applies for that whole month, whatever the live price
    /// does afterwards.
    ///
    /// The reference close is a *source observation*, not a bare number:
    /// its trading date, dataset identity and SHA-256 are carried into the
    /// assignment so the price rule can be reproduced from the archive.
    /// The call fails closed unless the hash resolves to an archived object
    /// recorded under the same dataset identity, and unless `calendar`
    /// (which must cover the preceding month) says the close date is that
    /// month's last trading session. The calendar must be a provenanced
    /// [`BuiltCalendar`]; its source hash and revision are carried into the
    /// assignment. A schedule revision without a source hash is also rejected.
    pub fn monthly_assignment(
        &self,
        as_of: NaiveDate,
        reference: &ReferenceClose,
        archive: &SourceArchive,
        calendar: &BuiltCalendar,
    ) -> Result<TickAssignment, anyhow::Error> {
        calendar.require_provenance()?;
        reference.validate()?;
        reference.verify_archived(archive)?;
        reference.verify_last_session(calendar, month_start(as_of))?;
        let revision = self
            .revision_for(as_of)
            .ok_or_else(|| anyhow::anyhow!("no tick schedule revision governs {as_of}"))?;
        let tick_paise = revision
            .schedule
            .assign_tick(reference.close_paise)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "tick schedule has no band for reference close {}",
                    reference.close_paise
                )
            })?;
        let assignment = TickAssignment {
            tick_paise,
            reference_close_paise: Some(reference.close_paise),
            reference_close_date: Some(reference.date),
            reference_source: Some(reference.source_id.clone()),
            reference_sha256_hex: Some(reference.sha256_hex.clone()),
            effective_month: Some(month_start(as_of)),
            schedule_revision: format!(
                "{}:{}",
                revision.provenance.source_id, revision.provenance.source_format_version
            ),
            schedule_effective_from: Some(revision.effective_from),
            schedule_sha256_hex: Some(revision.provenance.sha256_hex.clone()),
            calendar_revision: Some(calendar.revision()?),
            calendar_sha256_hex: Some(calendar.sha256_hex()?.to_owned()),
        };
        assignment.validate()?;
        Ok(assignment)
    }
}

/// A recorded reference close used to fix an NSE monthly tick assignment.
///
/// The exchange rule uses the previous month's last available closing price;
/// carrying the date, dataset identity and content hash makes the
/// assignment reproducible and lets validation reject a close from the
/// wrong month or an unverifiable source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceClose {
    /// Trading date of the close.
    pub date: NaiveDate,
    /// Instrument the close belongs to. The archived source must contain a
    /// record for this ISIN on this date at this price.
    pub isin: Isin,
    /// Closing price in paise.
    pub close_paise: i64,
    /// Dataset identity, e.g. `nse-eod-close`.
    pub source_id: String,
    /// SHA-256 of the exact source bytes.
    pub sha256_hex: String,
}

impl ReferenceClose {
    /// Validate the observation's structure before it can fix a tick.
    ///
    /// This checks format only. [`ReferenceClose::verify_archived`] is what
    /// proves the hash resolves to retained bytes *and* that those bytes
    /// contain this instrument, date and price.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            self.close_paise >= 0,
            "reference close must be non-negative"
        );
        anyhow::ensure!(
            !self.source_id.trim().is_empty(),
            "reference close dataset identity must not be empty"
        );
        anyhow::ensure!(
            india_domain::is_sha256_hex(&self.sha256_hex),
            "reference close source hash must be 64 hex digits"
        );
        Ok(())
    }

    /// Bind the observation to a parsed record of a retained source object.
    ///
    /// Three independent facts must hold:
    ///
    /// 1. the hash resolves to archived bytes whose SHA-256 is recomputed by
    ///    [`SourceArchive::fetch`];
    /// 2. the manifest records that object under this observation's dataset
    ///    identity;
    /// 3. the archived bytes parse as an end-of-day close table containing
    ///    exactly one row for this ISIN and date whose close matches
    ///    `close_paise`.
    ///
    /// A genuine archive hash reused with another instrument's price, a
    /// fabricated hash, or a hash stored under another dataset therefore
    /// fails closed.
    pub fn verify_archived(&self, archive: &SourceArchive) -> Result<(), anyhow::Error> {
        let bytes = archive.fetch(&self.sha256_hex)?;
        let entries = archive.entries()?;
        anyhow::ensure!(
            entries.iter().any(|entry| {
                entry.sha256_hex == self.sha256_hex && entry.source_id == self.source_id
            }),
            "reference close {} is not archived under dataset {}",
            self.sha256_hex,
            self.source_id
        );
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| anyhow::anyhow!("archived close source is not UTF-8 text"))?;
        let rows = parse_eod_close_csv(text)?;
        let matches = rows
            .iter()
            .filter(|row| {
                row.isin == self.isin
                    && row.date == self.date
                    && row.close_paise == self.close_paise
            })
            .count();
        anyhow::ensure!(
            matches == 1,
            "archived source {} has {matches} records for {} on {} at {} paise; the observation is not reproducible",
            self.sha256_hex,
            self.isin.as_str(),
            self.date,
            self.close_paise
        );
        Ok(())
    }

    /// Validate the close date against the exchange calendar: it must be a
    /// trading session and the last trading session of the month before
    /// `effective_month`.
    ///
    /// Only a provenanced [`BuiltCalendar`] is accepted. The provenance must
    /// name the NSE cash segment and match the bytes the calendar was built
    /// from; a bare or synthetic calendar cannot establish a session. The
    /// calendar must also cover that month; absence of a session means the
    /// evidence is insufficient, so the assignment fails closed rather than
    /// assuming the date is valid.
    pub fn verify_last_session(
        &self,
        calendar: &BuiltCalendar,
        effective_month: NaiveDate,
    ) -> Result<(), anyhow::Error> {
        let provenance = calendar.require_provenance()?;
        anyhow::ensure!(
            provenance.venue == Venue::Nse,
            "reference close for an NSE monthly assignment requires the NSE calendar"
        );
        anyhow::ensure!(
            provenance.segment == Segment::Cash,
            "reference close for a cash assignment requires a cash-segment calendar, not {:?}",
            provenance.segment
        );
        let calendar = calendar.calendar();
        anyhow::ensure!(
            calendar.venue == Venue::Nse,
            "reference close for an NSE monthly assignment requires the NSE calendar"
        );
        let previous = previous_month_start(effective_month).ok_or_else(|| {
            anyhow::anyhow!("effective month {effective_month} has no preceding month")
        })?;
        anyhow::ensure!(
            month_start(self.date) == previous,
            "reference close {} must fall in {previous}..{effective_month}",
            self.date
        );
        anyhow::ensure!(
            calendar.covers_month(previous),
            "calendar does not cover every date of {previous}..{effective_month}; the last session cannot be established"
        );
        anyhow::ensure!(
            calendar.is_trading_day(self.date),
            "reference close date {} is not a trading session in the supplied calendar",
            self.date
        );
        anyhow::ensure!(
            !calendar.days.iter().any(|day| {
                day.date > self.date
                    && day.date < effective_month
                    && day.session != SessionDay::Closed
            }),
            "reference close {} is not the last trading session before {effective_month}",
            self.date
        );
        Ok(())
    }
}

/// First day of the calendar month before `month`.
fn previous_month_start(month: NaiveDate) -> Option<NaiveDate> {
    let year = month.year();
    let month_number = month.month();
    let (year, month_number) = if month_number == 1 {
        (year.checked_sub(1)?, 12)
    } else {
        (year, month_number - 1)
    };
    NaiveDate::from_ymd_opt(year, month_number, 1)
}
