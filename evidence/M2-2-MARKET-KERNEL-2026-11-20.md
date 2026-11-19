# M2.2 — deterministic market-data kernel projection — 2026-11-20

**Objective:** first Rust kernel increment under the signed M2 authorization.
Baseline `91d6dfd3f8d18485453c6f446882263eeed3c360`. Builds on M2.1's
`market-event` identity contract; does not redefine identities or instruments.

## What was implemented

- New `crates/market-kernel` (engines layer, depends on `market-event`):
  `SourceObservation` is the input contract for one verified, instrument-local
  observation. Raw bytes remain the evidence authority upstream; the kernel
  does not acquire, decode or rewrite provider frames.
- `project_observation` derives M2.1 identities from the exact canonical
  payload: one raw-frame commitment may produce several distinct observation
  identities (multi-instrument frames), and content-addressed ids exclude all
  reset-prone local fields.
- `project_session` tracks continuity per source, marks gap/recovering events
  non-actionable, refuses duplicate observation identities within a session,
  and preserves unresolved instruments as `None` with no invented identity.
- Offline `kernel-session` binary publishes kernel events exclusively to a new
  path, fsyncs the output and reports a digest on stderr. No network, broker,
  order or trading behavior.

## Tests

Nine tests: multi-instrument frame separation, unresolved instrument not
actionable, gap degradation, reference revision binding, duplicate-session
refusal, invalid-input refusal, fresh-process replay, session-level revision
fallback, and missing-revision refusal. The replay test runs the binary twice
in separate processes over the same session and asserts byte-identical output
plus an identical non-empty digest reported on stderr.

## Verification

`cargo test -p market-kernel` — 7 passed. `cargo xtask check` PASS. `make check`
exit 0; Rust, source and prose style PASS. Hosted exact-head CI is recorded by
the PR.

## Boundaries and non-claims

- **The session fixture is synthetic.** No real Upstox or FYERS capture was
  acquired, decoded or replayed in this increment. M2.2's real-source evidence
  is not claimed; it requires an owner-provided captured session and entitlement
  review.
- No protobuf/provider decoding duplicated here; the existing
  `upstox-decode-export` remains the decoder. The kernel consumes an explicit
  verified observation contract, not raw vendor bytes.
- No canonical book/market-state reconstruction, strategy, portfolio, risk, OMS,
  broker adapter, live routing or multi-asset expansion.
- Determinism is proved for the synthetic session only; real-source continuity
  and recovery remain future gates.
- Price/quantity semantics stay in the existing domain types; this envelope
  carries identity, time, scope, reference and quality only.
