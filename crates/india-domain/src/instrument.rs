//! Canonical Indian financial instrument and listing domain (task P1-01/P1-02).
//!
//! Versioned wire contract: `schemas/india/instrument.v2.json` (NSE/BSE cash
//! equities and ETFs). v1 is frozen. The canonical ingress path is
//! `InstrumentWire` (untrusted) -> [`Instrument`] (validated); direct
//! deserialization into domain types is deliberately unsupported. Instrument
//! identity is platform-internal and never derived from a venue symbol.
//! Exchanges (NSE/BSE) are venues; Zerodha/Groww are brokers and never part
//! of instrument identity. Prices are integer paise, quantities integer lots.

use chrono::NaiveDate;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Venue: a regulated exchange facility. Distinct from a broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Venue {
    Nse,
    Bse,
}

/// Segment supported by the v2 instrument contract. Other segments arrive
/// under separate versioned contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Segment {
    Cash,
}

impl Segment {
    /// Supported segment set for this contract version (single-authority
    /// list: the schema and this enum must change together).
    #[must_use]
    pub const fn v2_supported() -> [Self; 1] {
        [Self::Cash]
    }
}

/// Instrument family. Derivatives reference their underlying by
/// [`InstrumentId`], never by symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentType {
    Equity,
    Etf,
    Index,
    Future,
    Option,
    CurrencyPair,
    Bond,
}

/// Option kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    Call,
    Put,
}

/// Platform-internal stable identity. Never a ticker, MIC, ISIN or token.
/// The value is private: it can only be created through [`InstrumentId::new`],
/// and deserialization goes through the same validation, so no caller can smuggle
/// an invalid identity past the constructors.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstrumentId {
    value: String,
}

impl InstrumentId {
    pub fn new(value: impl Into<String>) -> Result<Self, anyhow::Error> {
        let value = value.into();
        anyhow::ensure!(
            !value.is_empty()
                && value
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && value.chars().all(|c| c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == '.'
                    || c == '_'
                    || c == '-'),
            "invalid instrument id `{value}`: must be lowercase alphanumeric with . _ - separators"
        );
        Ok(Self { value })
    }

    /// The canonical identity string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value)
    }
}

impl Serialize for InstrumentId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.value)
    }
}

impl<'de> Deserialize<'de> for InstrumentId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// ISIN: cross-venue identity where issued (empty only for indices). Kept as a
/// private value validated through [`Isin::new`]; deserialization enforces the
/// same format and ISO 6166 check-digit rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Isin {
    value: String,
}

impl Isin {
    pub fn new(value: impl Into<String>) -> Result<Self, anyhow::Error> {
        let value = value.into();
        let bytes = value.as_bytes();
        anyhow::ensure!(
            bytes.len() == 12
                && bytes[0].is_ascii_uppercase()
                && bytes[1].is_ascii_uppercase()
                && bytes[2..11].iter().all(|b| b.is_ascii_alphanumeric())
                && bytes[11].is_ascii_digit(),
            "invalid ISIN `{value}`: expected 2 letters + 9 alphanumerics + 1 check digit"
        );
        anyhow::ensure!(
            Self::check_digit_valid(&value),
            "invalid ISIN `{value}`: ISO 6166 check digit mismatch"
        );
        Ok(Self { value })
    }

    /// The ISIN string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// ISO 6166 check digit: Luhn algorithm over the base-36 digit stream
    /// produced by mapping letters to 10..=35.
    #[must_use]
    fn check_digit_valid(isin: &str) -> bool {
        let mut digits: Vec<u32> = Vec::with_capacity(24);
        for byte in isin.bytes().take(11) {
            if byte.is_ascii_digit() {
                digits.push(u32::from(byte - b'0'));
            } else {
                let letter = u32::from(byte.to_ascii_uppercase() - b'A') + 10;
                digits.push(letter / 10);
                digits.push(letter % 10);
            }
        }
        // Luhn with doubling from the rightmost digit of the doubled stream.
        let mut sum = 0u32;
        let mut double = true;
        for digit in digits.iter().rev() {
            let mut value = *digit;
            if double {
                value *= 2;
                if value > 9 {
                    value -= 9;
                }
            }
            double = !double;
            sum += value;
        }
        let expected = (10 - (sum % 10)) % 10;
        expected == u32::from(isin.as_bytes()[11] - b'0')
    }
}

impl std::fmt::Display for Isin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value)
    }
}

impl Serialize for Isin {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.value)
    }
}

