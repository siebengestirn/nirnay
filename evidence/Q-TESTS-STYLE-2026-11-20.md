# q / kdb+ tests and style gate — 2026-11-20

Dated evidence for the q analytics test harness, style guide and gate. This
record is immutable; a later run adds a new dated file.

## Research basis

Authoritative sources consulted:

- KX "Advanced q" index links **"Remarks on Style"**
  (<https://github.com/qbists/style>, Stevan Apter). Rules N1–N4 adapt its
  line-length, semicolon, whitespace and naming pages.
- KX unit-testing KB (<https://code.kx.com/q/kb/unit-tests/>) documents the
  `k4unit` CSV-description model mirrored by `analytics/kdb/mbp-test.q`.
- KX operator references for `deltas`, `string`, `0:` and `vs` informed rules
  N5–N7, each tracing to a concrete bug found during development.

The Nirnay-specific rules and conflict resolutions are in
`engineering/q-kdb-style.md`.

## What changed

- Pure library `analytics/kdb/mbp.q` separated from CLI
  `analytics/kdb/mbp_metrics.q` so analytics can be tested in-process.
- In-process q unit harness `analytics/kdb/mbp-test.q` with CSV descriptions
  `analytics/kdb/tests/mbp-unit.csv` and a fixture `empty.csv`. 8 assertions:
  schema version, row load, gap with first seq > 1, duration, spread mean,
  duplicate gap case, unknown-metric failure and empty-set failure. A
  deliberately broken assertion was confirmed to exit non-zero.
- `scripts/check_q_style.py`: added-lines gate with 16 self-tests, wired into
  `make q-style`, `make check`, the `style` phony and the CI `source-style`
  job, and registered in the style-gate integrity checker. Detects dotted
  column access inside single-line lambdas as well as multi-line ones.
- The q harness validates the `expect` column strictly; an invalid value
  (for example a typo) fails the case closed rather than being read as false.
- `make kdb-q-test` target and `.editorconfig` `[*.q]` section.

## Measured results (2026-11-20)

- q unit harness: 8/8 PASS, exit 0. Broken-case probe: 1/1 FAIL, exit 1.
- q style checker: 16/16 self-tests PASS; 0 findings on the three checked-in
  `.q` files. A single-line-lambda dotted-access probe is caught by the gate.
- Style-gate integrity: 7/7 self-tests PASS; wiring validation PASS.
- The refactored library reproduces the byte-identical golden
  `apps/kdb-lab/tests/golden/mbp-metrics-v1.csv` through the CLI.

No credentials, licensed session data, broker writes or live orders. Full
capability/claim records: CAP-027, CLM-021, CLM-022.
