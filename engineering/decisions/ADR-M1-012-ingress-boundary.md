# ADR-M1-012 — Nirnay-owned raw ingress boundary

- **Status:** Accepted as a Nirnay engineering decision for this bounded boundary only. Not owner ratification of M2, a network service, a broker adapter, live data rights or trading.
- **Owner:** Market Data Engineering (Nirnay). Changes to this boundary require review and an exact-head CI pass.
- **Baseline:** `ea877ab6d784356c2e852412f5c904fd0ca15cc4`.
- **Relationship to ADR-M1-004:** ADR-M1-004 remains **Proposed** and is **not amended** by this record. It describes a target hexagonal port and authorizes no port implementation. This ADR makes a separate, narrower decision for one already-merged in-process boundary; it does not claim the general port question was settled.

## Decision

The shared ingress boundary is a library contract in `crates/market-archive`,
not a service and not a new storage engine. It is authorized for this scope:

- `IngressSource` adapters supply original provider bytes plus capture metadata; no provider decoding or canonical identity creation occurs at the boundary.
- `DurableIngestor` writes through the existing `JournalWriter` to one authoritative raw journal per source stream.
- Delivery identity is separate from payload digest. Identical bytes under different delivery identities are distinct observations; the same identity with different bytes is a conflict, never a silent overwrite.
- Sequence provenance is explicit: capture-session, provider-per-instrument, or none. Capture-session sequences are never interpreted as per-instrument provider sequences.
- Continuity state is bounded. Eviction is recorded as unknown continuity, never as a fresh clean stream.
- Resume and replay stream the journal through `JournalReader`; memory is bounded by record size and explicit state budgets, not archive size.

## Non-scope

No provider application is connected by this decision. `upstox_recorder` and
the FYERS recorder keep their current journals until their own integration PRs.
No canonical event model, instrument resolution, network listener, IPC service,
broker SDK, durable fundamental store or trading path is authorized here.

## Rationale

PRs #41–43 established the port and adapters with tests. This decision exists
because the port contract itself needed correction before producers depend on
it. Recording the boundary and its owner prevents the earlier Proposed ADR from
being silently treated as if it had authorized the implementation, and prevents
later work from assuming the boundary is a general platform port.

## Falsifiers

This decision is falsified if a producer writes to a second raw journal after
integration, if payload content is used as delivery identity, if a session
sequence is used for per-instrument continuity, if continuity state is silently
lost on eviction, or if replay memory grows with archive size.
