# Upstox ingress integration evidence — 2026-11-19

**Scope:** the `upstox-recorder` binary now writes its one authoritative raw
journal through `DurableIngestor` with `SequencePolicy::CaptureSession`. No
live provider session was captured; no network or entitlement claim is made.

## Preserved behavior

- Original protobuf bytes are passed unchanged into `SourceFrame.payload` and
  stored in the same `raw-frames.jsonl` path.
- Capture sequence remains a local receive-order counter, explicitly not a
  provider sequence; the manifest keeps `provider_sequence_available: false`
  and `capture_sequence_semantics`.
- Session manifest, frame count, SHA-256 of the raw journal, decoded
  observations, connection-event lifecycle file, reconnect budget and terminal
  error behavior are unchanged.
- `verify_journal` still validates the completed journal before the manifest is
  written atomically.

## Tests

- `recorder_writes_one_shared_ingress_journal`: two frames produce exactly two
  records in one raw journal, with the Upstox ingress source identity; the
  decoded and lifecycle journals remain distinct; the session manifest is
  written.
- `shared_ingress_journal_is_accepted_by_the_decoder` (in `upstox_decode_export`)
  proves the existing decoder accepts journals written by the shared ingress
  source identity.
- `ingress_refusal_is_not_retryable` makes refused frames terminal, not reconnectable.
- `recorder_journal_restart_replays_without_new_commits`: `DurableIngestor::resume`
  rebuilds the checkpoint and replay commits nothing.
- Existing payload-limit and failure-classification tests remain green.

## Verification

Recorder tests: 6 passed. Decode-export tests: 10 passed. Full `make check`
is recorded on the PR head.

## Boundaries

FYERS still uses its separate Python hash-chain recorder. Canonical event
projection, instrument resolution and live capture remain separate stages.
