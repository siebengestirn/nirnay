# Target architecture (destination, not current implementation)

The first release is a modular monolith. The following components are target
boundaries; a target box is not evidence that its code already exists.

```text
DATA PLANE
  Upstox and FYERS read-only adapters -> raw frame journals -> normalizers -> quality monitor
  instrument/reference ingestion -> point-in-time store -> Parquet datasets

RESEARCH PLANE
  Python feature/experiment lab -> pre-registration -> statistics/multiplicity
  -> chronological walk-forward -> frozen strategy artifact

SIMULATION PLANE
  Rust event replay -> conservative order matching -> costs/slippage/impact
  -> positions/equity/drawdown -> reproducible report

CONTROL & EXECUTION PLANE
  strategy intent -> portfolio sizing -> independent risk permit -> OMS
  -> Zerodha Kite adapter -> broker events -> accounting -> reconciliation

OPERATIONS PLANE
  audit journal, health checks, alerting, backup/restore, kill switch,
  configuration and release provenance
```

## Dependency direction

`market-types` and India reference models are foundations. `trading-core` and
`market-book` are domain contracts. The account journal and raw market archive
are storage concerns. Costs, risk and execution policy are services; the paper
engine composes them. Future strategy crates are pure decision logic. Network
adapters live above strategies and behind stable ports; applications are the
composition roots and leaves. Strategy crates must never depend on broker SDKs,
OMS internals, account mutation APIs or risk-authorization internals.

The Cargo graph is enforced by `architecture/layers.toml` and
`scripts/check_repo.py`. Do not create empty crates to represent roadmap work. Add a crate after its
API, ownership boundary and acceptance tests are defined.

## Canonical event envelope

Every normalized event must preserve source identity, provider instrument ID,
internal instrument ID, source timestamp and clock domain, receive UTC time,
receive monotonic time, availability time, record time, sequence/gap metadata,
schema version and raw-frame hash. Do not overwrite provider time with local
receive time. Do not sort by wall clock when a documented sequence/availability
ordering exists.

## Critical state boundaries

- Strategy: proposes target exposure/order intent only.
- Portfolio: computes aggregate exposure and desired order deltas.
- Risk authority: independently approves or denies exact order terms.
- OMS: owns durable order lifecycle and idempotency.
- Broker adapter: translates a validated order and reports broker state.
- Accounting/reconciliation: derives positions, cash, fees and P&L from fills
  and compares them against broker snapshots.

A network timeout after submit is an **unknown state**, not a failed order. The
OMS reconciles before retrying. Unknown positions, stale market data, missing
risk permits, failed persistence and reconciliation mismatches fail closed.

## Performance posture

The initial target is reliable broker-API execution and medium-frequency
research, not exchange-colocated HFT. C++, SBE, kdb+/q, Kafka, Redis and
Kubernetes require workload measurements and a decision record before adoption.
