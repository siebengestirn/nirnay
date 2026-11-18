#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Durable source archive, identity persistence and revision publication tests.

use std::path::{
    Path, //
    PathBuf,
};
use std::process::{
    Command, //
    Output,
};

use india_reference::{
    ArchiveError, //
    ReferenceStore,
    SourceArchive,
    StoreError,
    set_fail_point,
};
use trading_core::{
    Isin, //
    Venue,
};

const ISIN_A: &str = "INE848E01016";
const ISIN_B: &str = "INE002A01018";
const DIGEST_1: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_A: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const DIGEST_B: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn archive_root(directory: &Path) -> PathBuf {
    directory.join("archive")
}

#[test]
fn source_archive_verifies_content_and_refuses_torn_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let root = archive_root(directory.path());
    let mut archive = SourceArchive::create_new(&root).unwrap();
    let record = archive
        .put_source(b"nse-master-bytes", "nse.cash", "v1")
        .unwrap();
    assert_eq!(archive.verify().unwrap(), 1, "one archived source verifies");
    assert_eq!(record.size, 16, "source size is recorded");

    let object = root.join("objects").join(&record.hash_hex);
    std::fs::write(&object, b"tampered").unwrap();
    assert!(
        matches!(archive.verify(), Err(ArchiveError::HashMismatch { .. })),
        "content hash mismatch fails closed"
    );
    std::fs::write(&object, b"nse-master-bytes").unwrap();

    let mut manifest = std::fs::read(root.join("manifest.jsonl")).unwrap();
    manifest.extend_from_slice(b"{\"partial\":");
    std::fs::write(root.join("manifest.jsonl"), manifest).unwrap();
    assert!(
        matches!(SourceArchive::open(&root), Err(ArchiveError::TornManifest)),
        "normal open refuses a torn manifest"
    );
    let report = india_reference::recover(&root).unwrap();
    assert_eq!(report.complete_records, 1, "complete record is preserved");
    assert!(report.damaged_bytes > 0, "damaged bytes are reported");
    assert!(
        Path::new(&report.quarantine).exists(),
        "damaged tail is quarantined"
    );
    assert_eq!(
        SourceArchive::open(&root).unwrap().verify().unwrap(),
        1,
        "archive verifies after explicit recovery"
    );
}

#[test]
fn identity_allocation_is_persistent_and_collision_safe() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reference.sqlite3");
    let first_id;
    {
        let mut store = ReferenceStore::open(&path).unwrap();
        first_id = store
            .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
            .unwrap();
        let same = store
            .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Bse, "AAA", "in.ref")
            .unwrap();
        assert_eq!(first_id, same, "aliases resolve to one identity");
    }
    {
        let mut store = ReferenceStore::open(&path).unwrap();
        let reopened = store
            .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
            .unwrap();
        assert_eq!(
            first_id, reopened,
            "identity survives a process-style reopen"
        );
        assert!(
            store
                .resolve_identity(&Isin::new(ISIN_B).unwrap(), Venue::Nse, "AAA", "in.ref")
                .is_err(),
            "symbol reuse is rejected"
        );
    }
}

#[test]
fn revisions_enforce_parent_and_idempotency() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reference.sqlite3");
    let mut store = ReferenceStore::open(&path).unwrap();
    let first = store
        .publish_revision(None, DIGEST_1, Path::new("revision-1.json"), "key-1")
        .unwrap();
    assert!(!first.deduplicated, "first publication is new");
    let repeat = store
        .publish_revision(None, DIGEST_1, Path::new("revision-1.json"), "key-1")
        .unwrap();
    assert!(repeat.deduplicated, "same key and digest is idempotent");
    assert!(
        matches!(
            store.publish_revision(Some("wrong"), DIGEST_2, Path::new("r2"), "key-2"),
            Err(StoreError::ParentMismatch { .. })
        ),
        "wrong parent fails closed"
    );
    let second = store
        .publish_revision(Some(DIGEST_1), DIGEST_2, Path::new("r2"), "key-2")
        .unwrap();
    assert!(
        second.sequence > first.sequence,
        "revision sequence advances"
    );
    assert_eq!(
        store.head().unwrap().as_deref(),
        Some(DIGEST_2),
        "head advances to the last published revision"
    );
}

