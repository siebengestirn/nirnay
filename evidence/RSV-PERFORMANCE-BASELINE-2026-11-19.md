# RSV-12 performance baseline — 2026-11-19

**Classification:** measurement-only baseline. No regression threshold, no
throughput or latency certificate, no capability or claim promotion.
**Code commit:** `6a2c284e354a9353605ee86722a10e1af6c231c4`.
**Environment:** macOS 27.0.1 arm64; rustc/cargo 1.99.1; debug profile;
synchronous local `postgres`-style SQLite store under `/tmp`.

## Command

```bash
cargo test -p india-reference --test durability --offline \
  rsv12_performance_baseline -- --ignored --nocapture
```

The test is `#[ignore]` so it never runs in the default suite or CI.

## Measured record

Raw transcript: `evidence/rsv12-baseline-2026-11-19.log` (SHA-256
`d9b931315af2b4f48e30cfd2f9df58c3ac9edad14a08aa44a59f3e0e900c9646`). It was
captured from the code commit above, so it identifies those exact bytes; a
rerun will differ in timings.

| Field | Value |
|---|---|
| Deterministic dataset SHA-256 | `40c90988b91fec0e23e09c73a8fa6d733b6de8ec41c41fa4c6d1066272ae5c92` |
| Encoded payload bytes | 3,127 |
| Revisions published | 1 |
| Publish (write) time | 6.960 ms |
| Reopen time | 0.506 ms |
| `reference()` query time | 0.849 ms |
| Store bytes before | 160 |
| Store bytes after | 53,408 |
| Disk growth | 53,248 |

The dataset is a deterministic synthetic fixture, not a real exchange product.
The measurements are single-run observations on one host and debug build;
timings are not stable identifiers and are not compared against a threshold.
No claim is made about release-mode throughput, concurrency scaling or a
production durability boundary.

## Coverage note

RSV-12 now has a reproducible measurement procedure and a first dated baseline.
RSV-08 remains an unresolved owner decision (`evidence/RSV-COMPLETION-2026-11-19.md`)
and is not claimed passed. RSV-01…07 and 09…11 have active implementations and
tests. `.local/extras/` is unchanged throughout.
