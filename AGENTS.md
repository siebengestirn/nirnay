# Agent and contributor rules

## Read first

- `README.md` — current product scope, current-state block and document map.
- `engineering/verification-policy.md` — Proposed / Implemented / Verified /
  Falsified semantics.
- `engineering/capabilities.json` and `claims/ledger.json` — capability and
  claim records; verify each material statement against source and tests.
- The relevant ADR under `engineering/decisions/`.

Do not ingest every Markdown file or historical archive for an ordinary task.
Dated evidence answers "what was verified then", not "what is true now".

## Behaviour rules

- Do not claim profitability, live readiness, broker support or data rights
  without dated evidence. A passing CI run on one commit is not a capability
  claim for another commit.
- Do not add provider/exchange observations, credentials or account
  identifiers to Git.
- Financial money, prices, charges and P&L stay in exact Rust domain types;
  Python is for research statistics.
- A strategy emits intents only. It never accesses broker SDKs, OMS internals,
  risk permits, cash or positions.
- Do not add a new crate, language, database or service without a contract,
  test and measured reason.
- Write the hypothesis and falsifier before parameter search; preserve failed
  experiments.
- Keep live trading disabled. No CI result authorizes live routing.
- Update `engineering/capabilities.json` and `claims/ledger.json` truthfully;
  implemented is not verified.
- Rust changes must follow the Linux kernel Rust coding guidelines
  (`https://docs.kernel.org/rust/coding-guidelines.html`) and the Google Rust
  style guide (`https://google.github.io/styleguide/rustguide.html`) without
  exception. Where they conflict, satisfy both: one module per `use`
  statement plus kernel vertical import layout with the trailing empty
  comment. `cargo fmt`, Clippy `-D warnings` and
  `python3 scripts/check_rust_style.py` (part of `make check`) bind new lines.

## Documentation lifecycle

Canonical documents are maintained continuously with the code:

- `README.md` — single current-state entry point.
- `AGENTS.md` — these rules.
- `CONTRIBUTING.md` — workflow and evidence expectations.
- `SECURITY.md` — disclosure and secret-handling policy.

Dated evidence is immutable once published:

- `BUILD-RESULTS.md` — owner-maintained bundle record; do not edit.
- `evidence/VERIFICATION-2026-11-17.md`
- `evidence/UPGRADE-VERIFICATION-2026-11-13.md`
- `evidence/BUILD-INVENTORY-2026-11-13.json`

Never rewrite a dated record to make it look current. Add a new dated file
under `evidence/` and point `README.md` at it. Keep the current commit and CI
run in the README current-state block; `scripts/check_docs.py` enforces this
in `make check` and CI.

Owner work is excluded from agent edits: `.github/workflows/release.yml`,
`BUILD-RESULTS.md`, `engineering/supply-chain.md` and `scripts/make_bundle.py`.
Do not stage, reformat, revert or publish those files without an explicit
owner task. Audit and recovery records are retained in the external verified
archive; the working tree has no `.local/` directory.

## Authorization state

M0 is complete. The owner accepted the M1 exit and signed bounded M2
implementation authority on 2026-11-20; the controlling decision is
`evidence/M2-AUTHORIZATION-2026-11-20.md`. M2 covers one NSE cash-equity
vertical in paper and shadow modes. Live trading, platform-readiness claims,
multi-asset expansion and R5/R6 promotion remain unauthorized and require a
separate owner decision. No agent may sign or imply a further milestone.

## Mandatory workflow

1. Record commit, branch, working-tree state, worktrees and relevant backups.
2. Declare objective, allowed files, non-goals, exclusions and acceptance
   criteria before editing.
3. Read the source-of-truth documents and only relevant subsystem code.
4. State invariants and failure cases; add a test that fails on old behavior.
5. Keep one coherent change per branch/PR; never rewrite published history.
6. Run targeted tests, then the full deterministic gates for affected
   workspaces and artifacts (`make check`).
7. Inspect the final diff for generated files, secrets, formatting drift,
   stale claims, dependency drift and owner-work contamination.
8. Obtain independent review for financial, authorization, persistence,
   replay, concurrency and security changes.
9. Update capability, claim and evidence records only when implementation or
   evidence changes; record exact commit, environment, command, exit status,
   counts and hashes.
10. Report completed checks, blocked checks, residual risks and authorization
    state. Never label an unavailable check as passed.
