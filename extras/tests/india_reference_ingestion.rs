//! Acceptance tests for NSE/BSE reference-master ingestion (task P1-03,
//! capability `INDIA-REF-INGEST`), extended per the P1-03 reviews.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::NaiveDate;
use india_domain::{
    CalendarDay, CorporateActionKind, InstrumentId, InstrumentType, Isin, ListingStatus, Segment,
    SessionDay, SettlementRegime, TickAssignment, TradingCalendar, Venue,
};
use nirnay_extras::venue::india::{
    BuiltCalendar, CorporateActionLedger, CorporateActionRecord, ExchangeTickSchedule,
    InstrumentChange, NormalizedListing, ReferenceClose, ReferenceSnapshot, ReferenceStore,
    SourceArchive, SourceProvenance, TickScheduleBook, bse::BseSecurityType, bse_carry,
    build_calendar, classify_bse_group, nse_carry, parse_bse_master, parse_holiday_csv,
    parse_nse_cm_master, parse_nse_holiday_csv, parse_tick_schedule_csv, provenance_for_source,
    reconcile,
};

const RELIANCE_ISIN: &str = "INE002A01018";
const HDFC_ISIN: &str = "INE040A01026";

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn flat_assignment(paise: i64, revision: &str) -> TickAssignment {
    TickAssignment {
        tick_paise: paise,
        reference_close_paise: None,
        reference_close_date: None,
        reference_source: None,
        reference_sha256_hex: None,
        effective_month: None,
        schedule_revision: revision.to_owned(),
        schedule_effective_from: None,
        schedule_sha256_hex: None,
        calendar_revision: None,
        calendar_sha256_hex: None,
    }
}

/// A provenanced no-holiday NSE calendar for `[from, to]`.
///
/// The holiday file lists no exceptions, so the builder's weekday rule is the
/// only input; provenance is bound to the exact CSV bytes, which are retained
/// in `archive` under `nse-cash-calendar`.
fn provenanced_range(
    archive: &SourceArchive,
    from: NaiveDate,
    to: NaiveDate,
    csv: &str,
) -> BuiltCalendar {
    archive
        .store("nse-cash-calendar", csv.as_bytes(), at(from))
        .unwrap();
    let provenance = provenance_for_source(
        "nse-cash-calendar",
        Venue::Nse,
        Segment::Cash,
        from.and_hms_opt(9, 0, 0).unwrap().and_utc(),
        "nse-cash-calendar-v1",
        "test",
        csv.as_bytes(),
    );
    build_calendar(Venue::Nse, from, to, csv, provenance, archive).unwrap()
}

/// A provenanced full-month NSE calendar with no holidays.
fn provenanced_month(archive: &SourceArchive, year: i32, month: u32) -> BuiltCalendar {
    let start = date(year, month, 1);
    let next = if month == 12 {
        date(year + 1, 1, 1)
    } else {
        date(year, month + 1, 1)
    };
    provenanced_range(
        archive,
        start,
        next.pred_opt().unwrap(),
        "DATE,DAY,HOLIDAY,PURPOSE\n",
    )
}

/// Store a one-row NSE end-of-day close file and return the matching
/// observation. The archived bytes carry the ISIN, date and price, so the
/// assignment can be reproduced only from this record.
fn archived_reference_for(
    archive: &SourceArchive,
    isin: &str,
    symbol: &str,
    day: NaiveDate,
    close_paise: i64,
) -> ReferenceClose {
    let bytes = format!("DATE,SYMBOL,ISIN,CLOSE_PAISE\n{day},{symbol},{isin},{close_paise}\n");
    let stored = archive
        .store("nse-eod-close", bytes.as_bytes(), at(day))
        .unwrap();
    ReferenceClose {
        date: day,
        isin: Isin::new(isin).unwrap(),
        close_paise,
        source_id: "nse-eod-close".to_owned(),
        sha256_hex: stored.sha256_hex,
    }
}

fn archived_reference(archive: &SourceArchive, day: NaiveDate, close_paise: i64) -> ReferenceClose {
    archived_reference_for(archive, RELIANCE_ISIN, "RELIANCE", day, close_paise)
}

fn provenance(venue: Venue, csv: &str) -> SourceProvenance {
    provenance_for_source(
        "nse-cm-master",
        venue,
        Segment::Cash,
        date(2026, 1, 1).and_hms_opt(10, 0, 0).unwrap().and_utc(),
        "nse-cm-master-v1",
        "test",
        csv.as_bytes(),
    )
}

fn at(day: NaiveDate) -> chrono::DateTime<chrono::Utc> {
    day.and_hms_opt(10, 0, 0).unwrap().and_utc()
}

const NSE_CSV: &str = "SYMBOL,NAME OF COMPANY,SERIES,MARKET LOT,ISIN NUMBER\n\
RELIANCE,Reliance Industries,EQ,1,INE002A01018\n\
HDFCBANK,HDFC Bank,EQ,1,INE040A01026\n";

// Official BSE scrip specs: Scrip Code numeric id, Instrument Code symbol,
// Scrip Name full name, Group Name settlement group, New Tick Size tick,
// Market Lot the lot size, Security Type Flag the category.
const BSE_CSV: &str = "Scrip Code,Instrument Code,Scrip Name,Group Name,ISIN CODE,New Tick Size,Market Lot,Security Type Flag\n\
500325,RELIANCE,Reliance Industries,T,INE002A01018,0.05,1,EQ\n";

fn normalized_nse(symbol: &str, isin: &str, as_of: NaiveDate) -> NormalizedListing {
    NormalizedListing {
        venue: Venue::Nse,
        segment: Segment::Cash,
        instrument_type: InstrumentType::Equity,
        venue_symbol: symbol.to_owned(),
        token: None,
        isin: Isin::new(isin).unwrap(),
        name: symbol.to_owned(),
        status: ListingStatus::Active,
        settlement_regime: SettlementRegime::Standard,
        z_category: false,
        settlement_group: None,
        tick_assignment: flat_assignment(5, "test-fixture"),
        market_lot: 1,
        as_of,
    }
}

fn normalized_with(
    symbol: &str,
    isin: &str,
    as_of: NaiveDate,
    status: ListingStatus,
    regime: SettlementRegime,
    z: bool,
) -> NormalizedListing {
    let mut row = normalized_nse(symbol, isin, as_of);
    row.status = status;
    row.settlement_regime = regime;
    row.z_category = z;
    row
}

fn normalized_bse(symbol: &str, isin: &str, as_of: NaiveDate) -> NormalizedListing {
    NormalizedListing {
        venue: Venue::Bse,
        segment: Segment::Cash,
        instrument_type: InstrumentType::Equity,
        venue_symbol: symbol.to_owned(),
        token: Some("500325".to_owned()),
        isin: Isin::new(isin).unwrap(),
        name: symbol.to_owned(),
        status: ListingStatus::Active,
        settlement_regime: SettlementRegime::Standard,
        z_category: false,
        settlement_group: Some("T".to_owned()),
        tick_assignment: flat_assignment(5, "bse-security-master:bse-scrip-v1"),
        market_lot: 1,
        as_of,
    }
}

fn reconciled(
    day: NaiveDate,
    rows: Vec<NormalizedListing>,
) -> nirnay_extras::venue::india::ReconciledUniverse {
    let prov = provenance(Venue::Nse, NSE_CSV);
    reconcile(&[], rows, day, false, at(day).max(prov.retrieved_at), &prov).unwrap()
}

fn reconcile_from(
    existing: &[india_domain::Instrument],
    incoming: Vec<NormalizedListing>,
    as_of: NaiveDate,
    full: bool,
) -> nirnay_extras::venue::india::ReconciledUniverse {
    let prov = provenance(Venue::Nse, NSE_CSV);
    let recorded = at(as_of).max(prov.retrieved_at);
    reconcile(existing, incoming, as_of, full, recorded, &prov).unwrap()
}

fn bootstrapped(symbol: &str, isin: &str, as_of: NaiveDate) -> Vec<india_domain::Instrument> {
    reconciled(as_of, vec![normalized_nse(symbol, isin, as_of)]).instruments
}

// ---- NSE parser: series semantics (findings 1, A) ----

#[test]
fn parse_nse_accepts_valid_rows() {
    let rows = parse_nse_cm_master(NSE_CSV).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].symbol, "RELIANCE");
    assert_eq!(rows[0].isin.as_str(), RELIANCE_ISIN);
    assert_eq!(rows[0].status, ListingStatus::Active);
    assert_eq!(rows[0].settlement_regime, SettlementRegime::Standard);
    assert!(!rows[0].z_category);
    assert_eq!(rows[1].symbol, "HDFCBANK");
}

#[test]
fn nse_be_is_active_with_trade_for_trade_regime() {
    let csv = "SYMBOL,NAME OF COMPANY,SERIES,ISIN NUMBER\n\
HDFCBANK,HDFC Bank,BE,INE040A01026\n";
    let rows = parse_nse_cm_master(csv).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, ListingStatus::Active, "BE trades");
    assert_eq!(
        rows[0].settlement_regime,
        SettlementRegime::TradeForTrade,
        "BE is a settlement regime, not a suspension"
    );
    assert!(!rows[0].z_category);
}

