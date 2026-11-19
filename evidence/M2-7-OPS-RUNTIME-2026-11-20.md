# M2.7 — operating-environment primitives — 2026-11-20

**Objective:** first operating-environment increment under the signed M2
authorization. Baseline `a765c75bc965d8fb7d52db83dcfa3ec92b8b06e9`. Executable
primitives and tests, not a production deployment.

## What was implemented

- New `crates/ops-runtime` in the services layer, with no broker, network,
  async or deployment dependency. Placement below `engines` keeps a future
  health-to-risk-authority wiring layer-legal.
- `RuntimeLayout` separates binaries, config, secrets, raw frames, journals,
  reference revisions, research datasets, backups and logs. It refuses a root
  inside the source tree, creates owner-only secret permissions on Unix, and
  validates that no directory symlink escapes the root.
- `create_backup` stages evidence files, records a SHA-256 digest and size per
  entry in a manifest, and publishes atomically by rename. Backup names are
  sanitized so a caller cannot escape the backup directory. `verify_backup`
  rehashes every entry; `restore_backup` verifies first, restores into a fresh
  path, then verifies the copy again. Tampered bundles and existing targets are
  refused.
- `HealthSnapshot` exposes feed age, sequence gaps, reconnects, journal append
  latency and queue depth, disk capacity, open and UNKNOWN orders,
  reconciliation differences, kill switch and account-snapshot availability.
  `evaluate` turns these into a `safe_for_new_risk` verdict with reasons, so
  monitoring failures stop new exposure instead of only alerting.
  `metrics_text` emits Prometheus-style text without a metrics dependency,
  including account-snapshot availability.

## Tests

Seventeen tests: layout creation/validation and source-tree refusal; manifest
digests; verified fresh-path restore; tampered backup refusal; exclusive backup
publication; restore refusing an existing target; fresh snapshot permits risk;
stale feed, sequence gaps, UNKNOWN orders, reconciliation differences, disk
pressure, journal backlog, kill switch and missing snapshot each block new
risk; metrics exposure, unsafe backup-name refusal, and the snapshot-availability
metric.

## Verification

`cargo test -p ops-runtime` — 17 passed; Clippy `-D warnings` clean;
`cargo xtask check` PASS; `make check` exit 0; Rust, source and prose style PASS.
Hosted exact-head CI is recorded by the PR.

## Boundaries and non-claims

- No Linux host was provisioned and no systemd service, service identity,
  secret store, egress rule or monitoring stack was deployed. Real deployment
  remains an owner-provided-infrastructure task; this stage supplies tested
  primitives only.
- No process-termination, machine-restart, disk-exhaustion, network-outage or
  expired-authentication test is run here. Journal crash recovery and broker
  ambiguity were exercised at the OMS/adapter stages; backup restoration is
  exercised on a fresh local path.
- No live trading, no broker endpoint, no automatic deployment, no claims or
  capability promotion. `configs/live.toml` remains disabled and untouched.
- The health verdict is produced here; wiring it into the risk authority at
  runtime is a later integration step, not claimed in this stage.
