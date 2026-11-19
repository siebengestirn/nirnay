# RSV-08 active-store evidence — 2026-11-19

**Scope:** closes the four evidence gaps identified by the independent review of
`evidence/RSV-08-OWNER-DECISION-2026-11-19.md` on the **active**
`DurableReferenceStore`. It does **not** mark RSV-08 passed or superseded; the
owner decision remains pending. No publication journal was added and the frozen
contract was not edited.

## What changed in the active store

- `FailPoint` gains four active-store points: `ActiveDuringTransaction`,
  `ActiveAfterInsertBeforeCommit`, `ActiveBeforeCommit`, `ActiveAfterCommit`,
  plus `ActiveStorageFull`. Failpoints remain `#[cfg(debug_assertions)]` and
  thread-local; they are test seams, not a runtime feature.
- `DurableReferenceStore::publish` injects at each surviving boundary, and a
  commit outcome that cannot be proven poisons the handle. A poisoned handle
  refuses reads and writes with `StoreError::Poisoned`; reopening resolves the
  publication by key.
- `StoreError` gains `UncertainCommit`, `Poisoned` and `StorageFull` variants.
  A busy/locked commit remains a known rollback (`Sqlite`); other commit
  failures are classified as unknown.

## Executable evidence

All tests are in `crates/india-reference/tests/durability.rs`:

| Previously uncovered objective | Test | Result |
|---|---|---|
| Crash before commit of a real publication | `rsv08_active_crash_before_commit_leaves_no_publication` | injected failure before commit; after reopen head is `None` and revision count is 0 |
| Process termination during transaction / after insert before commit / before commit / after commit | `rsv08_active_process_termination_at_publication_boundaries` with child worker `child_active_publication_boundary` | real child process exits at each boundary; the first three leave no head and no revision; after-commit leaves exactly revision 0 |
| Uncertain commit | `rsv08_uncertain_commit_poisons_writer_and_reopen_resolves_by_key` | post-commit failure returns `UncertainCommit`; the handle refuses reads and writes; a fresh handle resolves the same key with `replayed: true` and one revision |
| Disk full / permission | `rsv08_full_disk_fails_closed_and_reopen_recovers` and `rsv08_readonly_path_fails_closed` | injected full-disk condition fails explicitly with no revision and a successful retry; an unwritable root fails closed |

## Honest scope of the fault model

- `ActiveStorageFull` is a **fault injection** simulating ENOSPC at the revision
  insert; it does not fill a physical disk. The read-only-path test uses real
  filesystem permissions on Unix.
- Process termination uses `std::process::exit` immediately after the injected
  boundary, which skips destructors as a crash would. It is not a SIGKILL test.
- The frozen contract's multi-stage crash points at archive object write and
  manifest append still do not exist in the active protocol. The old
  multi-stage crash tests are **not equivalent** to these transactional tests.

## Verification

`cargo test -p india-reference` — all tests pass (including the six new
RSV-08 tests and the pre-existing suite). `make check` PASS; source-style and
Rust-style PASS.

## Status

RSV-08 remains **not passed and not superseded**. This record supplies the
active evidence the owner conditioned a decision on; the owner must still
ratify option A or B. No capability or claim state is promoted here.