#[test]
fn nse_bz_is_active_trade_for_trade_with_z_category() {
    let csv = "SYMBOL,NAME OF COMPANY,SERIES,ISIN NUMBER\n\
X,X Corp,BZ,INE002A01018\n";
    let rows = parse_nse_cm_master(csv).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, ListingStatus::Active, "BZ rows still trade");
    assert_eq!(rows[0].settlement_regime, SettlementRegime::TradeForTrade);
    assert!(
        rows[0].z_category,
        "the Z-category designation and its restrictions are preserved separately from the status"
    );
}

#[test]
fn nse_unknown_series_is_rejected() {
    let unknown = "SYMBOL,NAME OF COMPANY,SERIES,ISIN NUMBER\n\
X,X Corp,ZZ,INE002A01018\n";
    assert!(
        parse_nse_cm_master(unknown).is_err(),
        "unknown series must fail"
    );
    let et = "SYMBOL,NAME OF COMPANY,SERIES,ISIN NUMBER\n\
X,X Corp,ET,INE002A01018\n";
    assert!(
        parse_nse_cm_master(et).is_err(),
        "ET is out of this contract"
    );
}

#[test]
fn parse_nse_rejects_bad_isin_and_empty_symbol() {
    let bad_isin = "SYMBOL,SERIES,ISIN NUMBER\nX,EQ,INE002A01017\n";
    assert!(
        parse_nse_cm_master(bad_isin).is_err(),
        "check digit must fail"
    );

    let empty_symbol = "SYMBOL,SERIES,ISIN NUMBER\n,EQ,INE002A01018\n";
    assert!(parse_nse_cm_master(empty_symbol).is_err());
}

// ---- BSE parser: official spec mapping and classification ----

const BSE_HEADER: &str = "Scrip Code,Instrument Code,Scrip Name,Group Name,ISIN CODE,New Tick Size,Market Lot,Security Type Flag";

#[test]
fn parse_bse_accepts_official_header() {
    let rows = parse_bse_master(BSE_CSV).unwrap();
    assert_eq!(rows.len(), 1);
    // Scrip Code is the numeric security identifier (token); Instrument
    // Code is the security symbol. They are not interchangeable.
    assert_eq!(rows[0].scrip_code, "500325");
    assert_eq!(rows[0].symbol, "RELIANCE");
    assert_eq!(rows[0].name, "Reliance Industries");
    assert_eq!(rows[0].settlement_group, "T");
    assert_eq!(
        rows[0].group_class.settlement_regime,
        SettlementRegime::TradeForTrade
    );
    assert!(!rows[0].group_class.z_category);
    // `New Tick Size` is a decimal rupee amount: 0.05 rupees = 5 paise.
    assert_eq!(rows[0].tick_paise, 5);
    assert_eq!(rows[0].market_lot, 1);
    assert_eq!(rows[0].security_type, BseSecurityType::Equity);
    assert_eq!(rows[0].status, ListingStatus::Active);
}

#[test]
fn bse_tick_size_parses_decimal_rupees_without_floating_point() {
    for (text, paise) in [
        ("0.01", 1),
        ("0.05", 5),
        ("0.10", 10),
        ("5.00", 500),
        ("9.99", 999),
        ("5", 500),
    ] {
        let csv = format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,A,INE002A01018,{text},1,EQ\n");
        let rows = parse_bse_master(&csv).unwrap();
        assert_eq!(
            rows[0].tick_paise, paise,
            "tick {text} must be {paise} paise"
        );
    }
    for bad in ["0.005", "0.00", "abc", "10.00", "-0.05", "1.234"] {
        let csv = format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,A,INE002A01018,{bad},1,EQ\n");
        assert!(
            parse_bse_master(&csv).is_err(),
            "tick {bad} must be rejected"
        );
    }
}

#[test]
fn bse_group_classification_is_explicit() {
    for group in ["T", "MT", "TS", "IT", "Z", "R", "ZP", "P", "Y", "ZY", "XT"] {
        let csv =
            format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,{group},INE002A01018,0.05,1,EQ\n");
        let rows = parse_bse_master(&csv).unwrap();
        assert_eq!(
            rows[0].group_class.settlement_regime,
            SettlementRegime::TradeForTrade,
            "group {group} settles trade-to-trade"
        );
        assert_eq!(
            rows[0].group_class.z_category,
            matches!(group, "Z" | "ZP" | "ZY"),
            "group {group} Z-category designation"
        );
    }
    for group in ["A", "B", "M", "MS", "S", "X", "XC", "XD"] {
        let csv =
            format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,{group},INE002A01018,0.05,1,EQ\n");
        let rows = parse_bse_master(&csv).unwrap();
        assert_eq!(
            rows[0].group_class.settlement_regime,
            SettlementRegime::Standard,
            "group {group} settles rolling"
        );
        assert!(!rows[0].group_class.z_category);
    }
    // An unclassified group is never assumed to settle rolling.
    let unknown = format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,QQ,INE002A01018,0.05,1,EQ\n");
    let error = parse_bse_master(&unknown).expect_err("unknown group must fail");
    assert!(error.to_string().contains("Group Name"));
    // The group column itself is required.
    let missing_group = "Scrip Code,Instrument Code,Scrip Name,ISIN CODE,New Tick Size,Market Lot,Security Type Flag\n\
500325,RELIANCE,Reliance,INE002A01018,0.05,1,EQ\n";
    assert!(
        parse_bse_master(missing_group).is_err(),
        "Group Name is required"
    );
}

#[test]
fn bse_security_type_flag_accepts_equity_and_rejects_other_categories() {
    for (flag, label) in [
        ("EQ", "equity"),
        ("MF", "mutual fund"),
        ("DB", "debt"),
        ("GS", "government security"),
    ] {
        let csv = format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,T,INE002A01018,0.05,1,{flag}\n");
        let parsed = parse_bse_master(&csv);
        if flag == "EQ" {
            let rows = parsed.unwrap();
            assert_eq!(rows[0].security_type, BseSecurityType::Equity);
        } else {
            let error = parsed.expect_err("non-equity categories are out of contract");
            let message = error.to_string();
            assert!(
                message.contains(flag) && message.contains(label),
                "rejection must name the category: {message}"
            );
        }
    }
    // An unknown flag is never guessed.
    let unknown = format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,T,INE002A01018,0.05,1,XX\n");
    assert!(parse_bse_master(&unknown).is_err());
    // The flag column is required; its absence is a contract violation.
    let missing_flag = "Scrip Code,Instrument Code,Scrip Name,Group Name,ISIN CODE,New Tick Size,Market Lot\n\
500325,RELIANCE,Reliance,T,INE002A01018,0.05,1\n";
    assert!(parse_bse_master(missing_flag).is_err());
}

#[test]
fn bse_rejects_bad_rows() {
    let bad_isin = format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,T,INE002A01017,0.05,1,EQ\n");
    assert!(parse_bse_master(&bad_isin).is_err());
    let zero_lot = format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,T,INE002A01018,0.05,0,EQ\n");
    assert!(parse_bse_master(&zero_lot).is_err(), "non-positive lot");
    let text_lot = format!("{BSE_HEADER}\n500325,RELIANCE,Reliance,T,INE002A01018,0.05,many,EQ\n");
    assert!(
        parse_bse_master(&text_lot).is_err(),
        "lot must be an integer"
    );
    let dup =
        format!("{BSE_HEADER}\n1,X,A,T,INE002A01018,0.01,1,EQ\n2,X,B,T,INE040A01026,0.01,1,EQ\n");
    assert!(parse_bse_master(&dup).is_err(), "duplicate symbol");
    let missing_col = "Scrip Code,Scrip Name,ISIN CODE,New Tick Size,Market Lot,Security Type Flag\n\
500325,Reliance,INE002A01018,0.05,1,EQ\n";
    assert!(
        parse_bse_master(missing_col).is_err(),
        "Instrument Code is required"
    );
    // A missing Market Lot column is not silently defaulted to one.
    let missing_lot = "Scrip Code,Instrument Code,Scrip Name,Group Name,ISIN CODE,New Tick Size,Security Type Flag\n\
500325,RELIANCE,Reliance,T,INE002A01018,0.05,EQ\n";
    assert!(
        parse_bse_master(missing_lot).is_err(),
        "Market Lot is required"
    );
}

#[test]
fn bse_optional_status_column_maps_lifecycle() {
    let csv = format!(
        "{BSE_HEADER},STATUS\n1,A,X,T,INE002A01018,0.05,1,EQ,ACTIVE\n2,B,Y,T,INE040A01026,0.05,1,EQ,SUSPENDED\n"
    );
    let rows = parse_bse_master(&csv).unwrap();
    assert_eq!(rows[0].status, ListingStatus::Active);
    assert_eq!(rows[1].status, ListingStatus::Suspended);
    let bad = format!("{BSE_HEADER},STATUS\n1,A,X,T,INE002A01018,0.05,1,EQ,HALTED\n");
    assert!(parse_bse_master(&bad).is_err(), "unknown status must fail");
}

