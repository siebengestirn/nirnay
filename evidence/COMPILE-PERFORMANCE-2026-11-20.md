# Compile-performance and workspace hygiene — 2026-11-20

## Scope

This record covers the local-only retirement of the predecessor archive, the
removal of Kalshi/Polymarket material from the active workspace, and measured
compile-cost reductions. It makes no live-trading or real-source claim.

## Structural changes

- The predecessor workspace moved from tracked `extras/` to untracked
  `.local/extras/`. Makefile, CI, nightly CI, schema validation and all active
  path references were updated; no build or CI job compiles the archive.
  `scripts/validate_contracts.py` now excludes `.local` from discovery.
- Kalshi/Polymarket prediction-market types (`PriceRange`, `MarketInfo`,
  `PriceBand`, `NumericSpec`, `Ratio`, price/quantity units and two legacy
  conversion helpers) were removed from `market-types`. They had no non-test
  caller; the removal is limited to this dead prediction-market cluster. Their
  provenance remains in the archived workspace. A separate ADR classifies a
  different legacy event-metadata group as non-authoritative scaffolding; it is
  not the rationale for this removal.
- The synthetic SBE golden was re-symbolized in place from `kalshi:TEST-A/B`
  to `nirnay:TEST-A/B`. Its 4096-byte structure and message semantics are
  unchanged. The independent C++ dump was regenerated from the archived oracle
  and both digests were updated. Historical evidence records retain their
  original digests with a dated amendment note.
- All remaining `engineering_challenge` / `engineering-challenge` strings in
  operator-local archives were renamed to `nirnay`.

## Measurements

Measured on this host with private target directories (cold builds):

| Phase | Before | After |
|---|---:|---:|
| `make check` (warm workspace) | 35 s | 24 s |
| Debug target size | 2.7 GiB | 1.78 GiB |
| Contract schema discovery | 1099 files | 59 files |

Cold `cargo check` and test-build timings were measured repeatedly; results
varied between 19 s and 41 s on an otherwise idle host, so no speed claim is
made for those phases. The deterministic wins are workspace scope, target size
and schema discovery cost.

The `make check` and target-size reductions come from removing the archived
workspace from every build and from emitting line-table debug information in
development/test profiles instead of full DWARF. Source-line backtraces remain
available; release quality is unchanged (`lto = "thin"`, one codegen unit,
abort on panic).

Compile cost is now enforced by `make perf` and a parallel CI job:
`scripts/check_compile_budget.py` performs cold `check` and test-build phases
in a private target directory and fails if check exceeds 120 s, test build
exceeds 180 s, or the target exceeds 6 GiB.

## Boundary

These are build-engineering changes only. No trading logic, contract, policy
or M2 behaviour was changed. Historical evidence files are preserved with
explicit amendment notes rather than rewritten.
