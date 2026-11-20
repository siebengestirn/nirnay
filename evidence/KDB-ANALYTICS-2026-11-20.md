# KDB-X analytics integration — 2026-11-20

Dated evidence for the checked-in KDB-X analytics capability. This record is
immutable; a later run adds a new dated file.

## Scope

- Checked-in q contract: `analytics/kdb/mbp_metrics.q`, one source per run,
  never merges or sums books.
- Repo-native harness: `apps/kdb-lab` (`nirnay-kdb`), golden files under
  `apps/kdb-lab/tests/golden/`, report schema
  `schemas/analytics/kdb-metrics-report.v1.schema.json`.
- Engine: KDB-X `5f` (`5.0.20261023`) at `/Users/tourist/.kx/bin/q`.

## Measured results

### Synthetic golden (contract pin)

Input `apps/kdb-lab/tests/golden/mbp-observation-v1.csv`: 6 observations with
one sequence gap. Fresh-process q replay is byte-identical to
`apps/kdb-lab/tests/golden/mbp-metrics-v1.csv` (SHA-256
`b3b6566e24097a14432e3dadd19ca7e8f81ac43a9b31bec5106c92c690d36a61`).
Input SHA-256 `40811f73f7d3e9a4b1a73920b0a794db332585d040be891dba297f185f6178a5`.

### Real Upstox capture (research statistic, not a trading claim)

Input: the 36 five-level observations exported from the 2026-11-20 real
Upstox read-only capture. KDB-X result, generated through `nirnay-kdb`:

| Metric | Value |
|---|---|
| rows | 36 |
| duration_s | 87.07369 |
| gap_count | 0 |
| spread_mean | 0.02194444 |
| spread_median | 0.02 |
| spread_max | 0.04 |
| imbalance_mean | -0.3033901 |
| imbalance_min | -0.637065 |
| imbalance_max | 0.19821 |
| levels_min | 5 |
| bid1_mean | 76.43667 |
| ask1_mean | 76.45861 |
| bid1_max_qty | 4103 |
| ask1_max_qty | 3382 |

Prices and spreads are in the input unit (rupees). Input SHA-256
`4d8f61ade950f9218467feb6c00c292387fc3af7ad9ed9af87d5c58667804ad9`; output
SHA-256 `0e5af277bab536c8cbf205f05dd6fa83b4013765fa775084c2ff4c456ce4df11`.

## Verification boundary

These measurements verify the analytics contract and the captured data only.
They do not establish profitability, live readiness, data rights or any broker
capability. No order was placed, no broker write occurred, and no licensed
session data is stored in Git. Full capability and claim records are
`engineering/capabilities.json` CAP-027 and `claims/ledger.json` CLM-021.
