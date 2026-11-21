# External-review remediation programme — 2026-11-21

Dated evidence for the work performed in response to the 21 November technical
review (`Task.md`, untracked owner input). This record is immutable. All figures
below are commit-stamped results, not aspirational claims.

## Verified baseline

| Item | Value |
|---|---|
| Code snapshot | `5b421b7040ce422fb10d011407485de184b0cbe4` |
| Final main CI | run `44864845344`, 12/12 jobs PASS at `5b421b7` |
| Latest head | `ee7b59f` — prose-only CI-run record after the code snapshot |
| Local gate | `make check` exit 0 at `5b421b7` |
| Rust test cases | 677 passed, 0 failed |
| Python tests | 245 passed, 2 subtests passed |
| Contract validation | 69 JSON / 42 TOML / 24 Draft 2020-12 schemas; 1 registry instance validated with FormatChecker |

## Work packages landed

| PR | Merge commit | Package |
|---|---|---|
| #97 | `95a9584` | Opt-in measured benchmark harnesses (CAP-032, CLM-027) |
| #98 | `029bcef` | Data-rights enforcement at capture/store/replay boundaries (CAP-028, CLM-023 updated) |
| #99 | `33ec436` | Typed event-driven research simulation core (two research schemas) |
| #100 | `eac4b1e` | Experiment registry with atomic holdout claims and feature lineage (CAP-033/034, CLM-028/029) |
| #101 | `5b421b7` | Account ledger, reconciliation and health-to-risk authority (CAP-035/036/037, CLM-030/031/032) |

Each package had an independent written review, exact-head CI 12/12, and a
clean merge. Two reviews returned REJECT and the findings were fixed before
merge: simulation core (stale `UnmarkedResidual`, overstated certification
comment, Python 3.12 `super()` incompatibility) and ledger/health (forged
observations, double-reversal money corruption, capability/claim ID
collisions, four secondary findings). All fixes were re-reviewed to APPROVE.

## What is now implemented

- Seven-operation tri-state rights gate at recorder and replay boundaries; all
  21 rights remain `UNKNOWN`, all entitlements unverified. No right granted.
- Typed simulation events with explicit lineage and `origin_class`, pure
  reducer, `ExecutionReport`-only cash/position mutation, verifier-only
  `pnl_basis`, deterministic replay across hash seeds, and a conservative MBP
  baseline that refuses FIFO claims.
- Append-only experiment store with hash chain, INSERT-only triggers and
  atomic holdout claims; `alpha-search` refuses unregistered runs; byte-stable
  comparison; declarative point-in-time feature lineage.
- OMS fill events projected idempotently into the account journal with
  evidenced opening balance, reversal chains, exact paise folds and a replay
  cursor; typed reconciliation with a sticky blocked latch; risk decisions
  enforced inside `RiskAuthority` with registered owner tokens, typed failure
  reasons and a persisted kill switch.
- Measured, opt-in benchmarks with real counters, explicitly outside `make
  check` and CI; no latency or throughput claim is granted.

## Boundaries and residual gaps

- **Data foundation:** licensed institutional history, complete universes,
  delisted securities and historical depth remain absent. Rights are UNKNOWN;
  no capture, retention or research replay on real provider data is authorized.
- **Research framework:** the target directory structure and registry now
  exist, but the six strategy families, feature libraries, attribution and
  research-risk reporting remain documented gaps — no empty placeholder
  modules were added.
- **Execution realism:** L3 queue truth, distribution-calibrated latency and
  impact, venue-specific priority, borrow, margin, funding, auctions and
  corporate-action cashflows are not modelled. The simulation core is a
  research tool, not an exchange matching engine.
- **Infrastructure:** no distributed experiment execution, scheduler or
  multi-user resource allocation was added; each requires a separate owner
  contract/ADR. KDB-X remains an owner-local analytics complement.
- **Fieldbook:** deliberately untouched. No files, branches, commits or
  code from `/Users/tourist/Documents/fieldbook` were read into or written
  from this repository's changes; the boundary remains separate.
- **No profitability, live-readiness, broker-support, data-right or HFT claim**
  is made. Live routing remains hard-disabled.

## Archive

`/Users/tourist/Documents/nirnay-full-2026-11-20.zip`, SHA-256
`8bb739ea282d83a9ca9a82d0b38a3b71283342e33698f5310376879b9de25749`, source
snapshot `5b421b7`, 565 source files, 102 review-evidence files, declared-ref
and source-tree agreement PASS. Licenses and live licensed session captures
remain excluded from the source tree; the single owner-approved real Upstox
capture is included only under `review-evidence/` for audit.
