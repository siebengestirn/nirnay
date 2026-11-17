//! Trading-calendar ingestion from exchange holiday files.
//!
//! The domain owns [`TradingCalendar`] (`crates/domain`); this module owns
//! the file layout: `DATE,DAY,HOLIDAY,PURPOSE` in `DD-MMM-YYYY` or ISO dates.
//!
//! A holiday file lists *exceptions*, not the full session calendar: equities
//! trade on every weekday other than the declared holidays, and special
//! sessions (such as a Muhurat session on a Sunday) are announced separately.
//! [`build_calendar`] therefore expands the exchange's trading-week rule over
//! a date range, applies the exception file, and keeps the source provenance.
//! A raw holiday parse is never a complete calendar on its own.
//!
//! Classification is *explicit* (reviewer finding 2), never substring-only:
//!
//! | Purpose text (upper-cased)                                 | Class  |
//! | ------------------------------------------------------------ | ------ |
//! | contains `HOLIDAY` (incl. `TRADING HOLIDAY`)                  | Closed |
//! | contains `SPECIAL` or `LIVE TRADING SESSION`                  | Special|
//! | contains `TRADING` without `HOLIDAY`                            | Trading|
//! | anything else                                                 | **rejected** |
//!
//! The order matters: `TRADING HOLIDAY` *contains* `TRADING`, yet means the
//! exchange is closed. Unknown descriptions fail closed — a maintenance
//! window, a misspelled purpose, or an empty string must never silently
//! become a trading session.

use chrono::{Datelike, NaiveDate, Weekday};
use india_domain::{CalendarDay, Segment, SessionDay, TradingCalendar, Venue};

use crate::util::sha256_hex;

use super::archive::SourceArchive;
use super::csv::{column, optional_column, split_csv_file};
use super::provenance::SourceProvenance;

/// A complete daily calendar together with the source that produced it.
///
/// The calendar is dense (one entry per date in the requested range), so
/// [`TradingCalendar::covers_month`] can be proved. Provenance is bound to
/// the exact holiday-source bytes: the fields are private and the only
/// production constructor verifies the SHA-256, venue, cash segment and
/// record count before the calendar exists. A monthly tick assignment can
/// therefore only cite a calendar whose inputs are auditable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltCalendar {
    calendar: TradingCalendar,
    provenance: Option<SourceProvenance>,
}

impl BuiltCalendar {
    /// The dense calendar used by tick assignment and session checks.
    #[must_use]
    pub fn calendar(&self) -> &TradingCalendar {
        &self.calendar
    }

    /// Source provenance bound to the exact holiday bytes, when this calendar
    /// was built through [`build_calendar`].
    #[must_use]
    pub fn provenance(&self) -> Option<&SourceProvenance> {
        self.provenance.as_ref()
    }

    /// `{source_id}:{source_format_version}` of the holiday source.
    ///
    /// Fails closed for a bare calendar: a revision identity cannot be
    /// invented for a calendar that names no source.
    pub fn revision(&self) -> Result<String, anyhow::Error> {
        let source = self.require_provenance()?;
        Ok(format!(
            "{}:{}",
            source.source_id, source.source_format_version
        ))
    }

    /// SHA-256 of the exact holiday-source bytes.
    pub fn sha256_hex(&self) -> Result<&str, anyhow::Error> {
        Ok(self.require_provenance()?.sha256_hex.as_str())
    }

    /// Require that this calendar was built from an auditable source.
    ///
    /// Used by the monthly tick path: a hand-built calendar with false
    /// holiday entries must never fix an exchange tick.
    pub fn require_provenance(&self) -> Result<&SourceProvenance, anyhow::Error> {
        self.provenance.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "calendar for {:?} has no source provenance; a bare calendar cannot fix a tick assignment",
                self.calendar.venue
            )
        })
    }

    /// Wrap a hand-built calendar for tests that exercise non-lineage
    /// behaviour (coverage, session classification, rejection paths).
    ///
    /// The result carries no provenance and is therefore rejected by
    /// every monthly tick-assignment path. This is the only constructor that
    /// bypasses source verification; production code must use
    /// [`build_calendar`].
    #[must_use]
    pub fn synthetic_for_tests(calendar: TradingCalendar) -> Self {
        Self {
            calendar,
            provenance: None,
        }
    }
}

