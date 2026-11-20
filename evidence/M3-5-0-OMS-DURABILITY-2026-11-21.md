# M3.5.0 — OMS journal durability — 2026-11-21

Dated evidence for the OMS writer-hardening prerequisite. Synthetic fixtures
and injected faults only; no provider data, broker endpoint or live order.

## Defects

1. `OmsStore::open` did not take an exclusive writer lock, so two processes
   could append from stale replayed state.
2. A failed append or `sync_all` left the instance usable. Reusing it after a
   partial write or uncertain durability result could corrupt the sequence or
   claim a commit that was never durable.

## Correction

- Opening the journal now calls `try_lock_exclusive`; a second writer fails
  with `WriterLocked` instead of waiting or appending concurrently.
- `append` marks the writer poisoned before writing and clears it only after a
  successful `sync_all`. Any later error leaves the instance poisoned, and all
  mutating methods reject with `WriterPoisoned`.
- `OmsStore::recover(path)` replays the journal into a fresh writer; the
  poisoned instance is never silently reused.
- A documented fault-injection hook (`CommitFault`, `force_commit_fault`)
  exercises the partial-write and sync-failure paths deterministically.

## Tests

Five new tests cover: a second writer refused; lock released on drop; partial
write poisons and recovery fails closed on the torn tail; sync failure poisons
and recovery replays the durable prefix; restart preserves contiguous event
order. All 17 `authoritative-oms` tests pass, plus full `make check`.

## Boundary

This milestone establishes preservation and recovery of received execution
state. It does not yet persist typed per-fill events, project them into the
account journal, or claim broker correctness. No account ledger or risk
integration is included here.
