# RSV coverage record — PR1 foundation (2026-11-19)

**Scope:** first promotion step for GAP-1. The frozen P1-04 source contract is
`extras/engineering/reference-store-contract.json` (status `frozen`,
RSV-01…RSV-12). This record describes what the **active** crate now covers and
what remains deferred. It does not change the frozen contract and does not
touch `extras/`.

## Implemented in this step

| Group | Active behavior | Executable evidence (`crates/india-reference/tests/durability.rs`) |
|---|---|---|
| RSV-01 snapshot persistence/validation | `snapshot.rs` versioned envelope, `validate()`, `encode_payload`/`decode_payload`; `DurableReferenceStore::publish/revision` stores and reopens the exact bytes | `rsv01_snapshot_persists_reopen_and_compares_all_fields`, `validation_rejects_noncanonical_and_unknown_fields` |
| RSV-02 revision identity | contiguous zero-based `revision_id`, numeric parent chain, singleton head, unique publication key, immutable revision/head triggers | `rsv02_revision_ids_contiguous_and_parent_chain_survive_restart`, `stale_expected_parent_rejects_and_publication_key_replay_resolves`, `reference_revisions_and_head_are_immutable` |
| RSV-03 hashes | semantic hash over exactly `{valid_at, instruments, calendar, listing_states}`; `payload_sha256` over the exact stored envelope bytes; decode order checksum → version → validate → semantic hash → metadata | `rsv03_hashes_stable_across_restart_and_detect_tamper` |

The exact publication key is
`sha256_hex(json!(["p1-04-publication-v1", expected_parent_or_null, schema_version, payload_sha256]))`.
An exact key replay is resolved before the parent fence so a retry of an
acknowledged delivery cannot be turned into a stale-parent error by its own
earlier success.

## Deferred groups (not claimed)

RSV-04 bitemporal query, RSV-05 listing lifecycle, RSV-06 lineage resolution,
RSV-07 process concurrency, RSV-08 crash boundaries, RSV-09 disk/filesystem
failures, RSV-10 corruption/torn-manifest handling, RSV-11 backup/restore and
RSV-12 performance baseline remain **uncovered by the active crate**. They are
planned as separate, independently testable PRs. No capability or claim record
is promoted by this step.

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

`extras/` remains the unchanged reference baseline. This step does not retire
or modify it.
