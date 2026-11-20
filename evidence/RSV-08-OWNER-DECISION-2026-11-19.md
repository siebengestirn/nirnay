# RSV-08 owner decision record — 2026-11-19

**Status: PREPARED — owner ratification pending.** RSV-08 is **not passed** and
**not superseded** by this record. No publication journal was implemented and no
capability or claim state is promoted here.

## 1. Decision requested

The frozen P1-04 storage contract
`.local/extras/engineering/reference-store-contract.json` (`status: frozen`,
`release_gate: P1-04`, baseline `12f4b27`) defines RSV-08 as: "Kill a writer at
each publication boundary: before archive, after archive, during the
transaction, after insert before commit and after commit before return." The
active `crates/india-reference` implementation does not reproduce those
boundaries. The owner must choose:

- **Option A — retain the frozen contract.** Implement a Nirnay-owned
  publication journal and process-kill tests at every frozen boundary.
- **Option B — supersede the frozen contract.** Accept the active SQLite
  transaction plus immutable-revision model as the publication protocol, with
  explicit tests for every replacement invariant.

## 2. The divergence (source-grounded)

- The active publication method is `DurableReferenceStore::publish`
  (`crates/india-reference/src/store.rs:803`). It validates, resolves lineage,
  derives the publication key, and commits the immutable revision row and the
  singleton head in **one** `BEGIN IMMEDIATE` transaction
  (`TransactionBehavior::Immediate`, `store.rs:819–821`; revision insert
  `883–896`; head insert/update `897–907`; commit `store.rs:908`). The earlier
  `ReferenceStore::publish_revision` (`store.rs:292`) is the **legacy** store: it
  writes no head row and is not the active protocol.
- Raw source bytes are archived earlier by `put_source` (`archive.rs:201`).
  `publish` only calls `resolve_lineage` (`store.rs:789–799`), which performs a
  read/resolve check (`archive.resolve_with_timeout`). There is no archive
  object write and no manifest append inside `publish`, so the frozen "before
  archive" and "after archive" kill points do not exist in the active protocol.
- The surviving boundaries are governed by SQLite atomicity, **not by active
  executable crash tests**. `FailPoint` has three variants — `BeforeCommit`,
  `AfterCommit`, `RestoreSync` (`store.rs:68–73`) — with injection sites only
  at `285/287`, `361/363` and `551`, all in the legacy store. Every process-kill
  test in `tests/durability.rs` targets `ReferenceStore`; none kills a writer
  inside `DurableReferenceStore::publish`.

## 3. Objectives and executable evidence (corrected)

The objective is the thing the frozen contract protects
(`durability_promise`, `publication.atomicity`, `uncertain_commit`,
`source_archive.orphan_rule`, `failure_semantics`, `backup_restore`). The
mapping below distinguishes **active executable evidence** from mechanism-only
guarantees and from legacy-store evidence.

