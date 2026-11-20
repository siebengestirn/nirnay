# `.local/extras/` retirement decision — 2026-11-19

**Status: RETIREMENT REMAINS BLOCKED (owner-confirmed).** `.local/extras/` is retained
with its provenance and licence. No file was deleted.

## Decision

The owner confirmed the retirement block after the SBE GAP-2 promotion and the
corrected re-audit. Retirement is not authorized while the residual obligations
lack an active home or an explicit abandonment decision.

## What now has an active home

- Challenge SBE schema, decoder, golden corpus and licence → `crates/sbe-codec`,
  with independent differential evidence (PR #54).
- NSE/BSE ingestion, snapshot validation, durable store, archive and
  backup/restore → `crates/india-reference`.
- Shared temporal goldens and India fixtures → `crates/india-domain` tests and
  root `fixtures/india/`; provenance and corporate-action modules are active.

## Residual obligations that keep retirement blocked

- Rust book conformance driver (`.local/extras/rust/conform_driver.rs`): note the
  active book differential added in PR #59 uses new drivers in
  `crates/market-book` and `verification/cpp-oracle`; the predecessor driver
  itself has not been promoted or abandoned.
- `.local/extras/scripts/conformance.py`: no active equivalent.
- India helper modules `calendar`, `tick_schedule`, `resolution`, `eod_close`:
  no demonstrated active home.
- Predecessor-only `backup_filename_matches_chrono_golden` test: no active
  equivalent.
- Frozen reference-store contract and MIT licence: retained in any case.

## Scope

Decision record only. No deletion, no `.local/extras/` edit, no capability or claim
change, no live path or trading component. The earlier re-audit
(`evidence/EXTRAS-REAUDIT-2026-11-19.md`) remains the current classification;
this record adds the owner's retirement disposition.

Owner signature: **recorded in this session** (owner-directed keep-blocked
decision). Decision date: 2026-11-19.