#[test]
fn backup_and_restore_preserve_db_manifest_and_objects() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reference.sqlite3");
    let mut store = ReferenceStore::open(&source).unwrap();
    let identity = store
        .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
        .unwrap();
    store
        .publish_revision(None, DIGEST_1, Path::new("revision-1.json"), "key-1")
        .unwrap();
    let archive_root = directory.path().join("archive");
    let mut archive = SourceArchive::create_new(&archive_root).unwrap();
    archive
        .put_source(b"nse-master-bytes", "nse.cash", "v1")
        .unwrap();

    let backup_dir = directory.path().join("backup");
    let files = store
        .backup_with_source(&backup_dir, &archive_root)
        .unwrap();
    assert!(
        files.iter().any(|path| path.ends_with("SHA256SUMS")),
        "backup carries incremental checksums"
    );
    assert!(
        backup_dir.join("source-archive/manifest.jsonl").is_file(),
        "backup carries the source manifest"
    );

    let restore_dir = directory.path().join("restored");
    let outcome = india_reference::restore_verified(&backup_dir, &restore_dir).unwrap();
    assert_eq!(
        outcome,
        india_reference::RestoreOutcome::Verified,
        "restore completes as a verified state"
    );
    let mut restored_store = ReferenceStore::open(restore_dir.join("reference.sqlite3")).unwrap();
    assert_eq!(
        restored_store.head().unwrap().as_deref(),
        Some(DIGEST_1),
        "restored store retains the revision head"
    );
    let restored_archive = SourceArchive::open(restore_dir.join("source-archive")).unwrap();
    assert_eq!(
        restored_archive.verify().unwrap(),
        1,
        "source objects restored"
    );
    assert_eq!(
        restored_store
            .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
            .unwrap(),
        identity,
        "identity survives backup and restore"
    );

    std::fs::write(
        restore_dir.join("source-archive/manifest.jsonl"),
        b"tampered\n",
    )
    .unwrap();
    let tampered_target = directory.path().join("tampered-target");
    assert!(
        india_reference::restore_verified(&restore_dir, &tampered_target).is_err(),
        "a tampered backup fails verification before restore"
    );
}

#[test]
fn crash_before_commit_leaves_no_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("crash.sqlite3");
    let mut store = ReferenceStore::open(&path).unwrap();
    set_fail_point(Some("before_commit"));
    let result = store.resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref");
    set_fail_point(None);
    assert!(result.is_err(), "injected failure aborts the transaction");
    let mut reopened = ReferenceStore::open(&path).unwrap();
    let recovered = reopened
        .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
        .unwrap();
    assert_eq!(
        recovered.as_str(),
        "in.ref00000000",
        "identity counter was not consumed"
    );
}

/// Child-only worker: must never pass when its required environment is absent.
#[test]
#[ignore = "child-only worker invoked by parent with --ignored"]
fn child_requires_environment() {
    let required = "NIRNAY_REFERENCE_CHILD";
    assert!(
        std::env::var_os(required).is_some(),
        "child worker requires {required}"
    );
}

#[test]
fn parent_invokes_child_worker_with_environment() {
    let executable = std::env::current_exe().unwrap();
    let output: Output = Command::new(executable)
        .arg("--ignored")
        .arg("child_requires_environment")
        .env("NIRNAY_REFERENCE_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child worker ran with its environment"
    );

    let without_env = Command::new(std::env::current_exe().unwrap())
        .arg("--ignored")
        .arg("child_requires_environment")
        .env_remove("NIRNAY_REFERENCE_CHILD")
        .output()
        .unwrap();
    assert!(
        !without_env.status.success(),
        "child worker fails closed without its environment"
    );
}

