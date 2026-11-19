# M2.5 — one authoritative OMS — 2026-11-20

**Objective:** first OMS increment under the signed M2 authorization.
Baseline `e4c7231c16394de8b4b64c2d36a18c44d027e7f0`. Reuses the full lifecycle
vocabulary of the target architecture rather than creating a fourth order model.

## What was implemented

- New `crates/authoritative-oms` in the engines layer, depending only on
  `trading-core` for the typed `OrderIntent`. No strategy, risk-authority,
  broker SDK, venue SDK or live-routing dependency.
- `OmsStore` owns one durable transactional store: an append-only commit
  journal. Every successful transition appends a complete state record and
  calls `sync_all`; replay requires a contiguous sequence chain and verifies
  a SHA-256 hash chain over every record, so a truncated, reordered or
  semantically tampered journal is refused. This avoids introducing a restricted
  SQLite dependency into the engines layer without an owner policy decision.
- Client order identity, submission-attempt identity, outbox command state
  (`PENDING → CLAIMED → DISPATCHING → ACKNOWLEDGED | REJECTED | UNKNOWN`),
  broker order identity, execution identities, fills and event dedup keys are
  all owned by the store.
- Full order lifecycle: `Created → RiskAccepted → PendingSubmission → Working
  → PartiallyFilled/Filled`, with `CancelPending/Cancelled`, `Rejected`, and the
  ambiguity branch `Unknown → Reconciling → Resolved → final`. Terminal states
  never move; every transition is guarded by an explicit `permits` table.
- A timeout after `DISPATCHING` yields `Unknown` and records
  `SUBMISSION_OUTCOME_UNKNOWN`. No retry command is created and no pending
  command remains claimable. `Unknown` can only leave through
  `begin_reconciliation` + `resolve_unknown` with broker evidence.
- Idempotent `create` on identical terms; conflicting terms refused. An
  invalid typed request is refused before it reaches the journal. Duplicate
  broker event and execution ids are ignored, preventing double fills.
  Resolving an UNKNOWN order as filled requires and records the reconciled fill
  quantity and notional, so accounting fields are never left at zero.
- `PermitRef` stores the risk permit as evidence. The OMS binds the complete
  typed order request; a permit covering only account, instrument and notional
  would be insufficient.

## Tests

Seventeen tests: accepted order enqueues one submission command; client-id
idempotency and conflicts; acknowledgement binds broker identity; dispatch
timeout is UNKNOWN with no automatic retry; evidence-gated reconciliation;
duplicate broker events ignored; fill dedup and over-fill refusal; cancel flow
with its own outbox command; illegal transitions refused; commit state survives
restart; lifecycle guards; no broker/strategy/live coupling; identifier
validation; and journal sequence and digest corruption refused on replay, reconciled-fill
accounting, and invalid typed-request refusal.

## Verification

`cargo test -p authoritative-oms` — 17 passed; Clippy `-D warnings` clean;
`cargo xtask check` PASS; `make check` exit 0; Rust, source and prose style PASS.
Hosted exact-head CI is recorded by the PR.

## Boundaries and non-claims

- This is the order lifecycle and durable state authority, not an execution
  adapter. No broker request is built, signed or sent; broker behaviour is
  simulated only inside tests.
- No account journal integration, charge/P&L posting or position update from
  fills is included here; the OMS records fills but does not own accounting.
- No strategy, portfolio or risk policy logic. No real source or broker data.
- `PermitRef` is an opaque evidence record, not a verified authority
  attestation; a future end-to-end stage must wire it to `risk-authority`.
- Live trading, R5/R6 promotion and automatic deployment remain separately
  authorized and unexercised.
