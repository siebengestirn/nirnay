#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Parser and identity conformance tests.
//!
//! These are synthetic parser-conformance inputs. They establish the CSV
//! grammar and rule behavior only; they are not evidence that a dated exchange
//! source file has any particular layout.

use india_reference::{
    IdentityRegistry, //
    Limits,
    SnapshotScope,
    csv,
    source::bse,
    source::nse,
};
use trading_core::{
    Isin, //
    Venue,
};

const ISIN_A: &str = "INE848E01016";
const ISIN_B: &str = "INE002A01018";

fn nse_csv() -> String {
    format!(
        "SYMBOL,SERIES,ISIN NUMBER,NAME OF COMPANY,MARKET LOT\n\
         AAA,EQ,{ISIN_A},Alpha Limited,1\n\
         BBB,BE,{ISIN_B},Beta Limited,1\n\
         CCC,XX,INE009A01021,Gamma Limited,1\n\
         DDD,EQ,{ISIN_A},Delta Limited,1\n\
         EEE,EQ,,Empty Isin Limited,1\n"
    )
}

#[test]
fn nse_rows_have_explicit_outcomes_and_no_silent_skips() {
    let batch = nse::parse_nse_cash_master(
        nse_csv().as_bytes(),
        "nse-cash-equity-v1",
        SnapshotScope::Full {
            covered_venues: [Venue::Nse].into_iter().collect(),
        },
        Limits::default(),
    )
    .unwrap();
    assert!(
        batch.is_complete(),
        "every physical row has one disposition"
    );
    assert_eq!(batch.accepted.len(), 2, "two in-scope rows accepted");
    assert_eq!(
        batch.excluded.len(),
        1,
        "out-of-scope series excluded by rule"
    );
    assert_eq!(
        batch.rejected.len(),
        2,
        "duplicate ISIN and empty ISIN rejected"
    );
    assert!(
        batch.excluded[0].rule.contains("outside nse_cash_equity"),
        "scope rule is named"
    );
    assert!(
        batch
            .rejected
            .iter()
            .any(|item| item.reason.contains("duplicate")),
        "duplicate ISIN is rejected with a reason"
    );
}

#[test]
fn nse_missing_name_or_lot_is_rejected_not_defaulted() {
    let csv = format!("SYMBOL,SERIES,ISIN NUMBER,NAME OF COMPANY,MARKET LOT\nAAA,EQ,{ISIN_A},,1\n");
    let batch = nse::parse_nse_cash_master(
        csv.as_bytes(),
        "nse-cash-equity-v1",
        SnapshotScope::Partial,
        Limits::default(),
    )
    .unwrap();
    assert!(
        batch.accepted.is_empty(),
        "missing name must not be defaulted"
    );
    assert_eq!(batch.rejected.len(), 1, "the row is an explicit rejection");
}

fn bse_csv() -> String {
    format!(
        "SCRIP CODE,INSTRUMENT CODE,SCRIP NAME,ISIN CODE,NEW TICK SIZE,MARKET LOT,SECURITY TYPE FLAG,GROUP NAME,STATUS\n\
         500001,AAA,Alpha Limited,{ISIN_A},0.05,1,EQ,A,ACTIVE\n\
         500002,BBB,Beta Limited,{ISIN_B},0.05,1,EQ,ZZ,ACTIVE\n\
         500003,CCC,Gamma Limited,{ISIN_A},0.05,1,EQ,A,ACTIVE\n"
    )
}

#[test]
fn bse_group_classification_is_versioned_and_fails_closed() {
    let batch = bse::parse_bse_master(
        bse_csv().as_bytes(),
        "bse-security-master-v1",
        SnapshotScope::Partial,
        Limits::default(),
    )
    .unwrap();
    assert!(
        batch.is_complete(),
        "every physical row has one disposition"
    );
    assert_eq!(batch.accepted.len(), 1, "only classified group accepted");
    assert_eq!(
        batch.rejected.len(),
        2,
        "unknown group and duplicate ISIN rejected"
    );
    assert_eq!(
        batch.accepted[0].tick_paise, 5,
        "0.05 rupees is 5 paise exactly"
    );
    assert!(
        bse::parse_bse_master(
            bse_csv().as_bytes(),
            "unknown-version",
            SnapshotScope::Partial,
            Limits::default(),
        )
        .is_err(),
        "unknown source version must fail closed"
    );
}

