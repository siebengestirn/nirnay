# kdb-lab

Repo-native KDB-X analytics harness. This crate owns the typed contracts
around the checked-in q script `analytics/kdb/mbp_metrics.q`; it does not
reimplement analytics in Rust. KDB-X remains the engine of record.

## Why this exists

Research needs repeatable spread, imbalance, gap and depth measurements from
captured five-level books. An operator-local q script was not versioned, had no
typed contract, no golden result and no CI-safe validation, so a drifted script
or a different engine build could silently change research numbers. This crate
gives the analytics a checked-in contract, a deterministic golden, a schema for
its report and a measured reason to exist: one book per source, never merged.

## Contract

- Input: header-prefixed CSV with columns
  `seq,recv_ns,bid1_p,bid1_q,ask1_p,ask1_q,spread,imbalance,levels`.
- Output: deterministic `metric,value` lines with the names in
  `src/metrics.rs::METRIC_NAMES`.
- Units: prices and spreads are f64 in the input unit (rupees as exported by
  `upstox-decode-export`). No metric name implies paise. This is a research
  statistics surface, not the authoritative money domain.

## Use

```bash
make kdb-golden   # real-q replay of the synthetic golden; skipped if q absent
make kdb-analyze  # analyse KDB_INPUT into KDB_OUT (metrics.csv + report.json)
```

The binary is also callable directly:

```bash
cargo run -p kdb-lab --bin nirnay-kdb -- check-golden
cargo run -p kdb-lab --bin nirnay-kdb -- analyze \
  --input .local/perf/kdb/book.csv --source upstox-live \
  --out .local/perf/kdb-lab-run
```

The q binary is taken from `NIRNAY_Q_BIN` or
`/Users/tourist/.kx/bin/q`. No credential is read, printed or exported by this
crate. Input and output SHA-256 digests in the report make each run reproducible.

## Gates

- `cargo test -p kdb-lab`: contract, reference, report and CLI tests; the
  real-engine integration test replays the golden when q is installed.
- `make check`: the same golden is checked numerically by Rust without q, and
  `scripts/validate_contracts.py` validates the report JSON schema and the
  checked-in report vector.

Live orders, broker writes, licensed session data and cross-source book merging
are outside this crate's scope.
