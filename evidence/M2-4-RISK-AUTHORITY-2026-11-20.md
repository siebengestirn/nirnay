# M2.4 — stateful risk authority — 2026-11-20

**Objective:** first portfolio/risk increment under the signed M2 authorization.
Baseline `c5844effa9ca70e0b8efafcddcb969ae88bea521`. Builds on the existing
`risk-engine::evaluate_risk`; it does not fork or duplicate policy checks.

## What was implemented

- New `crates/risk-authority` in the engines layer, depending downward only on
  `risk-engine` and `trading-core`. No OMS, broker, venue SDK, account-journal
  or order-submission dependency.
- `RiskAuthority` owns every fact the risk engine consumes: account identity,
  effective-dated instrument reference data, logical clock, positions, pending
  buy/sell quantities, current and pending gross exposure, daily P&L, open-order
  count, per-second order rate, session phase, account-snapshot availability,
  kill switch and reconciliation state. Callers supply only an `OrderIntent` and
  a raw `FeedObservation`; there is no account, exposure or health input.
- `FeedHealth` is derived from observation facts — quote presence, two-sidedness,
  crossed book, sequence-gap evidence, book completeness and age against the
  authority's own clock. No caller-supplied `Healthy` flag exists in the type.
- `evaluate_and_reserve` is the single gate: under one mutex lock it validates
  the intent, applies idempotency and an optional snapshot-revision fence,
  builds the `RiskContext` from owned facts, calls `evaluate_risk`, and applies
  the reservation (pending quantities, notional, open orders, rate state,
  revision) only on full acceptance. On any rejection nothing is reserved.
- Accepted replays of the same intent id return the original permit without a
  second reservation; reusing an id with different terms is refused. Consumed
  (released or filled) intents are reported distinctly from rejections.
- Accounting uses checked arithmetic and fails closed on overflow; order-rate
  history is pruned to a bounded window. Observation receipt times ahead of the
  authority clock are refused, so a caller cannot defeat staleness detection
  with a future timestamp. Permits carry the authority's account identity and
  expose `validate_invariants` so a fabricated or deserialized permit can be
  rejected by a future consumer.
- `release` and `apply_fill` move a reservation back to capacity or into a
  filled position; `snapshot` exposes a read-only state view for operating
  evidence.

## Tests

Fifteen tests: baseline acceptance; two concurrent reservations against one
aggregate limit (exactly one passes, binding check is `GrossExposure`); stale
snapshot-revision rejection; no caller-supplied feed health (gapped and stale
observations fail `MarketFeedHealth`); idempotent replay and conflicting reuse;
authority-owned kill switch, reconciliation and snapshot gates; caller cannot
claim another account; filled exposure accumulates in owned state; releasing a
reservation restores capacity; no OMS/broker/live coupling; the feed-health
derivation table; future receipt-time rejection; authority-owned permit account;
distinct consumed-intent replay; and permit self-validation against forgery.

## Verification

`cargo test -p risk-authority` — 15 passed; Clippy `-D warnings` clean;
`cargo xtask check` PASS; `make check` exit 0; Rust, source and prose style PASS.
Hosted exact-head CI is recorded by the PR.

## Boundaries and non-claims

- This is an authorization boundary and reservation ledger, not an OMS. It
  emits permits only; there is no submission attempt, attempt identity, broker
  event, outbox or UNKNOWN handling.
- No portfolio target construction or sizing from strategy targets is included;
  the authority consumes an already-formed `OrderIntent`. Portfolio sizing and
  target reconciliation remain a separate concern.
- No live trading, no broker adapter, no real account or market data. Fixtures
  are synthetic and the concurrency threads operate on in-process state, not a
  distributed system.
- `evaluate_risk` remains the sole policy engine; the authority adds ownership
  of facts and atomicity, not new policy rules. Validation fields and policy
  versions are consumed as contracts, not recomputed.
- R5/R6 live promotion remains separately authorized and unexercised.