#[test]
fn provider_identity_uniqueness_is_enforced() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("provider.sqlite3");
    let mut store = ReferenceStore::open(&path).unwrap();
    store
        .resolve_identity_with_provider(
            &Isin::new(ISIN_A).unwrap(),
            Venue::Nse,
            "AAA",
            "in.ref",
            Some("upstox:NSE_EQ|AAA"),
        )
        .unwrap();
    assert!(
        store
            .resolve_identity_with_provider(
                &Isin::new(ISIN_B).unwrap(),
                Venue::Bse,
                "BBB",
                "in.ref",
                Some("upstox:NSE_EQ|AAA"),
            )
            .is_err(),
        "a provider identity cannot move to another instrument"
    );
}

#[test]
fn revisions_are_immutable_on_update() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = ReferenceStore::open(directory.path().join("immutable.sqlite3")).unwrap();
    store
        .publish_revision(None, DIGEST_1, Path::new("r1"), "key-1")
        .unwrap();
    assert!(
        store
            .connection_for_test()
            .execute("UPDATE revisions SET digest = 'changed'", [])
            .is_err(),
        "revision rows cannot be mutated"
    );
}

#[test]
fn concurrent_publishers_cannot_bypass_the_parent_check() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("concurrent.sqlite3");
    drop(ReferenceStore::open(&path).unwrap());
    let first = std::thread::spawn({
        let path = path.clone();
        move || {
            let mut store = ReferenceStore::open(&path).unwrap();
            store
                .publish_revision(None, DIGEST_A, Path::new("a"), "key-a")
                .map(|_| ())
                .map_err(|e| format!("{e}"))
        }
    });
    let second = std::thread::spawn(move || {
        let mut store = ReferenceStore::open(&path).unwrap();
        store
            .publish_revision(None, DIGEST_B, Path::new("b"), "key-b")
            .map(|_| ())
            .map_err(|e| format!("{e}"))
    });
    let first_result = first.join().unwrap();
    let second_result = second.join().unwrap();
    assert!(
        first_result.is_ok() ^ second_result.is_ok(),
        "exactly one root revision is published: first={first_result:?} second={second_result:?}"
    );
    let loser = if first_result.is_err() {
        &first_result
    } else {
        &second_result
    };
    assert!(
        format!("{loser:?}").contains("expected parent"),
        "the losing publisher reports a parent mismatch"
    );
}

/// Child-only crash worker. Invoked by the parent with a fresh process.
#[test]
#[ignore = "child-only crash worker invoked by parent with --ignored"]
fn child_crash_before_commit() {
    let path = std::env::var_os("NIRNAY_CRASH_DB").expect("child requires NIRNAY_CRASH_DB");
    india_reference::set_fail_point(Some("before_commit"));
    let mut store = ReferenceStore::open(PathBuf::from(path)).unwrap();
    let _ = store.resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref");
    india_reference::set_fail_point(None);
}

#[test]
fn fresh_process_crash_leaves_no_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("crash-process.sqlite3");
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("--ignored")
        .arg("child_crash_before_commit")
        .env("NIRNAY_CRASH_DB", &path)
        .output()
        .unwrap();
    assert!(child.status.success(), "child worker exited normally");

    let mut store = ReferenceStore::open(&path).unwrap();
    let recovered = store
        .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
        .unwrap();
    assert_eq!(
        recovered.as_str(),
        "in.ref00000000",
        "crashed transaction consumed no identity"
    );
}

#[test]
fn non_hex_digest_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = ReferenceStore::open(directory.path().join("digest.sqlite3")).unwrap();
    assert!(
        store
            .publish_revision(None, "not-a-digest", Path::new("r"), "key")
            .is_err(),
        "a digest must be content-bound hex"
    );
}

