//! Durable reference-store verification suite.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]
//!
//! Maps onto the 12 frozen verification groups in
//! `engineering/reference-store-contract.json` (RSV-01 … RSV-12).
//!
//! Child processes are used for kill/fault injection: each child re-runs a
//! single `#[test]` worker that branches on environment and performs the
//! fault before the parent re-opens the store and checks durable state.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{DateTime, NaiveDate, Utc};
use india_domain::{
    InstrumentType, Isin, ListingStatus, Segment, SettlementRegime, TickAssignment, Venue,
};
use nirnay_extras::venue::india::archive::SourceArchive;
use nirnay_extras::venue::india::backup::{backup as backup_store, restore as restore_backup};
use nirnay_extras::venue::india::provenance::provenance_for_source;
use nirnay_extras::venue::india::resolution::{ReconciledUniverse, reconcile};
use nirnay_extras::venue::india::store::ReferenceSnapshot;
use nirnay_extras::venue::india::{DurableReferenceStore, NormalizedListing};

const SOURCE_BYTES: &[u8] =
    b"SYMBOL,NAME OF COMPANY,SERIES,MARKET LOT,ISIN NUMBER\nRELIANCE,Reliance Industries,EQ,1,INE002A01018\nHDFCBANK,HDFC Bank,EQ,1,INE040A01026\n";

/// Distinct bytes for the crash-injection children: their provenance source
/// must take the *new-object* write path in the archive (the seed already
/// archived `SOURCE_BYTES`, which would short-circuit into the dedup path).
const CHILD_SOURCE_BYTES: &[u8] =
    b"SYMBOL,NAME OF COMPANY,SERIES,MARKET LOT,ISIN NUMBER\nTCS,Tata Consultancy Services,EQ,1,INE467B01029\n";

/// The child's source digest, computed the same way the archive does.
fn child_source_digest() -> String {
    nirnay_extras::venue::india::durable::payload_digest_for_test(CHILD_SOURCE_BYTES)
}

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn at(day: NaiveDate) -> DateTime<Utc> {
    day.and_hms_opt(10, 0, 0).unwrap().and_utc()
}

fn flat_assignment(revision: &str) -> TickAssignment {
    TickAssignment {
        tick_paise: 5,
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

/// Build a syntactically valid ISIN for a synthetic index, computing the
/// ISO 6166 check digit like `india_domain::Isin`.
fn synthetic_isin(index: usize) -> String {
    let base = format!("INE{index:06}A{}", index % 10);
    let mut digits: Vec<u32> = Vec::with_capacity(24);
    for byte in base.bytes().take(11) {
        if byte.is_ascii_digit() {
            digits.push(u32::from(byte - b'0'));
        } else {
            let letter = u32::from(byte.to_ascii_uppercase() - b'A') + 10;
            digits.push(letter / 10);
            digits.push(letter % 10);
        }
    }
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
    let check = (10 - (sum % 10)) % 10;
    format!("{base}{check}")
}

/// A normalized NSE equity row with an explicit settlement regime; used by
/// the end-to-end ingestion test to map parsed SERIES values.
fn normalized_equity(
    symbol: &str,
    isin: &str,
    as_of: NaiveDate,
    status: ListingStatus,
    regime: SettlementRegime,
) -> NormalizedListing {
    let mut listing = row(symbol, isin, as_of, status);
    listing.settlement_regime = regime;
    listing
}

fn row(symbol: &str, isin: &str, as_of: NaiveDate, status: ListingStatus) -> NormalizedListing {
    NormalizedListing {
        venue: Venue::Nse,
        segment: Segment::Cash,
        instrument_type: InstrumentType::Equity,
        venue_symbol: symbol.to_owned(),
        token: None,
        isin: Isin::new(isin).unwrap(),
        name: symbol.to_owned(),
        status,
        settlement_regime: SettlementRegime::Standard,
        z_category: false,
        settlement_group: None,
        tick_assignment: flat_assignment("test-fixture"),
        market_lot: 1,
        as_of,
    }
}

fn snapshot(
    root: &Path,
    valid_at: NaiveDate,
    recorded_at: DateTime<Utc>,
    rows: Vec<NormalizedListing>,
    base: Option<&ReferenceSnapshot>,
    extra_assignments: Option<TickAssignment>,
) -> ReferenceSnapshot {
    snapshot_from_source(
        root,
        valid_at,
        recorded_at,
        rows,
        base,
        extra_assignments,
        SOURCE_BYTES,
    )
}

fn snapshot_from_source(
    root: &Path,
    valid_at: NaiveDate,
    recorded_at: DateTime<Utc>,
    rows: Vec<NormalizedListing>,
    base: Option<&ReferenceSnapshot>,
    extra_assignments: Option<TickAssignment>,
    source_bytes: &[u8],
) -> ReferenceSnapshot {
    snapshot_with_provenance(
        root,
        valid_at,
        recorded_at,
        rows,
        base,
        extra_assignments,
        source_bytes,
        "nse-cm-master",
        Venue::Nse,
        "nse-cm-master-v1",
    )
}

/// Build a snapshot whose provenance names an explicit source identity,
/// venue and format version — used when the archived bytes are not the
/// default NSE fixture.
#[allow(clippy::too_many_arguments)]
fn snapshot_with_provenance(
    root: &Path,
    valid_at: NaiveDate,
    recorded_at: DateTime<Utc>,
    rows: Vec<NormalizedListing>,
    base: Option<&ReferenceSnapshot>,
    extra_assignments: Option<TickAssignment>,
    source_bytes: &[u8],
    source_id: &str,
    venue: Venue,
    source_format_version: &str,
) -> ReferenceSnapshot {
    let archive = SourceArchive::open(&root.join("source-archive")).unwrap();
    let retrieved_at = at(recorded_at.date_naive());
    archive
        .store(source_id, source_bytes, retrieved_at)
        .unwrap();
    let provenance = provenance_for_source(
        source_id,
        venue,
        Segment::Cash,
        retrieved_at,
        source_format_version,
        "test",
        source_bytes,
    );
    let mut effective = rows;
    if let Some(assignment) = extra_assignments {
        for listing in &mut effective {
            listing.tick_assignment = assignment.clone();
        }
    }
    let base_instruments = base.map(|snap| snap.instruments.as_slice()).unwrap_or(&[]);
    let reconciled = reconcile(
        base_instruments,
        effective,
        valid_at,
        false,
        recorded_at,
        &provenance,
    )
    .unwrap();
    let previous = base
        .map(|snap| snap.listing_states.as_slice())
        .unwrap_or(&[]);
    ReferenceSnapshot::from_reconciled(
        ReconciledUniverse {
            instruments: reconciled.instruments,
            reconciliation: reconciled.reconciliation,
            listing_states: reconciled.listing_states,
        },
        valid_at,
        recorded_at,
        provenance,
        previous,
    )
    .unwrap()
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        // Parallel test threads share the pid; SystemTime resolution is
        // microseconds, so stamp a process-global counter to stay unique.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "p1-04-{}-{}-{}",
            std::process::id(),
            unique,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn data_dir(&self) -> PathBuf {
        self.0.join("data")
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn spawn_child(test_name: &str, envs: &[(&str, String)]) -> std::process::Output {
    let exe = std::env::current_exe().unwrap();
    let mut command = Command::new(exe);
    command
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .arg("--ignored");
    for (key, value) in envs {
        command.env(key, value);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command.output().unwrap()
}

// ---------------------------------------------------------------------------
// RSV-08 crash-injection worker. Parent tests invoke this ignored test in a child process.
// ---------------------------------------------------------------------------
#[cfg(debug_assertions)]
#[test]
#[ignore = "child-process worker; invoked by RSV-08 parent tests"]
fn rsv_08_crash_worker() {
    let dir = PathBuf::from(
        std::env::var("P1_04_CHILD_DIR").expect("P1_04_CHILD_DIR must be set by the parent test"),
    );
    let action = std::env::var("P1_04_CHILD_ACTION").unwrap();
    let crash_point = std::env::var("P1_04_CHILD_CRASH_POINT").ok();
    if let Some(point) = crash_point {
        nirnay_extras::venue::india::durable::debug_set_crash_point(Some(point.parse().unwrap()));
    }
    let day = date(2026, 1, 10);
    let feb = date(2026, 2, 10);
    let snap1 = snapshot(
        &dir,
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    match action.as_str() {
        "seed_revision_zero" => {
            let mut store = DurableReferenceStore::open(&dir).unwrap();
            let outcome = store.publish(&snap1, None).unwrap();
            println!("seeded revision_id={}", outcome.revision_id);
        }
        "publish_revision_one_without_crash" => {
            let head = {
                let mut store = DurableReferenceStore::open(&dir).unwrap();
                store.revision_with_meta(0).unwrap().unwrap().snapshot
            };
            let snap2 = snapshot(
                &dir,
                feb,
                at(feb),
                vec![row("HDFCBANK", "INE040A01026", feb, ListingStatus::Active)],
                Some(&head),
                None,
            );
            let mut store = DurableReferenceStore::open(&dir).unwrap();
            match store.publish(&snap2, Some(0)) {
                Ok(outcome) => println!(
                    "outcome revision_id={} replayed={}",
                    outcome.revision_id, outcome.replayed
                ),
                Err(err) => println!("error={err:#}"),
            }
        }
        "publish_revision_one" => {
            let head = {
                let mut store = DurableReferenceStore::open(&dir).unwrap();
                store.revision_with_meta(0).unwrap().unwrap().snapshot
            };
            // The child's provenance uses NEW source bytes so the archive
            // takes the object-write path and the archive-stage crash points
            // actually fire.
            let snap2 = snapshot_from_source(
                &dir,
                feb,
                at(feb),
                vec![row("HDFCBANK", "INE040A01026", feb, ListingStatus::Active)],
                Some(&head),
                None,
                CHILD_SOURCE_BYTES,
            );
            let mut store = DurableReferenceStore::open(&dir).unwrap();
            match store.publish(&snap2, Some(0)) {
                Ok(outcome) => println!(
                    "published revision_id={} replayed={}",
                    outcome.revision_id, outcome.replayed
                ),
                Err(err) => println!("error={err:#}"),
            }
        }
        "publish_revision_one_same_snapshot" => {
            // Idempotent retry path: the child publishes the *same* snapshot
            // (same source bytes, hence same publication key) the crashing
            // child would have published.
            let head = {
                let mut store = DurableReferenceStore::open(&dir).unwrap();
                store.revision_with_meta(0).unwrap().unwrap().snapshot
            };
            let snap2 = snapshot_from_source(
                &dir,
                feb,
                at(feb),
                vec![row("HDFCBANK", "INE040A01026", feb, ListingStatus::Active)],
                Some(&head),
                None,
                CHILD_SOURCE_BYTES,
            );
            let mut store = DurableReferenceStore::open(&dir).unwrap();
            match store.publish(&snap2, Some(0)) {
                Ok(outcome) => println!(
                    "outcome revision_id={} replayed={}",
                    outcome.revision_id, outcome.replayed
                ),
                Err(err) => println!("error={err:#}"),
            }
        }
        other => panic!("unknown child action {other}"),
    }
}

fn seed_revision_zero(root: &TestRoot) {
    let day = date(2026, 1, 10);
    let snap1 = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    store.publish(&snap1, None).unwrap();
}

#[cfg(debug_assertions)]
#[test]
fn rsv_08_crash_before_archive_leaves_no_revision() {
    // The archive-stage crash fires inside SourceArchive::store, before the
    // object is even written: no object, no manifest entry, no revision.
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let output = spawn_child(
        "rsv_08_crash_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_ACTION", "publish_revision_one".to_string()),
            (
                "P1_04_CHILD_CRASH_POINT",
                "before_archive_object_write".to_string(),
            ),
        ],
    );
    assert!(!output.status.success(), "crash child must die");
    // Archive state: the store's snapshot sources are unharmed (they were
    // archived by the seed), and the failed publication added nothing new.
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    let verification = archive.verify_full().unwrap();
    let unindexed_before = verification.unindexed_objects.len();
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.head().unwrap(), Some(0));
    assert_eq!(store.revision_count().unwrap(), 1);
    // The child's crash happened before the second snapshot's source object
    // write, so no new object appeared and the manifest is intact.
    assert_eq!(verification.entries.len(), 1);
    let _ = unindexed_before;
}

#[cfg(debug_assertions)]
#[test]
fn rsv_08_crash_after_object_rename_leaves_unindexed_object() {
    // Crash after the object rename + directory sync, before the manifest
    // append: the object is crash residue, the manifest entry never
    // existed, and no revision is published.
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let output = spawn_child(
        "rsv_08_crash_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_ACTION", "publish_revision_one".to_string()),
            (
                "P1_04_CHILD_CRASH_POINT",
                "after_archive_object_rename".to_string(),
            ),
        ],
    );
    assert!(!output.status.success(), "crash child must die");
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    let verification = archive.verify_full().unwrap();
    // The new object exists on disk but is unindexed.
    assert!(
        verification
            .unindexed_objects
            .contains(&child_source_digest())
    );
    assert_eq!(verification.entries.len(), 1);
    // An unindexed object is never committed source data.
    assert!(archive.fetch(&child_source_digest()).is_err());
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.head().unwrap(), Some(0));
    assert_eq!(store.revision_count().unwrap(), 1);
}

