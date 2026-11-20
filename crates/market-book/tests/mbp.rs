#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.1-E MBP book tests. Synthetic levels; no provider data or live claim.

use market_book::{
    BookPhase, //
    MbpBook,
    MbpBookError,
    SnapshotEvidence,
};
use market_types::{
    Side,
    model::{BookSnapshot, Level, Price, Quantity, QuantityChange, QuantityDelta},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn price(value: i64) -> Price {
    Price::from_raw(value)
}

fn level(value: i64, quantity: i64) -> Level {
    Level::new(value, quantity).expect("level")
}

fn snapshot(levels: usize) -> BookSnapshot {
    BookSnapshot {
        bids: (0..levels)
            .map(|index| level(10_000 - index as i64, 100))
            .collect(),
        asks: (0..levels)
            .map(|index| level(10_001 + index as i64, 100))
            .collect(),
    }
}

fn evidence(source: &str, id: &str) -> SnapshotEvidence {
    SnapshotEvidence {
        source_id: source.to_owned(),
        snapshot_id: id.to_owned(),
        source_digest: "ab".repeat(32),
        received_utc_ns: 1_700_000_000_000_000_000,
    }
}

fn seeded(depth: u32) -> Result<MbpBook, Box<dyn std::error::Error>> {
    let mut book = MbpBook::new("upstox", depth)?;
    let evidence = evidence("upstox", "snap-1");
    book.begin_snapshot(&evidence)?;
    book.apply_snapshot(&snapshot(5), &evidence)?;
    book.mark_live()?;
    Ok(book)
}

#[test]
fn evidenced_snapshot_then_live_updates() -> TestResult {
    let mut book = seeded(5)?;
    assert_eq!(
        book.phase(),
        BookPhase::LiveUpdates,
        "book is live after baseline"
    );
    assert_eq!(
        book.baseline().map(|value| value.snapshot_id.as_str()),
        Some("snap-1"),
        "baseline is recorded"
    );
    book.apply_delta(
        Side::Buy,
        price(10_000),
        QuantityChange::Delta(QuantityDelta::from_raw(5)),
    )?;
    assert_eq!(
        book.best_bid().map(|level| level.quantity.raw()),
        Some(105),
        "delta changes the level"
    );
    Ok(())
}

#[test]
fn delta_before_snapshot_is_refused() -> TestResult {
    let mut book = MbpBook::new("fyers.data_socket", 5)?;
    let result = book.apply_delta(
        Side::Buy,
        price(10_000),
        QuantityChange::Absolute(Quantity::new(10)?),
    );
    assert!(
        matches!(result, Err(MbpBookError::InvalidPhase { .. })),
        "no book may be updated before an evidenced snapshot"
    );
    Ok(())
}

#[test]
fn declared_depth_is_enforced() -> TestResult {
    let mut book = MbpBook::new("upstox", 3)?;
    let evidence = evidence("upstox", "snap-deep");
    book.begin_snapshot(&evidence)?;
    let result = book.apply_snapshot(&snapshot(5), &evidence);
    assert!(
        matches!(
            result,
            Err(MbpBookError::DepthExceeded {
                actual: 5,
                declared: 3
            })
        ),
        "a five-level feed cannot fill a three-level book"
    );
    Ok(())
}

#[test]
fn gap_blocks_updates_until_a_new_evidenced_snapshot() -> TestResult {
    let mut book = seeded(5)?;
    book.mark_gap(42)?;
    assert_eq!(book.gap_sequence(), Some(42), "gap sequence is recorded");
    assert!(
        book.apply_delta(
            Side::Buy,
            price(10_000),
            QuantityChange::Absolute(Quantity::new(1)?)
        )
        .is_err(),
        "deltas stop at a gap"
    );
    assert!(
        book.begin_snapshot(&evidence("upstox", "snap-recovery"))
            .is_err(),
        "recovery must be declared before a new baseline"
    );
    book.begin_recovery()?;
    let recovery = evidence("upstox", "snap-recovery");
    book.begin_snapshot(&recovery)?;
    book.apply_snapshot(&snapshot(5), &recovery)?;
    book.mark_live()?;
    assert_eq!(
        book.phase(),
        BookPhase::LiveUpdates,
        "new baseline restores live updates"
    );
    Ok(())
}

#[test]
fn snapshot_evidence_must_match_source_and_be_well_formed() -> TestResult {
    let mut book = MbpBook::new("upstox", 5)?;
    assert!(
        book.begin_snapshot(&evidence("fyers.data_socket", "snap-x"))
            .is_err(),
        "wrong source is refused"
    );
    let mut bad = evidence("upstox", "snap-x");
    bad.source_digest = "not-a-digest".to_owned();
    assert!(
        book.begin_snapshot(&bad).is_err(),
        "malformed digest is refused"
    );
    Ok(())
}

#[test]
fn crossed_snapshot_is_refused() -> TestResult {
    let mut book = MbpBook::new("upstox", 5)?;
    let evidence = evidence("upstox", "snap-crossed");
    book.begin_snapshot(&evidence)?;
    let crossed = BookSnapshot {
        bids: vec![level(10_100, 10)],
        asks: vec![level(10_000, 10)],
    };
    assert!(
        matches!(
            book.apply_snapshot(&crossed, &evidence),
            Err(MbpBookError::CrossedSnapshot)
        ),
        "a crossed snapshot is refused"
    );
    Ok(())
}

#[test]
fn books_are_per_source_and_never_merged() -> TestResult {
    let mut upstox = seeded(5)?;
    let mut fyers = MbpBook::new("fyers.data_socket", 5)?;
    assert_eq!(
        fyers.phase(),
        BookPhase::Uninitialized,
        "another source starts uninitialized"
    );
    upstox.mark_gap(7)?;
    assert_eq!(
        fyers.phase(),
        BookPhase::Uninitialized,
        "one source's gap does not affect another"
    );
    assert!(
        fyers
            .begin_snapshot(&evidence("upstox", "snap-wrong"))
            .is_err(),
        "evidence cannot cross sources"
    );
    Ok(())
}

#[test]
fn zero_depth_is_refused() -> TestResult {
    assert!(
        matches!(MbpBook::new("upstox", 0), Err(MbpBookError::InvalidDepth)),
        "zero depth is refused"
    );
    Ok(())
}
