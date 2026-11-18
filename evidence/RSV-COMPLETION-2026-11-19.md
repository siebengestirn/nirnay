# RSV completion record — PR3 and PR4 (2026-11-19)

**Scope:** third promotion step for GAP-1. Active crate `india-reference`,
base `0c05641`. The frozen P1-04 source contract is
`extras/engineering/reference-store-contract.json`. `extras/` is unchanged.

## Covered in this step

| Requirement | Active behavior | Test |
|---|---|---|
| RSV-07 concurrency | Competing OS processes publish one revision: the parent fence and unique publication key resolve the loser. Same-key concurrent retries resolve by key without a duplicate row. | `rsv07_competing_processes_publish_once_and_replay_is_idempotent` |
| RSV-09 filesystem | Bounded `fs2` shared/exclusive lock per archive with explicit `LockBusy`; writable open repairs a torn tail under the exclusive lock; read-only open never repairs. Missing or unwritable paths return explicit I/O errors, never false success. | `rsv09_lock_contention_times_out_without_mutation`, `rsv09_torn_tail_is_recovered_only_by_writable_open` |
| RSV-10 corruption | Conflicting duplicate digest, malformed complete manifest line, corrupted object and missing source all fail closed; unindexed objects are reported and never served. | `rsv10_corruption_and_duplicate_digest_fail_closed` |

## RSV-08 — explicit contract divergence (NOT passed, NOT superseded)

- **Frozen contract requirement:** kill a writer at each publication boundary
  (before archive object write, after object rename, after manifest append,
  in transaction, after insert before commit, after commit before return) and
  verify recovery at each.
- **Active semantics:** archive objects are written earlier by `put_source`;
  `publish` only resolves lineage and commits one SQLite `BEGIN IMMEDIATE`
  transaction containing the immutable revision and head. There is no archive
  object write or manifest append inside `publish`, so the contract's first
  three boundaries do not exist in this implementation.
- **Why no journal was added:** a publication journal would change the active
  storage protocol without evidence that it improves its guarantees. The
  owner explicitly deferred that decision; no new on-disk format was introduced.
- **Replacement invariants with executable evidence:**
  - crash before SQLite commit leaves no revision — existing
    `crash_before_commit_leaves_no_identity` (legacy ledger) and the checked
    `BEGIN IMMEDIATE` transaction in `DurableReferenceStore::publish`;
  - a successful commit survives restart —
    `rsv01_snapshot_persists_reopen_and_compares_all_fields`;
  - a persisted source with no publication is an unreferenced archive object,
    not a revision — `rsv08_replacement_orphan_source_is_not_a_publication`;
  - retry after an acknowledged/unknown outcome resolves by key —
    `stale_expected_parent_rejects_and_publication_key_replay_resolves`;
  - concurrent publishers cannot create conflicting roots — RSV-07 test above;
  - archive corruption is detected before use — RSV-10 test above.
- **Remaining divergence:** the frozen contract's multi-step crash points are
  not reproduced, and no active test kills a process at the removed boundaries.
  **An owner decision is required** to formally supersede RSV-08 or to require
  a Nirnay-owned crash-boundary protocol. This record does not make that
  decision.

## PR4 addition — RSV-11 backup/restore

`backup_verified` first runs `verify_full`, copies the database with `VACUUM
INTO`, copies the referenced archive objects, writes a versioned `backup.json`
with database and archive-manifest hashes, syncs files and directories, then
renames the staging directory into `<root>/backups/<id>`. `restore_verified_store`
checks versions and both hashes, verifies the staged store (revisions, parent
chain, head and source lineage), checks the declared head, and only then swaps
the staged tree into the target with rollback of any previous target.

| Requirement | Test |
|---|---|
| Restore verifies every revision, head, parent chain and source reference | `rsv11_restore_verifies_revisions_head_chain_and_sources` |
| Tampered database or wrong declared head fails closed with no target | `rsv11_restore_fails_closed_without_publishing_a_target` |

## RSV-12 measurement-only baseline

`evidence/RSV-PERFORMANCE-BASELINE-2026-11-19.md` records the first dated
baseline from an ignored, measurement-only test: dataset hash, encoded payload
size, write/reopen/query times and directory growth, with the exact command and
raw transcript hash. There is no regression threshold, throughput certificate
or capability claim.

## Still open

RSV-08 remains an unresolved owner decision as recorded above. No capability or
claim record is promoted by any promotion step.
