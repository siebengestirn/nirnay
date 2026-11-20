# M3.1 authorization — real-source NSE cash market state — 2026-11-20

**Status: OWNER-DIRECTED.** The owner explicitly assigned M3.1 in this session.
This record bounds that direction. It does not authorize later M3 milestones,
MCX, derivatives, D30/TBT depth expansion, live trading or broker order writes.

## Authorized scope

- One NSE cash-equity market-data vertical in paper/shadow mode.
- Upstox Analytics V3 Full (five levels) as one source.
- FYERS Data Socket (five levels) as an independent second source.
- Zerodha Kite remains a future execution boundary only; no order writes.
- Separate source journals, typed MBP snapshots/deltas, source-aware
  continuity, snapshot recovery, fresh-process replay, and strategy
  consumption of kernel market state.

## Explicit exclusions

- No live order submission, no R5/R6 promotion, no broker credential use.
- No MCX, BSE, derivatives, D30, FYERS TBT, MBO or nanosecond work.
- No distributed infrastructure, no new infrastructure dependency without a
  measured requirement and contract.
- Synthetic fixtures remain unit tests only and never count as real-source
  certification.

## Blocking input

Real-source certification requires an entitlement-approved captured NSE cash
session from both providers, including initial snapshot and reconnect
behaviour, preserved under controlled storage. That session has not been
supplied. All code work can proceed and be verified against existing synthetic
conformance vectors, but no real-source claim will be made until the captures
are supplied and replayed.

## Execution order

Priority 0: separate review and merge of the cleanup branch.
Then PRs A–G: capability registry, scope-aware continuity, Upstox capture
integration, FYERS typed projection, MBP book/recovery, paper-runner
integration, real-source replay and cross-provider reconciliation.