| Objective | Active evidence | Verdict |
|---|---|---|
| A successful commit is visible after reopen | `rsv01_snapshot_persists_reopen_and_compares_all_fields` (`tests/durability.rs:773`); head after reopen in `rsv02` (846) | **Covered (active)** |
| Published revisions are immutable; a correction appends | `reference_revisions_and_head_are_immutable` (987); contract triggers at `store.rs:615–620`; multi-revision parent chain in `rsv02`/`rsv11` | **Covered (active)** |
| A stale expected parent cannot overwrite newer knowledge | parent check in `publish` (`store.rs:875–881`); `stale_expected_parent_rejects_and_publication_key_replay_resolves` (891) | **Covered (active)** |
| Concurrent publishers cannot create conflicting roots | `rsv07_competing_processes_publish_once_and_replay_is_idempotent` (1330) | **Covered (active)** |
| An archived source with no publication is residue, not a revision | `rsv08_replacement_orphan_source_is_not_a_publication` (1494) | **Covered (active)** |
| Archive corruption and torn tails fail closed or recover by truncation | `rsv09_torn_tail_is_recovered_only_by_writable_open` (1398); `rsv10_corruption_and_duplicate_digest_fail_closed` (1432); content hash in `source_archive_verifies_content_and_refuses_torn_manifest` (37); missing source in `rsv06_lineage_resolves_after_restart_and_missing_source_fails_closed` (1200) | **Covered in aggregate** |
| Backup and restore verify revisions, head, parent chain and sources | `rsv11_restore_verifies_revisions_head_chain_and_sources` (1526); `rsv11_restore_fails_closed_without_publishing_a_target` (1586) | **Covered (active)** |
| Retry of the **same** publication key is idempotent | exact-key replay portion of `stale_expected_parent…` (891) | **Covered (active)** |
| A crash before commit of a real publication publishes neither revision nor head | none active; `crash_before_commit_leaves_no_identity` (213) tests **legacy identity-counter rollback**, not publication | **NOT covered** — mechanism only (single Immediate transaction) |
| Process kill during transaction / after insert before commit / after commit before return on the active store | none; all failpoints and kill workers are legacy-only | **NOT covered** |
| Uncertain commit: poison writer, close without success, reopen, resolve by publication key, fail closed on mismatched content | none; successful exact-key replay (891) is a different scenario. No poison handling exists in the active store | **NOT covered — not implemented** |
| Disk-full / permission failure → no false success and uncertain commit resolved after reopen | `rsv09_lock_contention_times_out_without_mutation` (1368) covers lock contention only; the rest is a dated record, not active executable evidence | **Partially covered** |

## 4. Required acknowledgement — tests are not equivalent

The frozen multi-stage crash tests and the new transactional tests are **not
equivalent** and must not be treated as interchangeable:

- The old procedure kills a process at five protocol boundaries, including
  archive object write and manifest append. The active protocol performs no
  archive write inside publication, so two of those boundaries do not exist.
- The new procedure exercises recovery around a single SQLite commit, and a
  retry of the same publication key resolves by that key. It does **not**
  implement the frozen uncertain-commit protocol (poison handle, close without
  success, reopen and resolve); see sections 3 and 5.
- The equivalence being claimed is at the **objective** level — no partial
  publication, committed state survives restart, retry resolves by key — not
  at the **procedure** level. Killing a writer at a removed boundary is not a
  test the active implementation can run.
- Section 3 explicitly marks which objectives lack active executable evidence.
  A mechanism-only guarantee is **not** presented as test coverage; in
  particular the legacy identity-crash test is not evidence that a publication
  crash leaves no revision, and exact-key replay is not evidence of uncertain
  commit resolution.
- RSV-08 must not be marked `passed` by relabelling transactional tests as the
  frozen crash-boundary tests, and must not be marked `superseded` until the
  owner signs option B.

## 5. Independent review outcome and engineering recommendation

Independent review (`.local/nirnay-review-archive/
rsv08-owner-decision-independent-review.md`, 2026-11-19) returned
**CONDITION NOT CONFIRMED**: the active store has genuine atomicity and several
covered objectives, but the crash-before-commit, active-store kill boundaries,
uncertain-commit handling, and disk/permission-failure objectives have no active
executable evidence.

Engineering recommendation: **defer ratification of option B**. Option B remains
the preferred direction, but it is not ratifiable on current evidence. Before
signing B the owner should either require active executable evidence for the
four uncovered objectives above, or explicitly accept them as residual risks
with the mechanism-only justification recorded. Closing that gap is not a
publication-journal implementation and does not change the frozen contract; it
is active tests and handling on `DurableReferenceStore`. This recommendation is
not a decision and does not authorize any code change.

## 6. Scope and exclusions

- No publication-journal implementation, ADR change or frozen-contract edit.
- No `capabilities.json`, `claims/ledger.json` or `README.md` capability update.
- No `.local/extras/` change, no live path, no broker or trading integration.
- This record is an owner decision pack, not evidence that RSV-08 passed.

## 7. Owner decision (to be completed by the owner)

- [ ] Option A — retain the frozen contract and implement the journal and
      multi-stage crash tests.
- [ ] Option B — supersede the frozen contract and accept SQLite transaction
      atomicity plus immutable revisions. **Condition not yet confirmed by
      independent review** (see section 5); signing B requires either the
      missing active evidence or an explicit residual-risk acceptance.

Owner signature: **pending**. Decision date: **pending**.
