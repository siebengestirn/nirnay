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
use trading_core::{Isin, Venue};

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
    assert_eq!(store.head().unwrap().as_deref(), Some("digest-2"));
}

#[test]
fn backup_creates_a_reopenable_database() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reference.sqlite3");
    let mut store = ReferenceStore::open(&source).unwrap();
    store
        .resolve_identity(&Isin::new(ISIN_A).unwrap(), Venue::Nse, "AAA", "in.ref")
        .unwrap();
    let backup_dir = directory.path().join("backup");
    let files = store.backup(&backup_dir).unwrap();
    assert_eq!(files.len(), 1, "backup contains the database");
    assert!(files[0].is_file(), "backup database is a regular file");
    let reopened = ReferenceStore::open(&files[0]).unwrap();
    assert_eq!(reopened.head().unwrap(), None, "backup database is valid");
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
