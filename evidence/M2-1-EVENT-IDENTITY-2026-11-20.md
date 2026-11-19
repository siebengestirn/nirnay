# M2.1 — kernel event identity and continuity — 2026-11-20

**Objective:** one versioned Rust kernel contract for event identity and
sequence continuity, per the signed M2 authorization
(`evidence/M2-AUTHORIZATION-2026-11-20.md`) and owner direction that M2.1 is
the first implementation. Baseline `92c9b685b471948f2e7093561eb1b86136dad959`.

## What was implemented

New `crates/market-event` in the `domain` layer, depending downward only on
`market-types`. It does not define a second instrument identity or a second
book model; `InstrumentId` remains `india-domain`'s opaque identity and the
existing Python `MarketEvent` stays the research payload model.

- `KernelEvent` v1 with `capture_frame_id`, `observation_id`, optional
  `business_event_id` with an explicit `source_observation_id` back-reference,
  `origin_class`, source/session/connection/receive identity, sequence and
  scope, source-vs-receive time domains with explicit clock domain, raw-frame
  hash, reference revision, quality and evidence token.
- Three content-addressed identity classes per ADR-M1-005 D-005.1. Locally
  assigned, reset-prone fields (capture counter, connection epoch, receive
  times, sequence) are excluded from identity. A bare state observation gets
  no invented business id.
- `origin_class` is fail-closed: `exchange_event` cannot be asserted by the
  constructor without evidence; connected sources produce `sdk_callback`.
- `ContinuityMachine` implements `Clean → Gap → Recovering → Recovered`, with
  the rule that ordering alone never clears a gap. Recovery requires an
  authoritative snapshot identity and a clear step in the same epoch.
- v1 JSON schema `schemas/market-data/kernel-event.v1.schema.json` with
  conditional requirements for derived and exchange-originated events.

## Tests (owner's twelve named cases)

All twelve are in `crates/market-event/tests/kernel_contract.rs`:
duplicate delivery; same identity with different content; recorder restart;
connection restart; sequence reset; sequence gap; out-of-order delivery;
multi-instrument frame; snapshot recovery; reference revision mismatch;
unknown instrument; distinct source and receive time domains. Three additional
tests cover business-identity separation, fence/scope validation and fail-closed
exchange origin.

`crates/market-event/tests/fresh_process_replay.rs` runs a child process over a
four-frame captured fixture and asserts byte-identical identity and continuity
lines with an in-process run. That is the M2.1 exit condition at the identity
level, not full market-state reconstruction.

Invariant locks: `validate_invariants` rejects a business id without its source
back-reference, an exchange origin without a valid evidence token, and a broken
receive fence; `is_actionable` consults it, and a custom `Deserialize` applies it
to all wire input. Recovery requires a real authority in the current epoch and
stores it before `clear_recovery` may return the stream to clean. Six tests
cover forged input, stale authority, recovered-quality evidence and schema
consistency so a regression fails CI.

## Verification

`cargo test -p market-event` — 22 passed (21 contract + 1 fresh-process replay,
plus two ignored generator/worker tests). One test validates the crate's own derived-event
output is checked into `tests/golden/derived-event-v1.json`. The repository's
schema gate validates that vector against the shipped v1 schema using the real
`jsonschema` validator, and a Rust test asserts live output matches the golden;
neither side can drift silently. Contract validation: 20 Draft 2020-12 schemas
checked and one kernel-event vector validated by the schema gate. `make check`
exit 0; Rust, source and prose style PASS.

## Boundaries and non-claims

- No order, broker, OMS, risk or trading behavior; live routing stays disabled.
- No Python authoritative model change; no new schema file for the research
  `MarketEvent`; no new infrastructure dependency.
- No single-asset or venue model beyond the identity envelope. No live captured
  session was used — the replay fixture is synthetic, so this is not evidence
  of real-source ingestion (M2.2).
- No claim that two genuinely distinct observations producing identical bytes
  can be separated; ADR-M1-005 records that residual risk. Identity stability
  is not safe collapse outside known re-delivery/replay.
- Schema-version stability and cross-language parity are future gates; this
  record does not claim them.

Hosted exact-head CI is recorded by the PR.
