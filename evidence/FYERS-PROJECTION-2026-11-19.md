# FYERS canonical projection evidence — 2026-11-19

**Scope:** second provider on the canonical projection path. Uses the existing
`FyersV3Normalizer` and `MarketEvent` model; no competing Rust event model.

## Contract

- Input is the real Python SDK hash-chain journal produced by the FYERS
  recorder fixture. The projection verifies the chain and refuses drifted
  records.
- `--reference-revision` binds the exact instrument map; an unknown instrument
  remains `instrument_id: None`, with its provider id preserved.
- The journal's record hash becomes `source_frame_sha256` on each event.
- Explicit gap records are counted in the quality report and require
  `--allow-gaps`; the output is marked non-research-grade.
- The report records `fyers-v3-v1`, event schema version 1 and the exact
  reference revision hash.

## Tests

`research/python/tests/test_fyers_projection.py`:

- two market observations project from the chained journal; a known symbol
  resolves, an unknown symbol stays unresolved; all events carry a source-frame
  hash; the report records versions, binding hash and the single gap;
- a map that differs from the bound revision fails closed with no output;
- two fresh processes produce byte-identical event and quality-report files.

## Verification

`python3 -m pytest -q research/python/tests/test_fyers_projection.py` — 3 passed.
Full `make check` PASS with 169 Python tests.

## Boundaries

FYERS supplies decoded callback JSON, not wire frames; no exchange sequence is
inferred. Gap-bearing output is degraded research data, not a continuous tape.
No live IPC, strategy consumption or trading is included.
