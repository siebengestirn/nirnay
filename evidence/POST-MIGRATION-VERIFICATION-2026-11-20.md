# Post-migration application verification — 2026-11-20

**Scope:** confirm the active Nirnay application still builds and runs after the
predecessor archive moved to `.local/extras`, Kalshi/Polymarket material was
removed from the active workspace, SBE golden symbols were neutralized, and
compile budgets were introduced. Commit `fe044e99`, branch
`cleanup/archive-and-perf`. No live data, broker or deployment was exercised.

## Build and test

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo build --workspace --all-targets --locked` | PASS |
| `cargo test --workspace --all-targets --locked` | PASS — 560 tests, 0 failed |
| `cargo test --workspace --doc --locked` | PASS — 1 doc test |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo deny check` | PASS — advisories, bans, licenses, sources |
| `cargo xtask check` | PASS |
| `python3 -m pytest -q` | PASS — 175 tests, 2 subtests |
| Python compile, repo policy, schema, fundamental vectors | PASS |
| Rust/source/prose/style-gate/shellcheck/generated-path/cross-pollution/docs | PASS |
| `make cpp-test` (active numeric/order-book oracle) | PASS — 1/1 |
| `make check` | PASS |

## Runtime smoke tests

All 12 workspace binaries were launched. `cost-cli` and `paper_run` take
positional arguments/subcommands rather than `--help`; both ran successfully
with real inputs.

- `paper_run` over a 10-quote synthetic session: 10 events, 2 intents,
  2 reservations, both orders `Filled` with distinct broker identities and an
  input-bound report digest.
- Fresh-process replay of the same fixture: report files byte-identical.
- Timeout script: orders `Unknown`, no broker identity, final health verdict
  `false`.
- Stale-feed script: zero reservations, no OMS order created.
- `kernel_session`: projected a verified session into one clean kernel event.
- `cost-cli`: computed exact integer charges for an effective-dated schedule.
- `upstox-*` CLI tools, `nirnay-fundamental`, `nirnay-fyers-import`, `paper-run`,
  `numeric_driver`, `book_driver` and `xtask` all started and reported usage
  successfully.

## Post-migration audit

- Zero tracked paths under `extras/`; the archive exists only as
  `.local/extras/`, outside version control and every build/CI graph.
- No active Kalshi/Polymarket or `engineering_challenge` references in Rust,
  Python, config, schema, workflow or canonical documentation. Historical
  evidence retains dated amendment notes and the signed M2 exclusion record.
- CI YAML parses; CI has no archived-workspace jobs and includes the parallel
  compile-budget job.

## Boundary

This verifies the paper/shadow monorepo only. It does not establish real-source
capture, broker certification, Linux deployment, regulatory approval or any
R5/R6 live capability. Those remain separately owner-gated.