impl<'de> Deserialize<'de> for Isin {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// Exact price in paise (1 INR = 100 paise). i64-compatible; never a float.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Paise(pub i64);

/// Exact quantity in lots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Lots(pub i64);

/// One venue admission of an instrument: symbol and token are scoped here.
/// Trading terms live here (not on the instrument), each with its own
/// validity interval: the terms that apply at a decision date are the terms
/// effective at that date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listing {
    pub venue: Venue,
    pub segment: Segment,
    pub venue_symbol: String,
    /// Venue-assigned numeric token (e.g. NSE instrument token).
    pub token: Option<String>,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub status: ListingStatus,
    /// Settlement regime while the listing is tradable.
    pub settlement_regime: SettlementRegime,
    /// Whether the exchange marks the security as a Z-category security
    /// (NSE `BZ` series): still trade-for-trade and tradable, but under
    /// added surveillance/compliance restrictions. Preserved separately
    /// from status — it is a designation, not a suspension.
    pub z_category: bool,
    /// Venue settlement group for this listing (BSE `Group Name`), when the
    /// venue publishes one. Part of the canonical listing state: a group
    /// change closes the old state and opens a new one, and historical
    /// queries return the group that was valid at the time.
    pub settlement_group: Option<String>,
    /// Exchange trading terms valid for this listing; at least one.
    pub terms: Vec<ListingTerms>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingTerms {
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    /// The tick size assigned to this instrument for this term interval.
    ///
    /// (NSE assigns tick sizes monthly from the previous month-end close;
    /// see [`TickAssignment`]. This is an assignment record, never a
    /// live price-band lookup.)
    pub tick_assignment: TickAssignment,
    /// Market lot in units.
    pub market_lot: i64,
    /// Quantity step in units.
    pub quantity_increment: i64,
}

/// Settlement regime of a listing. This is a property of how the security
/// settles, orthogonal to trading status: `BE` (trade-for-trade) securities
/// are tradable, under a distinct regime; they are never represented as
/// `Suspended`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementRegime {
    /// Normal two- or three-day settlement (NSE `EQ`).
    Standard,
    /// Trade-for-trade settlement (NSE `BE`): delivery from the specific
    /// contra trade, never netted.
    TradeForTrade,
}

/// One price band of an exchange price-linked tick schedule. Prices are
/// integer paise; a price belongs to the first band whose
/// `[lower_paise, upper_paise]` interval contains it (`upper_paise: None`
/// means unbounded above and must be the last band).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickBand {
    /// Inclusive lower price bound in paise.
    pub lower_paise: i64,
    /// Inclusive upper price bound in paise; `None` is unbounded.
    pub upper_paise: Option<i64>,
    /// Tick size for prices in this band, in paise.
    pub tick_paise: i64,
}

/// An exchange-applied, price-linked tick schedule: an ordered, contiguous
/// set of bands covering `[0, +∞)`. Schedules are exchange data, not
/// constants: they are ingested (see `src/venue/india/tick_schedule.rs`) and
/// versioned by the dates of their source circular.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickSchedule {
    /// Bands ordered by `lower_paise`, contiguous from `0` to unbounded.
    pub bands: Vec<TickBand>,
}

impl TickSchedule {
    /// Validate structure: non-empty, ordered, contiguous, positive ticks,
    /// coverage from `0` to unbounded, unbounded only on the last band.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(!self.bands.is_empty(), "tick schedule has no bands");
        for (index, band) in self.bands.iter().enumerate() {
            anyhow::ensure!(
                band.tick_paise > 0,
                "tick schedule band {index} has a non-positive tick"
            );
            anyhow::ensure!(
                band.lower_paise >= 0,
                "tick schedule band {index} has a negative lower bound"
            );
            if let Some(upper) = band.upper_paise {
                anyhow::ensure!(
                    upper >= band.lower_paise,
                    "tick schedule band {index} ends before it starts"
                );
                anyhow::ensure!(
                    index + 1 < self.bands.len(),
                    "tick schedule band {index} is bounded but follows the last band"
                );
            }
        }
        anyhow::ensure!(
            self.bands[0].lower_paise == 0,
            "tick schedule must start at 0, got {}",
            self.bands[0].lower_paise
        );
        anyhow::ensure!(
            self.bands
                .last()
                .is_some_and(|band| band.upper_paise.is_none()),
            "tick schedule must end unbounded"
        );
        for pair in self.bands.windows(2) {
            let expected = pair[0]
                .upper_paise
                .ok_or_else(|| anyhow::anyhow!("bounded band must be followed"))?
                + 1;
            anyhow::ensure!(
                pair[1].lower_paise == expected,
                "tick schedule bands are not contiguous at {} (want {expected})",
                pair[1].lower_paise
            );
        }
        Ok(())
    }

    /// Tick size (paise) for `price_paise`, or `None` when the price is
    /// negative or uncovered (callers must fail closed).
    #[must_use]
    pub fn tick_for(&self, price_paise: i64) -> Option<i64> {
        if price_paise < 0 {
            return None;
        }
        self.bands
            .iter()
            .find(|band| {
                band.lower_paise <= price_paise
                    && band.upper_paise.is_none_or(|upper| price_paise <= upper)
            })
            .map(|band| band.tick_paise)
    }

    /// Whether `price_paise` sits on the grid of its band's tick.
    #[must_use]
    pub fn is_aligned(&self, price_paise: i64) -> bool {
        self.tick_for(price_paise)
            .is_some_and(|tick| price_paise % tick == 0)
    }
}