#[cfg(debug_assertions)]
#[test]
fn rsv_08_crash_after_manifest_append_keeps_archive_but_no_revision() {
    // Crash after the manifest append + sync: the archive is fully
    // committed (object + entry), the revision is not.
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let output = spawn_child(
        "rsv_08_crash_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_ACTION", "publish_revision_one".to_string()),
            (
                "P1_04_CHILD_CRASH_POINT",
                "after_archive_manifest_append".to_string(),
            ),
        ],
    );
    assert!(!output.status.success(), "crash child must die");
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    let verification = archive.verify_full().unwrap();
    assert_eq!(verification.entries.len(), 2);
    assert!(verification.unindexed_objects.is_empty());
    // The archived bytes are fetchable and hash-correct.
    assert!(archive.fetch(&child_source_digest()).is_ok());
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.head().unwrap(), Some(0));
    assert_eq!(store.revision_count().unwrap(), 1);
    // The publication may be retried normally: the same snapshot publishes
    // as revision 1.
    drop(store);
    let retry = spawn_child(
        "rsv_08_crash_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            (
                "P1_04_CHILD_ACTION",
                "publish_revision_one_same_snapshot".to_string(),
            ),
        ],
    );
    assert!(retry.status.success());
    let stdout = String::from_utf8_lossy(&retry.stdout);
    assert!(stdout.contains("revision_id=1"), "{stdout}");
    assert!(stdout.contains("replayed=false"), "{stdout}");
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.revision_count().unwrap(), 2);
}

#[cfg(debug_assertions)]
#[test]
fn rsv_08_crash_in_transaction_rolls_back() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let output = spawn_child(
        "rsv_08_crash_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_ACTION", "publish_revision_one".to_string()),
            ("P1_04_CHILD_CRASH_POINT", "in_transaction".to_string()),
        ],
    );
    assert!(!output.status.success());
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.head().unwrap(), Some(0));
    assert_eq!(store.revision_count().unwrap(), 1);
}

#[cfg(debug_assertions)]
#[test]
fn rsv_08_crash_after_insert_rolls_back() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let output = spawn_child(
        "rsv_08_crash_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_ACTION", "publish_revision_one".to_string()),
            (
                "P1_04_CHILD_CRASH_POINT",
                "after_insert_before_commit".to_string(),
            ),
        ],
    );
    assert!(!output.status.success());
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.head().unwrap(), Some(0));
    assert_eq!(store.revision_count().unwrap(), 1);
}

#[cfg(debug_assertions)]
#[test]
fn rsv_08_crash_after_commit_keeps_revision_and_retry_replays() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let output = spawn_child(
        "rsv_08_crash_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_ACTION", "publish_revision_one".to_string()),
            (
                "P1_04_CHILD_CRASH_POINT",
                "after_commit_before_return".to_string(),
            ),
        ],
    );
    assert!(!output.status.success());
    // The commit completed: both revision and head are visible.
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.head().unwrap(), Some(1));
    assert_eq!(store.revision_count().unwrap(), 2);
    drop(store);
    // Idempotent retry returns the same revision instead of duplicating.
    let retry = spawn_child(
        "rsv_08_crash_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            (
                "P1_04_CHILD_ACTION",
                "publish_revision_one_same_snapshot".to_string(),
            ),
        ],
    );
    let stdout = String::from_utf8_lossy(&retry.stdout);
    assert!(
        stdout.contains("replayed=true"),
        "expected key resolution, got: {stdout}"
    );
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.revision_count().unwrap(), 2);
}

