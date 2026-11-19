# Fundamental-data domain evidence — 2026-11-19

**Code commit:** `05429b86c1f84a4874674f8712f1b7c0a6abec28` (the later
evidence/style commit does not change domain behavior).
**Scope:** a domain and offline validation vertical only. No source
acquisition, XBRL/PDF parsing, durable fundamental store, valuation, strategy
or trading is delivered or authorized by this record.

## Review blocker closure

| Finding | Closure |
|---|---|
| F1 point-in-time dependency closure | `select_point_in_time` propagates exclusions through revision and derivation edges; tests `point_in_time_propagates_revision_ineligibility` and `point_in_time_propagates_derived_input_ineligibility` fail under the old one-pass rule. |
| F2 historical CIN conflicts | `validate_effective_identities` is shared by current and historical selection; CIN claims use sort-and-sweep instead of pairwise scans. `historical_cin_collision_is_not_hidden_by_a_later_correction` covers the reported case. |
| F3 schema/Rust disagreement | Published schemas now constrain namespace URIs, decimal coefficients and JSON pointers; shared vectors under `research/fundamentals/contract-vectors/` are checked by `scripts/check_fundamental_contract_vectors.py` and by `shared_contract_vectors_match_the_rust_domain_validator`. |
| F4 opaque identity | Example company id is `co-00000001`; identity allocation is explicitly not claimed. |
| F5 release authorization | `.github/workflows/release.yml` is unchanged; no new release asset. CI validation only. |
| F6 fact semantics | ADR-FND-001 §10 records decisions on accuracy, nil facts, lexical values, taxonomy, units/dimensions and the derivation digest; `expected_derivation_sha256` recomputes and enforces the commitment. |

The supplied patch did not compile on this toolchain. Six compile errors were
fixed (ASCII predicate call, temporary borrows, missing `jiff` dev dependency)
and three broken tests were corrected before the semantic fixes.

## Verification

- `cargo check --workspace --all-targets --locked`: PASS.
- `cargo test --workspace --all-targets --locked`: PASS.
  - `fundamental-domain`: 20 passed.
  - `fundamental-tools` contract suite: 6 passed.
- `python3 scripts/check_fundamental_contract_vectors.py`: 8 vectors match
  schema expectations.
- `make check`: PASS, including Rust style, source style, docs, schemas,
  generated paths and 162 Python tests.

## Scope limits

Fixture digests are synthetic test values, not source evidence. A digest alone
does not prove publisher, completeness or entitlement. `ADR-FND-001` remains
Proposed for owner ratification; this implementation is a bounded vertical and
is not blanket M2 authorization.