#[test]
fn after_commit_publication_failure_is_visible_on_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("after-commit.sqlite3");
    let mut store = ReferenceStore::open(&path).unwrap();
    india_reference::set_fail_point(Some("after_commit"));
    let result = store.publish_revision(None, DIGEST_1, Path::new("r1"), "key-1");
    india_reference::set_fail_point(None);
    assert!(result.is_err(), "injected post-commit failure is reported");
    let reopened = ReferenceStore::open(&path).unwrap();
    assert_eq!(
        reopened.head().unwrap().as_deref(),
        Some(DIGEST_1),
        "the committed revision is visible after restart"
    );
}

#[test]
fn restore_sync_failure_is_uncertain_not_clean_rollback() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("sync.sqlite3");
    let store = ReferenceStore::open(&source).unwrap();
    let backup_root = directory.path().join("backup");
    let archive_root = directory.path().join("archive");
    std::fs::create_dir(&archive_root).unwrap();
    std::fs::write(archive_root.join("manifest.jsonl"), b"").unwrap();
    store
        .backup_with_source(&backup_root, &archive_root)
        .unwrap();
    india_reference::set_fail_point(Some("restore_sync"));
    let outcome =
        india_reference::restore_verified(&backup_root, &directory.path().join("restored"));
    india_reference::set_fail_point(None);
    assert_eq!(
        outcome.unwrap(),
        india_reference::RestoreOutcome::Uncertain {
            message: "parent directory sync failed after restore; target may have changed"
                .to_owned()
        },
        "an uncertain restore is represented explicitly"
    );
}

/// Child-only process-termination worker: commits an identity and a revision,
/// emits proof of both, then aborts before any output publication.
#[test]
#[ignore = "child-only crash worker invoked by parent with --ignored"]
fn child_abort_after_commit() {
    let path = std::env::var_os("NIRNAY_ABORT_DB").expect("child requires NIRNAY_ABORT_DB");
    let mut store = ReferenceStore::open(PathBuf::from(path)).unwrap();
    let identity = store
        .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
        .unwrap();
    store
        .publish_revision(None, DIGEST_1, Path::new("revision.json"), "crash-key")
        .unwrap();
    let proof_path = std::env::var_os("NIRNAY_PROOF_FILE").expect("child requires proof file");
    std::fs::write(
        PathBuf::from(proof_path),
        format!("COMMITTED:{}:{}", identity.as_str(), DIGEST_1),
    )
    .expect("proof must be on disk before abort");
    std::process::abort();
}

#[test]
fn process_termination_after_commit_does_not_duplicate_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("abort.sqlite3");
    let proof_path = directory.path().join("proof.txt");
    let child = Command::new(std::env::current_exe().unwrap())
        .arg("--ignored")
        .arg("child_abort_after_commit")
        .env("NIRNAY_ABORT_DB", &path)
        .env("NIRNAY_PROOF_FILE", &proof_path)
        .output()
        .unwrap();
    assert!(!child.status.success(), "child process was terminated");
    assert!(proof_path.is_file(), "child wrote proof before termination");
    let proof = std::fs::read_to_string(&proof_path).unwrap();
    assert!(
        proof == format!("COMMITTED:in.ref00000000:{DIGEST_1}"),
        "child emitted proof of a committed identity and revision before abort"
    );

    let mut store = ReferenceStore::open(&path).unwrap();
    assert_eq!(
        store.head().unwrap().as_deref(),
        Some(DIGEST_1),
        "fresh process verifies the committed revision"
    );
    let identity = store
        .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
        .unwrap();
    assert_eq!(
        identity.as_str(),
        "in.ref00000000",
        "committed identity is recovered once, without reuse"
    );
}