// ---- Calendar: explicit classification (finding 2) ----

#[test]
fn calendar_classification_is_explicit() {
    let csv = "DATE,DAY,HOLIDAY,PURPOSE\n\
02-Jan-2026,FRI,New Year,Trading holidays\n\
01-Jan-2026,THU,,Regular trading session\n\
03-Jan-2026,SAT,Special,Special live trading session\n";
    let calendar = parse_nse_holiday_csv(csv).unwrap();
    assert!(
        !calendar.is_trading_day(date(2026, 1, 2)),
        "a TRADING HOLIDAY is closed, not trading"
    );
    assert!(
        calendar.is_trading_day(date(2026, 1, 1)),
        "a regular session trades"
    );
    // Sorted by date: 01 (regular session), 02 (trading holiday), 03 (special).
    assert_eq!(calendar.days[0].session, SessionDay::Trading);
    assert_eq!(calendar.days[1].session, SessionDay::Closed);
    assert_eq!(calendar.days[2].session, SessionDay::Special);
}

#[test]
fn calendar_unknown_purpose_is_rejected() {
    assert!(parse_holiday_csv(Venue::Bse, "DATE,PURPOSE\n2026-01-01,\n").is_err());
    assert!(parse_holiday_csv(Venue::Bse, "DATE,NOTE\n2026-01-01,maintenance\n").is_err());
    assert!(parse_holiday_csv(Venue::Bse, "DATE\n2026-01-01\n").is_err());
}

// ---- Identity reconciliation: history preserved (finding 3) ----

#[test]
fn symbol_change_preserves_instrument_id() {
    let day1 = date(2026, 1, 1);
    let day2 = date(2026, 3, 1);
    let first = bootstrapped("RELIANCE", RELIANCE_ISIN, day1);
    let id_before = first[0].instrument_id.clone();

    let second = reconcile_from(
        &first,
        vec![normalized_nse("RELIND", RELIANCE_ISIN, day2)],
        day2,
        false,
    );
    let id_after = second.instruments[0].instrument_id.clone();
    assert_eq!(id_before, id_after, "identity survives rename");
    assert_eq!(
        second.reconciliation.changed,
        vec![InstrumentChange {
            instrument_id: id_after.clone(),
            closed_listing: Some("NSE:RELIANCE".to_owned()),
            opened_listing: Some("NSE:RELIND".to_owned()),
        }]
    );
    let listing = &second.instruments[0].listings;
    let active: Vec<_> = listing
        .iter()
        .filter(|item| item.effective_to.is_none())
        .collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].venue_symbol, "RELIND");
    assert_eq!(active[0].effective_from, day2);
}

#[test]
fn status_change_preserves_valid_time_history() {
    // Active (Jan) -> Suspended (Feb) -> Active again (Mar): each date must
    // keep the status that was valid on it.
    let jan = date(2026, 1, 1);
    let feb = date(2026, 2, 2);
    let mar = date(2026, 3, 1);

    let jan_u = reconciled(jan, vec![normalized_nse("RELIANCE", RELIANCE_ISIN, jan)]);
    let feb_u = reconcile_from(
        &jan_u.instruments,
        vec![normalized_with(
            "RELIANCE",
            RELIANCE_ISIN,
            feb,
            ListingStatus::Suspended,
            SettlementRegime::Standard,
            false,
        )],
        feb,
        false,
    );
    let mar_u = reconcile_from(
        &feb_u.instruments,
        vec![normalized_nse("RELIANCE", RELIANCE_ISIN, mar)],
        mar,
        false,
    );

    let status_on = |universe: &[india_domain::Instrument], day: NaiveDate| {
        universe[0]
            .listings
            .iter()
            .find(|listing| {
                listing.effective_from <= day && listing.effective_to.is_none_or(|e| day <= e)
            })
            .map(|listing| listing.status)
    };
    assert_eq!(
        status_on(&mar_u.instruments, date(2026, 1, 15)),
        Some(ListingStatus::Active)
    );
    assert_eq!(
        status_on(&mar_u.instruments, date(2026, 2, 15)),
        Some(ListingStatus::Suspended)
    );
    assert_eq!(
        status_on(&mar_u.instruments, date(2026, 3, 15)),
        Some(ListingStatus::Active)
    );
    assert_eq!(
        mar_u.instruments[0].listings.len(),
        3,
        "each change closes and reopens"
    );
}

#[test]
fn tick_revision_applies_as_new_terms_interval() {
    // A new tick assignment must close the old interval and apply from its
    // own date (not retroactively, never ignored).
    let jan = date(2026, 1, 1);
    let feb = date(2026, 2, 1);
    let jan_u = reconciled(jan, vec![normalized_nse("RELIANCE", RELIANCE_ISIN, jan)]);
    let mut new_assignment = jan_u.instruments[0].listings[0].terms[0]
        .tick_assignment
        .clone();
    new_assignment.tick_paise = 10;
    new_assignment.schedule_revision = "nse-tick-schedule:effective:2026-02-01".to_owned();
    let mut row = normalized_nse("RELIANCE", RELIANCE_ISIN, feb);
    row.tick_assignment = new_assignment;
    let feb_u = reconcile_from(&jan_u.instruments, vec![row], feb, false);
    assert_eq!(feb_u.reconciliation.changed.len(), 1);
    let instrument = &feb_u.instruments[0];
    assert_eq!(
        instrument.listings.len(),
        2,
        "old tick interval closed, new one opened"
    );
    let old = &instrument.listings[0];
    assert_eq!(old.terms[0].tick_assignment.tick_paise, 5);
    assert_eq!(old.effective_to, Some(date(2026, 1, 31)));
    let new = &instrument.listings[1];
    assert_eq!(new.effective_from, feb);
    assert_eq!(new.terms[0].tick_assignment.tick_paise, 10);
    assert_eq!(new.terms[0].effective_from, feb);
}

#[test]
fn delisting_closes_listing_interval() {
    let day1 = date(2026, 1, 1);
    let day2 = date(2026, 2, 1);
    let first = bootstrapped("RELIANCE", RELIANCE_ISIN, day1);

    // Full snapshot without Reliance: its NSE listing closes, never deleted.
    let second = reconcile_from(
        &first,
        vec![normalized_nse("HDFCBANK", HDFC_ISIN, day2)],
        day2,
        true,
    );
    let removed = second
        .instruments
        .iter()
        .find(|item| item.instrument_id == InstrumentId::new("in.ine002a01018").unwrap())
        .unwrap();
    assert!(
        removed
            .listings
            .iter()
            .all(|item| item.effective_to.is_some()),
        "delisted listing must be closed, not removed"
    );
    assert_eq!(removed.listings[0].effective_to, Some(date(2026, 1, 31)));
    assert_eq!(second.reconciliation.removed.len(), 1);

    // Partial ingest must not delist by accident.
    let partial = reconcile_from(
        &first,
        vec![normalized_nse("HDFCBANK", HDFC_ISIN, day2)],
        day2,
        false,
    );
    assert!(partial.reconciliation.removed.is_empty());
    assert!(
        partial
            .instruments
            .iter()
            .find(|item| item.instrument_id == removed.instrument_id)
            .unwrap()
            .listings[0]
            .effective_to
            .is_none()
    );
}

#[test]
fn overlapping_listings_are_rejected() {
    let day = date(2026, 1, 1);
    let prov = provenance(Venue::Nse, NSE_CSV);
    let reconciled = reconcile(
        &[],
        vec![
            normalized_nse("RELIANCE", RELIANCE_ISIN, day),
            normalized_nse("RELIANCE", HDFC_ISIN, day),
        ],
        day,
        false,
        at(day),
        &prov,
    );
    assert!(
        reconciled.is_err(),
        "two instruments cannot share (venue, symbol) concurrently"
    );
}

#[test]
fn ingest_is_idempotent() {
    let day = date(2026, 1, 1);
    let first = bootstrapped("RELIANCE", RELIANCE_ISIN, day);
    let second = reconcile_from(
        &first,
        vec![normalized_nse("RELIANCE", RELIANCE_ISIN, day)],
        day,
        false,
    );
    assert!(second.reconciliation.added.is_empty());
    assert!(second.reconciliation.changed.is_empty());
    assert!(second.reconciliation.removed.is_empty());
    assert_eq!(first, second.instruments, "same input, same universe");
    assert!(second.listing_states.is_empty(), "no new state records");
}

// ---- Cross-venue identity (finding 8 + D) ----

#[test]
fn same_isin_from_two_venues_is_one_instrument() {
    let day = date(2026, 1, 1);
    let universe = reconciled(
        day,
        vec![
            normalized_nse("RELIANCE", RELIANCE_ISIN, day),
            normalized_bse("RELIANCE", RELIANCE_ISIN, day),
        ],
    );
    assert_eq!(universe.instruments.len(), 1, "same ISIN, one instrument");
    let instrument = &universe.instruments[0];
    assert_eq!(instrument.listings.len(), 2);
    let mut venues: Vec<_> = instrument.listings.iter().map(|item| item.venue).collect();
    venues.sort_by_key(|venue| *venue as u8);
    assert_eq!(venues, [Venue::Nse, Venue::Bse]);
    assert_eq!(universe.reconciliation.added.len(), 1);
    assert_eq!(universe.listing_states.len(), 2);
}

