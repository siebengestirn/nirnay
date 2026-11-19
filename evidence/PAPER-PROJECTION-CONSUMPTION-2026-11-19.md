# Paper engine projection consumption — 2026-11-19

**Scope:** offline consumption of canonical events by the existing single-instrument paper engine. No engine rewrite, strategy semantics, live path, trading or `extras/` change.

## Contract

- `quantlab paper-replay --events <canonical.jsonl> --provider <upstox|fyers> --output <result.json>` loads canonical events through `market_event_from_dict`.
- Every event must carry a resolved `instrument_id` and a `source_frame_sha256`; unresolved or unlinked events are refused before the engine runs.
- `--provider` is explicit and every event must match it, so mixed-provider liquidity is never summed.
- `--reference-revision` must match the projection's quality sidecar binding hash, otherwise replay fails closed.
- A no-intent strategy drives the engine: the result proves deterministic consumption without inventing trading behavior.
- Output is written exclusively to a non-existing path with an fsync; it carries the input SHA-256, lineage count and the engine summary.

## Tests

`research/python/tests/test_paper_projection_consumption.py`:

- three projected events are consumed with zero orders and full lineage accounting;
- unresolved instrument and missing source-frame hash each fail closed;
- provider mismatch fails closed;
- a reference revision must match the projection sidecar;
- two fresh processes produce byte-identical result files.

## Verification

`pytest` 5/5 for this suite; full suite 175 passed; `make check` PASS.

## Limits

The single-instrument engine remains unchanged. Portfolio consumption, strategy semantics and live routing are separate; no performance or profitability claim is made.
