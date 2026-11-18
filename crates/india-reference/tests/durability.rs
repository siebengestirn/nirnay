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
        .publish_revision(None, "digest-1", Path::new("revision-1.json"), "key-1")
        .unwrap();
    assert!(!first.deduplicated, "first publication is new");
    let repeat = store
        .publish_revision(None, "digest-1", Path::new("revision-1.json"), "key-1")
        .unwrap();
    assert!(repeat.deduplicated, "same key and digest is idempotent");
    assert!(
        matches!(
            store.publish_revision(Some("wrong"), "digest-2", Path::new("r2"), "key-2"),
            Err(StoreError::ParentMismatch { .. })
        ),
        "wrong parent fails closed"
    );
    let second = store
        .publish_revision(Some("digest-1"), "digest-2", Path::new("r2"), "key-2")
        .unwrap();
    assert!(
        second.sequence > first.sequence,
        "revision sequence advances"
    );
    assert_eq!(
        store.head().unwrap().as_deref(),
        Some("digest-2"),
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
        .publish_revision(None, "digest-1", Path::new("revision-1.json"), "key-1")
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
        Some("digest-1"),
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
        .publish_revision(None, "digest-1", Path::new("r1"), "key-1")
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
                .publish_revision(None, "digest-a", Path::new("a"), "key-a")
                .map(|_| ())
                .map_err(|e| format!("{e}"))
        }
    });
    let second = std::thread::spawn(move || {
        let mut store = ReferenceStore::open(&path).unwrap();
        store
            .publish_revision(None, "digest-b", Path::new("b"), "key-b")
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
