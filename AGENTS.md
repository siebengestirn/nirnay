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
owner task. Preserve `.local/recovery/`.

## Authorization state

M0 is complete and the M1 design record is accepted with amendments. M1 formal
exit acceptance requires the owner's signature; no agent may sign or imply
it. M2 implementation, platform readiness and live trading are not
authorized. Only the owner can authorize a new milestone.

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