// ---------------------------------------------------------------------------
// RSV-07 concurrency worker.
// ---------------------------------------------------------------------------
#[test]
#[ignore = "child-process worker; invoked by RSV-07 parent test"]
fn rsv_07_concurrency_worker() {
    let dir = PathBuf::from(
        std::env::var("P1_04_CHILD_DIR").expect("P1_04_CHILD_DIR must be set by the parent test"),
    );
    let name = std::env::var("P1_04_CHILD_NAME").unwrap_or_else(|_| "a".to_string());
    let feb = date(2026, 2, 10);
    // Two children build the *same* snapshot deterministically: one commits
    // it; the other resolves by publication key.
    let head = {
        let mut store = DurableReferenceStore::open(&dir).unwrap();
        store.revision_with_meta(0).unwrap().unwrap().snapshot
    };
    let snap2 = snapshot(
        &dir,
        feb,
        at(feb),
        vec![row("HDFCBANK", "INE040A01026", feb, ListingStatus::Active)],
        Some(&head),
        None,
    );
    let mut store = DurableReferenceStore::open(&dir).unwrap();
    match store.publish(&snap2, Some(0)) {
        Ok(outcome) => println!(
            "winner={name} revision_id={} replayed={}",
            outcome.revision_id, outcome.replayed
        ),
        Err(err) => println!("error={err:#}"),
    }
}

#[test]
fn rsv_07_competing_processes_publish_once() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let dir = root.data_dir().display().to_string();
    let left = spawn_child(
        "rsv_07_concurrency_worker",
        &[
            ("P1_04_CHILD_DIR", dir.clone()),
            ("P1_04_CHILD_NAME", "left".to_string()),
        ],
    );
    let right = spawn_child(
        "rsv_07_concurrency_worker",
        &[
            ("P1_04_CHILD_DIR", dir),
            ("P1_04_CHILD_NAME", "right".to_string()),
        ],
    );
    let left_out = String::from_utf8_lossy(&left.stdout);
    let right_out = String::from_utf8_lossy(&right.stdout);
    assert!(
        left.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&left.stderr)
    );
    assert!(
        right.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&right.stderr)
    );
    assert!(left_out.contains("revision_id=1"), "{left_out}");
    assert!(right_out.contains("revision_id=1"), "{right_out}");
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.revision_count().unwrap(), 2);
    assert_eq!(store.head().unwrap(), Some(1));
}

// ---------------------------------------------------------------------------
// RSV-01 serialisation
// ---------------------------------------------------------------------------
#[test]
fn rsv_01_persist_reopen_compare_full_snapshot() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![
            row("RELIANCE", "INE002A01018", day, ListingStatus::Active),
            row("HDFCBANK", "INE040A01026", day, ListingStatus::Active),
        ],
        None,
        None,
    );
    let before = snap.clone();
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        store.publish(&before, None).unwrap();
    }
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let head = store.head_snapshot().unwrap().unwrap();
        assert_eq!(&head, &before, "complete snapshot must survive reopen");
        assert!(head.reconciliation.is_some());
        assert_eq!(head.listing_states.len(), 2);
        assert_eq!(head.provenance.source_id, "nse-cm-master");
    }
}

// ---------------------------------------------------------------------------
// RSV-02 revision identity
// ---------------------------------------------------------------------------
#[test]
fn rsv_02_ids_contiguous_and_parents_survive_restart() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let feb = date(2026, 2, 10);
    let mar = date(2026, 3, 10);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        for (index, (d, previous)) in [(day, None), (feb, Some(0u64)), (mar, Some(1u64))]
            .into_iter()
            .enumerate()
        {
            let rows = vec![row("RELIANCE", "INE002A01018", d, ListingStatus::Active)];
            let base = previous.map(|_| {
                store
                    .revision_with_meta(previous.unwrap())
                    .unwrap()
                    .unwrap()
                    .snapshot
            });
            let snap = snapshot(&root.data_dir(), d, at(d), rows, base.as_ref(), None);
            let outcome = store.publish(&snap, previous).unwrap();
            assert_eq!(outcome.revision_id, index as u64);
        }
    }
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        assert_eq!(store.head().unwrap(), Some(2));
        for (id, parent) in [(0u64, None), (1u64, Some(0u64)), (2u64, Some(1u64))] {
            let revision = store.revision_with_meta(id).unwrap().unwrap();
            assert_eq!(revision.revision_id, id);
            assert_eq!(revision.parent_revision_id, parent);
        }
    }
}

// ---------------------------------------------------------------------------
// RSV-03 hashing
// ---------------------------------------------------------------------------
#[test]
fn rsv_03_hashes_stable_across_restart() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let before_hash: String;
    let before_checksum: String;
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        before_hash = snap.snapshot_hash().unwrap();
        store.publish(&snap, None).unwrap();
        before_checksum = store.revision_with_meta(0).unwrap().unwrap().payload_sha256;
    }
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let revision = store.revision_with_meta(0).unwrap().unwrap();
        assert_eq!(revision.snapshot_hash, before_hash);
        assert_eq!(revision.payload_sha256, before_checksum);
    }
}

#[test]
fn rsv_03_changed_payload_bytes_fail_checksum() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let before: Vec<u8>;
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap, None).unwrap();
        before = store
            .revision_with_meta(0)
            .unwrap()
            .unwrap()
            .payload_sha256
            .into_bytes();
        let _ = before;
    }
    // Directly tamper with the database file, simulating a storage fault:
    // drop the immutability trigger, corrupt a payload byte, close.
    let db = root.data_dir().join("reference.sqlite3");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch("DROP TRIGGER reference_revision_no_update;")
            .unwrap();
        let bytes: Vec<u8> = connection
            .query_row(
                "SELECT payload FROM reference_revision WHERE revision_id = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut corrupted = bytes;
        let middle = corrupted.len() / 2;
        corrupted[middle] ^= 0x01;
        connection
            .execute(
                "UPDATE reference_revision SET payload = ?1 WHERE revision_id = 0",
                rusqlite::params![corrupted],
            )
            .unwrap();
    }
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert!(
        store.revision_with_meta(0).is_err(),
        "tampered payload must fail its integrity checksum"
    );
}

#[test]
fn rsv_03_semantic_field_change_changes_hash() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let later = date(2026, 1, 11);
    let a = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let b = snapshot(
        &root.data_dir(),
        later,
        at(later),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    assert_ne!(a.snapshot_hash().unwrap(), b.snapshot_hash().unwrap());
}

// ---------------------------------------------------------------------------
// RSV-04 bitemporal queries identical across restart
// ---------------------------------------------------------------------------
#[test]
fn rsv_04_bitemporal_identical_across_restart() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let feb = date(2026, 2, 10);
    let historic_before: Vec<_>;
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap1 = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap1, None).unwrap();
        let snap2 = snapshot(
            &root.data_dir(),
            feb,
            at(feb),
            vec![row(
                "RELIANCE",
                "INE002A01018",
                feb,
                ListingStatus::Suspended,
            )],
            Some(&snap1),
            None,
        );
        store.publish(&snap2, Some(0)).unwrap();
        historic_before = store.reference(day, at(date(2026, 1, 15))).unwrap();
    }
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let historic_after = store.reference(day, at(date(2026, 1, 15))).unwrap();
        assert_eq!(
            historic_before, historic_after,
            "Reference(t_valid, t_recorded) must not change when the store is reopened"
        );
        let current = store.reference(feb, at(feb)).unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].status, ListingStatus::Suspended);
    }
}