#[test]
fn full_snapshot_removal_is_per_venue() {
    let day1 = date(2026, 1, 1);
    let day2 = date(2026, 2, 1);
    let first = reconciled(
        day1,
        vec![
            normalized_nse("RELIANCE", RELIANCE_ISIN, day1),
            normalized_bse("RELIANCE", RELIANCE_ISIN, day1),
        ],
    );

    // Full NSE master omits it: only the NSE listing closes; BSE stays.
    let second = reconcile_from(
        &first.instruments,
        vec![normalized_nse("HDFCBANK", HDFC_ISIN, day2)],
        day2,
        true,
    );
    let instrument = second
        .instruments
        .iter()
        .find(|item| item.instrument_id == InstrumentId::new("in.ine002a01018").unwrap())
        .unwrap();
    let nse = instrument
        .listings
        .iter()
        .find(|item| item.venue == Venue::Nse)
        .unwrap();
    let bse = instrument
        .listings
        .iter()
        .find(|item| item.venue == Venue::Bse)
        .unwrap();
    assert!(
        nse.effective_to.is_some(),
        "NSE listing closed by the NSE master"
    );
    assert!(
        bse.effective_to.is_none(),
        "BSE listing untouched: different venue"
    );
    assert!(
        second.reconciliation.removed.is_empty(),
        "instrument with an open BSE listing is not removed"
    );
}

#[test]
fn symbol_reuse_after_interval_end_is_allowed() {
    // A different instrument takes over a symbol after the previous
    // listing ended: no overlap, no collision. Reconciliations are
    // date-disciplined (strictly increasing `as_of` per universe):
    // a same-day restatement fails closed (see below).
    let jan = date(2026, 1, 1);
    let feb = date(2026, 2, 1);
    let mar = date(2026, 3, 1);
    let old = bootstrapped("RELIANCE", RELIANCE_ISIN, jan);
    // Close the old one's NSE interval by reconciling its full-snapshot
    // absence, then hand the symbol to a new ISIN.
    let closed = reconcile_from(
        &old,
        vec![normalized_nse("HDFCBANK", HDFC_ISIN, feb)],
        feb,
        true,
    );
    let reuse = reconcile_from(
        &closed.instruments,
        vec![normalized_nse("RELIANCE", HDFC_ISIN, mar)],
        mar,
        false,
    );
    assert!(reuse.instruments.iter().any(|item| {
        item.listings
            .iter()
            .any(|listing| listing.venue_symbol == "RELIANCE" && listing.effective_from == mar)
    }));
}

#[test]
fn same_day_restatement_fails_closed() {
    // Two different universes under the same `as_of` cannot be ordered at
    // day granularity: fail closed instead of destroying history.
    let feb = date(2026, 2, 1);
    let first = bootstrapped("RELIANCE", RELIANCE_ISIN, feb);
    let prov = provenance(Venue::Nse, NSE_CSV);
    let attempt = reconcile(
        &first,
        vec![normalized_nse("RELIND", RELIANCE_ISIN, feb)],
        feb,
        false,
        at(feb).max(prov.retrieved_at),
        &prov,
    );
    assert!(
        attempt.is_err(),
        "same-day restatement of a listing must fail closed"
    );
}

#[test]
fn be_listing_roundtrip_is_trade_for_trade_not_suspended() {
    let day = date(2026, 1, 1);
    let universe = reconciled(
        day,
        vec![normalized_with(
            "HDFCBANK",
            HDFC_ISIN,
            day,
            ListingStatus::Active,
            SettlementRegime::TradeForTrade,
            true,
        )],
    );
    let listing = &universe.instruments[0].listings[0];
    assert_eq!(listing.status, ListingStatus::Active);
    assert_eq!(listing.settlement_regime, SettlementRegime::TradeForTrade);
    assert!(listing.z_category);
    assert!(universe.listing_states[0].z_category);
}

// ---- Provenance / determinism / store ----

#[test]
fn provenance_hash_is_stable_and_recorded() {
    let first = provenance(Venue::Nse, NSE_CSV);
    let second = provenance(Venue::Nse, NSE_CSV);
    assert_eq!(first.sha256_hex, second.sha256_hex, "same bytes, same hash");
    assert_eq!(first.record_count, 2);
    let different = provenance(Venue::Bse, BSE_CSV);
    assert_ne!(different.sha256_hex, first.sha256_hex);
    first.validate().unwrap();
}

#[test]
fn rebuild_snapshot_is_deterministic() {
    let day = date(2026, 1, 1);
    let build = |rows: Vec<NormalizedListing>| {
        let prov = provenance(Venue::Nse, NSE_CSV);
        let reconciled = reconcile(&[], rows, day, false, at(day), &prov).unwrap();
        ReferenceSnapshot::from_reconciled(reconciled, day, at(day), prov, &[]).unwrap()
    };
    let a = build(vec![
        normalized_nse("RELIANCE", RELIANCE_ISIN, day),
        normalized_nse("HDFCBANK", HDFC_ISIN, day),
    ]);
    let b = build(vec![
        normalized_nse("HDFCBANK", HDFC_ISIN, day),
        normalized_nse("RELIANCE", RELIANCE_ISIN, day),
    ]);
    assert_eq!(
        a.snapshot_hash().unwrap(),
        b.snapshot_hash().unwrap(),
        "row order cannot matter"
    );
    assert_eq!(a.universe_on(day).len(), 2);
    assert_eq!(
        a.symbol_on(&a.instruments[0].instrument_id, Venue::Nse, day)
            .unwrap(),
        "RELIANCE"
    );
    assert_eq!(
        a.symbol_on(&a.instruments[1].instrument_id, Venue::Nse, day)
            .unwrap(),
        "HDFCBANK"
    );
    assert_eq!(a.bitemporal_listings().len(), 2);
    assert_eq!(a.bitemporal_listings()[0].recorded_at, at(day));
}

#[test]
fn snapshot_hash_covers_listing_state_history() {
    // History-only mutation: same instrument state, different recorded
    // history -> different hash (finding E).
    let jan = date(2026, 1, 10);
    let feb = date(2026, 2, 10);
    let prov = provenance(Venue::Nse, NSE_CSV);

    let first = reconciled(jan, vec![normalized_nse("RELIANCE", RELIANCE_ISIN, jan)]);
    let snap1 = ReferenceSnapshot::from_reconciled(first, jan, at(jan), prov.clone(), &[]).unwrap();

    let second = reconcile_from(
        &snap1.clone().instruments,
        vec![normalized_with(
            "RELIANCE",
            RELIANCE_ISIN,
            feb,
            ListingStatus::Suspended,
            SettlementRegime::Standard,
            false,
        )],
        feb,
        false,
    );
    let snap2 = ReferenceSnapshot::from_reconciled(
        second,
        feb,
        at(feb),
        prov.clone(),
        snap1.bitemporal_listings(),
    )
    .unwrap();

    assert_ne!(
        snap1.snapshot_hash().unwrap(),
        snap2.snapshot_hash().unwrap(),
        "different histories must not collide"
    );
    // States are in the hash: rebuild with identical states is stable.
    let rebuild = ReferenceSnapshot::from_reconciled(
        reconciled(jan, vec![normalized_nse("RELIANCE", RELIANCE_ISIN, jan)]),
        jan,
        at(jan),
        prov.clone(),
        &[],
    )
    .unwrap();
    assert_eq!(
        snap1.snapshot_hash().unwrap(),
        rebuild.snapshot_hash().unwrap()
    );
}

#[test]
fn bitemporal_reference_answers_both_axes() {
    let jan = date(2026, 1, 10);
    let feb = date(2026, 2, 10);
    let prov = provenance(Venue::Nse, NSE_CSV);

    let first = reconciled(jan, vec![normalized_nse("RELIANCE", RELIANCE_ISIN, jan)]);
    let snap1 = ReferenceSnapshot::from_reconciled(first, jan, at(jan), prov.clone(), &[]).unwrap();

    let second = reconcile_from(
        &snap1.instruments.clone(),
        vec![normalized_with(
            "RELIANCE",
            RELIANCE_ISIN,
            feb,
            ListingStatus::Suspended,
            SettlementRegime::Standard,
            false,
        )],
        feb,
        false,
    );
    let snap2 = ReferenceSnapshot::from_reconciled(
        second,
        feb,
        at(feb),
        prov.clone(),
        snap1.bitemporal_listings(),
    )
    .unwrap();

    let mut store = ReferenceStore::default();
    store.publish(snap1).unwrap();
    store.publish(snap2).unwrap();
    assert_eq!(store.revision_count(), 2);

    // As known in January, the security was Active on 2026-01-15.
    let then = store.reference(date(2026, 1, 15), at(date(2026, 1, 20)));
    assert_eq!(then.len(), 1);
    assert_eq!(then[0].status, ListingStatus::Active);
    // As known in March, it was Suspended on 2026-02-15.
    let later = store.reference(date(2026, 2, 15), at(date(2026, 3, 1)));
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].status, ListingStatus::Suspended);
    // Knowledge-time travel over valid-time January: still Active.
    let known = store.known_universe(date(2026, 1, 20), at(date(2026, 3, 1)));
    assert!(!known.is_empty());
    // Before it was ever listed: nothing known.
    assert!(
        store
            .reference(date(2025, 12, 1), at(date(2026, 3, 1)))
            .is_empty()
    );
}

