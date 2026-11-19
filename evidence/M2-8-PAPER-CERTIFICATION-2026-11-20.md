# M2.8 — bounded end-to-end paper wiring — 2026-11-20

**Objective:** final bounded M2 increment under the signed M2 authorization.
Baseline `7c431952abee41e5721904ce329e552fbc299fd2`. Wires M2.1–M2.7 into one
deterministic pass over a synthetic verified session.

## What was implemented

- New `apps/paper-runner` in the apps layer: a `PaperInput` contract, a
  `run_paper` pipeline and a `paper_run` binary. Dependencies are only the M2
  crates and shared domain types; no SDK, HTTP, async runtime or live path.
- Pipeline order: synthetic source observation → M2.1/M2.2 kernel event and
  continuity → frozen-artifact EMA strategy → trading-core `OrderIntent` →
  risk authority `evaluate_and_reserve` → OMS create/accept/dispatch →
  broker-neutral adapter to the simulated broker → acknowledgement and fills →
  OMS lifecycle → digest-bound report.
- Authority-owned position and exposure are updated from accepted fills, so a
  later sell intent is evaluated against the authority's own position rather
  than a caller claim. Duplicate broker events do not double-count a fill.
- Health verdict from M2.7 gates all new exposure before risk evaluation. The
  snapshot is refreshed from live OMS open/UNKNOWN order state before each
  decision, so it is not a one-time precheck.
- Report binds event count, intent count, reservation count, health verdict,
  every decision (permit, broker identity, fill state), a SHA-256 digest of
  the exact input, and a run digest that binds both the input digest and the
  derived report.
- A defect found by integration was fixed in `broker-adapter`: the
  acknowledgement now carries the ack event id, not the last fill event id, so
  a following fill is not mistaken for a duplicate.

## Tests

Eight tests: full end-to-end path with bound decisions; byte-identical
fresh-process replay through the binary; stale feed blocking all exposure;
risk rejection creating no OMS order; dispatch timeout leaving UNKNOWN orders;
report digest binding inputs including policy version; no live/broker-SDK dependency; and no public
submission path.

## Verification

`cargo test -p paper-runner` — 8 passed; Clippy `-D warnings` clean;
`cargo xtask check` PASS; `make check` exit 0; Rust, source and prose style PASS.
Hosted exact-head CI is recorded by the PR.

## Boundaries and non-claims

- **This is not real-source certification.** No captured Upstox or FYERS
  session, entitlement review, real broker response or live account was used.
  The session, quotes and broker are synthetic fixtures.
- No validated alpha is claimed. The EMA artifact is the existing systems
  fixture; the frozen artifact's validation fields are consumed as contract
  text, not recomputed.
- No Linux deployment, real monitoring stack, backup of production evidence or
  machine-restart test is performed here. M2.7 supplied tested primitives only.
- The health verdict is evaluated in-process; it is not yet wired to an
  external alerting or service-control system.
- Live trading, R5/R6 canary promotion and broker order submission remain
  separately authorized and unexercised. This stage produces operating evidence
  for the paper/shadow vertical only.
