# Canonical event projection evidence — 2026-11-19

**Code commit:** `11a5a0d36c6d52e5f5ea85496b04c32fa7d5d1c2`.
**Scope:** first offline projection from verified decoded Upstox frames using
the existing `MarketEvent` model and `UpstoxV3Normalizer`. No competing Rust
event model, no instrument resolution by inference, no live provider path.

## Contract

- `--reference-revision` binds the projection to one exact reference content
  revision. `revision_sha256` is SHA-256 of the canonical object excluding the
  binding field; instrument and venue maps used by the normalizer must match
  the bound revision exactly, or the command fails closed.
- Unknown provider instruments stay `instrument_id: None`; no identity is
  inferred from a symbol or CIN.
- Every archived observation carries `source_frame_sha256`; when a reference
  revision is bound, a missing or malformed frame hash is rejected.
- The quality report records `event_schema_version` (1), `normalizer_version`
  (`upstox-v3-v1`) and `reference_revision_sha256`, alongside the existing
  output digest and limitations.

## Tests

`research/python/tests/test_canonical_projection.py`:

- reference binding succeeds and records exact versions/hash;
- an unknown instrument remains unresolved while its provider id is preserved;
- a drifted instrument map fails closed with no published output;
- the same command in two fresh processes produces byte-identical output.

Existing `test_cli_pipeline.py` normalization tests remain green. Full suite:
166 Python tests passed.

## Boundaries

This is a projection of Upstox decoded frames only. FYERS projection, canonical
instrument resolution against a real exchange reference revision, strategy
consumption and live processing remain separate. Provider diagnostics are not
exchange sequence or trade prints; those limitations remain in the report.