fn standardised_bse_csv() -> String {
    format!(
        "FININSTRMID,TCKRSYMB,FININSTRMNM,ISIN,TICKSZ,NEWBRDLOTQTY,SCTYTPFLG,SCTYSRS,STS\n\
         500001,AAA,Alpha Limited,{ISIN_A},0.05,1,EQ,A,A\n\
         500002,BBB,Beta Limited,{ISIN_B},0.05,1,EQ,NS,A\n\
         500003,CCC,Gamma Limited,{ISIN_A},0.05,1,EQ,A,S\n\
         500004,DDD,Delta Mutual Fund,{ISIN_B},0.05,1,MF,A,A\n\
         500005,EEE,Echo Limited,{ISIN_A},0.05,1,EQ,A,A\n"
    )
}

#[test]
fn standardised_bse_master_accounts_for_every_declared_state() {
    let batch = bse::parse_bse_master(
        standardised_bse_csv().as_bytes(),
        "bse-security-master-v2",
        SnapshotScope::Partial,
        Limits::default(),
    )
    .unwrap();
    assert!(
        batch.is_complete(),
        "every physical row has exactly one outcome"
    );
    assert_eq!(batch.accepted.len(), 1, "active classified EQ row accepted");
    assert_eq!(
        batch.excluded.len(),
        2,
        "suspended status and non-EQ type are declared exclusions"
    );
    assert_eq!(
        batch.rejected.len(),
        2,
        "unknown group and duplicate ISIN fail closed"
    );
    assert_eq!(
        batch.accepted[0].scrip_code, "500001",
        "the active classified scrip keeps its source identifier"
    );
    assert_eq!(
        batch.accepted[0].tick_paise, 5,
        "0.05 rupees is 5 paise exactly"
    );
    assert!(
        bse::parse_bse_master(
            standardised_bse_csv().as_bytes(),
            "bse-security-master-v3",
            SnapshotScope::Partial,
            Limits::default(),
        )
        .is_err(),
        "an undeclared standardised version must still fail closed"
    );
}

#[test]
fn csv_parser_handles_logical_records_and_bom() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(b"A,B\r\n\"multi\r\nline\",\"quoted,comma\"\r\n");
    let document = csv::parse_document(&bytes, Limits::default()).unwrap();
    assert_eq!(document.header, vec!["A", "B"], "BOM is stripped");
    assert_eq!(
        document.rows.len(),
        1,
        "quoted newline is one logical record"
    );
    assert_eq!(
        document.rows[0].fields[0], "multi\r\nline",
        "embedded newline preserved"
    );
    assert_eq!(
        document.rows[0].fields[1], "quoted,comma",
        "quoted comma preserved"
    );
}

#[test]
fn csv_parser_rejects_malformed_quotes_and_wrong_field_counts() {
    assert!(
        csv::parse_document(b"A,B\n\"unterminated\n", Limits::default()).is_err(),
        "unterminated quote fails closed"
    );
    assert!(
        csv::parse_document(b"A,B\n\"closed\"x,y\n", Limits::default()).is_err(),
        "characters after closing quote fail closed"
    );
    assert!(
        csv::parse_document(b"A,B\n1\n", Limits::default()).is_err(),
        "field count mismatch fails closed"
    );
}

#[test]
fn csv_limits_are_enforced() {
    let limits = Limits {
        max_file_bytes: 7,
        ..Limits::default()
    };
    assert_eq!(
        csv::parse_document(b"A,B\n1,2\n", limits),
        Err(csv::CsvError::FileTooLarge { limit: 7 }),
        "file byte limit is enforced"
    );
    let limits = Limits {
        max_rows: 1,
        ..Limits::default()
    };
    assert!(
        csv::parse_document(b"A\n1\n2\n", limits).is_err(),
        "row limit fails closed"
    );
}

#[test]
fn identity_is_opaque_stable_and_collision_safe() {
    let mut registry = IdentityRegistry::new("in.ref").unwrap();
    let isin_a = Isin::new(ISIN_A).unwrap();
    let isin_b = Isin::new(ISIN_B).unwrap();
    let first = registry.resolve(&isin_a, Venue::Nse, "AAA").unwrap();
    let again = registry.resolve(&isin_a, Venue::Bse, "AAA").unwrap();
    assert_eq!(first, again, "aliases resolve to one stable identity");
    assert!(
        !first.as_str().contains(&isin_a.as_str().to_lowercase()),
        "identity is not derived from ISIN"
    );
    assert!(
        registry.resolve(&isin_b, Venue::Nse, "AAA").is_err(),
        "symbol reuse by another security is rejected"
    );
    assert_eq!(registry.len(), 1, "no extra identity was allocated");
}