// ---------------------------------------------------------------------------
// RSV-05 history: open, close, listing relist, late correction
// ---------------------------------------------------------------------------
#[test]
fn rsv_05_history_appends_without_duplication() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let feb = date(2026, 2, 10);
    let mar = date(2026, 3, 10);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap1 = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap1, None).unwrap();
        let snap2 = snapshot(
            &root.data_dir(),
            feb,
            at(feb),
            vec![row(
                "RELIANCE",
                "INE002A01018",
                feb,
                ListingStatus::Suspended,
            )],
            Some(&snap1),
            None,
        );
        store.publish(&snap2, Some(0)).unwrap();
        let snap3 = snapshot(
            &root.data_dir(),
            mar,
            at(mar),
            vec![row("RELIANCE", "INE002A01018", mar, ListingStatus::Active)],
            Some(&snap2),
            None,
        );
        store.publish(&snap3, Some(1)).unwrap();
    }
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let head = store.head_snapshot().unwrap().unwrap();
        // Each status change closes the old interval and opens a new one:
        // open(day) + close(day)/open(feb) + close(feb)/open(mar) = 5 states.
        assert_eq!(head.listing_states.len(), 5);
        // No duplicate observations: (instrument, venue, valid_from,
        // recorded_at) is unique (this is also enforced by validate()).
        let mut keys: Vec<_> = head
            .listing_states
            .iter()
            .map(|state| {
                (
                    state.instrument_id.clone(),
                    state.venue as u8,
                    state.valid_from,
                    state.recorded_at,
                )
            })
            .collect();
        keys.sort();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "no duplicate listing observations");
        let current = store.reference(mar, at(mar)).unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].status, ListingStatus::Active);
        let frozen = store.reference(feb, at(feb)).unwrap();
        assert_eq!(frozen.len(), 1);
        assert_eq!(frozen[0].status, ListingStatus::Suspended);
    }
}

// ---------------------------------------------------------------------------
// RSV-06 lineage resolves after restart and after restore
// ---------------------------------------------------------------------------
fn snapshot_with_lineage(
    root: &Path,
    day: NaiveDate,
    reference_bytes: &[u8],
    schedule_bytes: &[u8],
    calendar_bytes: &[u8],
) -> ReferenceSnapshot {
    let archive = SourceArchive::open(&root.join("source-archive")).unwrap();
    let reference = archive
        .store("nse-eod-close", reference_bytes, at(day))
        .unwrap();
    let schedule = archive
        .store("nse-tick-schedule", schedule_bytes, at(day))
        .unwrap();
    let calendar = archive
        .store("nse-cash-calendar", calendar_bytes, at(day))
        .unwrap();
    let assignment = TickAssignment {
        tick_paise: 5,
        reference_close_paise: Some(290_000),
        reference_close_date: Some(day),
        reference_source: Some("nse-eod-close".to_owned()),
        reference_sha256_hex: Some(reference.sha256_hex.clone()),
        effective_month: Some(date(2026, 2, 1)),
        schedule_revision: "nse-tick-schedule:nse-tick-schedule-2026-04-15".to_owned(),
        schedule_effective_from: Some("2026-01-01".parse::<NaiveDate>().unwrap()),
        schedule_sha256_hex: Some(schedule.sha256_hex.clone()),
        calendar_revision: Some("nse-cash-calendar:nse-cash-calendar-v1".to_owned()),
        calendar_sha256_hex: Some(calendar.sha256_hex.clone()),
    };
    snapshot(
        root,
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        Some(assignment),
    )
}

#[test]
fn rsv_06_lineage_resolves_after_restart_and_restore() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let reference_bytes =
        b"DATE,SYMBOL,ISIN,CLOSE_PAISE\n2026-01-10,RELIANCE,INE002A01018,290000\n";
    let schedule_bytes = b"LOWER,UPPER,TICK\n0,10000000,5\n10000000,999999999999999,10\n";
    let calendar_bytes = b"DATE,DAY,HOLIDAY,PURPOSE\n2026-01-01,Thu,New Year,\n";
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap = snapshot_with_lineage(
            &root.data_dir(),
            day,
            reference_bytes,
            schedule_bytes,
            calendar_bytes,
        );
        store.publish(&snap, None).unwrap();
    }
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let verification = store.verify_full().unwrap();
        assert_eq!(verification.revision_count, 1);
        assert!(verification.orphans.is_empty());
    }
    // Backup and restore into a fresh location; lineage must still resolve.
    let backup = {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        backup_store(&mut store).unwrap()
    };
    let restored = root.0.join("restored");
    let verification = restore_backup(&backup.path, &restored).unwrap();
    assert_eq!(verification.revision_count, 1);
}

#[test]
fn rsv_06_missing_source_fails_closed() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let snap = snapshot_with_lineage(
        &root.data_dir(),
        day,
        b"\n",
        b"schedule-bytes\n",
        b"calendar-bytes\n",
    );
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        store.publish(&snap, None).unwrap();
    }
    // Delete the reference-close object; verification must fail.
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    let reference = archive
        .verify()
        .unwrap()
        .into_iter()
        .find(|entry| entry.source_id == "nse-eod-close")
        .unwrap();
    std::fs::remove_file(
        root.data_dir()
            .join("source-archive")
            .join(format!("{}.src", reference.sha256_hex)),
    )
    .unwrap();
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert!(store.verify_full().is_err());
}

// ---------------------------------------------------------------------------
// RSV-09 disk and filesystem
// ---------------------------------------------------------------------------
#[test]
#[ignore = "child-process worker; invoked by RSV-09 parent tests"]
fn rsv_09_worker() {
    let dir = PathBuf::from(
        std::env::var("P1_04_CHILD_DIR").expect("P1_04_CHILD_DIR must be set by the parent test"),
    );
    let mode = std::env::var("P1_04_CHILD_MODE").unwrap_or_default();
    let day = date(2026, 1, 10);
    let make_snap = |dir: &Path| {
        snapshot(
            dir,
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        )
    };
    match mode.as_str() {
        "enospc" => {
            // Cap the write size and ignore SIGXFSZ so writes fail with
            // EFBIG instead of killing the process.
            #[allow(unsafe_code)]
            unsafe {
                // The seeded database and its WAL already exceed this cap,
                // so any further write fails with EFBIG (the POSIX stand-in
                // for ENOSPC on this filesystem).
                let limit = libc::rlimit {
                    rlim_cur: 8 * 1024,
                    rlim_max: libc::RLIM_INFINITY,
                };
                libc::setrlimit(libc::RLIMIT_FSIZE, &limit);
                libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            }
            let result = (|| {
                let snap = make_snap(&dir);
                let mut store = DurableReferenceStore::open(&dir)?;
                store.publish(&snap, None)
            })();
            match result {
                Ok(_) => println!("outcome=success (no failure injected)"),
                Err(err) => println!("error={err:#}"),
            }
            #[allow(unsafe_code)]
            unsafe {
                let unbounded = libc::rlimit {
                    rlim_cur: libc::RLIM_INFINITY,
                    rlim_max: libc::RLIM_INFINITY,
                };
                libc::setrlimit(libc::RLIMIT_FSIZE, &unbounded);
            }
        }
        "read_only" => {
            // Make the directory and the database files unwritable before
            // the store is opened, so the write path fails closed.
            #[allow(clippy::permissions_set_readonly_false)]
            let permissions = {
                let mut permissions = std::fs::metadata(&dir).unwrap().permissions();
                permissions.set_readonly(true);
                permissions
            };
            std::fs::set_permissions(&dir, permissions).unwrap();
            for name in [
                "reference.sqlite3",
                "reference.sqlite3-wal",
                "reference.sqlite3-shm",
            ] {
                let path = dir.join(name);
                if path.exists() {
                    let mut file_permissions = std::fs::metadata(&path).unwrap().permissions();
                    file_permissions.set_readonly(true);
                    std::fs::set_permissions(&path, file_permissions).unwrap();
                }
            }
            let result = (|| {
                let snap = make_snap(&dir);
                let mut store = DurableReferenceStore::open(&dir)?;
                store.publish(&snap, None)
            })();
            match result {
                Ok(_) => println!("outcome=success (should not happen)"),
                Err(err) => println!("error={err:#}"),
            }
            #[allow(clippy::permissions_set_readonly_false)]
            let restore = {
                let mut restore = std::fs::metadata(&dir).unwrap().permissions();
                restore.set_readonly(false);
                restore
            };
            std::fs::set_permissions(&dir, restore).unwrap();
        }
        "archive_busy" => {
            // The parent holds the archive lock. Every archive touch is
            // bounded by the 500 ms wait, so the open itself must surface
            // busy, and the store must not mutate.
            let result = DurableReferenceStore::open_with_options(&dir, 5_000, 500)
                .map(|mut store| store.head());
            match result {
                Ok(Ok(_)) => println!("outcome=success (no failure injected)"),
                Ok(Err(err)) => println!("error={err:#}"),
                Err(err) => println!("error={err:#}"),
            }
        }
        other => panic!("unknown mode {other}"),
    }
}