#[test]
fn restore_rejects_tampered_objects_without_publishing_a_target() {
    let directory = tempfile::tempdir().unwrap();
    let store_path = directory.path().join("tamper.sqlite3");
    let store = ReferenceStore::open(&store_path).unwrap();
    let archive_root = directory.path().join("archive");
    let mut archive = SourceArchive::create_new(&archive_root).unwrap();
    let record = archive.put_source(b"original-bytes", "nse", "v1").unwrap();
    let backup = directory.path().join("backup");
    store.backup_with_source(&backup, &archive_root).unwrap();

    let cases = [
        ("altered", directory.path().join("target-altered")),
        ("removed", directory.path().join("target-removed")),
        ("mismatch", directory.path().join("target-mismatch")),
    ];
    for (case, target) in cases {
        let tampered = directory.path().join(format!("tampered-{case}"));
        copy_directory(&backup, &tampered);
        let object = tampered
            .join("source-archive/objects")
            .join(&record.hash_hex);
        match case {
            "altered" => std::fs::write(&object, b"changed-bytes").unwrap(),
            "removed" => std::fs::remove_file(&object).unwrap(),
            "mismatch" => {
                let manifest = tampered.join("source-archive/manifest.jsonl");
                let mut bytes = std::fs::read(&manifest).unwrap();
                if let Some(byte) = bytes.iter_mut().find(|byte| byte.is_ascii_hexdigit()) {
                    *byte = if *byte == b'0' { b'1' } else { b'0' };
                }
                std::fs::write(&manifest, bytes).unwrap();
            }
            _ => unreachable!("named cases only"),
        }
        let tampered_archive = SourceArchive::open(tampered.join("source-archive"));
        let archive_rejected = match &tampered_archive {
            Ok(archive) => archive.verify().is_err(),
            Err(_) => true,
        };
        assert!(
            archive_rejected,
            "tampering must be detected by archive-level verification"
        );
        assert!(
            india_reference::restore_verified(&tampered, &target).is_err(),
            "{case} backup must be rejected"
        );
        assert!(
            !target.exists(),
            "{case} rejection must not publish a partial target"
        );
    }
}