/// Build a complete daily calendar for `venue` over `[from, to]`.
///
/// Exchange equities trade on the standard trading week (Monday to Friday)
/// unless the exception file marks the date `Closed` (a declared holiday) or
/// `Special` (an announced special session, which may fall on a weekend).
/// Every date in the range receives an entry, so the result can prove month
/// coverage rather than inferring it from the absence of a holiday row.
/// Exceptions outside `[from, to]` cannot affect the requested range and are
/// ignored; the provenance still records the whole input file.
///
/// The holiday CSV is parsed here, not by the caller, and the provenance must
/// describe *these exact bytes*: the SHA-256 is recomputed, the venue and cash
/// segment must match, and the recorded row count must equal the number of
/// parsed exceptions. The source must also be retained in `archive` under the
/// same dataset identity, so the assignment's calendar hash names recoverable
/// bytes rather than an untracked file. A provenance record for a different
/// file, exchange, segment, row count or archive entry fails closed.
pub fn build_calendar(
    venue: Venue,
    from: NaiveDate,
    to: NaiveDate,
    holiday_csv: &str,
    provenance: SourceProvenance,
    archive: &SourceArchive,
) -> Result<BuiltCalendar, anyhow::Error> {
    anyhow::ensure!(from <= to, "calendar range {from}..{to} is inverted");
    provenance.validate()?;
    anyhow::ensure!(
        provenance.venue == venue,
        "calendar provenance {} is for {:?}, not {:?}",
        provenance.source_id,
        provenance.venue,
        venue
    );
    anyhow::ensure!(
        provenance.segment == Segment::Cash,
        "calendar provenance {} covers {:?}; a cash calendar requires the cash segment",
        provenance.source_id,
        provenance.segment
    );
    let actual_hash = sha256_hex(holiday_csv.as_bytes());
    anyhow::ensure!(
        actual_hash == provenance.sha256_hex,
        "calendar provenance hash {} does not match the supplied holiday bytes ({actual_hash}); the calendar cannot be reproduced",
        provenance.sha256_hex
    );
    let archived = archive.fetch(&provenance.sha256_hex)?;
    anyhow::ensure!(
        archived == holiday_csv.as_bytes(),
        "archived calendar source {} does not contain the supplied holiday bytes",
        provenance.sha256_hex
    );
    anyhow::ensure!(
        archive.entries()?.iter().any(|entry| {
            entry.sha256_hex == provenance.sha256_hex && entry.source_id == provenance.source_id
        }),
        "calendar source {} is not archived under dataset {}",
        provenance.sha256_hex,
        provenance.source_id
    );
    let exceptions = parse_holiday_csv(venue, holiday_csv)?;
    anyhow::ensure!(
        provenance.record_count == exceptions.days.len(),
        "calendar provenance records {} rows but the holiday file parsed {} exceptions",
        provenance.record_count,
        exceptions.days.len()
    );
    build_dense_calendar(venue, from, to, &exceptions, provenance)
}