#[test]
fn rsv_09_disk_full_no_false_success() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let output = spawn_child(
        "rsv_09_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_MODE", "enospc".to_string()),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("error="),
        "write failure must surface: {stdout}"
    );
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.revision_count().unwrap(), 1);
}

#[test]
fn rsv_09_read_only_path_no_false_success() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let output = spawn_child(
        "rsv_09_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_MODE", "read_only".to_string()),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("error="),
        "read-only path must fail: {stdout}"
    );
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.revision_count().unwrap(), 1);
}

#[test]
fn rsv_09_archive_busy_timeout() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    // Hold the archive lock in this process; the child must time out.
    let lock_path = root.data_dir().join("source-archive").join(".lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    lock.lock().unwrap();
    let output = spawn_child(
        "rsv_09_worker",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_MODE", "archive_busy".to_string()),
        ],
    );
    drop(lock);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("error=") && stdout.to_lowercase().contains("busy"),
        "archive busy must surface as an error: {stdout}"
    );
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.revision_count().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// RSV-10 corruption
// ---------------------------------------------------------------------------
#[test]
fn rsv_10_immutability_triggers_abort_mutation() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let db = root.data_dir().join("reference.sqlite3");
    let connection = rusqlite::Connection::open(&db).unwrap();
    let update = connection.execute(
        "UPDATE reference_revision SET snapshot_hash = ?1 WHERE revision_id = 0",
        rusqlite::params![format!("{:064}", 'f')],
    );
    assert!(update.is_err(), "UPDATE on a revision must be aborted");
    let delete = connection.execute("DELETE FROM reference_revision", []);
    assert!(delete.is_err(), "DELETE on a revision must be aborted");
    let head_delete = connection.execute("DELETE FROM reference_head", []);
    assert!(head_delete.is_err(), "head deletion must be aborted");
}

#[test]
fn rsv_10_missing_source_rejects_publication() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    // Build a snapshot whose provenance hash is absent from the archive.
    let mut snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    snap.provenance.sha256_hex = format!("{:064}", 'a');
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    let result = store.publish(&snap, None);
    assert!(result.is_err(), "missing source must reject publication");
    assert_eq!(store.revision_count().unwrap(), 0);
}

#[test]
fn rsv_10_tampered_source_bytes_fail_verification() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap, None).unwrap();
    }
    // Overwrite one archived object with different bytes of equal length.
    let archive_root = root.data_dir().join("source-archive");
    let object = archive_root.join(format!(
        "{}.src",
        SourceArchive::open(&archive_root)
            .unwrap()
            .verify()
            .unwrap()[0]
            .sha256_hex
    ));
    std::fs::write(&object, b"XAMBER,NAME OF COMPANY,SERIES,MARKET LOT,ISIN NUMBER\nRELIANCE,Reliance Industries,EQ,1,INE002A01018\nHDFCBANK,HDFC Bank,EQ,1,INE040A01026\n").unwrap();
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert!(
        store.verify_full().is_err(),
        "hash-wrong bytes must fail verification"
    );
    assert_eq!(store.revision_count().unwrap(), 1);
}

#[test]
fn rsv_10_malformed_manifest_line_fails_closed() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    {
        let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
        archive
            .store("nse-cm-master", SOURCE_BYTES, at(day))
            .unwrap();
    }
    let manifest = root
        .data_dir()
        .join("source-archive")
        .join("manifest.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&manifest)
        .unwrap();
    file.write_all(b"not-a-valid-json-line\n").unwrap();
    drop(file);
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    assert!(
        archive.entries().is_err(),
        "malformed complete line must fail closed"
    );
}

#[test]
fn rsv_10_duplicate_conflicting_digest_fails_closed() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    {
        let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
        archive
            .store("nse-cm-master", SOURCE_BYTES, at(day))
            .unwrap();
    }
    let manifest = root
        .data_dir()
        .join("source-archive")
        .join("manifest.jsonl");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let first = text.lines().next().unwrap();
    let entry: serde_json::Value = serde_json::from_str(first).unwrap();
    let mut conflicting = entry.clone();
    conflicting["bytes"] = serde_json::json!(entry["bytes"].as_u64().unwrap() + 1);
    let line = format!("{}\n{}\n", first, conflicting);
    std::fs::write(&manifest, line).unwrap();
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    assert!(archive.entries().is_err());
}

#[test]
fn rsv_10_torn_manifest_tail_is_recovered_by_truncation() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    {
        let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
        archive
            .store("nse-cm-master", SOURCE_BYTES, at(day))
            .unwrap();
    }
    let manifest = root
        .data_dir()
        .join("source-archive")
        .join("manifest.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&manifest)
        .unwrap();
    file.write_all(b"{\"schema_version\":1,\"source_id\":\"torn")
        .unwrap();
    drop(file);
    // Reopen: recovery truncates the unterminated fragment.
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    assert_eq!(archive.entries().unwrap().len(), 1);
    // Read-only open refuses to repair.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&manifest)
        .unwrap();
    file.write_all(b"torn-again").unwrap();
    drop(file);
    let readonly = SourceArchive::open_readonly(&root.data_dir().join("source-archive"), 500);
    assert!(
        readonly.is_err(),
        "read-only open must fail closed on torn tail"
    );
}

#[test]
fn rsv_10_unindexed_object_never_served_and_reported() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    let stored = archive
        .store("nse-cm-master", SOURCE_BYTES, at(day))
        .unwrap();
    // Delete the manifest, leaving the object unindexed.
    std::fs::remove_file(
        root.data_dir()
            .join("source-archive")
            .join("manifest.jsonl"),
    )
    .unwrap();
    let archive = SourceArchive::open(&root.data_dir().join("source-archive")).unwrap();
    assert!(archive.fetch(&stored.sha256_hex).is_err());
    let verification = archive.verify_full().unwrap();
    assert!(verification.unindexed_objects.contains(&stored.sha256_hex));
}

// ---------------------------------------------------------------------------
// Publication-protocol semantics (from the contract failure table)
// ---------------------------------------------------------------------------
#[test]
fn stale_expected_parent_rejects_publication() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let feb = date(2026, 2, 10);
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    // Reconcile against an older head, then expect parent 0.
    let head0 = store.revision_with_meta(0).unwrap().unwrap().snapshot;
    let snap = snapshot(
        &root.data_dir(),
        feb,
        at(feb),
        vec![row("HDFCBANK", "INE040A01026", feb, ListingStatus::Active)],
        Some(&head0),
        None,
    );
    let result = store.publish(&snap, Some(1));
    assert!(result.is_err(), "stale parent must reject");
    let message = format!("{:#}", result.unwrap_err());
    assert!(message.contains("stale expected parent"), "{message}");
    assert_eq!(store.revision_count().unwrap(), 1);
}

#[test]
fn duplicate_publication_retry_resolves_by_key() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    let snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let first = store.publish(&snap, None).unwrap();
    let second = store.publish(&snap, None).unwrap();
    assert_eq!(first.revision_id, second.revision_id);
    assert!(!first.replayed && second.replayed);
    assert_eq!(store.revision_count().unwrap(), 1);
}

#[test]
fn non_canonical_instrument_order_is_rejected() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let mut snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![
            row("RELIANCE", "INE002A01018", day, ListingStatus::Active),
            row("HDFCBANK", "INE040A01026", day, ListingStatus::Active),
        ],
        None,
        None,
    );
    snap.instruments.reverse();
    let result = snap.validate();
    assert!(
        result.is_err(),
        "descending instrument order must be rejected"
    );
}

#[test]
fn duplicate_listing_observation_is_rejected() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let mut snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let duplicate = snap.listing_states[0].clone();
    snap.listing_states.push(duplicate);
    let result = snap.validate();
    assert!(result.is_err(), "duplicate observation must be rejected");
}

#[test]
fn unsupported_newer_store_schema_fails_open() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let db = root.data_dir().join("reference.sqlite3");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch("PRAGMA user_version = 2;")
            .unwrap();
    }
    let result = DurableReferenceStore::open(&root.data_dir());
    assert!(result.is_err(), "newer schema must fail open");
}