fn copy_directory(source: &std::path::Path, destination: &std::path::Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// Promoted P1-04 reference-store contract: RSV-01/02/03 foundation.
// ---------------------------------------------------------------------------

use india_domain::{
    CalendarDay, //
    Instrument,
    InstrumentId,
    Listing,
    ListingStatus,
    ListingTerms,
    Segment,
    SessionDay,
    SettlementRegime,
    SourceProvenance,
    TickAssignment,
    TradingCalendar,
};
use india_reference::{
    DurableReferenceStore, //
    ListingStateRecord,
    Reconciliation,
    ReferenceSnapshot,
    STORE_SCHEMA_VERSION,
    StoredRevision,
    decode_payload,
    durable_connection_for_test,
};
use jiff::{
    Timestamp, //
    civil::Date,
};

fn contract_provenance(digest: &str) -> SourceProvenance {
    SourceProvenance {
        schema_version: 1,
        source_id: "nse.cash_equity_master".to_owned(),
        venue: Venue::Nse,
        segment: Segment::Cash,
        retrieved_at: "2026-11-18T00:00:00Z"
            .parse()
            .expect("valid retrieval timestamp"),
        sha256_hex: digest.to_owned(),
        source_format_version: "nse-cash-equity-v1".to_owned(),
        record_count: 1,
        declared_by: "operator".to_owned(),
    }
}

fn contract_tick() -> TickAssignment {
    TickAssignment {
        tick_paise: 5,
        reference_close_paise: None,
        reference_close_date: None,
        reference_source: None,
        reference_sha256_hex: None,
        effective_month: None,
        schedule_revision: "nse:tick-v1".to_owned(),
        schedule_effective_from: None,
        schedule_sha256_hex: None,
        calendar_revision: None,
        calendar_sha256_hex: None,
    }
}

fn contract_instrument(id: &str, isin: &str, symbol: &str) -> Instrument {
    let effective_from = Date::new(2026, 1, 1).expect("valid date");
    Instrument {
        instrument_id: InstrumentId::new(id).expect("valid id"),
        instrument_type: india_domain::InstrumentType::Equity,
        isin: Some(Isin::new(isin).expect("valid ISIN")),
        name: format!("{symbol} Limited"),
        underlying_instrument_id: None,
        expiry_date: None,
        strike_price_paise: None,
        option_kind: None,
        listings: vec![Listing {
            venue: Venue::Nse,
            segment: Segment::Cash,
            venue_symbol: symbol.to_owned(),
            token: None,
            effective_from,
            effective_to: None,
            status: ListingStatus::Active,
            settlement_regime: SettlementRegime::Standard,
            z_category: false,
            settlement_group: None,
            terms: vec![ListingTerms {
                effective_from,
                effective_to: None,
                tick_assignment: contract_tick(),
                market_lot: 1,
                quantity_increment: 1,
            }],
        }],
        corporate_actions: Vec::new(),
    }
}

fn contract_state(
    instrument_id: &InstrumentId,
    symbol: &str,
    valid_from: Date,
    valid_to: Option<Date>,
    recorded_at: Timestamp,
) -> ListingStateRecord {
    ListingStateRecord {
        instrument_id: instrument_id.clone(),
        venue: Venue::Nse,
        symbol: symbol.to_owned(),
        token: None,
        status: ListingStatus::Active,
        settlement_regime: SettlementRegime::Standard,
        z_category: false,
        settlement_group: None,
        market_lot: 1,
        tick_assignment: contract_tick(),
        valid_from,
        valid_to,
        recorded_at,
        provenance: contract_provenance(DIGEST_A),
    }
}

fn contract_snapshot(suffix: &str) -> ReferenceSnapshot {
    let instrument_id = InstrumentId::new(format!("eq.{suffix}")).expect("valid id");
    let recorded_at: Timestamp = "2026-11-18T12:00:00Z".parse().expect("valid timestamp");
    let later: Timestamp = "2026-11-19T12:00:00Z".parse().expect("valid timestamp");
    let day = Date::new(2026, 11, 18).expect("valid date");
    ReferenceSnapshot {
        valid_at: day,
        recorded_at,
        provenance: contract_provenance(DIGEST_A),
        instruments: vec![contract_instrument(
            instrument_id.as_str(),
            "INE848E01016",
            suffix,
        )],
        calendar: Some(TradingCalendar {
            venue: Venue::Nse,
            days: vec![CalendarDay {
                venue: Venue::Nse,
                date: day,
                session: SessionDay::Trading,
                note: None,
            }],
        }),
        reconciliation: Some(Reconciliation {
            added: vec![instrument_id.clone()],
            removed: Vec::new(),
            changed: Vec::new(),
        }),
        // Canonical order is by recorded_at first, not by instrument or
        // validity. Both a calendar and a multi-record history are exercised.
        listing_states: vec![
            contract_state(
                &instrument_id,
                suffix,
                Date::new(2026, 1, 1).expect("valid date"),
                Some(Date::new(2026, 11, 17).expect("valid date")),
                recorded_at,
            ),
            contract_state(
                &instrument_id,
                suffix,
                Date::new(2026, 11, 18).expect("valid date"),
                None,
                later,
            ),
        ],
    }
}

fn contract_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "nirnay-rsv-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("main")
    ));
    let _ = std::fs::remove_dir_all(&root);
    root
}

fn published_meta(store: &DurableReferenceStore, id: u64) -> StoredRevision {
    store
        .revision_with_meta(id)
        .expect("revision decodes")
        .expect("revision exists")
}

