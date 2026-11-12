# Extras: reference ingestion, durable store and conformance probes

`extras/` is the promoted home for useful material from the predecessor
engineering-challenge project that is **not** part of the production Nirnay
graph. It is a separate Cargo workspace, is not imported by active Python code,
and is not linked into any runtime, broker or live-trading path.

It is deliberately *not* called "legacy": the code here is tested, licensed and
retained as a reference for future promotion, not as dead code. It remains
outside the active workspace so that the production build does not acquire
SQLite, filesystem or predecessor dependencies by accident.

## Package

- Cargo package: `nirnay-extras` (library crate `nirnay_extras`)
- Location: `extras/` (its own `[workspace]`)
- Depends on the active `crates/india-domain` by path
- Not a member of the root workspace

## What is retained

| Path | Purpose |
|---|---|
| `src/venue/india/**` | NSE/BSE reference-master ingestion, ISIN-first reconciliation, bitemporal snapshots, durable SQLite store, backup/restore and source archive. Unique predecessor logic not present in the active Rust crates. |
| `tests/india_reference_ingestion.rs` | Acceptance tests for the NSE/BSE parsers, calendars, tick schedules and reconciliation. |
| `tests/reference_store.rs` | Durable-store crash/restart, backup/restore and snapshot-hash verification suite. |
| `engineering/reference-store-contract.json` | Frozen verification groups referenced by the durable-store suite. |
| `tests/fixtures/`, `tests/golden/` | Synthetic order-book/numeric fixtures and the synthetic SBE golden capture. |
| `cpp/` | SBE decoder and Rust/C++ conformance probes. The shared fixed-point/order-book sources are taken from `verification/cpp-oracle/`. |
| `rust/` | Rust reference drivers for the conformance/SBE harness. These still target the archived monolith types and are not wired into this trimmed crate. |
| `scripts/conformance.py` | Fail-closed differential gate used by the original conformance harness. |
| `schemas/challenge/`, `schemas/internal/` | Challenge SBE schema and internal envelope contract. |
| `fixtures/india/` | Synthetic India reference-master fixtures. |
| `README.upstream.archive`, `UPSTREAM-ARCHIVE.txt`, `LICENSE` | Predecessor README, archive provenance and MIT licence. |
| `evidence/conformance_report.json` | Historical report showing the original gate did **not** pass (`gate_passed: false`). Do not cite the predecessor claim ledger as verified evidence. |

## What was removed

The following were removed from the active tree because they were auxiliary,
parallel to active code, or byte-identical duplicates:

- `crates/**` — parallel copies of the active `crates/*` (active crates are
  canonical; most files were SHA-256 identical).
- The predecessor monolith (`src/` outside `venue/india`), including collector,
  coordinator, session, journal, paper, replay, Kalshi and Polymarket adapters.
- `experimental/**`, `archive/**`, `claims/**`, `company/**`,
  `engineering/**` (except the reference-store contract), `measurements/**`,
  `q/**`, `benches/**`, `deploy/**`, `.github/**`, `xtask/**`.
- Duplicate schemas already present under active `schemas/`.
- Duplicate C++ fixed-point/order-book sources already in
  `verification/cpp-oracle/`.
- Non-India tests and prediction-market fixtures.

A complete pre-cleanup snapshot is preserved outside the runtime repository:

```text
../legacy-snapshot-2026-11-13/legacy-full-backup-2026-11-13.tar.gz
SHA-256: a49e864308a7e58d667a86049dd80e8ff2c631cd762e50a74aff60007464f9b1
```

The exact removed-file list is in
`../legacy-snapshot-2026-11-13/legacy-removed-manifest-2026-11-13.txt`.

## Running the extras

```text
cargo test --manifest-path extras/Cargo.toml
cmake -S extras/cpp -B /tmp/extras-cpp -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/extras-cpp --parallel
ctest --test-dir /tmp/extras-cpp --output-on-failure
```

Verified in the 2026-11-13 cleanup and again after the `extras/` promotion:
`cargo test` passed **96 tests** (5 unit, 42 ingestion, 49 durable-store)
against the active `india-domain`, and the C++ `ctest` passed **1/1**.

The C++ SBE test decodes the synthetic 4,096-byte golden capture. The golden is
valid but contains ten synthetic Kalshi `TEST-A`/`TEST-B` messages, not real
market data. The retained Rust drivers under `rust/` are reference material and
will not compile against this trimmed crate; port them to the active
`market-book`/decoder crates before use.

## Boundaries

- No provider, broker, Keychain or live account access is required or permitted.
- The fixtures are synthetic; do not restore captured provider payloads.
- The code is MIT-licensed (Copyright (c) 2026 Kartik Jangid). Keep `LICENSE`
  with any copy or promotion.
- Promotion of the India ingestion/store code into the active workspace requires
  a new crate, an architecture dependency edge, an owner and executable tests;
  it must not be imported merely because it exists here.
- `extras/` is excluded from the root Cargo workspace and from default Python
  test discovery. It is not required at runtime. A runtime-only distribution may
  omit it; if distributed, ship it with this README and the licence.