#[test]
fn empty_store_has_no_revision_rows_and_no_head() {
    let root = TestRoot::new();
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.head().unwrap(), None);
    assert_eq!(store.revision_count().unwrap(), 0);
    assert!(store.head_snapshot().unwrap().is_none());
    assert!(
        store
            .reference(date(2026, 1, 10), at(date(2026, 1, 10)))
            .unwrap()
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// RSV-11 backup and restore
// ---------------------------------------------------------------------------
#[test]
fn rsv_11_restore_verifies_everything() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let feb = date(2026, 2, 10);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap1 = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap1, None).unwrap();
        let snap2 = snapshot(
            &root.data_dir(),
            feb,
            at(feb),
            vec![row("HDFCBANK", "INE040A01026", feb, ListingStatus::Active)],
            Some(&snap1),
            None,
        );
        store.publish(&snap2, Some(0)).unwrap();
    }
    let backup = {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        backup_store(&mut store).unwrap()
    };
    let restored_root = root.0.join("restored");
    let verification = restore_backup(&backup.path, &restored_root).unwrap();
    assert_eq!(verification.revision_count, 2);
    assert_eq!(verification.head_revision_id, Some(1));

    let mut store = DurableReferenceStore::open(&restored_root).unwrap();
    assert_eq!(store.revision_count().unwrap(), 2);
    assert_eq!(store.head().unwrap(), Some(1));
    for id in 0..2u64 {
        let revision = store.revision_with_meta(id).unwrap().unwrap();
        assert_eq!(revision.snapshot_hash.len(), 64);
    }
    assert!(store.verify_full().is_ok());
}

#[test]
fn rsv_11_restore_fails_closed_on_hash_mismatch() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap, None).unwrap();
    }
    let backup = {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        backup_store(&mut store).unwrap()
    };
    // Tamper with the backup database; restore must fail and leave any
    // existing store untouched.
    let database = backup.path.join("reference.sqlite3");
    let mut bytes = std::fs::read(&database).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&database, &bytes).unwrap();
    let result = restore_backup(&backup.path, &root.data_dir());
    assert!(result.is_err(), "tampered backup must fail restore");
    // Existing store untouched.
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.revision_count().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// RSV-12 performance baseline (no thresholds; baseline evidence only)
// ---------------------------------------------------------------------------
/// A BSE-shaped row: Scrip Code token, settlement group, decimal-rupee tick
/// converted to exact paise.
fn bse_row(
    scrip_code: &str,
    isin: &str,
    as_of: NaiveDate,
    status: ListingStatus,
) -> NormalizedListing {
    NormalizedListing {
        venue: Venue::Bse,
        segment: Segment::Cash,
        instrument_type: InstrumentType::Equity,
        venue_symbol: format!("SYM{scrip_code}"),
        token: Some(scrip_code.to_owned()),
        isin: Isin::new(isin).unwrap(),
        name: format!("Issuer {scrip_code}"),
        status,
        settlement_regime: SettlementRegime::Standard,
        z_category: false,
        settlement_group: Some("T".to_owned()),
        tick_assignment: flat_assignment("test-fixture"),
        market_lot: 1,
        as_of,
    }
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let metadata = entry.metadata().unwrap();
            if metadata.is_file() {
                total += metadata.len();
            } else if metadata.is_dir() {
                total += dir_size(&entry.path());
            }
        }
    }
    total
}

#[test]
fn rsv_12_performance_baseline() {
    // Representative full NSE CM and BSE equity snapshots at realistic
    // scales (NSE CM ~2,500 securities; BSE equity ~4,700). This is a
    // synthetic-baseline harness: thresholds are set only after this
    // baseline exists, per the frozen contract.
    let root = TestRoot::new();
    let day = date(2026, 1, 10);

    let nse_count = 2_500usize;
    let bse_count = 4_700usize;
    let nse_rows: Vec<NormalizedListing> = (0..nse_count)
        .map(|index| {
            row(
                &format!("SYM{index:05}"),
                &synthetic_isin(index),
                day,
                ListingStatus::Active,
            )
        })
        .collect();
    let bse_rows: Vec<NormalizedListing> = (0..bse_count)
        .map(|index| {
            bse_row(
                &format!("{:06}", 500_000 + index),
                &synthetic_isin(10_000 + index),
                day,
                ListingStatus::Active,
            )
        })
        .collect();

    let platform = format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH);
    let cpu_cores = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(0);
    let hardware = format!(
        "logical_cores={cpu_cores} hostname={}",
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".to_owned())
    );
    let dir_before = dir_size(&root.data_dir());

    // Deterministic generated master fixtures sized to the row counts, so
    // the dataset hashes identify the actual measured inputs rather than the
    // small shared test fixture.
    let nse_master = {
        let mut csv = String::from("SYMBOL,NAME OF COMPANY,SERIES,MARKET LOT,ISIN NUMBER\n");
        for index in 0..nse_count {
            csv.push_str(&format!(
                "SYM{index:05},Issuer {index:05},EQ,1,{}\n",
                synthetic_isin(index)
            ));
        }
        csv.into_bytes()
    };
    let bse_master = {
        let mut csv = String::from(
            "Scrip Code,Instrument Code,Scrip Name,Group Name,ISIN CODE,New Tick Size,Market Lot,Security Type Flag\n",
        );
        for index in 0..bse_count {
            csv.push_str(&format!(
                "{:06},SYM{index:06},Issuer {index:06},T,{},0.05,1,EQ\n",
                500_000 + index,
                synthetic_isin(10_000 + index)
            ));
        }
        csv.into_bytes()
    };

    // --- NSE CM snapshot (revision 0) ---
    let start = std::time::Instant::now();
    let nse_snapshot = snapshot_with_provenance(
        &root.data_dir(),
        day,
        at(day),
        nse_rows,
        None,
        None,
        &nse_master,
        "nse-cm-master",
        Venue::Nse,
        "nse-cm-master-v1",
    );
    let nse_dataset_hash = nse_snapshot.provenance.sha256_hex.clone();
    let nse_build = start.elapsed();
    assert_eq!(
        nse_dataset_hash,
        nirnay_extras::venue::india::durable::payload_digest_for_test(&nse_master)
    );

    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    let start = std::time::Instant::now();
    store.publish(&nse_snapshot, None).unwrap();
    let nse_write = start.elapsed();
    let nse_payload_bytes = serde_json::to_vec(&nirnay_extras::venue::india::PayloadEnvelope {
        schema_version: 1,
        snapshot: nse_snapshot.clone(),
    })
    .unwrap()
    .len() as u64;

    // --- BSE equity snapshot (revision 1, accumulated history) ---
    let feb = date(2026, 2, 10);
    let start = std::time::Instant::now();
    let bse_snapshot = snapshot_with_provenance(
        &root.data_dir(),
        feb,
        at(feb),
        bse_rows,
        Some(&nse_snapshot),
        None,
        &bse_master,
        "bse-equity-master",
        Venue::Bse,
        "bse-equity-master-v1",
    );
    let bse_dataset_hash = bse_snapshot.provenance.sha256_hex.clone();
    let bse_build = start.elapsed();
    assert_eq!(
        bse_dataset_hash,
        nirnay_extras::venue::india::durable::payload_digest_for_test(&bse_master)
    );
    let start = std::time::Instant::now();
    store.publish(&bse_snapshot, Some(0)).unwrap();
    let bse_write = start.elapsed();
    let bse_payload_bytes = serde_json::to_vec(&nirnay_extras::venue::india::PayloadEnvelope {
        schema_version: 1,
        snapshot: bse_snapshot.clone(),
    })
    .unwrap()
    .len() as u64;
    drop(store);

    // --- reopen ---
    let start = std::time::Instant::now();
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    let reopen = start.elapsed();

    // --- query ---
    let start = std::time::Instant::now();
    let reference = store.reference(feb, at(feb)).unwrap();
    let query = start.elapsed();
    assert_eq!(reference.len(), nse_count + bse_count);

    let dir_after = dir_size(&root.data_dir());
    println!(
        "RSV-12 baseline: platform={platform} hardware={hardware} source_revision=nse-cm-master-v1+bse-equity-master-v1 \
         nse_dataset_hash={nse_dataset_hash} bse_dataset_hash={bse_dataset_hash} \
         nse_instruments={nse_count} bse_instruments={bse_count} \
         nse_payload_bytes={nse_payload_bytes} bse_payload_bytes={bse_payload_bytes} \
         nse_build={nse_build:?} nse_write={nse_write:?} \
         bse_build={bse_build:?} bse_write={bse_write:?} \
         reopen={reopen:?} query={query:?} \
         disk_before={dir_before} disk_after={dir_after} disk_growth={}",
        dir_after - dir_before,
    );
}