#[test]
fn rsv01_snapshot_persists_reopen_and_compares_all_fields() {
    let root = contract_root("rsv01");
    let original = contract_snapshot("alpha");
    {
        let mut store = DurableReferenceStore::open(&root).expect("store opens");
        let outcome = store.publish(&original, None).expect("first publish");
        assert_eq!(outcome.revision_id, 0, "genesis revision is zero");
    }
    let reopened = DurableReferenceStore::open(&root).expect("store reopens");
    let restored = reopened
        .revision(0)
        .expect("revision verifies")
        .expect("revision exists");
    assert_eq!(restored, original, "every snapshot field survives reopen");
}

#[test]
fn validation_rejects_noncanonical_and_unknown_fields() {
    let directory = tempfile::tempdir().expect("tempdir");
    let mut reversed = contract_snapshot("alpha");
    reversed
        .instruments
        .insert(0, contract_instrument("eq.zeta", "INE002A01018", "zeta"));
    assert!(
        reversed.validate().is_err(),
        "non-ascending instruments are refused"
    );

    let mut duplicate = contract_snapshot("alpha");
    duplicate
        .listing_states
        .push(duplicate.listing_states[0].clone());
    assert!(
        duplicate.validate().is_err(),
        "duplicate observation keys are refused"
    );

    let mut conflicting = contract_snapshot("alpha");
    let mut conflict = conflicting.listing_states[0].clone();
    conflict.symbol = "other-symbol".to_owned();
    conflicting.listing_states.insert(1, conflict);
    assert!(
        conflicting.validate().is_err(),
        "a conflicting observation with the same four-part key is refused"
    );

    let mut noncanonical = contract_snapshot("alpha");
    noncanonical.listing_states.reverse();
    assert!(
        noncanonical.validate().is_err(),
        "non-canonical listing-state order is refused, not silently sorted"
    );

    let mut unknown = serde_json::to_value(contract_snapshot("alpha")).expect("snapshot value");
    unknown
        .as_object_mut()
        .expect("snapshot object")
        .insert("surprise".to_owned(), serde_json::json!(1));
    assert!(
        decode_payload(&serde_json::to_vec(&unknown).expect("bytes")).is_err(),
        "unknown fields fail closed"
    );

    let root = directory.path().join("store");
    let mut store = DurableReferenceStore::open(&root).expect("store opens");
    assert!(
        store.publish(&reversed, None).is_err(),
        "invalid snapshots create no revision"
    );
    assert_eq!(store.head().expect("head"), None, "no head was published");
}

#[test]
fn rsv02_revision_ids_contiguous_and_parent_chain_survive_restart() {
    let root = contract_root("rsv02");
    {
        let mut store = DurableReferenceStore::open(&root).expect("store opens");
        for (index, suffix) in ["alpha", "beta", "gamma"].iter().enumerate() {
            let expected = if index == 0 {
                None
            } else {
                Some(index as u64 - 1)
            };
            let outcome = store
                .publish(&contract_snapshot(suffix), expected)
                .expect("publish");
            assert_eq!(
                outcome.revision_id, index as u64,
                "revision ids are contiguous from zero"
            );
        }
    }
    let store = DurableReferenceStore::open(&root).expect("store reopens");
    let report = store.verify_full().expect("full verification");
    assert_eq!(report.revision_count, 3, "all revisions are present");
    assert_eq!(
        report.head_revision_id,
        Some(2),
        "head is the last revision"
    );
    assert_eq!(
        published_meta(&store, 0).parent_revision_id,
        None,
        "the genesis revision has no parent"
    );
    assert_eq!(
        published_meta(&store, 1).parent_revision_id,
        Some(0),
        "the second revision points at genesis"
    );
    assert_eq!(
        published_meta(&store, 2).parent_revision_id,
        Some(1),
        "the third revision points at its predecessor"
    );
}