/// One instrument's assigned tick for one term interval.
///
/// NSE assigns tick sizes *monthly, per instrument*: the tick that applies
/// for the whole month M is determined by the closing price at the end of
/// month M-1 (NSE circular CMTR67133, effective 2025-04-15; monthly review
/// thereafter). The assignment is therefore a record — reference closing
/// price, the month it governs, the assigned tick, and the schedule
/// revision that produced it — not a live lookup against the price at
/// decision time. A venue whose master assigns a per-security tick
/// directly (BSE `New Tick Size`) leaves the two monthly fields `None`.
///
/// Trading engines must use `tick_paise` verbatim for the term interval;
/// recomputing from the order price would wrongly change the tick
/// whenever the price crosses a band boundary mid-month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickAssignment {
    /// Assigned tick size, in paise.
    pub tick_paise: i64,
    /// Reference closing price (paise) that fixed the assignment; `None`
    /// for non-monthly assignments.
    pub reference_close_paise: Option<i64>,
    /// Trading date of the reference close. For an NSE monthly assignment
    /// this is the last available close of the month immediately preceding
    /// `effective_month`; an arbitrary price cannot be assigned.
    pub reference_close_date: Option<NaiveDate>,
    /// Identity of the dataset the reference close came from (e.g.
    /// `nse-eod-close`). Never empty for a monthly assignment.
    pub reference_source: Option<String>,
    /// SHA-256 of the exact reference-close source bytes, so the price can
    /// be reproduced from the archive. 64 lowercase hex digits.
    pub reference_sha256_hex: Option<String>,
    /// First day of the month this assignment governs; `None` for
    /// non-monthly assignments.
    pub effective_month: Option<NaiveDate>,
    /// Exchange tick-schedule revision the assignment was computed under,
    /// as `{source_id}:{source_format_version}`. Never empty.
    pub schedule_revision: String,
    /// Date from which that schedule revision is in force.
    pub schedule_effective_from: Option<NaiveDate>,
    /// SHA-256 of the exact schedule source bytes.
    pub schedule_sha256_hex: Option<String>,
    /// Exchange-calendar revision the assignment was determined under, as
    /// `{source_id}:{source_format_version}`. Required for a monthly
    /// assignment; `None` for a non-monthly assignment.
    #[serde(default)]
    pub calendar_revision: Option<String>,
    /// SHA-256 of the exact holiday-calendar source bytes whose sessions
    /// fixed the reference close date. Required for a monthly assignment;
    /// `None` for a non-monthly assignment.
    #[serde(default)]
    pub calendar_sha256_hex: Option<String>,
}

/// `true` when `value` is a 64-digit hex SHA-256.
#[must_use]
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// First day of `date`'s calendar month.
#[must_use]
pub fn month_start(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(
        chrono::Datelike::year(&date),
        chrono::Datelike::month(&date),
        1,
    )
    .unwrap_or(date)
}

/// First day of the calendar month before `month_start`.
fn previous_month_start(month_start: NaiveDate) -> Option<NaiveDate> {
    let year = chrono::Datelike::year(&month_start);
    let month = chrono::Datelike::month(&month_start);
    let (year, month) = if month == 1 {
        (year.checked_sub(1)?, 12)
    } else {
        (year, month - 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1)
}

impl TickAssignment {
    /// Validate the assignment record and its price-source lineage.
    ///
    /// A monthly assignment is only reproducible when it names the
    /// reference close date, the dataset, that dataset's hash, the schedule
    /// revision, the schedule hash, the calendar revision and the calendar
    /// hash, and the effective month. The reference
    /// close must fall in the month immediately preceding the effective
    /// month — the NSE determination rule (previous month's last available
    /// close) — so a close from the wrong month is rejected rather than
    /// silently applied.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(self.tick_paise > 0, "assigned tick must be positive");
        anyhow::ensure!(
            !self.schedule_revision.trim().is_empty(),
            "tick assignment source revision must not be empty"
        );
        if let Some(hash) = &self.schedule_sha256_hex {
            anyhow::ensure!(
                is_sha256_hex(hash),
                "tick schedule source hash must be 64 hex digits"
            );
        }
        let monthly = self.effective_month.is_some()
            || self.reference_close_paise.is_some()
            || self.reference_close_date.is_some()
            || self.calendar_revision.is_some()
            || self.calendar_sha256_hex.is_some();
        if !monthly {
            anyhow::ensure!(
                self.reference_source.is_none() && self.reference_sha256_hex.is_none(),
                "non-monthly tick assignment cannot carry a reference close source"
            );
            anyhow::ensure!(
                self.calendar_revision.is_none() && self.calendar_sha256_hex.is_none(),
                "non-monthly tick assignment cannot carry a calendar source"
            );
            return Ok(());
        }
        let month = self.effective_month.ok_or_else(|| {
            anyhow::anyhow!("monthly tick assignment requires an effective month")
        })?;
        anyhow::ensure!(
            month_start(month) == month,
            "effective month must be the first day of a month"
        );
        let close = self
            .reference_close_paise
            .ok_or_else(|| anyhow::anyhow!("monthly tick assignment requires a reference close"))?;
        anyhow::ensure!(close >= 0, "reference closing price must be non-negative");
        let close_date = self.reference_close_date.ok_or_else(|| {
            anyhow::anyhow!("monthly tick assignment requires a reference close date")
        })?;
        let previous = previous_month_start(month)
            .ok_or_else(|| anyhow::anyhow!("effective month {month} has no preceding month"))?;
        anyhow::ensure!(
            close_date >= previous && close_date < month,
            "reference close {close_date} must fall in {previous}..{month} (the month before the assignment)"
        );
        let source = self.reference_source.as_deref().unwrap_or_default();
        anyhow::ensure!(
            !source.trim().is_empty(),
            "monthly tick assignment requires the reference dataset identity"
        );
        anyhow::ensure!(
            is_sha256_hex(self.reference_sha256_hex.as_deref().unwrap_or_default()),
            "monthly tick assignment requires the reference source hash (64 hex digits)"
        );
        anyhow::ensure!(
            is_sha256_hex(self.schedule_sha256_hex.as_deref().unwrap_or_default()),
            "monthly tick assignment requires the schedule source hash (64 hex digits)"
        );
        let calendar_revision = self.calendar_revision.as_deref().unwrap_or_default();
        anyhow::ensure!(
            !calendar_revision.trim().is_empty(),
            "monthly tick assignment requires the calendar source revision"
        );
        anyhow::ensure!(
            is_sha256_hex(self.calendar_sha256_hex.as_deref().unwrap_or_default()),
            "monthly tick assignment requires the calendar source hash (64 hex digits)"
        );
        let schedule_from = self.schedule_effective_from.ok_or_else(|| {
            anyhow::anyhow!("monthly tick assignment requires the schedule revision date")
        })?;
        anyhow::ensure!(
            schedule_from <= month,
            "schedule revision {schedule_from} must be in force by the assignment month {month}"
        );
        Ok(())
    }
}

