# M3.1 real-source status — 2026-11-20

## Verified engineering result

The five-level NSE cash pipeline is implemented end to end and verified with
synthetic conformance fixtures:

- source capability and entitlement registry (`configs/providers/source-products.v1.json`);
- scope-aware continuity with fail-closed reconnect and `Unverified` state;
- typed five-level MBP snapshots from Upstox V3 Full and FYERS Data Socket
  callbacks, with explicit price/time conversion and no MBO claim;
- one depth-bounded MBP book per source with evidenced snapshot recovery;
- kernel envelope carrying typed payloads, with the paper runner and risk
  authority consuming kernel-derived book state (parallel quote path removed);
- cross-source reconciliation that reports differences and never merges books;
- deterministic fresh-process replay from a verified session into a digest-bound
  report.

Local gates: `make check` exit 0; workspace tests, clippy `-D warnings`, 21 JSON
schemas, 175 Python tests and 2 subtests all pass. Each PR had exact-head CI
12/12 and independent review closure.

## Real-source certification: BLOCKED

No entitlement-approved captured NSE cash session from Upstox or FYERS was
supplied. The only real provider frames in the repository are two Upstox
MarketInfo + LTPC frames with **no Full depth**; the FYERS session fixture is
explicitly synthetic. NSE/BSE files under `.local/nirnay-source-fixtures/` are
reference masters, not captured market data.

Therefore none of the twelve real-source acceptance conditions is claimed as
met. Passing synthetic systems fixtures are not real-source evidence.

## Required owner input

One entitled NSE cash session per provider including initial snapshot,
continuous updates, reconnect, gap and close phases, with entitlement and
storage/replay rights confirmation, preserved outside public Git. Replay that
evidence through `session-replay` and the paper runner, then issue a separate
certification record with session digests.

No live order submission, broker write, D30, FYERS TBT, MBO, BSE, MCX or
derivatives work was performed.