// ---------------------------------------------------------------------------
// Finding A: nanosecond timestamp round-trip
// ---------------------------------------------------------------------------
#[test]
fn nanosecond_timestamp_round_trips_through_metadata() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    // A timestamp NOT aligned to microseconds or milliseconds.
    let recorded = day
        .and_hms_nano_opt(10, 0, 0, 123_456_789u32)
        .unwrap()
        .and_utc();
    let snap = snapshot(
        &root.data_dir(),
        day,
        recorded,
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        store.publish(&snap, None).unwrap();
    }
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    let revision = store
        .revision_with_meta(0)
        .expect("publish must remain readable after reopen")
        .expect("revision 0 must exist");
    assert_eq!(revision.recorded_at, recorded);
    assert_eq!(revision.snapshot.recorded_at, recorded);
    // Metadata and payload agree at full precision.
    let metadata_str: String = {
        let connection =
            rusqlite::Connection::open(root.data_dir().join("reference.sqlite3")).unwrap();
        connection
            .query_row(
                "SELECT recorded_at FROM reference_revision WHERE revision_id = 0",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        DateTime::parse_from_rfc3339(&metadata_str).unwrap(),
        recorded,
        "metadata must carry full nanosecond precision"
    );
}

// ---------------------------------------------------------------------------
// Finding B: replay must verify the actual payload bytes
// ---------------------------------------------------------------------------
#[test]
fn replay_rejects_corrupted_blob_even_with_preserved_checksum_column() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap, None).unwrap();
    }
    // Corrupt the payload BLOB while preserving the checksum column: drop
    // the update trigger, flip one byte of the payload, restore nothing.
    let db = root.data_dir().join("reference.sqlite3");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch("DROP TRIGGER reference_revision_no_update;")
            .unwrap();
        let bytes: Vec<u8> = connection
            .query_row(
                "SELECT payload FROM reference_revision WHERE revision_id = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut corrupted = bytes;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x01;
        connection
            .execute(
                "UPDATE reference_revision SET payload = ?1 WHERE revision_id = 0",
                rusqlite::params![corrupted],
            )
            .unwrap();
    }
    // Replay must fail closed: the checksum column matches but the bytes
    // do not, so this is corruption, not a successful replay.
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    let snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let result = store.publish(&snap, None);
    assert!(
        result.is_err(),
        "replay over a corrupted BLOB must fail closed"
    );
    let message = format!("{:#}", result.unwrap_err());
    assert!(
        message.contains("corruption") || message.contains("checksum"),
        "the error must name the corruption: {message}"
    );
}

// ---------------------------------------------------------------------------
// Finding D: unsupported schema causes no mutation
// ---------------------------------------------------------------------------
#[test]
fn unsupported_schema_fails_open_without_any_mutation() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    let day = date(2026, 1, 10);
    // Create a torn archive tail that recovery WOULD fix on a normal open.
    let manifest = root
        .data_dir()
        .join("source-archive")
        .join("manifest.jsonl");
    {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&manifest)
            .unwrap();
        file.write_all(b"{\"schema_version\":1,\"source_id\":\"torn")
            .unwrap();
    }
    let before_db = std::fs::read(root.data_dir().join("reference.sqlite3")).unwrap();
    let before_manifest = std::fs::read(&manifest).unwrap();
    let before_wal = std::fs::read_dir(root.data_dir())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().to_string(),
                entry.metadata().unwrap().len(),
            )
        })
        .collect::<Vec<_>>();

    // Set an unsupported newer schema version.
    {
        let connection =
            rusqlite::Connection::open(root.data_dir().join("reference.sqlite3")).unwrap();
        connection
            .execute_batch("PRAGMA user_version = 2;")
            .unwrap();
    }
    let db_before_open = std::fs::read(root.data_dir().join("reference.sqlite3")).unwrap();

    let result = DurableReferenceStore::open(&root.data_dir());
    assert!(result.is_err(), "unsupported schema must fail open");

    // Nothing changed: not the database, not the archive manifest (the torn
    // tail is still present, unrepaired), not the directory contents.
    let after_db = std::fs::read(root.data_dir().join("reference.sqlite3")).unwrap();
    assert_eq!(
        db_before_open, after_db,
        "the database must not be modified by the rejected open"
    );
    let after_manifest = std::fs::read(&manifest).unwrap();
    assert_eq!(
        before_manifest, after_manifest,
        "the archive manifest must not be recovered before the schema check"
    );
    let after_names: Vec<String> = std::fs::read_dir(root.data_dir())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    let before_names: Vec<String> = before_wal.iter().map(|(name, _)| name.clone()).collect();
    for name in &after_names {
        assert!(
            before_names.contains(name),
            "the rejected open created {name}"
        );
    }
    let _ = before_db;
    let _ = day;
}

// ---------------------------------------------------------------------------
// Finding E: backup head metadata must match the copied database
// ---------------------------------------------------------------------------
#[test]
fn restore_rejects_backup_with_wrong_head_metadata() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap, None).unwrap();
    }
    let backup = {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        backup_store(&mut store).unwrap()
    };
    // Tamper with the recorded head: 1 instead of 0.
    let manifest_path = backup.path.join("backup.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["head_revision_id"] = serde_json::json!(1);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let target = root.0.join("restored-wrong-head");
    let result = restore_backup(&backup.path, &target);
    assert!(result.is_err(), "head mismatch must fail restore");
    assert!(
        !target.exists(),
        "failed restore must not create the target"
    );
    // Existing store untouched.
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.head().unwrap(), Some(0));
}

// ---------------------------------------------------------------------------
// Finding F: nested payload types reject unknown fields
// ---------------------------------------------------------------------------
#[test]
fn unknown_nested_provenance_field_is_rejected() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let envelope = serde_json::json!({
        "schema_version": 1,
        "snapshot": {
            "valid_at": "2026-01-10",
            "recorded_at": snap.recorded_at.to_rfc3339(),
            "provenance": {
                "schema_version": 1,
                "source_id": "nse-cm-master",
                "venue": "NSE",
                "segment": "cash",
                "retrieved_at": snap.provenance.retrieved_at.to_rfc3339(),
                "sha256_hex": snap.provenance.sha256_hex,
                "source_format_version": "nse-cm-master-v1",
                "record_count": snap.provenance.record_count,
                "declared_by": "test",
                "sneaky_extra": "must be rejected",
            },
            "instruments": snap.instruments,
            "calendar": null,
            "reconciliation": null,
            "listing_states": snap.listing_states,
        },
    });
    let decoded: Result<nirnay_extras::venue::india::PayloadEnvelope, serde_json::Error> =
        serde_json::from_value(envelope);
    assert!(
        decoded.is_err(),
        "unknown provenance fields must be rejected"
    );
}

#[test]
fn unknown_nested_tick_assignment_field_is_rejected() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let state = snap.listing_states[0].clone();
    // The tick assignment is embedded in the listing-state record: injecting
    // an unknown field into its JSON must fail deserialization outright.
    let mut tick = serde_json::to_value(&state.tick_assignment).unwrap();
    tick["fabricated_tick"] = serde_json::json!(7);
    let rejected: Result<india_domain::TickAssignment, serde_json::Error> =
        serde_json::from_value(tick);
    assert!(
        rejected.is_err(),
        "unknown tick-assignment fields must be rejected"
    );
    assert!(
        rejected
            .unwrap_err()
            .to_string()
            .contains("fabricated_tick"),
        "the error must name the unknown field"
    );
    // And the payload path: inject the unknown field into the serialized
    // listing-state JSON and decode the full envelope.
    let mut listing_json = serde_json::to_value(&state).unwrap();
    listing_json["tick_assignment"]["fabricated_tick"] = serde_json::json!(7);
    let envelope = serde_json::json!({
        "schema_version": 1,
        "snapshot": {
            "valid_at": "2026-01-10",
            "recorded_at": snap.recorded_at.to_rfc3339(),
            "provenance": snap.provenance,
            "instruments": snap.instruments,
            "calendar": null,
            "reconciliation": null,
            "listing_states": [listing_json],
        },
    });
    let decoded: Result<nirnay_extras::venue::india::PayloadEnvelope, serde_json::Error> =
        serde_json::from_value(envelope);
    assert!(
        decoded.is_err(),
        "unknown tick-assignment fields must be rejected in the payload path"
    );
}