#[test]
fn stale_expected_parent_rejects_and_publication_key_replay_resolves() {
    let root = contract_root("rsv02-replay");
    let mut first = DurableReferenceStore::open(&root).expect("store opens");
    let snapshot = contract_snapshot("alpha");
    assert!(
        !first.publish(&snapshot, None).expect("first").replayed,
        "the first publication is not a replay"
    );

    let mut stale = DurableReferenceStore::open(&root).expect("second store opens");
    assert!(
        matches!(
            stale.publish(&contract_snapshot("beta"), None),
            Err(StoreError::ParentMismatch { .. })
        ),
        "a stale parent is refused"
    );

    let mut retry = DurableReferenceStore::open(&root).expect("retry store opens");
    let replay = retry.publish(&snapshot, None).expect("key replay");
    assert!(
        replay.replayed,
        "an exact retry resolves by publication key"
    );
    assert_eq!(
        replay.revision_id, 0,
        "the replay points at the committed row"
    );
    assert_eq!(
        retry.verify_full().expect("verification").revision_count,
        1,
        "replay inserted no duplicate"
    );
}

#[test]
fn rsv03_hashes_stable_across_restart_and_detect_tamper() {
    let root = contract_root("rsv03");
    let original = contract_snapshot("alpha");
    let before = {
        let mut store = DurableReferenceStore::open(&root).expect("store opens");
        store.publish(&original, None).expect("publish");
        published_meta(&store, 0)
    };
    let after = {
        let store = DurableReferenceStore::open(&root).expect("store reopens");
        published_meta(&store, 0)
    };
    assert_eq!(
        before.snapshot_hash, after.snapshot_hash,
        "semantic hash is stable across restart"
    );
    assert_eq!(
        before.payload_sha256, after.payload_sha256,
        "payload checksum is stable across restart"
    );

    let mut semantic_change = original.clone();
    semantic_change.instruments[0].name = "Changed Limited".to_owned();
    assert_ne!(
        semantic_change.snapshot_hash().expect("hash"),
        original.snapshot_hash().expect("hash"),
        "a semantic field change changes the semantic hash"
    );

    let mut metadata_change = original.clone();
    metadata_change.reconciliation = Some(Reconciliation::default());
    assert_eq!(
        metadata_change.snapshot_hash().expect("hash"),
        original.snapshot_hash().expect("hash"),
        "capture metadata is outside the semantic hash"
    );
    assert_ne!(
        india_reference::encode_payload(&metadata_change).expect("bytes"),
        india_reference::encode_payload(&original).expect("bytes"),
        "capture metadata still changes the payload checksum"
    );

    let store = DurableReferenceStore::open(&root).expect("store reopens");
    let connection = durable_connection_for_test(&store);
    connection
        .execute_batch("DROP TRIGGER reference_revision_no_update;")
        .expect("simulate on-disk corruption bypassing the app trigger");
    connection
        .execute(
            "UPDATE reference_revision SET payload = ?1 WHERE revision_id = 0",
            rusqlite::params![b"not-a-valid-envelope".to_vec()],
        )
        .expect("corrupt stored bytes");
    assert!(
        store.revision_with_meta(0).is_err(),
        "a flipped payload fails closed even with the checksum column intact"
    );
}

#[test]
fn reference_revisions_and_head_are_immutable() {
    let root = contract_root("immutable");
    let mut store = DurableReferenceStore::open(&root).expect("store opens");
    store
        .publish(&contract_snapshot("alpha"), None)
        .expect("publish");
    let connection = durable_connection_for_test(&store);
    assert!(
        connection
            .execute("UPDATE reference_revision SET valid_at = '2000-01-01'", [])
            .is_err(),
        "revision rows cannot be updated"
    );
    assert!(
        connection
            .execute("DELETE FROM reference_revision", [])
            .is_err(),
        "revision rows cannot be deleted"
    );
    assert!(
        connection
            .execute("DELETE FROM reference_head", [])
            .is_err(),
        "the head row cannot be deleted"
    );
    assert_eq!(
        store.verify_full().expect("verification").revision_count,
        1,
        "negative controls left the store intact"
    );
    assert_eq!(
        STORE_SCHEMA_VERSION, 1,
        "the contract schema version is pinned"
    );
}
