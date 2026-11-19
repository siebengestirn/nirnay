# M2.6 — broker-neutral adapter and simulator — 2026-11-20

**Objective:** first execution-adapter increment under the signed M2
authorization. Baseline `e04a83392d56404e376377997977e09f10487d96`. One
broker-neutral contract, tested against a deterministic simulator; no real
endpoint.

## What was implemented

- New `crates/broker-adapter` in the adapters layer, depending downward on
  `authoritative-oms` and `trading-core`. No broker SDK, HTTP client, async
  runtime, strategy or risk dependency.
- `BrokerOrderRequest` has private fields and is built only from an OMS
  `DispatchClaim`, so the adapter binds the complete typed order request:
  venue, product, side, quantity, limit price, validity and decision evidence,
  plus client order id, submission attempt id and the opaque permit evidence. A
  permit covering only broker, account, strategy, instrument and notional would
  be insufficient. `matches` rechecks claim, permit and target; the simulator
  refuses a request addressed to another adapter.
- Broker-neutral `BrokerAdapter` interface: `submit`, `cancel`, `poll_events`
  and `query_order`. `SubmissionOutcome::TimedOut` is explicitly ambiguous and
  maps to UNKNOWN at the OMS, never to an automatic retry.
- `SimulatedBroker` is scripted for immediate acknowledgement, timeout,
  rejection, and acknowledgement with partial fills. It can emit duplicate
  events and genuinely deliver them out of order; `poll_events` restores the
  declared broker sequence before events reach the OMS, so an early fill cannot
  drive an illegal transition. Cancellation binds both the broker order and the
  client order identity.
- Broker events carry stable event ids and execution ids for OMS dedup.

## Tests

Twelve tests: complete typed request binding and claim matching; unauthenticated
and incomplete request refusal; acknowledgement driving the OMS to Working;
timeout becoming UNKNOWN with no retry plus reconciliation from the broker
view; rejection moving the OMS to Rejected; duplicate and reversed events not
double-filling; cancel via the bound broker identity; no strategy/risk/SDK/live
coupling; and simulator configuration validation, wrong-target request refusal,
cancel client-identity binding, and a proof that raw broker delivery is out of
order while `poll_events` normalizes it.

## Verification

`cargo test -p broker-adapter` — 12 passed; Clippy `-D warnings` clean;
`cargo xtask check` PASS; `make check` exit 0; Rust, source and prose style PASS.
Hosted exact-head CI is recorded by the PR.

## Boundaries and non-claims

- Simulator only. No real broker authentication, order endpoint, network call,
  credential, order tagging or regulatory classification is exercised. This
  stage does not satisfy SEBI/NSE access requirements.
- No strategy, risk, portfolio, accounting or OMS logic duplicated. The
  adapter translates requests and events; the OMS remains the lifecycle
  authority.
- No real market data, no live routing, no deployment. R5/R6 promotion remains
  separately authorized and unexercised.