// ---------------------------------------------------------------------------
// Busy timeout is honoured (finding: configured value was overridden)
// ---------------------------------------------------------------------------
#[test]
#[ignore = "child-process worker; invoked by the busy-timeout parent test"]
fn rsv_09_worker_tiny_busy_timeout() {
    let dir = PathBuf::from(
        std::env::var("P1_04_CHILD_DIR").expect("P1_04_CHILD_DIR must be set by the parent test"),
    );
    // Hold a write transaction in this process; the child's configured
    // 250 ms busy timeout must surface a busy error at publish time.
    let day = date(2026, 1, 10);
    let snap = snapshot(
        &dir,
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let result = (|| {
        let mut store = DurableReferenceStore::open_with_options(&dir, 250, 10_000)?;
        store.publish(&snap, None)
    })();
    match result {
        Ok(_) => println!("outcome=success (no failure injected)"),
        Err(err) => println!("error={err:#}"),
    }
}

#[test]
fn configured_busy_timeout_is_honoured() {
    let root = TestRoot::new();
    seed_revision_zero(&root);
    // Hold an IMMEDIATE transaction on a second connection in this process,
    // then ask the child to publish with a tiny busy timeout: the publish
    // must fail with a busy error rather than waiting 5 seconds.
    let db = root.data_dir().join("reference.sqlite3");
    let mut connection = rusqlite::Connection::open(&db).unwrap();
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let output = spawn_child(
        "rsv_09_worker_tiny_busy_timeout",
        &[
            ("P1_04_CHILD_DIR", root.data_dir().display().to_string()),
            ("P1_04_CHILD_MODE", "tiny_busy".to_string()),
        ],
    );
    drop(transaction);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("error=") && stdout.to_lowercase().contains("busy"),
        "configured busy timeout must surface busy errors: {stdout}"
    );
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    assert_eq!(store.revision_count().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// Integration boundary: the ingestion workflow end-to-end through durable
// storage (parse -> reconcile -> snapshot -> durable publish -> restart ->
// reference query), using the real NSE CM master parser.
// ---------------------------------------------------------------------------
#[test]
fn ingestion_workflow_persists_through_durable_storage() {
    use nirnay_extras::venue::india::parse_nse_cm_master;

    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    let feb = date(2026, 2, 10);

    // Day 1: parse the real master (parser in the loop), normalize the
    // rows, reconcile into a fresh universe.
    let parsed = parse_nse_cm_master(std::str::from_utf8(SOURCE_BYTES).unwrap()).unwrap();
    assert_eq!(parsed.len(), 2);
    let rows = parsed
        .iter()
        .map(|parsed_row| {
            let mut listing = normalized_equity(
                &parsed_row.symbol,
                parsed_row.isin.as_str(),
                day,
                ListingStatus::Active,
                SettlementRegime::Standard,
            );
            listing.name = parsed_row.name.clone();
            listing
        })
        .collect();
    let snap1 = snapshot(&root.data_dir(), day, at(day), rows, None, None);
    assert_eq!(snap1.instruments.len(), 2);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        store.publish(&snap1, None).unwrap();
    }

    // Day 2: reconcile against the previous head through the durable store.
    let head = {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        store.revision_with_meta(0).unwrap().unwrap().snapshot
    };
    let suspension = "SYMBOL,NAME OF COMPANY,SERIES,MARKET LOT,ISIN NUMBER\n\
HDFCBANK,HDFC Bank,BE,1,INE040A01026\n";
    let feb_parsed = parse_nse_cm_master(suspension).unwrap();
    let feb_rows = feb_parsed
        .iter()
        .map(|parsed_row| {
            // SERIES `BE` is a trade-for-trade settlement regime, never a
            // suspension (see the ingestion module invariants).
            let mut listing = normalized_equity(
                &parsed_row.symbol,
                parsed_row.isin.as_str(),
                feb,
                ListingStatus::Active,
                SettlementRegime::TradeForTrade,
            );
            listing.name = parsed_row.name.clone();
            listing
        })
        .collect();
    let snap2 = snapshot(&root.data_dir(), feb, at(feb), feb_rows, Some(&head), None);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let outcome = store.publish(&snap2, Some(0)).unwrap();
        assert_eq!(outcome.revision_id, 1);
    }

    // Restart: the historical query must be identical pre/post restart, and
    // RELIANCE must remain listed on day 1 while HDFCBANK moved to BE.
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    let historical = store.reference(day, at(day)).unwrap();
    assert_eq!(historical.len(), 2);
    let reliance = historical
        .iter()
        .find(|state| state.symbol == "RELIANCE")
        .unwrap();
    assert_eq!(reliance.settlement_regime, SettlementRegime::Standard);
    let current = store.reference(feb, at(feb)).unwrap();
    let hdfc = current
        .iter()
        .find(|state| state.symbol == "HDFCBANK")
        .unwrap();
    assert_eq!(hdfc.settlement_regime, SettlementRegime::TradeForTrade);
    assert!(store.verify_full().is_ok());
}

// ---------------------------------------------------------------------------
// Review round 2, finding 1: replay compares the semantic hash column
// ---------------------------------------------------------------------------
#[test]
fn replay_rejects_corrupted_semantic_hash_column() {
    let root = TestRoot::new();
    let day = date(2026, 1, 10);
    {
        let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
        let snap = snapshot(
            &root.data_dir(),
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap, None).unwrap();
    }
    // Corrupt ONLY the snapshot_hash column: payload bytes, payload_sha256
    // and publication_key all stay intact.
    let db = root.data_dir().join("reference.sqlite3");
    {
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch("DROP TRIGGER reference_revision_no_update;")
            .unwrap();
        connection
            .execute(
                "UPDATE reference_revision SET snapshot_hash = ?1 WHERE revision_id = 0",
                rusqlite::params!["e".repeat(64)],
            )
            .unwrap();
    }
    // The replay path must run full row verification and fail closed
    // instead of reporting a successful replay.
    let mut store = DurableReferenceStore::open(&root.data_dir()).unwrap();
    let snap = snapshot(
        &root.data_dir(),
        day,
        at(day),
        vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
        None,
        None,
    );
    let result = store.publish(&snap, None);
    assert!(
        result.is_err(),
        "replay over a corrupted semantic hash must fail closed"
    );
    let message = format!("{:#}", result.unwrap_err());
    assert!(
        message.contains("semantic snapshot hash mismatch"),
        "the error must name the semantic hash mismatch: {message}"
    );
}

// ---------------------------------------------------------------------------
// Review round 2, finding 2: schema preflight before directory creation
// ---------------------------------------------------------------------------
#[test]
fn unsupported_schema_fails_before_creating_missing_directories() {
    let root = TestRoot::new();
    // A store with ONLY the database file: backups/ and source-archive/ do
    // not exist yet.
    let data_dir = root.data_dir();
    std::fs::create_dir_all(&data_dir).unwrap();
    let day = date(2026, 1, 10);
    {
        let mut store = DurableReferenceStore::open(&data_dir).unwrap();
        let snap = snapshot(
            &data_dir,
            day,
            at(day),
            vec![row("RELIANCE", "INE002A01018", day, ListingStatus::Active)],
            None,
            None,
        );
        store.publish(&snap, None).unwrap();
    }
    // Remove the directories, then mark the schema unsupported.
    std::fs::remove_dir_all(data_dir.join("backups")).unwrap();
    std::fs::remove_dir_all(data_dir.join("source-archive")).unwrap();
    {
        let connection = rusqlite::Connection::open(data_dir.join("reference.sqlite3")).unwrap();
        connection
            .execute_batch("PRAGMA user_version = 2;")
            .unwrap();
    }
    let tree_before = dir_entries_recursive(&data_dir);
    assert!(
        !data_dir.join("backups").exists(),
        "test setup: backups must be missing"
    );
    assert!(
        !data_dir.join("source-archive").exists(),
        "test setup: source-archive must be missing"
    );

    let result = DurableReferenceStore::open(&data_dir);
    assert!(result.is_err(), "unsupported schema must fail open");

    let tree_after = dir_entries_recursive(&data_dir);
    assert_eq!(
        tree_before, tree_after,
        "the rejected open must not create directories or files"
    );
}

fn dir_entries_recursive(path: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            out.push(
                entry
                    .path()
                    .strip_prefix(path)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
            );
            if entry.path().is_dir() {
                out.extend(dir_entries_recursive(&entry.path()));
            }
        }
    }
    out.sort();
    out
}
