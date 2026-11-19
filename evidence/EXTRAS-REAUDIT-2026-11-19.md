# `extras/` re-audit — 2026-11-19

**Scope:** fresh classification of retained `extras/` obligations after the SBE
GAP-2 promotion in PR #54. This is a status review, not a retirement decision.
No `extras/` file was edited or deleted.

## Classification

| Obligation | Status after GAP-2 | Evidence |
|---|---|---|
| Challenge SBE schema, decoder, golden corpus and licence | **Active home** | `crates/sbe-codec` with five differential tests; `evidence/SBE-GAP-2-PROMOTION-2026-11-19.md` |
| C++ SBE conformance probes (`extras/cpp`) | **Reference gate retained; obligation covered** | CTest `sbe_roundtrip` still runs in CI; Rust dump compared byte-for-byte with the frozen independent C++ dump |
| C++ numeric/book oracle | **Active verification target, partial** | `verification/cpp-oracle` is built and tested in CI; its README explicitly says it does not yet assert byte-for-byte equivalence with Rust |
| Rust book conformance driver (`extras/rust/conform_driver.rs`) | **Still retained, no active equivalent** | `crates/market-book` owns book operations but no Rust↔C++ byte-equivalence test exists |
| `extras/scripts/conformance.py` | **Still retained, no active equivalent** | no active gate script found in `scripts/` |
| NSE/BSE ingestion, CSV parsing, snapshot validation, durable store, archive and backup/restore | **Active home** | `crates/india-reference/src/source/{nse,bse}.rs`, `store.rs`, `archive.rs`, `snapshot.rs`; tests `ingestion.rs`, `durability.rs`, `source_conformance.rs` |
| Frozen reference-store contract and RSV-08 | **Retained; owner decision pending** | `extras/engineering/reference-store-contract.json` unchanged; `evidence/RSV-08-OWNER-DECISION-2026-11-19.md` reports the condition not confirmed; not passed or superseded |
| India helper modules (`calendar`, `tick_schedule`, `resolution`, `provenance`, `corporate_actions`, `eod_close`) | **Retained; active coverage not demonstrated** | present in `extras/src/venue/india/`; no same-named active module in `crates/india-reference` |
| Temporal wire goldens (`extras/tests/temporal_compat.rs`) | **Still retained, no active equivalent** | no active chrono-to-jiff golden test found |
| Synthetic India fixtures (`extras/fixtures/india/`) | **Retained** | no active test reads these files; no demonstrated active home |
| MIT licence and external snapshot provenance | **Preserved** | `extras/LICENSE`; SBE copy travels with `crates/sbe-codec/LICENSE`; external snapshot hash recorded in `engineering/extras-audit.md` |

## Result

`extras/` retirement remains **BLOCKED**. The SBE obligation now has an active
home and independent differential evidence, which closes GAP-2. It does not
close the remaining blockers:

1. RSV-08 is an unresolved owner decision; the independent review did not
   confirm the replacement's condition and no capability was superseded.
2. The C++ numeric/book oracle is an independent executable but has no
   byte-for-byte differential assertion against active Rust book code.
3. The book conformance driver, `scripts/conformance.py`, several India helper
   modules, temporal goldens and synthetic fixtures have no demonstrated active
   home or explicit abandonment decision.
4. No owner has approved retirement, and the frozen contract and licence must
   remain retained in any case.

## Boundaries

No deletion, no `extras/` edit, no contract change, no capability or claim
promotion, no live path and no trading work in this re-audit. Evidence records
describe their own scopes; a passing CI run on one commit is not a claim about
another commit.
