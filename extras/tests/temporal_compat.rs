//! Temporal wire-compatibility goldens for the chrono -> jiff migration.
//!
//! Every literal below was captured from chrono 0.4.45 at baseline. Jiff must
//! reproduce it byte for byte, otherwise a persisted format or wire contract
//! has silently changed. The durable-store integration tests in
//! `tests/reference_store.rs` exercise the same round-trips through the real
//! SQLite payload path.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use jiff::{Span, Timestamp, civil::Date};
use nirnay_extras::venue::india::backup::backup_id_timestamp;

fn ts(nanos: i32) -> Timestamp {
    Timestamp::new(1_763_366_400, nanos).unwrap()
}

#[test]
fn backup_filename_matches_chrono_golden() {
    // chrono: Utc::now() at 2025-11-17T08:00:00.123456789Z, formatted with
    // "%Y%m%dT%H%M%S%9fZ" -> nine fractional digits and NO decimal point.
    assert_eq!(
        backup_id_timestamp(ts(123_456_789)),
        "20251117T080000123456789Z"
    );
    // Zero and full-scale nanoseconds keep the fixed nine-digit width.
    assert_eq!(backup_id_timestamp(ts(0)), "20251117T080000000000000Z");
    assert_eq!(
        backup_id_timestamp(ts(999_999_999)),
        "20251117T080000999999999Z"
    );
}

#[test]
fn date_display_and_serde_match_chrono_golden() {
    for (parts, golden) in [
        ((2026, 11, 17), "2026-11-17"),
        ((2024, 2, 29), "2024-02-29"),
        ((2026, 1, 31), "2026-01-31"),
        ((2000, 2, 29), "2000-02-29"),
        ((1999, 12, 31), "1999-12-31"),
    ] {
        let date = Date::new(parts.0, parts.1, parts.2).unwrap();
        assert_eq!(date.to_string(), golden);
        assert_eq!(
            serde_json::to_string(&date).unwrap(),
            format!("\"{golden}\"")
        );
    }
    // Both source date formats used by the venue parsers.
    assert_eq!(
        Date::strptime("%Y-%m-%d", "2025-05-30")
            .unwrap()
            .to_string(),
        "2025-05-30"
    );
    assert_eq!(
        Date::strptime("%d-%b-%Y", "17-Nov-2026")
            .unwrap()
            .to_string(),
        "2026-11-17"
    );
}

#[test]
fn rfc3339_autosi_and_micros_match_chrono_golden() {
    // AutoSi (Timestamp Display / serde) and the fixed Micros strftime form
    // for a microsecond-aligned value are both "…123456Z".
    let micros = ts(123_456_000);
    assert_eq!(micros.to_string(), "2025-11-17T08:00:00.123456Z");
    assert_eq!(
        micros.strftime("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),
        "2025-11-17T08:00:00.123456Z"
    );
    assert_eq!(
        serde_json::to_string(&micros).unwrap(),
        "\"2025-11-17T08:00:00.123456Z\""
    );

    // AutoSi emits only the precision needed; fixed Micros always emits six.
    assert_eq!(ts(0).to_string(), "2025-11-17T08:00:00Z");
    assert_eq!(
        ts(0).strftime("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),
        "2025-11-17T08:00:00.000000Z"
    );
    // Millis and nanos fixed forms, for completeness.
    assert_eq!(
        micros.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        "2025-11-17T08:00:00.123Z"
    );
    assert_eq!(
        ts(123_456_789)
            .strftime("%Y-%m-%dT%H:%M:%S%.9fZ")
            .to_string(),
        "2025-11-17T08:00:00.123456789Z"
    );
}

#[test]
fn persisted_metadata_decode_encode_round_trip() {
    // The exact bytes the durable store writes into `recorded_at`.
    let recorded = ts(123_456_789);
    let metadata = recorded.to_string();
    assert_eq!(metadata, "2025-11-17T08:00:00.123456789Z");
    assert_eq!(metadata.parse::<Timestamp>().unwrap(), recorded);

    // A full persisted metadata envelope decodes to the same strings and
    // re-encodes byte-for-byte, matching chrono's serde output.
    let baseline = r#"{"trade_date":"2026-11-17","recorded_at":"2025-11-17T08:00:00.123456789Z"}"#;
    let decoded: serde_json::Value = serde_json::from_str(baseline).unwrap();
    let date: Date = serde_json::from_value(decoded["trade_date"].clone()).unwrap();
    let timestamp: Timestamp = serde_json::from_value(decoded["recorded_at"].clone()).unwrap();
    assert_eq!(date.to_string(), "2026-11-17");
    assert_eq!(timestamp.to_string(), "2025-11-17T08:00:00.123456789Z");
    let reencoded = format!(
        "{{\"trade_date\":{},\"recorded_at\":{}}}",
        serde_json::to_string(&date).unwrap(),
        serde_json::to_string(&timestamp).unwrap()
    );
    assert_eq!(reencoded, baseline);
}

#[test]
fn month_end_and_leap_year_arithmetic() {
    // jiff clamps day-of-month, matching chrono's checked_add_months.
    let month_end = Date::new(2026, 1, 31)
        .unwrap()
        .checked_add(Span::new().months(1))
        .unwrap();
    assert_eq!(month_end.to_string(), "2026-02-28");

    let leap = Date::new(2024, 2, 29).unwrap();
    assert_eq!(
        leap.checked_add(Span::new().years(1)).unwrap().to_string(),
        "2025-02-28"
    );
    assert_eq!(Date::new(2000, 2, 29).unwrap().to_string(), "2000-02-29");
    assert!(Date::new(1900, 2, 29).is_err());
    assert!(Date::new(2100, 2, 29).is_err());

    // The `previous_day` / `succ_opt` replacements used by the venue modules.
    assert_eq!(
        Date::new(2026, 1, 1)
            .unwrap()
            .checked_add(Span::new().days(-1))
            .unwrap()
            .to_string(),
        "2025-12-31"
    );
    assert_eq!(
        Date::new(2026, 12, 31)
            .unwrap()
            .checked_add(Span::new().days(1))
            .unwrap()
            .to_string(),
        "2027-01-01"
    );
}
