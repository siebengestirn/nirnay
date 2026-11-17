# M1 accepted architecture decisions (design record)

These eleven ADRs were produced during the M1 programme and recorded as accepted with amendments by the repository owner on 2026-11-17. They are design records for the initial Nirnay workload and its shared data foundations. They do **not** authorize implementation, M2 work, platform-readiness claims or live trading.

The original independent challenger reports and the full M1 reconciliation package remain in the owner's local recovery archive; this directory exists so contributors can find the accepted decisions without reading private recovery material.

| File | Decision |
| --- | --- |
| `ADR-M1-001-platform-constitution.md` | ADR-M1-001 — Platform constitution: domains, ownership, non-responsibilities and dependency policy |
| `ADR-M1-002-authorization-authority.md` | ADR-M1-002 — One authorization authority (resolve the importable risk-token boundary) |
| `ADR-M1-003-oms-authority.md` | ADR-M1-003 — One authoritative OMS lifecycle and owner (UNKNOWN + reconciliation) |
| `ADR-M1-004-dependency-and-port-rules.md` | ADR-M1-004 — Dependency and port rules under the modular monolith |
| `ADR-M1-005-event-identity.md` | ADR-M1-005 — Canonical event identity: observation identity vs business identity |
| `ADR-M1-006-ordering-and-partitions.md` | ADR-M1-006 — Ordering scopes and partition semantics |
| `ADR-M1-007-runtime-profiles.md` | ADR-M1-007 — Runtime profiles and their resource/latency/durability contracts (design only) |
| `ADR-M1-008-durability-acknowledgement.md` | ADR-M1-008 — Durability acknowledgement levels, fault model and RPO/RTO requirements |
| `ADR-M1-009-replay-contract.md` | ADR-M1-009 — Replay and verification-equal contract (no copied-forward P&L) |
| `ADR-M1-010-provenance-anchor.md` | ADR-M1-010 — Signed off-host provenance anchor and source-trust boundary |
| `ADR-M1-011-evidence-model.md` | ADR-M1-011 — Evidence model: maturity vs verification status, fail-closed claims |

See `engineering/decisions/ADR-0001..0004` for the earlier accepted decisions. Low-latency and HFT remain excluded from the current scope.