#[test]
fn corporate_action_ledger_rejects_duplicate_slot_and_backdating() {
    let prov = provenance_for_source(
        "nse-corp-actions",
        Venue::Nse,
        Segment::Cash,
        date(2026, 1, 10).and_hms_opt(10, 0, 0).unwrap().and_utc(),
        "nse-corp-actions-v1",
        "test",
        b"action",
    );
    let record = CorporateActionRecord {
        instrument_id: InstrumentId::new("in.ine002a01018").unwrap(),
        kind: CorporateActionKind::Split,
        effective_date: date(2026, 2, 1),
        numerator: 1,
        denominator: 2,
        amount_paise: None,
        recorded_at: date(2026, 1, 10).and_hms_opt(10, 0, 0).unwrap().and_utc(),
        provenance: prov.clone(),
    };
    let mut ledger = CorporateActionLedger::default();
    ledger.append(record.clone()).unwrap();
    assert!(
        ledger.append(record.clone()).is_err(),
        "one slot, one record"
    );

    let mut bad = record.clone();
    bad.numerator = 0;
    assert!(ledger.append(bad).is_err(), "ratio must be positive");

    let mut backdated = record.clone();
    backdated.recorded_at = date(2026, 1, 1).and_hms_opt(10, 0, 0).unwrap().and_utc();
    backdated.effective_date = date(2026, 2, 2);
    assert!(
        ledger.append(backdated).is_err(),
        "backdating must be rejected"
    );

    let known = ledger.known_by(
        date(2026, 2, 2),
        date(2026, 1, 10).and_hms_opt(11, 0, 0).unwrap().and_utc(),
    );
    assert_eq!(known.len(), 1);
    let unknown = ledger.known_by(
        date(2026, 2, 2),
        date(2026, 1, 9).and_hms_opt(11, 0, 0).unwrap().and_utc(),
    );
    assert!(unknown.is_empty(), "knowledge time gates availability");
}

// ---- Monthly tick assignment (finding C) ----

