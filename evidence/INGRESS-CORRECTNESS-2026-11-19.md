# Ingress correctness evidence — 2026-11-19

**Code commit:** `c3495aea736c9dbb06170529aab149839c386b02`.
**Scope:** ingress library contract only. Producer applications are not
connected; no live provider integration or canonical event projection is claimed.

## Acceptance scenarios

| Scenario | Test |
|---|---|
| Same payload, different capture sequence → two records | `same_payload_with_different_capture_sequences_is_preserved` |
| Same sequence and payload → one record | `same_delivery_and_payload_is_an_idempotent_duplicate` |
| Same sequence, different payload → conflict | `same_delivery_with_different_payload_is_a_conflict` |
| FYERS session sequence across two symbols → no false per-symbol gap | `fyers_session_sequence_across_symbols_has_no_false_per_symbol_gap` |
| More than 1,024 instrument identifiers → no silent continuity loss | `more_than_1024_instruments_have_no_silent_continuity_loss` |
| Replay of a large journal → bounded memory | `large_journal_replay_uses_bounded_memory` (20,000 frames) |
| Restart followed by replay → no duplicate commits | `restart_and_replay_commit_nothing_twice`, `replay_fence_holds_beyond_the_delivery_window` (5,000 frames) |
| Torn or corrupt journal → fail closed | `torn_or_corrupt_journal_fails_closed` |
| Fresh-process recovery → same records and checkpoint | `fresh_process_recovery_matches_committed_records_and_checkpoint` |

Additional tests cover foreign-source rejection, malformed/oversized payloads,
disconnect stops, replay byte fidelity, source-boundary preservation and the
absence of content-based deduplication for unsequenced observations.

Delivery identity is derived from `(source, instrument, sequence scope,
sequence value)` or, when no sequence exists, from receive timestamps. The
payload SHA-256 verifies bytes only; it never decides whether two observations
are the same delivery.

## Replay benchmark

Command:

```bash
cargo test -p market-archive --test ingress --offline \
  replay_benchmark_large_generated_journal -- --ignored --nocapture
```

Raw transcript: `evidence/ingress-replay-benchmark-2026-11-19.log` (SHA-256
`d935723949360a156b4f942bb8f7f450fec52bdfc1a82f6702458c15ebfb51f8`).

```text
INGRESS-REPLAY-BENCH frames=200000 elapsed_ms=3259 frames_per_sec=61366.5 memory_frames_held=1
```

The benchmark uses 200,000 generated frames, not 5,000. Replay holds one
decoded frame at a time via `JournalReader`; memory is bounded by the record
limit, not the archive size. Timings are one observation on this host and are
not a throughput certificate.

## Verification

`make check` PASS; `NIRNAY_STYLE_BASE=origin/main` source-style PASS; Rust
added-line style PASS; `cargo test -p market-archive --test ingress` — 16 passed,
2 ignored, 0 failed.

## Open boundaries

RSV-08 remains unresolved and `.local/extras/` is unchanged. `ADR-M1-012` authorizes
this specific ingress boundary without amending the Proposed `ADR-M1-004`.
