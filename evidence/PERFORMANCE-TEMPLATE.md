# Performance evidence template (no measurements)

This template defines the required fields of a new dated performance record.
It contains placeholders only. No measured number appears in this file, and no
benchmark run is claimed here. A published record copies this file to
`evidence/PERFORMANCE-YYYY-MM-DD.md`, replaces every placeholder with a value
captured from an actual run, and is then immutable. Reusing this template for a
different commit is forbidden; add a new dated file instead.

Grant rule: filling this template grants no latency, throughput, capacity or
readiness claim. Harnesses are opt-in and excluded from `make check` and CI
pass/fail gates. Local synthetic measurements are not deployment evidence.

## Identity and provenance

| Field | Required value |
| --- | --- |
| Record id | `PERFORMANCE-<YYYY-MM-DD>` |
| Commit | Full 40-character commit SHA measured |
| Branch | Exact branch name measured |
| Working tree | Clean or list of uncommitted paths |
| Toolchain | Rust version, Python version and pinned constraint file |
| Hardware | Host model, CPU, core count, memory and storage type |
| Environment | OS, kernel, locale and relevant environment variables |
| Date | ISO date of the run |

## Commands and outcomes

| Field | Required value |
| --- | --- |
| Harness | One of `bench-ingress`, `bench-research`, `bench-experiments`, `bench-risk-oms` |
| Exact command | Command as invoked, including flags and repeat count |
| Exit status | Observed exit code |
| Repeats | Integer count of independent repetitions, at least two |
| Failure policy | Latency and throughput are recorded, never asserted as gates |

## Distributions and counters

| Metric | Required value |
| --- | --- |
| p50 | Measured median from the emitted distribution |
| p99 | Measured 99th percentile from the emitted distribution |
| p99.9 | Measured 99.9th percentile from the emitted distribution |
| Throughput | Measured rate with explicit unit and numerator definition |
| Counter provenance | Each counter mapped to an ingestor checkpoint, a run summary or a timed code path |
| Peak RSS | Measured high-water resident set size with unit and platform source |

## Hashes and integrity

| Field | Required value |
| --- | --- |
| Output hash | SHA-256 of the canonical benchmark output |
| Dataset hash | SHA-256 of the immutable synthetic dataset |
| Code digest | SHA-256 of the source files exercised |
| Determinism | Statement that output hashes matched across all repeats |
| Artifact paths | Where raw `BENCH-*` lines were saved |

## Scope limits (copy verbatim unless a narrower limit applies)

- All fixtures are synthetic and local; no licensed provider feed was used.
- Measured latencies include local process and filesystem overhead.
- No broker, deployed risk service, live adapter or target infrastructure was involved.
- No rights, entitlement, profitability or live-readiness claim is implied.
- The `BENCH-*` lines are observations, not thresholds, certificates or gates.

## Acceptance checklist

- [ ] Commit, branch, toolchain and hardware fields are filled from the run.
- [ ] Command, exit status and repeat count are recorded verbatim.
- [ ] p50, p99 and p99.9 are present for each reported latency.
- [ ] Every counter names its measured source; no literal stands in for a counter.
- [ ] Output and dataset hashes matched across all repeats.
- [ ] README document map points at this dated file when it is published.
