#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.7 operating-environment tests. All paths are temporary local
//! directories; no real deployment or Linux host is involved.

use std::{
    fs, //
    path::{Path, PathBuf},
};

use ops_runtime::{
    BackupManifest, //
    HealthSnapshot,
    OpsError,
    RuntimeDir,
    RuntimeLayout,
    RuntimeThresholds,
    create_backup,
    restore_backup,
    verify_backup,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn thresholds() -> RuntimeThresholds {
    RuntimeThresholds {
        max_feed_age_ns: 1_500_000_000,
        max_journal_append_latency_ns: 10_000_000,
        max_journal_queue_depth: 100,
        min_disk_available_bytes: 1_000_000,
        max_unknown_orders: 0,
        max_reconciliation_differences: 0,
        max_sequence_gaps: 0,
    }
}

fn healthy() -> HealthSnapshot {
    HealthSnapshot {
        feed_age_ns: 100_000,
        sequence_gaps: 0,
        reconnect_count: 0,
        journal_append_latency_ns: 1_000,
        journal_queue_depth: 0,
        disk_available_bytes: 10_000_000,
        open_orders: 1,
        unknown_orders: 0,
        reconciliation_differences: 0,
        kill_switch_engaged: false,
        account_snapshot_available: true,
    }
}

fn layout_in(temp: &tempfile::TempDir) -> Result<RuntimeLayout, Box<dyn std::error::Error>> {
    let layout = RuntimeLayout::new(temp.path().join("nirnay-runtime"))?;
    layout.create_all()?;
    Ok(layout)
}

fn evidence_file(
    temp: &tempfile::TempDir,
    name: &str,
    content: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = temp.path().join(name);
    fs::write(&path, content)?;
    Ok(path)
}

#[test]
fn layout_creates_and_validates_all_state_directories() -> TestResult {
    let temp = tempfile::tempdir()?;
    let layout = layout_in(&temp)?;
    layout.validate()?;
    for directory in RuntimeDir::ALL {
        assert!(
            layout.path(directory).is_dir(),
            "runtime state directory exists"
        );
    }
    Ok(())
}

#[test]
fn layout_refuses_a_root_inside_the_source_tree() -> TestResult {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or("no workspace root")?;
    assert!(
        matches!(
            RuntimeLayout::new(source),
            Err(OpsError::RootInsideSourceTree)
        ),
        "runtime evidence may not be written into the source tree"
    );
    Ok(())
}

#[test]
fn backup_manifest_records_a_digest_per_entry() -> TestResult {
    let temp = tempfile::tempdir()?;
    let layout = layout_in(&temp)?;
    let source = evidence_file(&temp, "journal.jsonl", "{\"seq\":1}\n")?;
    let dir = create_backup(&layout, &[source], "backup-1", 42)?;
    let manifest: BackupManifest = serde_json::from_slice(&fs::read(dir.join("manifest.json"))?)?;
    assert_eq!(manifest.version, 1, "manifest version is recorded");
    assert_eq!(manifest.entries.len(), 1, "one entry is recorded");
    assert_eq!(
        manifest.entries[0].sha256.len(),
        64,
        "a SHA-256 digest is recorded"
    );
    assert_eq!(manifest.created_at_ns, 42, "creation time is recorded");
    Ok(())
}

#[test]
fn verified_backup_restores_to_a_fresh_path() -> TestResult {
    let temp = tempfile::tempdir()?;
    let layout = layout_in(&temp)?;
    let source = evidence_file(&temp, "journal.jsonl", "{\"seq\":1}\n")?;
    let dir = create_backup(&layout, &[source], "backup-2", 43)?;
    verify_backup(&dir)?;
    let target = temp.path().join("restored-fresh");
    restore_backup(&dir, &target)?;
    assert_eq!(
        fs::read_to_string(target.join("journal.jsonl"))?,
        "{\"seq\":1}\n",
        "restored bytes are identical"
    );
    Ok(())
}

#[test]
fn tampered_backup_is_refused() -> TestResult {
    let temp = tempfile::tempdir()?;
    let layout = layout_in(&temp)?;
    let source = evidence_file(&temp, "journal.jsonl", "original")?;
    let dir = create_backup(&layout, &[source], "backup-3", 44)?;
    fs::write(dir.join("journal.jsonl"), "tampered")?;
    assert!(
        matches!(verify_backup(&dir), Err(OpsError::DigestMismatch(_))),
        "a modified backup entry is detected"
    );
    let target = temp.path().join("restored-tampered");
    assert!(
        restore_backup(&dir, &target).is_err(),
        "a tampered backup is never restored"
    );
    Ok(())
}

#[test]
fn restore_refuses_an_existing_target() -> TestResult {
    let temp = tempfile::tempdir()?;
    let layout = layout_in(&temp)?;
    let source = evidence_file(&temp, "journal.jsonl", "data")?;
    let dir = create_backup(&layout, &[source], "backup-4", 45)?;
    let target = temp.path().join("already-there");
    fs::create_dir(&target)?;
    assert!(
        matches!(
            restore_backup(&dir, &target),
            Err(OpsError::TargetExists(_))
        ),
        "restore never overwrites an existing target"
    );
    Ok(())
}

#[test]
fn backup_publication_is_exclusive() -> TestResult {
    let temp = tempfile::tempdir()?;
    let layout = layout_in(&temp)?;
    let source = evidence_file(&temp, "journal.jsonl", "data")?;
    create_backup(&layout, std::slice::from_ref(&source), "backup-5", 46)?;
    assert!(
        matches!(
            create_backup(&layout, &[source], "backup-5", 47),
            Err(OpsError::TargetExists(_))
        ),
        "a second backup with the same name is refused"
    );
    Ok(())
}

#[test]
fn fresh_snapshot_permits_new_risk() -> TestResult {
    let verdict = healthy().evaluate(&thresholds());
    assert!(
        verdict.safe_for_new_risk,
        "a healthy runtime permits new risk"
    );
    assert!(verdict.reasons.is_empty(), "there are no blocking reasons");
    Ok(())
}

#[test]
fn stale_feed_blocks_new_risk() -> TestResult {
    let mut snapshot = healthy();
    snapshot.feed_age_ns = thresholds().max_feed_age_ns + 1;
    let verdict = snapshot.evaluate(&thresholds());
    assert!(!verdict.safe_for_new_risk, "a stale feed blocks new risk");
    assert!(
        verdict
            .reasons
            .iter()
            .any(|reason| reason.contains("stale")),
        "the stale-feed reason is reported"
    );
    Ok(())
}

#[test]
fn sequence_gaps_block_new_risk() -> TestResult {
    let mut snapshot = healthy();
    snapshot.sequence_gaps = 1;
    assert!(
        !snapshot.evaluate(&thresholds()).safe_for_new_risk,
        "unresolved gaps block new risk"
    );
    Ok(())
}

#[test]
fn unknown_orders_block_new_risk() -> TestResult {
    let mut snapshot = healthy();
    snapshot.unknown_orders = 1;
    assert!(
        !snapshot.evaluate(&thresholds()).safe_for_new_risk,
        "UNKNOWN orders block new risk"
    );
    Ok(())
}

#[test]
fn reconciliation_differences_block_new_risk() -> TestResult {
    let mut snapshot = healthy();
    snapshot.reconciliation_differences = 1;
    assert!(
        !snapshot.evaluate(&thresholds()).safe_for_new_risk,
        "account divergence blocks new risk"
    );
    Ok(())
}

#[test]
fn disk_pressure_and_journal_backlog_block_new_risk() -> TestResult {
    let mut snapshot = healthy();
    snapshot.disk_available_bytes = thresholds().min_disk_available_bytes - 1;
    assert!(
        !snapshot.evaluate(&thresholds()).safe_for_new_risk,
        "low disk blocks new risk"
    );
    let mut snapshot = healthy();
    snapshot.journal_queue_depth = thresholds().max_journal_queue_depth + 1;
    assert!(
        !snapshot.evaluate(&thresholds()).safe_for_new_risk,
        "queue backlog blocks new risk"
    );
    Ok(())
}

#[test]
fn kill_switch_and_missing_snapshot_block_new_risk() -> TestResult {
    let mut snapshot = healthy();
    snapshot.kill_switch_engaged = true;
    assert!(
        !snapshot.evaluate(&thresholds()).safe_for_new_risk,
        "kill switch blocks new risk"
    );
    let mut snapshot = healthy();
    snapshot.account_snapshot_available = false;
    assert!(
        !snapshot.evaluate(&thresholds()).safe_for_new_risk,
        "missing snapshot blocks new risk"
    );
    Ok(())
}

#[test]
fn metrics_text_exposes_the_operating_signals() -> TestResult {
    let text = healthy().metrics_text();
    for name in [
        "nirnay_feed_age_ns",
        "nirnay_sequence_gaps",
        "nirnay_journal_append_latency_ns",
        "nirnay_disk_available_bytes",
        "nirnay_unknown_orders",
        "nirnay_reconciliation_differences",
        "nirnay_kill_switch_engaged",
    ] {
        assert!(text.contains(name), "metrics expose {name}");
    }
    Ok(())
}

/// A caller-controlled backup name cannot escape the backup directory.
#[test]
fn unsafe_backup_names_are_refused() -> TestResult {
    let temp = tempfile::tempdir()?;
    let layout = layout_in(&temp)?;
    let source = evidence_file(&temp, "journal.jsonl", "data")?;
    for name in ["../escape", "/absolute", "a/b", "..", ""] {
        assert!(
            create_backup(&layout, std::slice::from_ref(&source), name, 48).is_err(),
            "an escaping backup name is refused"
        );
    }
    assert!(
        create_backup(&layout, std::slice::from_ref(&source), "safe-name_1", 49).is_ok(),
        "a safe backup name is accepted"
    );
    Ok(())
}

#[test]
fn metrics_expose_account_snapshot_availability() -> TestResult {
    let text = healthy().metrics_text();
    assert!(
        text.contains("nirnay_account_snapshot_available 1"),
        "account snapshot availability is exported as a metric"
    );
    Ok(())
}