impl TickSchedule {
    /// The tick a schedule assigns for an instrument whose reference
    /// closing price is `reference_close_paise`.
    #[must_use]
    pub fn assign_tick(&self, reference_close_paise: i64) -> Option<i64> {
        self.tick_for(reference_close_paise)
    }
}

impl ListingTerms {
    /// Whether this term set is valid on `date` (inclusive bounds).
    #[must_use]
    pub fn is_valid_on(&self, date: NaiveDate) -> bool {
        self.effective_from <= date && self.effective_to.is_none_or(|end| date <= end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingStatus {
    Active,
    Suspended,
    Delisted,
}

/// Corporate action with exact adjustment ratio (numerator/denominator).
/// This embedded record is a history snapshot, not a point-in-time event
/// ledger; the latter arrives with the corporate-action event contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorporateAction {
    pub action: CorporateActionKind,
    pub effective_date: NaiveDate,
    pub numerator: i64,
    pub denominator: i64,
    /// Per-share amount in paise (dividend/rights).
    pub amount_paise: Option<i64>,
    /// Source annotation.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorporateActionKind {
    Bonus,
    Split,
    Consolidation,
    Dividend,
    Rights,
    SymbolChange,
    Delisting,
}

/// The canonical financial object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instrument {
    pub instrument_id: InstrumentId,
    pub instrument_type: InstrumentType,
    pub isin: Option<Isin>,
    pub name: String,
    /// For derivatives: the canonical id of the underlying.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underlying_instrument_id: Option<InstrumentId>,
    /// For derivatives: contract expiry in exchange calendar time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry_date: Option<NaiveDate>,
    /// For options: exact strike in paise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike_price_paise: Option<Paise>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_kind: Option<OptionKind>,
    /// At least one listing; symbols are per-venue, never identity.
    pub listings: Vec<Listing>,
    /// Ordered by effective date; applied chronologically.
    pub corporate_actions: Vec<CorporateAction>,
}

impl Instrument {
    /// Validate structural invariants: identity rules, derivative linkage,
    /// option strikes, and chronological corporate actions.
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            !self.name.is_empty(),
            "instrument {} has an empty name",
            self.instrument_id
        );
        let derivative = matches!(
            self.instrument_type,
            InstrumentType::Future | InstrumentType::Option
        );
        if derivative {
            anyhow::ensure!(
                matches!(&self.underlying_instrument_id, Some(underlying) if *underlying != self.instrument_id),
                "derivative {} must reference a different underlying instrument",
                self.instrument_id
            );
            anyhow::ensure!(
                self.expiry_date.is_some(),
                "derivative {} must have an expiry date",
                self.instrument_id
            );
        }
        if self.instrument_type == InstrumentType::Option {
            anyhow::ensure!(
                self.strike_price_paise.is_some() && self.option_kind.is_some(),
                "option {} must have a strike price and kind",
                self.instrument_id
            );
            anyhow::ensure!(
                self.strike_price_paise.is_some_and(|strike| strike.0 > 0),
                "option {} strike price must be positive",
                self.instrument_id
            );
        }
        if self.instrument_type != InstrumentType::Index {
            anyhow::ensure!(
                self.isin.is_some(),
                "non-index instrument {} must carry an ISIN",
                self.instrument_id
            );
        }
        anyhow::ensure!(
            !self.listings.is_empty(),
            "instrument {} must have at least one listing",
            self.instrument_id
        );
        let mut previous: Option<NaiveDate> = None;
        for action in &self.corporate_actions {
            if let Some(last) = previous {
                anyhow::ensure!(
                    action.effective_date >= last,
                    "corporate actions for {} must be chronologically ordered",
                    self.instrument_id
                );
            }
            anyhow::ensure!(
                action.numerator > 0 && action.denominator > 0,
                "corporate action for {} has a non-positive adjustment ratio",
                self.instrument_id
            );
            anyhow::ensure!(
                action.amount_paise.is_none_or(|amount| amount >= 0),
                "corporate action for {} has a negative amount",
                self.instrument_id
            );
            previous = Some(action.effective_date);
        }
        // Listings: a listing identity (venue, segment, symbol) may repeat
        // only for historical intervals that do not overlap. Each listing
        // must carry trading terms that are valid (ordered bounds cover
        // their effective ranges) and non-overlapping.
        let mut seen: Vec<(Venue, Segment, &str, NaiveDate, Option<NaiveDate>)> = Vec::new();
        for listing in &self.listings {
            anyhow::ensure!(
                !listing.venue_symbol.is_empty(),
                "instrument {} has an empty venue symbol",
                self.instrument_id
            );
            if let Some(end) = listing.effective_to {
                anyhow::ensure!(
                    end >= listing.effective_from,
                    "instrument {} has a listing ending before it starts",
                    self.instrument_id
                );
            }
            let overlaps = seen.iter().any(|(venue, segment, symbol, from, to)| {
                *venue == listing.venue
                    && *segment == listing.segment
                    && *symbol == listing.venue_symbol
                    && ranges_overlap(*from, *to, listing.effective_from, listing.effective_to)
            });
            anyhow::ensure!(
                !overlaps,
                "instrument {} has overlapping listings for {:?} {:?} `{}`",
                self.instrument_id,
                listing.venue,
                listing.segment,
                listing.venue_symbol
            );
            seen.push((
                listing.venue,
                listing.segment,
                listing.venue_symbol.as_str(),
                listing.effective_from,
                listing.effective_to,
            ));
            if let Some(group) = &listing.settlement_group {
                anyhow::ensure!(
                    !group.trim().is_empty(),
                    "instrument {} has an empty settlement group",
                    self.instrument_id
                );
            }
            anyhow::ensure!(
                !listing.terms.is_empty(),
                "instrument {} has a listing with no trading terms",
                self.instrument_id
            );
            for term in &listing.terms {
                if let Some(end) = term.effective_to {
                    anyhow::ensure!(
                        end >= term.effective_from,
                        "instrument {} has listing terms ending before they start",
                        self.instrument_id
                    );
                }
                anyhow::ensure!(
                    term.tick_assignment.validate().is_ok(),
                    "instrument {} has an invalid tick assignment",
                    self.instrument_id
                );
                anyhow::ensure!(
                    term.market_lot > 0 && term.quantity_increment > 0,
                    "instrument {} has non-positive listing terms",
                    self.instrument_id
                );
            }
            let terms_overlap = listing.terms.windows(2).any(|pair| {
                ranges_overlap(
                    pair[0].effective_from,
                    pair[0].effective_to,
                    pair[1].effective_from,
                    pair[1].effective_to,
                )
            });
            anyhow::ensure!(
                !terms_overlap,
                "instrument {} has overlapping listing terms",
                self.instrument_id
            );
        }
        Ok(())
    }

    /// Whether the instrument trades on the venue during the period implied
    /// by its listings' effective ranges.
    #[must_use]
    pub fn lists_on(&self, venue: Venue, on_date: NaiveDate) -> bool {
        self.listings.iter().any(|listing| {
            listing.venue == venue
                && listing.effective_from <= on_date
                && listing.effective_to.is_none_or(|end| on_date <= end)
                && listing.status == ListingStatus::Active
        })
    }

    /// The listing that is active on `date` (status `Active` and interval
    /// covering the date), if any. Product eligibility, settlement regime
    /// and Z-category all come from this one state — never from a live
    /// snapshot of the current master.
    ///
    /// When an instrument trades on several venues, callers must use
    /// [`Instrument::effective_listing_on`] with the intended execution
    /// venue: the first active listing is not necessarily the one the order
    /// targets.
    #[must_use]
    pub fn effective_listing(&self, on_date: NaiveDate) -> Option<&Listing> {
        self.listings.iter().find(|listing| {
            listing.status == ListingStatus::Active
                && listing.effective_from <= on_date
                && listing.effective_to.is_none_or(|end| on_date <= end)
        })
    }

    /// The active listing of `venue` on `date`, if any. This is the
    /// venue-scoped lookup execution eligibility must use; it never falls
    /// back to another venue's listing.
    #[must_use]
    pub fn effective_listing_on(&self, venue: Venue, on_date: NaiveDate) -> Option<&Listing> {
        self.listings.iter().find(|listing| {
            listing.venue == venue
                && listing.status == ListingStatus::Active
                && listing.effective_from <= on_date
                && listing.effective_to.is_none_or(|end| on_date <= end)
        })
    }

    /// Settlement regime in force on `date`, if the instrument is active.
    #[must_use]
    pub fn settlement_regime_on(&self, on_date: NaiveDate) -> Option<SettlementRegime> {
        self.effective_listing(on_date)
            .map(|listing| listing.settlement_regime)
    }

    /// Z-category designation in force on `date`, if the instrument is
    /// active. `Some(false)` is a definite "not Z"; `None` means the
    /// instrument is not active and the caller must fail closed.
    #[must_use]
    pub fn z_category_on(&self, on_date: NaiveDate) -> Option<bool> {
        self.effective_listing(on_date)
            .map(|listing| listing.z_category)
    }

    /// The exchange trading terms effective on `date`: the term set of the
    /// listing currently active on that date whose validity interval
    /// covers it. `None` when no listing is active or it has no term cover
    /// on that date — callers must fail closed, never guess.
    #[must_use]
    pub fn effective_terms(&self, on_date: NaiveDate) -> Option<&ListingTerms> {
        self.effective_listing(on_date)
            .and_then(|listing| listing.terms.iter().find(|term| term.is_valid_on(on_date)))
    }

    /// The exchange trading terms effective on `date` for `venue`: the term
    /// set of that venue's active listing. `None` when the venue has no
    /// active listing or no covering term set — callers must fail closed.
    #[must_use]
    pub fn effective_terms_on(&self, venue: Venue, on_date: NaiveDate) -> Option<&ListingTerms> {
        self.effective_listing_on(venue, on_date)
            .and_then(|listing| listing.terms.iter().find(|term| term.is_valid_on(on_date)))
    }
}