#[test]
fn monthly_assignment_is_fixed_for_the_whole_month() {
    let csv_2025 = "LOWER_PAISE,UPPER_PAISE,TICK_PAISE\n\
0,24999,1\n25000,100000,5\n100001,500000,10\n500001,1000000,50\n1000001,2000000,100\n2000001,,500\n";
    let schedule = parse_tick_schedule_csv(date(2025, 4, 15), csv_2025).unwrap();
    let prov = provenance(Venue::Nse, NSE_CSV);
    let revision = nirnay_extras::venue::india::ExchangeTickSchedule {
        effective_from: date(2025, 4, 15),
        schedule,
        provenance: prov,
    };
    let book = TickScheduleBook::new(vec![revision]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let archive = SourceArchive::open(dir.path()).unwrap();
    let calendar = provenanced_month(&archive, 2025, 5);
    // Reference close 30,000 paise (₹300) on 2025-05-30, the last session of
    // May: tick fixed at 5 paise for all of June, even if the live price
    // crosses another band.
    let reference = archived_reference(&archive, date(2025, 5, 30), 30_000);
    let assignment = book
        .monthly_assignment(date(2025, 6, 15), &reference, &archive, &calendar)
        .unwrap();
    assert_eq!(assignment.tick_paise, 5);
    assert_eq!(assignment.reference_close_paise, Some(30_000));
    assert_eq!(assignment.reference_close_date, Some(date(2025, 5, 30)));
    assert_eq!(
        assignment.reference_source.as_deref(),
        Some("nse-eod-close")
    );
    assert_eq!(
        assignment.reference_sha256_hex.as_deref(),
        Some(reference.sha256_hex.as_str()),
        "the assignment carries the archived object's hash"
    );
    assert_eq!(assignment.effective_month, Some(date(2025, 6, 1)));
    assert_eq!(
        assignment.schedule_revision,
        "nse-cm-master:nse-cm-master-v1"
    );
    assert_eq!(assignment.schedule_effective_from, Some(date(2025, 4, 15)));
    assert_eq!(
        assignment.schedule_sha256_hex.as_deref(),
        Some(prov_sha256().as_str())
    );
    assignment.validate().unwrap();

    let low = book
        .monthly_assignment(
            date(2025, 6, 15),
            &archived_reference(&archive, date(2025, 5, 30), 10_000),
            &archive,
            &calendar,
        )
        .unwrap();
    assert_eq!(low.tick_paise, 1, "sub-₹250 reference close");
    assert!(
        book.monthly_assignment(date(2025, 4, 14), &reference, &archive, &calendar)
            .is_err(),
        "before the first revision"
    );
    assert_eq!(book.tick_for(date(2025, 6, 1), 24_999), Some(1));
    assert_eq!(book.tick_for(date(2025, 6, 1), 100_001), Some(10));

    // A close from the wrong month is not the reference close for June.
    assert!(
        book.monthly_assignment(
            date(2025, 6, 15),
            &archived_reference(&archive, date(2025, 6, 2), 30_000),
            &archive,
            &calendar,
        )
        .is_err(),
        "the close must come from the month before the assignment"
    );
    // The close must be the last trading session, not any date in the month.
    assert!(
        book.monthly_assignment(
            date(2025, 6, 15),
            &archived_reference(&archive, date(2025, 5, 29), 30_000),
            &archive,
            &calendar,
        )
        .is_err(),
        "an earlier May session is not the reference close"
    );
    // A syntactically valid but fabricated hash does not resolve to bytes.
    let mut unverifiable = reference.clone();
    unverifiable.sha256_hex = "cd".repeat(32);
    assert!(
        book.monthly_assignment(date(2025, 6, 15), &unverifiable, &archive, &calendar)
            .is_err(),
        "a source hash that is not archived must be rejected"
    );
    // A genuine archive hash does not license a different price: the
    // archived record must contain the observation's own close.
    let mut repriced = reference.clone();
    repriced.close_paise = 30_100;
    assert!(
        book.monthly_assignment(date(2025, 6, 15), &repriced, &archive, &calendar)
            .is_err(),
        "the archived record must contain the supplied close"
    );
    // Nor another security's close from the same file.
    let mut other_isin = reference.clone();
    other_isin.isin = Isin::new(HDFC_ISIN).unwrap();
    assert!(
        book.monthly_assignment(date(2025, 6, 15), &other_isin, &archive, &calendar)
            .is_err(),
        "the archived record must contain the observation's instrument"
    );
    // Nor a date the archived record does not have.
    let mut other_date = reference.clone();
    other_date.date = date(2025, 5, 29);
    assert!(
        book.monthly_assignment(date(2025, 6, 15), &other_date, &archive, &calendar)
            .is_err(),
        "the archived record must contain the observation's date"
    );
    // A sparse calendar cannot prove which session was last: only 29 May
    // has an entry, so absence of 30 May is not evidence.
    let sparse = TradingCalendar {
        venue: Venue::Nse,
        days: vec![CalendarDay {
            venue: Venue::Nse,
            date: date(2025, 5, 29),
            session: SessionDay::Trading,
            note: None,
        }],
    };
    let bare = BuiltCalendar::synthetic_for_tests(sparse);
    assert!(
        reference
            .verify_last_session(&bare, date(2025, 6, 1))
            .is_err(),
        "verify_last_session itself must reject a bare calendar"
    );
    assert!(
        book.monthly_assignment(date(2025, 6, 15), &reference, &archive, &bare)
            .is_err(),
        "a bare calendar without source provenance must not fix a tick"
    );
    // A provenanced calendar that does not cover the whole month cannot
    // prove the last session either.
    let partial = provenanced_range(
        &archive,
        date(2025, 5, 1),
        date(2025, 5, 15),
        "DATE,DAY,HOLIDAY,PURPOSE\n",
    );
    assert!(
        book.monthly_assignment(date(2025, 6, 15), &reference, &archive, &partial)
            .is_err(),
        "a calendar that does not cover the whole month cannot prove the last session"
    );
    // A full month is required, not just the close date.
    assert!(
        book.monthly_assignment(
            date(2025, 6, 15),
            &reference,
            &archive,
            &provenanced_month(&archive, 2025, 4),
        )
        .is_err(),
        "a calendar for the wrong month must fail closed"
    );
    // The archived object must be recorded under the observation's dataset.
    let mut wrong_source = reference;
    wrong_source.source_id = "some-other-dataset".to_owned();
    assert!(
        book.monthly_assignment(date(2025, 6, 15), &wrong_source, &archive, &calendar)
            .is_err(),
        "dataset identity must match the archived object"
    );
}

#[test]
fn calendar_builder_expands_holiday_exceptions_into_a_complete_month() {
    // An exchange holiday file lists exceptions only. 1 May and 30 May are
    // holidays, 25 May is a Sunday Muhurat special session, and every other
    // weekday is a normal session.
    // An annual exchange file: only April-June entries are relevant here,
    // and the out-of-range ones must not affect May.
    let holiday_csv = "DATE,DAY,HOLIDAY,PURPOSE\n\
2025-04-14,Monday,Ambedkar Jayanti,TRADING HOLIDAY\n\
2025-05-01,Thursday,Maharashtra Day,TRADING HOLIDAY\n\
2025-05-25,Sunday,Muhurat,SPECIAL LIVE TRADING SESSION\n\
2025-05-30,Friday,Special Holiday,TRADING HOLIDAY\n\
2025-06-02,Monday,Bakri Id,TRADING HOLIDAY\n";
    let exceptions = parse_nse_holiday_csv(holiday_csv).unwrap();
    assert_eq!(exceptions.days.len(), 5, "only exceptions are in the file");

    let dir = tempfile::tempdir().unwrap();
    let archive = SourceArchive::open(dir.path()).unwrap();
    archive
        .store(
            "nse-cash-calendar",
            holiday_csv.as_bytes(),
            at(date(2025, 5, 1)),
        )
        .unwrap();

    let source = provenance_for_source(
        "nse-cash-calendar",
        Venue::Nse,
        Segment::Cash,
        date(2025, 5, 1).and_hms_opt(9, 0, 0).unwrap().and_utc(),
        "nse-cash-calendar-v1",
        "test",
        holiday_csv.as_bytes(),
    );
    let built = build_calendar(
        Venue::Nse,
        date(2025, 5, 1),
        date(2025, 5, 31),
        holiday_csv,
        source.clone(),
        &archive,
    )
    .unwrap();

    assert_eq!(built.provenance(), Some(&source));
    assert_eq!(
        built.revision().unwrap(),
        "nse-cash-calendar:nse-cash-calendar-v1"
    );
    assert_eq!(built.sha256_hex().unwrap(), source.sha256_hex);
    assert_eq!(built.calendar().days.len(), 31);
    assert!(built.calendar().covers_month(date(2025, 5, 1)));
    assert!(!built.calendar().is_trading_day(date(2025, 5, 1)));
    assert!(built.calendar().is_trading_day(date(2025, 5, 2)));
    assert!(
        !built.calendar().is_trading_day(date(2025, 5, 3)),
        "Saturday"
    );
    assert!(
        built.calendar().is_trading_day(date(2025, 5, 25)),
        "a special session on Sunday is a trading day"
    );
    assert_eq!(
        built
            .calendar()
            .days
            .iter()
            .find(|day| day.date == date(2025, 5, 25))
            .map(|day| day.session),
        Some(SessionDay::Special)
    );
    assert!(!built.calendar().is_trading_day(date(2025, 5, 30)));
    assert!(
        !built.calendar().is_trading_day(date(2025, 5, 31)),
        "Saturday"
    );

    // End to end from parsed holiday input through the monthly assignment.
    let csv_2025 = "LOWER_PAISE,UPPER_PAISE,TICK_PAISE\n\
0,24999,1\n25000,100000,5\n100001,500000,10\n500001,1000000,50\n1000001,2000000,100\n2000001,,500\n";
    let schedule = parse_tick_schedule_csv(date(2025, 4, 15), csv_2025).unwrap();
    let revision = ExchangeTickSchedule {
        effective_from: date(2025, 4, 15),
        schedule,
        provenance: provenance(Venue::Nse, NSE_CSV),
    };
    let book = TickScheduleBook::new(vec![revision]).unwrap();

    // 30 May is a declared holiday, so the last session is 29 May.
    let reference = archived_reference(&archive, date(2025, 5, 29), 30_000);
    let assignment = book
        .monthly_assignment(date(2025, 6, 15), &reference, &archive, &built)
        .unwrap();
    assert_eq!(assignment.tick_paise, 5);
    assert_eq!(
        assignment.reference_close_date,
        Some(date(2025, 5, 29)),
        "the holiday on 30 May makes 29 May the reference session"
    );
    assert_eq!(
        assignment.calendar_revision.as_deref(),
        Some("nse-cash-calendar:nse-cash-calendar-v1"),
        "the assignment carries the calendar source identity and revision"
    );
    assert_eq!(
        assignment.calendar_sha256_hex.as_deref(),
        Some(source.sha256_hex.as_str()),
        "the assignment carries the exact holiday-source hash"
    );
    assignment.validate().unwrap();

    // A close on 28 May is no longer the last session in this calendar.
    assert!(
        book.monthly_assignment(
            date(2025, 6, 15),
            &archived_reference(&archive, date(2025, 5, 28), 30_000),
            &archive,
            &built,
        )
        .is_err(),
        "the builder must apply the holiday exception, not just weekday rules"
    );
}

#[test]
fn calendar_builder_binds_provenance_to_the_exact_holiday_bytes() {
    let holiday_csv = "DATE,DAY,HOLIDAY,PURPOSE\n\
2025-04-14,Monday,Ambedkar Jayanti,TRADING HOLIDAY\n\
2025-05-01,Thursday,Maharashtra Day,TRADING HOLIDAY\n\
2025-05-25,Sunday,Muhurat,SPECIAL LIVE TRADING SESSION\n\
2025-05-30,Friday,Special Holiday,TRADING HOLIDAY\n\
2025-06-02,Monday,Bakri Id,TRADING HOLIDAY\n";
    let dir = tempfile::tempdir().unwrap();
    let archive = SourceArchive::open(dir.path()).unwrap();
    archive
        .store(
            "nse-cash-calendar",
            holiday_csv.as_bytes(),
            at(date(2025, 5, 1)),
        )
        .unwrap();
    let source = provenance_for_source(
        "nse-cash-calendar",
        Venue::Nse,
        Segment::Cash,
        date(2025, 5, 1).and_hms_opt(9, 0, 0).unwrap().and_utc(),
        "nse-cash-calendar-v1",
        "test",
        holiday_csv.as_bytes(),
    );

    // The exact bytes build a provenanced calendar.
    let may = build_calendar(
        Venue::Nse,
        date(2025, 5, 1),
        date(2025, 5, 31),
        holiday_csv,
        source.clone(),
        &archive,
    )
    .unwrap();
    assert!(may.calendar().covers_month(date(2025, 5, 1)));

    // An annual file may carry exceptions outside the requested month; they
    // cannot affect the built range and are ignored, not mistaken for
    // missing sessions. The provenance still covers the whole file.
    let june = build_calendar(
        Venue::Nse,
        date(2025, 6, 1),
        date(2025, 6, 30),
        holiday_csv,
        source.clone(),
        &archive,
    )
    .unwrap();
    assert!(june.calendar().covers_month(date(2025, 6, 1)));
    assert_eq!(june.calendar().days.len(), 30);
    assert!(
        !june.calendar().is_trading_day(date(2025, 5, 1)),
        "May dates are not part of the June calendar"
    );

    // Provenance for a different file cannot describe these bytes: even a
    // single omitted row changes the hash.
    let other = provenance_for_source(
        "nse-cash-calendar",
        Venue::Nse,
        Segment::Cash,
        date(2025, 5, 1).and_hms_opt(9, 0, 0).unwrap().and_utc(),
        "nse-cash-calendar-v1",
        "test",
        b"DATE,DAY,HOLIDAY,PURPOSE\n",
    );
    assert!(
        build_calendar(
            Venue::Nse,
            date(2025, 5, 1),
            date(2025, 5, 31),
            holiday_csv,
            other,
            &archive,
        )
        .is_err(),
        "a provenance hash for different bytes must be rejected"
    );
    let truncated = holiday_csv.replace("2025-05-30,Friday,Special Holiday,TRADING HOLIDAY\n", "");
    assert!(
        build_calendar(
            Venue::Nse,
            date(2025, 5, 1),
            date(2025, 5, 31),
            &truncated,
            source.clone(),
            &archive,
        )
        .is_err(),
        "editing the holiday bytes invalidates the recorded hash"
    );

    // The recorded row count is part of the binding.
    let mut wrong_count = source.clone();
    wrong_count.record_count += 1;
    assert!(
        build_calendar(
            Venue::Nse,
            date(2025, 5, 1),
            date(2025, 5, 31),
            holiday_csv,
            wrong_count,
            &archive,
        )
        .is_err(),
        "a provenance row count that differs from the parsed file must be rejected"
    );

    // Provenance for another venue is not evidence for this calendar.
    let bse_source = provenance_for_source(
        "bse-cash-calendar",
        Venue::Bse,
        Segment::Cash,
        date(2025, 5, 1).and_hms_opt(9, 0, 0).unwrap().and_utc(),
        "bse-cash-calendar-v1",
        "test",
        holiday_csv.as_bytes(),
    );
    assert!(
        build_calendar(
            Venue::Nse,
            date(2025, 5, 1),
            date(2025, 5, 31),
            holiday_csv,
            bse_source,
            &archive,
        )
        .is_err()
    );

    // The archive entry must be recorded under the provenance's dataset,
    // not merely contain the same bytes.
    let mut wrong_dataset = source.clone();
    wrong_dataset.source_id = "nse-tick-schedule".to_owned();
    assert!(
        build_calendar(
            Venue::Nse,
            date(2025, 5, 1),
            date(2025, 5, 31),
            holiday_csv,
            wrong_dataset,
            &archive,
        )
        .is_err(),
        "the calendar source must be archived under its own dataset identity"
    );

    // Bytes that were never retained cannot back an assignment hash.
    let unarchived_csv =
        "DATE,DAY,HOLIDAY,PURPOSE\n2025-01-01,Wednesday,New Year,TRADING HOLIDAY\n";
    let unarchived = provenance_for_source(
        "nse-cash-calendar",
        Venue::Nse,
        Segment::Cash,
        date(2025, 5, 1).and_hms_opt(9, 0, 0).unwrap().and_utc(),
        "nse-cash-calendar-v1",
        "test",
        unarchived_csv.as_bytes(),
    );
    assert!(
        build_calendar(
            Venue::Nse,
            date(2025, 5, 1),
            date(2025, 5, 31),
            unarchived_csv,
            unarchived,
            &archive,
        )
        .is_err(),
        "a holiday source that is not in the archive must be rejected"
    );

    // Contract v2 defines only the cash segment, so a non-cash provenance
    // is not constructible; the builder still checks it explicitly so the
    // rule survives a future segment addition.
    assert_eq!(Segment::v2_supported(), [Segment::Cash]);

    // An inverted range cannot prove coverage.
    assert!(
        build_calendar(
            Venue::Nse,
            date(2025, 5, 31),
            date(2025, 5, 1),
            holiday_csv,
            source,
            &archive,
        )
        .is_err()
    );
}

#[test]
fn archived_close_rejects_a_conflicting_duplicate_row() {
    let dir = tempfile::tempdir().unwrap();
    let archive = SourceArchive::open(dir.path()).unwrap();
    let day = date(2025, 5, 30);
    // The proposed observation matches the first row, but the source also
    // carries a different close for the same instrument and date.
    let bytes = format!(
        "DATE,SYMBOL,ISIN,CLOSE_PAISE\n\
{day},RELIANCE,{RELIANCE_ISIN},30000\n\
{day},RELIANCE,{RELIANCE_ISIN},30100\n"
    );
    let stored = archive
        .store("nse-eod-close", bytes.as_bytes(), at(day))
        .unwrap();
    let reference = ReferenceClose {
        date: day,
        isin: Isin::new(RELIANCE_ISIN).unwrap(),
        close_paise: 30_000,
        source_id: "nse-eod-close".to_owned(),
        sha256_hex: stored.sha256_hex,
    };
    let error = reference
        .verify_archived(&archive)
        .expect_err("contradictory source rows are not a reproducible observation");
    assert!(
        error.to_string().contains("duplicate close record"),
        "unexpected error: {error}"
    );
}

fn prov_sha256() -> String {
    provenance(Venue::Nse, NSE_CSV).sha256_hex
}

#[test]
fn tick_schedule_csv_rejects_gaps() {
    let gap = "LOWER_PAISE,UPPER_PAISE,TICK_PAISE\n0,99,1\n101,,5\n";
    assert!(parse_tick_schedule_csv(date(2025, 4, 15), gap).is_err());
}

// ---- Raw-source archive (integration requirement + durability) ----

#[test]
fn archive_is_write_once_and_verifiable() {
    let dir = tempfile::tempdir().unwrap();
    let archive = SourceArchive::open(dir.path()).unwrap();
    let bytes = b"SYMBOL,ISIN\nA,INE002A01018\n";
    let first = archive
        .store("nse-cm-master", bytes, at(date(2026, 1, 1)))
        .unwrap();
    assert_eq!(first.bytes, bytes.len() as u64);
    assert_eq!(first.sha256_hex.len(), 64);
    // Same bytes again: the original reference, never a rewrite.
    let second = archive
        .store("nse-cm-master", bytes, at(date(2026, 1, 2)))
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(archive.fetch(&first.sha256_hex).unwrap(), bytes);
    let verified = archive.verify().unwrap();
    assert_eq!(verified.len(), 1);
    assert!(
        dir.path()
            .join(format!("{}.src", first.sha256_hex))
            .exists()
    );
    assert!(
        archive.fetch("0".repeat(64).as_str()).is_err(),
        "unknown object"
    );
}

#[test]
fn archive_survives_interrupted_publication() {
    let dir = tempfile::tempdir().unwrap();
    let archive = SourceArchive::open(dir.path()).unwrap();
    let bytes = b"partial data";
    // Simulate an interrupted writer: a leftover temp file and no entry.
    let tmp = dir.path().join("leftover.tmp");
    std::fs::write(&tmp, b"garbage").unwrap();
    let stored = archive
        .store("nse-cm-master", bytes, at(date(2026, 1, 1)))
        .unwrap();
    assert_eq!(archive.fetch(&stored.sha256_hex).unwrap(), bytes);
    let verified = archive.verify().unwrap();
    assert_eq!(
        verified.len(),
        1,
        "interrupted publication never poisons the manifest"
    );
    // Corrupting a stored object is detected, never silently accepted.
    let object = dir.path().join(format!("{}.src", stored.sha256_hex));
    std::fs::write(&object, b"tampered").unwrap();
    assert!(archive.fetch(&stored.sha256_hex).is_err());
    assert!(archive.verify().is_err());
}

// ---- Reconcile knowledge-time lineage ----

#[test]
fn reconcile_rejects_knowledge_before_retrieval() {
    let day = date(2026, 1, 1);
    let prov = provenance_for_source(
        "nse-cm-master",
        Venue::Nse,
        Segment::Cash,
        date(2026, 2, 1).and_hms_opt(10, 0, 0).unwrap().and_utc(),
        "nse-cm-master-v1",
        "test",
        NSE_CSV.as_bytes(),
    );
    let result = reconcile(
        &[],
        vec![normalized_nse("RELIANCE", RELIANCE_ISIN, day)],
        day,
        false,
        date(2026, 1, 15).and_hms_opt(10, 0, 0).unwrap().and_utc(),
        &prov,
    );
    assert!(result.is_err(), "recorded before retrieval is backdating");
}

// ---- BSE market lot and security type through the pipeline ----

fn bse_provenance(csv: &str) -> SourceProvenance {
    provenance_for_source(
        "bse-security-master",
        Venue::Bse,
        Segment::Cash,
        date(2026, 1, 1).and_hms_opt(10, 0, 0).unwrap().and_utc(),
        "bse-security-master-v1",
        "test",
        csv.as_bytes(),
    )
}

fn normalized_bse_group(
    symbol: &str,
    isin: &str,
    as_of: NaiveDate,
    group: &str,
) -> NormalizedListing {
    let mut row = normalized_bse(symbol, isin, as_of);
    row.settlement_group = Some(group.to_owned());
    let class = classify_bse_group(group).unwrap();
    row.settlement_regime = class.settlement_regime;
    row.z_category = class.z_category;
    row
}

#[test]
fn bse_market_lot_survives_the_whole_pipeline() {
    let csv =
        format!("{BSE_HEADER}\n500325,RELIANCE,Reliance Industries,T,INE002A01018,0.10,25,EQ\n");
    let rows = parse_bse_master(&csv).unwrap();
    assert_eq!(rows[0].market_lot, 25, "parser retains the exchange lot");
    let prov = bse_provenance(&csv);
    let carry = bse_carry(&rows[0], date(2026, 1, 1), &prov);
    assert_eq!(carry.market_lot, 25);
    assert_eq!(carry.token.as_deref(), Some("500325"));
    assert_eq!(carry.settlement_group.as_deref(), Some("T"));
    assert_eq!(carry.tick_assignment.tick_paise, 10);
    assert_eq!(
        carry.settlement_regime,
        SettlementRegime::TradeForTrade,
        "BSE group T must not be carried as standard"
    );
    assert_eq!(
        carry.tick_assignment.schedule_revision,
        "bse-security-master:bse-security-master-v1"
    );
    assert_eq!(
        carry.tick_assignment.schedule_sha256_hex.as_deref(),
        Some(prov.sha256_hex.as_str())
    );

    let universe = reconcile(
        &[],
        vec![carry],
        date(2026, 1, 1),
        true,
        at(date(2026, 1, 1)).max(prov.retrieved_at),
        &prov,
    )
    .unwrap();
    let instrument = &universe.instruments[0];
    assert_eq!(instrument.instrument_type, InstrumentType::Equity);
    let listing = &instrument.listings[0];
    assert_eq!(
        listing.terms[0].market_lot, 25,
        "lot is not flattened to one"
    );
    assert_eq!(listing.token.as_deref(), Some("500325"));
    assert_eq!(listing.settlement_group.as_deref(), Some("T"));
    assert_eq!(
        instrument
            .effective_terms(date(2026, 1, 1))
            .unwrap()
            .market_lot,
        25
    );
    let state = &universe.listing_states[0];
    assert_eq!(state.market_lot, 25);
    assert_eq!(state.token.as_deref(), Some("500325"));
    assert_eq!(state.settlement_group.as_deref(), Some("T"));
}

// ---- Settlement-group state history ----

#[test]
fn settlement_group_history_is_bitemporal() {
    let jan = date(2026, 1, 1);
    let feb = date(2026, 2, 1);
    let mar = date(2026, 3, 1);
    let apr = date(2026, 4, 1);
    let prov = bse_provenance(BSE_CSV);

    let jan_u = reconcile(
        &[],
        vec![normalized_bse_group("RELIANCE", RELIANCE_ISIN, jan, "T")],
        jan,
        false,
        at(jan).max(prov.retrieved_at),
        &prov,
    )
    .unwrap();
    let jan_snap =
        ReferenceSnapshot::from_reconciled(jan_u, jan, at(jan), prov.clone(), &[]).unwrap();

    let feb_u = reconcile(
        &jan_snap.instruments,
        vec![normalized_bse_group("RELIANCE", RELIANCE_ISIN, feb, "X")],
        feb,
        false,
        at(feb),
        &prov,
    )
    .unwrap();
    let feb_snap = ReferenceSnapshot::from_reconciled(
        feb_u,
        feb,
        at(feb),
        prov.clone(),
        jan_snap.bitemporal_listings(),
    )
    .unwrap();

    let mar_u = reconcile(
        &feb_snap.instruments,
        vec![normalized_bse_group("RELIANCE", RELIANCE_ISIN, mar, "Y")],
        mar,
        false,
        at(mar),
        &prov,
    )
    .unwrap();
    let mar_snap = ReferenceSnapshot::from_reconciled(
        mar_u,
        mar,
        at(mar),
        prov.clone(),
        feb_snap.bitemporal_listings(),
    )
    .unwrap();

    // April full BSE master omits the scrip: the March state closes with
    // the group that was actually in force (Y), not the incoming group.
    let apr_u = reconcile(
        &mar_snap.instruments,
        vec![normalized_bse("HDFCBANK", HDFC_ISIN, apr)],
        apr,
        true,
        at(apr),
        &prov,
    )
    .unwrap();
    let apr_snap = ReferenceSnapshot::from_reconciled(
        apr_u,
        apr,
        at(apr),
        prov.clone(),
        mar_snap.bitemporal_listings(),
    )
    .unwrap();

    let mut store = ReferenceStore::default();
    store.publish(jan_snap).unwrap();
    store.publish(feb_snap).unwrap();
    store.publish(mar_snap).unwrap();
    store.publish(apr_snap).unwrap();

    let group_at = |valid: NaiveDate, recorded: NaiveDate| {
        store
            .reference(valid, at(recorded))
            .into_iter()
            .find(|state| state.instrument_id == InstrumentId::new("in.ine002a01018").unwrap())
            .and_then(|state| state.settlement_group)
    };
    assert_eq!(
        group_at(date(2026, 1, 15), date(2026, 4, 2)).as_deref(),
        Some("T")
    );
    assert_eq!(
        group_at(date(2026, 2, 15), date(2026, 4, 2)).as_deref(),
        Some("X")
    );
    assert_eq!(
        group_at(date(2026, 3, 15), date(2026, 4, 2)).as_deref(),
        Some("Y")
    );
    assert_eq!(
        group_at(date(2026, 3, 31), date(2026, 4, 2)).as_deref(),
        Some("Y"),
        "the closing state keeps the previous group"
    );
    assert!(
        group_at(date(2026, 4, 2), date(2026, 4, 2)).is_none(),
        "delisted on 2026-04-01: no listing state is valid in April"
    );
    // Knowledge-time gate: in February the March group was not known.
    assert_eq!(
        group_at(date(2026, 3, 15), date(2026, 2, 15)).as_deref(),
        Some("X"),
        "as known in February, March still carried the February group"
    );
}

#[test]
fn bitemporal_reference_returns_the_full_contract() {
    let jan = date(2026, 1, 1);
    let prov = bse_provenance(BSE_CSV);
    let universe = reconcile(
        &[],
        vec![normalized_bse_group("RELIANCE", RELIANCE_ISIN, jan, "T")],
        jan,
        false,
        at(jan).max(prov.retrieved_at),
        &prov,
    )
    .unwrap();
    let snapshot =
        ReferenceSnapshot::from_reconciled(universe, jan, at(jan), prov.clone(), &[]).unwrap();
    let mut store = ReferenceStore::default();
    store.publish(snapshot).unwrap();

    let states = store.reference(date(2026, 1, 15), at(date(2026, 1, 20)));
    assert_eq!(states.len(), 1);
    let state = &states[0];
    assert_eq!(state.token.as_deref(), Some("500325"));
    assert_eq!(state.market_lot, 1);
    assert_eq!(state.tick_assignment.tick_paise, 5);
    assert_eq!(
        state.settlement_regime,
        SettlementRegime::TradeForTrade,
        "BSE group T is trade-to-trade"
    );
    assert_eq!(state.settlement_group.as_deref(), Some("T"));
    state
        .tick_assignment
        .validate()
        .expect("the historical view carries a valid assignment");
}

// ---- NSE monthly assignment through nse_carry ----

#[test]
fn nse_carry_binds_the_assignment_to_the_reference_observation() {
    let day = date(2025, 6, 15);
    let rows = parse_nse_cm_master(NSE_CSV).unwrap();
    let prov = provenance_for_source(
        "nse-tick-schedule",
        Venue::Nse,
        Segment::Cash,
        date(2025, 4, 15).and_hms_opt(9, 0, 0).unwrap().and_utc(),
        "nse-tick-bands-v1",
        "test",
        b"LOWER_PAISE,UPPER_PAISE,TICK_PAISE\n0,,1\n",
    );
    let schedule = TickScheduleBook::new(vec![ExchangeTickSchedule {
        effective_from: date(2025, 4, 15),
        schedule: parse_tick_schedule_csv(
            date(2025, 4, 15),
            "LOWER_PAISE,UPPER_PAISE,TICK_PAISE\n0,24999,1\n25000,,5\n",
        )
        .unwrap(),
        provenance: prov.clone(),
    }])
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let archive = SourceArchive::open(dir.path()).unwrap();
    let calendar = provenanced_month(&archive, 2025, 5);
    let reference = archived_reference(&archive, date(2025, 5, 30), 30_000);
    let carry = nse_carry(&rows[0], day, &reference, &schedule, &archive, &calendar).unwrap();
    let assignment = &carry.tick_assignment;
    assert_eq!(assignment.tick_paise, 5);
    assert_eq!(assignment.reference_close_date, Some(date(2025, 5, 30)));
    assert_eq!(
        assignment.reference_source.as_deref(),
        Some("nse-eod-close")
    );
    assert_eq!(assignment.effective_month, Some(date(2025, 6, 1)));
    assert_eq!(
        assignment.schedule_revision,
        "nse-tick-schedule:nse-tick-bands-v1"
    );
    assert_eq!(
        assignment.schedule_sha256_hex.as_deref(),
        Some(prov.sha256_hex.as_str())
    );
    assert_eq!(
        assignment.calendar_revision.as_deref(),
        Some("nse-cash-calendar:nse-cash-calendar-v1")
    );
    assert_eq!(
        assignment.calendar_sha256_hex.as_deref(),
        calendar
            .provenance()
            .map(|source| source.sha256_hex.as_str()),
        "the carry assigns the provenanced calendar's exact source hash"
    );
    assert_eq!(
        assignment.reference_sha256_hex.as_deref(),
        Some(reference.sha256_hex.as_str())
    );
    assignment.validate().unwrap();

    // The observation must belong to the instrument being carried: another
    // security's (reproducible) close cannot fix this instrument's tick.
    let other = archived_reference_for(&archive, HDFC_ISIN, "HDFCBANK", date(2025, 5, 30), 30_000);
    let error = nse_carry(&rows[0], day, &other, &schedule, &archive, &calendar)
        .expect_err("an observation for another ISIN must not assign this tick");
    assert!(
        error.to_string().contains("cannot fix the tick"),
        "the rejection must name the identity mismatch: {error}"
    );
}

// ---- Archive: concurrent idempotency ----

#[test]
fn archive_concurrent_identical_stores_append_one_entry() {
    use std::sync::{Arc, Barrier};

    let dir = tempfile::tempdir().unwrap();
    let archive = Arc::new(SourceArchive::open(dir.path()).unwrap());
    let barrier = Arc::new(Barrier::new(8));
    let bytes = b"concurrent identical bytes";
    let mut handles = Vec::new();
    for _ in 0..8 {
        let archive = Arc::clone(&archive);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            archive
                .store("nse-cm-master", bytes, at(date(2026, 1, 1)))
                .unwrap()
        }));
    }
    let stored: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert!(
        stored.iter().all(|source| *source == stored[0]),
        "every concurrent caller gets the same reference"
    );
    assert_eq!(
        archive.entries().unwrap().len(),
        1,
        "check-and-publish is one critical section"
    );
    assert_eq!(archive.verify().unwrap().len(), 1);
    assert_eq!(archive.fetch(&stored[0].sha256_hex).unwrap(), bytes);
}
