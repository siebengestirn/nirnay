# RSV coverage record — PR1 + PR2 (2026-11-19)

**Scope:** first two promotion steps for GAP-1. The frozen P1-04 source contract is
`extras/engineering/reference-store-contract.json` (status `frozen`,
RSV-01…RSV-12). This record describes what the **active** crate now covers and
what remains deferred. It does not change the frozen contract and does not
touch `extras/`.

## Implemented in this step

All test paths below are relative to `crates/india-reference/tests/durability.rs`.

- **RSV-01 snapshot persistence/validation.** `snapshot.rs` provides a versioned
  envelope, `validate()`, `encode_payload` and `decode_payload`;
  `DurableReferenceStore::publish/revision` stores and reopens the exact bytes.
  Tests: `rsv01_snapshot_persists_reopen_and_compares_all_fields`,
  `validation_rejects_noncanonical_and_unknown_fields`.
- **RSV-02 revision identity.** Contiguous zero-based `revision_id`, numeric
  parent chain, singleton head, unique publication key, immutable revision/head
  triggers. Tests:
  `rsv02_revision_ids_contiguous_and_parent_chain_survive_restart`,
  `stale_expected_parent_rejects_and_publication_key_replay_resolves`,
  `reference_revisions_and_head_are_immutable`.
- **RSV-03 hashes.** Semantic hash over exactly `{valid_at, instruments,
  calendar, listing_states}`; `payload_sha256` over the exact stored envelope
  bytes; decode order checksum → version → validate → semantic hash → metadata.
  Test: `rsv03_hashes_stable_across_restart_and_detect_tamper`.
- **RSV-04 bitemporal query.** `latest_states_at` and
  `DurableReferenceStore::reference(t_valid, t_recorded)` resolve against the
  HEAD snapshot only; older revisions are never concatenated. Knowledge at
  `t_recorded` supersedes an earlier open interval before coverage at
  `t_valid` is applied. Test: `rsv04_bitemporal_identical_across_restart`.
- **RSV-05 listing lifecycle.** Corrections append new observation records;
  a closed interval cannot resurrect a stale open record. Test:
  `rsv05_history_appends_without_duplication_or_stale_resurrection`.
- **RSV-06 source lineage.** `collect_lineage_hashes` gathers top-level and
  embedded provenance plus tick reference/schedule/calendar hashes;
  `SourceArchive::fetch/resolve` require a manifest entry and verify object
  bytes. Every hash resolves before a revision is written and on every
  `verify_full`. Tests:
  `rsv06_lineage_resolves_after_restart_and_missing_source_fails_closed` and
  `rsv06_tick_lineage_hashes_resolve_and_fail_closed`, which archives three
  distinct tick hashes and exercises every hash branch.

The exact publication key is `sha256_hex` of a JSON array containing the
marker `p1-04-publication-v1`, the expected parent (or null), the payload
schema version and `payload_sha256`.
An exact key replay is resolved before the parent fence so a retry of an
acknowledged delivery cannot be turned into a stale-parent error by its own
earlier success.

## Deferred groups (not claimed)

RSV-07 concurrency and RSV-09/RSV-10 filesystem and corruption handling are
covered by PR3; see `evidence/RSV-COMPLETION-2026-11-19.md`. RSV-08 crash
boundaries are **not passed**: the frozen protocol's archive-write and
manifest-append boundaries do not exist in the active design, and an owner
decision is required to supersede them. RSV-11 is covered by PR4 (see
`evidence/RSV-COMPLETION-2026-11-19.md`). RSV-12 has a dated measurement-only
baseline in `evidence/RSV-PERFORMANCE-BASELINE-2026-11-19.md`, with no threshold
or capability claim. No capability or claim record is promoted by these steps.

## Nirnay-owned decisions (not copied from the predecessor)

1. The contract tables (`reference_revision`, `reference_head`) are **additive**;
   the existing opaque-identity ledger and its API are unchanged.
2. Snapshots carry active `InstrumentId`s allocated by `resolve_identity*`;
   predecessor `bootstrap_id` derivation is never imported.
3. Strict durability-pragma read-back applies to `DurableReferenceStore::open`
   only; the legacy `ReferenceStore::open` is unchanged.
4. SQLite stores revision ids as `i64` with explicit checked conversion to the
   contract's non-negative `u64` identity; no unchecked casts.
5. Semantic-hash byte identity is fixed by Nirnay's v1 encoder
   (`serde_json::json!` construction, jiff canonical timestamp strings).
   `serde_json` key-order-preserving mode is not enabled.
6. Failure-injection and raw-connection access are `#[cfg(debug_assertions)]`
   test seams; the negative-control in RSV-03 drops the SQL immutability
   trigger through that seam to simulate on-disk corruption.
7. The source archive is opened or created under `<root>/archive` by
   `DurableReferenceStore::open`; the legacy `SourceArchive` API is unchanged.
8. `reference()` is HEAD-only. A correction supersedes the record it replaces;
   revisions are never concatenated into a query result.
9. A missing or hash-wrong archived object blocks publication before any
   revision row is written and also fails full verification after restart.

`extras/` remains the unchanged reference baseline. This step does not retire
or modify it.