/// Whether two effective intervals [from, to] (open-ended when `to` is None)
/// intersect. Intervals are inclusive at both ends.
fn ranges_overlap(
    a_from: NaiveDate,
    a_to: Option<NaiveDate>,
    b_from: NaiveDate,
    b_to: Option<NaiveDate>,
) -> bool {
    let a_starts_before_b_ends = b_to.is_none_or(|b_end| a_from <= b_end);
    let b_starts_before_a_ends = a_to.is_none_or(|a_end| b_from <= a_end);
    a_starts_before_b_ends && b_starts_before_a_ends
}

/// Untrusted v2 wire representation (`schemas/india/instrument.v2.json`).
/// This is the ONLY JSON ingress path into the domain: unknown fields and
/// unknown enum values are rejected, and [`TryFrom`] validates every value
/// before a canonical [`Instrument`] exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentWire {
    pub instrument_id: String,
    pub instrument_type: InstrumentWireType,
    pub isin: String,
    pub name: String,
    pub listings: Vec<ListingWire>,
    pub corporate_actions: Vec<CorporateActionWire>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentWireType {
    Equity,
    Etf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingWire {
    pub venue: Venue,
    pub segment: Segment,
    pub venue_symbol: String,
    pub token: Option<String>,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub status: ListingStatus,
    pub settlement_regime: SettlementRegime,
    pub z_category: bool,
    #[serde(default)]
    pub settlement_group: Option<String>,
    pub terms: Vec<ListingTermsWire>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingTermsWire {
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub tick_assignment: TickAssignmentWire,
    pub market_lot: i64,
    pub quantity_increment: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickAssignmentWire {
    pub tick_paise: i64,
    #[serde(default)]
    pub reference_close_paise: Option<i64>,
    #[serde(default)]
    pub reference_close_date: Option<NaiveDate>,
    #[serde(default)]
    pub reference_source: Option<String>,
    #[serde(default)]
    pub reference_sha256_hex: Option<String>,
    #[serde(default)]
    pub effective_month: Option<NaiveDate>,
    pub schedule_revision: String,
    #[serde(default)]
    pub schedule_effective_from: Option<NaiveDate>,
    #[serde(default)]
    pub schedule_sha256_hex: Option<String>,
    #[serde(default)]
    pub calendar_revision: Option<String>,
    #[serde(default)]
    pub calendar_sha256_hex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickBandWire {
    pub lower_paise: i64,
    pub upper_paise: Option<i64>,
    pub tick_paise: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickScheduleWire {
    pub bands: Vec<TickBandWire>,
}

impl TryFrom<TickBandWire> for TickBand {
    type Error = anyhow::Error;

    fn try_from(wire: TickBandWire) -> anyhow::Result<Self> {
        anyhow::ensure!(
            wire.lower_paise >= 0,
            "tick band lower_paise must be non-negative"
        );
        if let Some(upper) = wire.upper_paise {
            anyhow::ensure!(
                upper >= wire.lower_paise,
                "tick band upper_paise precedes lower_paise"
            );
        }
        Ok(Self {
            lower_paise: wire.lower_paise,
            upper_paise: wire.upper_paise,
            tick_paise: wire.tick_paise,
        })
    }
}

impl TryFrom<TickScheduleWire> for TickSchedule {
    type Error = anyhow::Error;

    fn try_from(wire: TickScheduleWire) -> anyhow::Result<Self> {
        let bands = wire
            .bands
            .into_iter()
            .map(TickBand::try_from)
            .collect::<anyhow::Result<Vec<_>>>()?;
        let schedule = Self { bands };
        schedule.validate()?;
        Ok(schedule)
    }
}

impl From<&TickSchedule> for TickScheduleWire {
    fn from(schedule: &TickSchedule) -> Self {
        Self {
            bands: schedule
                .bands
                .iter()
                .map(|band| TickBandWire {
                    lower_paise: band.lower_paise,
                    upper_paise: band.upper_paise,
                    tick_paise: band.tick_paise,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorporateActionWire {
    pub action: CorporateActionKind,
    pub effective_date: NaiveDate,
    pub numerator: i64,
    pub denominator: i64,
    pub amount_paise: Option<i64>,
    pub note: Option<String>,
}

impl TryFrom<TickAssignmentWire> for TickAssignment {
    type Error = anyhow::Error;

    fn try_from(wire: TickAssignmentWire) -> anyhow::Result<Self> {
        let assignment = Self {
            tick_paise: wire.tick_paise,
            reference_close_paise: wire.reference_close_paise,
            reference_close_date: wire.reference_close_date,
            reference_source: wire.reference_source,
            reference_sha256_hex: wire.reference_sha256_hex,
            effective_month: wire.effective_month,
            schedule_revision: wire.schedule_revision,
            schedule_effective_from: wire.schedule_effective_from,
            schedule_sha256_hex: wire.schedule_sha256_hex,
            calendar_revision: wire.calendar_revision,
            calendar_sha256_hex: wire.calendar_sha256_hex,
        };
        assignment.validate()?;
        Ok(assignment)
    }
}

impl From<&TickAssignment> for TickAssignmentWire {
    fn from(assignment: &TickAssignment) -> Self {
        Self {
            tick_paise: assignment.tick_paise,
            reference_close_paise: assignment.reference_close_paise,
            reference_close_date: assignment.reference_close_date,
            reference_source: assignment.reference_source.clone(),
            reference_sha256_hex: assignment.reference_sha256_hex.clone(),
            effective_month: assignment.effective_month,
            schedule_revision: assignment.schedule_revision.clone(),
            schedule_effective_from: assignment.schedule_effective_from,
            schedule_sha256_hex: assignment.schedule_sha256_hex.clone(),
            calendar_revision: assignment.calendar_revision.clone(),
            calendar_sha256_hex: assignment.calendar_sha256_hex.clone(),
        }
    }
}

impl TryFrom<ListingTermsWire> for ListingTerms {
    type Error = anyhow::Error;

    fn try_from(wire: ListingTermsWire) -> anyhow::Result<Self> {
        let tick_assignment = TickAssignment::try_from(wire.tick_assignment)?;
        anyhow::ensure!(
            wire.market_lot > 0 && wire.quantity_increment > 0,
            "listing terms must have positive market lot and quantity increment"
        );
        Ok(Self {
            effective_from: wire.effective_from,
            effective_to: wire.effective_to,
            tick_assignment,
            market_lot: wire.market_lot,
            quantity_increment: wire.quantity_increment,
        })
    }
}

impl TryFrom<ListingWire> for Listing {
    type Error = anyhow::Error;

    fn try_from(wire: ListingWire) -> anyhow::Result<Self> {
        let terms = wire
            .terms
            .into_iter()
            .map(ListingTerms::try_from)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            venue: wire.venue,
            segment: wire.segment,
            venue_symbol: wire.venue_symbol,
            token: wire.token,
            effective_from: wire.effective_from,
            effective_to: wire.effective_to,
            status: wire.status,
            settlement_regime: wire.settlement_regime,
            z_category: wire.z_category,
            settlement_group: wire.settlement_group,
            terms,
        })
    }
}

impl TryFrom<CorporateActionWire> for CorporateAction {
    type Error = anyhow::Error;

    fn try_from(wire: CorporateActionWire) -> anyhow::Result<Self> {
        anyhow::ensure!(
            wire.numerator > 0 && wire.denominator > 0,
            "corporate action adjustment ratio must be positive"
        );
        anyhow::ensure!(
            wire.amount_paise.is_none_or(|amount| amount >= 0),
            "corporate action amount must be non-negative"
        );
        Ok(Self {
            action: wire.action,
            effective_date: wire.effective_date,
            numerator: wire.numerator,
            denominator: wire.denominator,
            amount_paise: wire.amount_paise,
            note: wire.note,
        })
    }
}

impl TryFrom<InstrumentWire> for Instrument {
    type Error = anyhow::Error;

    /// Checked conversion from the u
    /// untrusted wire representation: constructs validated identities, maps
    /// every field, and runs [`Instrument::validate`] before returning.
    ///
    /// The v2 contract covers cash equities and ETFs only; the supported
    /// types and the cash segment are enforced by the wire enums above.
    fn try_from(wire: InstrumentWire) -> anyhow::Result<Self> {
        let instrument_type = match wire.instrument_type {
            InstrumentWireType::Equity => InstrumentType::Equity,
            InstrumentWireType::Etf => InstrumentType::Etf,
        };
        let instrument_id = InstrumentId::new(wire.instrument_id)?;
        let isin = Isin::new(wire.isin)?;
        anyhow::ensure!(!wire.name.is_empty(), "instrument name must not be empty");
        let listings = wire
            .listings
            .into_iter()
            .map(Listing::try_from)
            .collect::<anyhow::Result<Vec<_>>>()?;
        let corporate_actions = wire
            .corporate_actions
            .into_iter()
            .map(CorporateAction::try_from)
            .collect::<anyhow::Result<Vec<_>>>()?;
        let instrument = Self {
            instrument_id,
            instrument_type,
            isin: Some(isin),
            name: wire.name,
            underlying_instrument_id: None,
            expiry_date: None,
            strike_price_paise: None,
            option_kind: None,
            listings,
            corporate_actions,
        };
        instrument.validate()?;
        Ok(instrument)
    }
}

impl Instrument {
    /// The v2 wire serialization of this instrument: rejects
    /// out-of-scope types (indices and derivatives exist in memory but not
    /// in this contract) and missing ISINs (required by v2).
    pub fn to_wire(&self) -> anyhow::Result<InstrumentWire> {
        let instrument_type = match self.instrument_type {
            InstrumentType::Equity => InstrumentWireType::Equity,
            InstrumentType::Etf => InstrumentWireType::Etf,
            other => {
                anyhow::bail!("instrument type {other:?} is outside the v2 instrument contract")
            }
        };
        let isin = self.isin.as_ref().map(Isin::as_str).ok_or_else(|| {
            anyhow::anyhow!("instrument {} has no ISIN for v2", self.instrument_id)
        })?;
        Ok(InstrumentWire {
            instrument_id: self.instrument_id.as_str().to_owned(),
            instrument_type,
            isin: isin.to_owned(),
            name: self.name.clone(),
            listings: self
                .listings
                .iter()
                .map(|listing| ListingWire {
                    venue: listing.venue,
                    segment: listing.segment,
                    venue_symbol: listing.venue_symbol.clone(),
                    token: listing.token.clone(),
                    effective_from: listing.effective_from,
                    effective_to: listing.effective_to,
                    status: listing.status,
                    settlement_regime: listing.settlement_regime,
                    z_category: listing.z_category,
                    settlement_group: listing.settlement_group.clone(),
                    terms: listing
                        .terms
                        .iter()
                        .map(|term| ListingTermsWire {
                            effective_from: term.effective_from,
                            effective_to: term.effective_to,
                            tick_assignment: TickAssignmentWire::from(&term.tick_assignment),
                            market_lot: term.market_lot,
                            quantity_increment: term.quantity_increment,
                        })
                        .collect(),
                })
                .collect(),
            corporate_actions: self
                .corporate_actions
                .iter()
                .map(|action| CorporateActionWire {
                    action: action.action,
                    effective_date: action.effective_date,
                    numerator: action.numerator,
                    denominator: action.denominator,
                    amount_paise: action.amount_paise,
                    note: action.note.clone(),
                })
                .collect(),
        })
    }
}

/// Adjust a raw historical price (paise) across corporate actions so the
/// series is comparable through splits/bonuses. Applies actions strictly
/// after `as_of` chronologically. Integer-exact: returns None when the
/// adjustment is not representable in paise.
pub fn adjusted_price_paise(
    raw_paise: Paise,
    actions: &[CorporateAction],
    as_of: chrono::NaiveDate,
) -> anyhow::Result<Paise> {
    let mut value = i128::from(raw_paise.0);
    for action in actions
        .iter()
        .filter(|action| action.effective_date > as_of)
        .filter(|action| {
            matches!(
                action.action,
                CorporateActionKind::Split
                    | CorporateActionKind::Bonus
                    | CorporateActionKind::Consolidation
            )
        })
    {
        // A 1:2 split (num=1, den=2) doubles share count, halving the
        // adjusted price: new = old * numerator / denominator.
        anyhow::ensure!(
            action.numerator > 0 && action.denominator > 0,
            "corporate action has a non-positive adjustment ratio"
        );
        let scaled = value
            .checked_mul(i128::from(action.numerator))
            .ok_or_else(|| anyhow::anyhow!("adjustment of {value} paise overflows i128"))?;
        // Exact division only: truncation would silently corrupt history.
        anyhow::ensure!(
            scaled % i128::from(action.denominator) == 0,
            "adjustment of {value} paise by {}:{} is not representable in paise",
            action.numerator,
            action.denominator
        );
        value = scaled / i128::from(action.denominator);
    }
    if value > i128::from(i64::MAX) || value < 0 {
        anyhow::bail!("adjusted price {value} paise is outside i64 range");
    }
    Ok(Paise(value as i64))
}
