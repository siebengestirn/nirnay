#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.1-G cross-source reconciliation tests. No liquidity is merged.

use market_book::{
    MbpBook, //
    SnapshotEvidence,
    compare_books,
};
use market_types::model::{
    BookSnapshot, //
    Level,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn book(source: &str, bid: i64, ask: i64) -> Result<MbpBook, Box<dyn std::error::Error>> {
    let snapshot = BookSnapshot {
        bids: vec![Level::new(bid, 100)?],
        asks: vec![Level::new(ask, 100)?],
    };
    let evidence = SnapshotEvidence {
        source_id: source.to_owned(),
        snapshot_id: format!("{source}-snapshot"),
        source_digest: "ab".repeat(32),
        received_utc_ns: 1_700_000_000_000_000_000,
    };
    let mut result = MbpBook::new(source, 5)?;
    result.begin_snapshot(&evidence)?;
    result.apply_snapshot(&snapshot, &evidence)?;
    result.mark_live()?;
    Ok(result)
}

#[test]
fn matching_books_report_no_differences() -> TestResult {
    let left = book("upstox", 9_995, 10_005)?;
    let right = book("fyers.data_socket", 9_995, 10_005)?;
    let comparison = compare_books(&left, &right);
    assert!(comparison.matched, "matching books are reported as matched");
    assert!(comparison.differences.is_empty(), "no false differences");
    Ok(())
}

#[test]
fn differing_books_report_explicit_differences() -> TestResult {
    let left = book("upstox", 9_995, 10_005)?;
    let right = book("fyers.data_socket", 9_994, 10_006)?;
    let comparison = compare_books(&left, &right);
    assert!(!comparison.matched, "a mismatch is an explicit incident");
    assert!(
        comparison
            .differences
            .iter()
            .any(|item| item.contains("best bid")),
        "bid mismatch reported"
    );
    assert!(
        comparison
            .differences
            .iter()
            .any(|item| item.contains("best ask")),
        "ask mismatch reported"
    );
    Ok(())
}

#[test]
fn same_source_books_are_flagged_not_merged() -> TestResult {
    let left = book("upstox", 9_995, 10_005)?;
    let right = book("upstox", 9_995, 10_005)?;
    let comparison = compare_books(&left, &right);
    assert!(
        comparison
            .differences
            .iter()
            .any(|item| item.contains("same source")),
        "two books from one source are flagged"
    );
    Ok(())
}