fn build_dense_calendar(
    venue: Venue,
    from: NaiveDate,
    to: NaiveDate,
    exceptions: &TradingCalendar,
    provenance: SourceProvenance,
) -> Result<BuiltCalendar, anyhow::Error> {
    anyhow::ensure!(
        exceptions.venue == venue,
        "exception calendar is for {:?}, not {:?}",
        exceptions.venue,
        venue
    );

    let mut days = Vec::new();
    let mut current = from;
    loop {
        let exception = exceptions.days.iter().find(|day| day.date == current);
        let session = exception.map_or_else(
            || {
                if is_trading_weekday(current) {
                    SessionDay::Trading
                } else {
                    SessionDay::Closed
                }
            },
            |day| day.session,
        );
        days.push(CalendarDay {
            venue,
            date: current,
            session,
            note: exception.and_then(|day| day.note.clone()),
        });
        if current == to {
            break;
        }
        current = current
            .succ_opt()
            .ok_or_else(|| anyhow::anyhow!("calendar range overflows the date space"))?;
    }

    let calendar = TradingCalendar { venue, days };
    calendar.validate()?;
    Ok(BuiltCalendar {
        calendar,
        provenance: Some(provenance),
    })
}

/// Whether the standard equity trading week includes `date`.
///
/// NSE and BSE cash equities do not trade on Saturday or Sunday; a Sunday
/// Muhurat session is represented as a `Special` exception, never as a normal
/// weekend session.
#[must_use]
pub fn is_trading_weekday(date: NaiveDate) -> bool {
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Parse a holiday calendar for `venue`.
///
/// Fails closed on a bad date, a duplicate day, or an unclassifiable
/// purpose; the result is validated by [`TradingCalendar::validate`].
pub fn parse_holiday_csv(venue: Venue, csv: &str) -> Result<TradingCalendar, anyhow::Error> {
    let (header, rows) = split_csv_file(csv)?;
    let date_col = column(&header, &["DATE", "DAY DATE"])?;
    let purpose_col = optional_column(&header, &["PURPOSE", "DESCRIPTION", "TYPE"]);
    let note_col = optional_column(&header, &["HOLIDAY", "NOTE", "NAME"]);

    let mut days = Vec::with_capacity(rows.len());
    for (offset, row) in rows.iter().enumerate() {
        let line = offset + 2;
        let date_text = row[date_col].trim();
        let date = NaiveDate::parse_from_str(date_text, "%d-%b-%Y")
            .or_else(|_| NaiveDate::parse_from_str(date_text, "%Y-%m-%d"))
            .map_err(|_| anyhow::anyhow!("line {line}: unparseable DATE {date_text}"))?;
        let purpose = match purpose_col {
            Some(index) => row[index].trim().to_uppercase(),
            None => String::new(),
        };
        let session = classify_purpose(&purpose).ok_or_else(|| {
            anyhow::anyhow!(
                "line {line}: purpose \"{purpose}\" is not classifiable; refusing to guess"
            )
        })?;
        let note = match note_col {
            Some(index) => {
                let text = row[index].trim().to_owned();
                (!text.is_empty()).then_some(text)
            }
            None => None,
        };
        days.push(CalendarDay {
            venue,
            date,
            session,
            note,
        });
    }
    days.sort_by_key(|day| day.date);
    let calendar = TradingCalendar { venue, days };
    calendar.validate()?;
    Ok(calendar)
}

/// NSE holiday file: parse with [`Venue::Nse`].
pub fn parse_nse_holiday_csv(csv: &str) -> Result<TradingCalendar, anyhow::Error> {
    parse_holiday_csv(Venue::Nse, csv)
}

/// Classify one upper-cased purpose into a session class, or `None` when it
/// must be rejected. Holiday classification wins over trading (a
/// `TRADING HOLIDAY` is closed), and special sessions beat generic trading
/// text; anything outside this table is an error, never a default.
#[must_use]
pub fn classify_purpose(purpose_upper: &str) -> Option<SessionDay> {
    if purpose_upper.contains("HOLIDAY") {
        Some(SessionDay::Closed)
    } else if purpose_upper.contains("SPECIAL") || purpose_upper.contains("LIVE TRADING") {
        Some(SessionDay::Special)
    } else if purpose_upper.contains("TRADING") {
        Some(SessionDay::Trading)
    } else {
        None
    }
}
