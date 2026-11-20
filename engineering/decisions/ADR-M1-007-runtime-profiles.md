# ADR-M1-007 — Runtime profiles and their resource/latency/durability contracts (design only)

- **Status:** PROPOSED (design contract, not implemented, not enforced). M1 design work only.
- **Group:** Runtime Design Group
- **Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9` (all citations `01eb382e:`)
- **Scope:** six *logical* profile contracts. No config, no build flavor, no service, no optimization, no SLO numbers.
- **Critical scope note (A6-11):** the six profiles and workload classes C0–C5 are an **audit lens**, not repo constructs. No profile enum, config, or enforcement exists at baseline (`configs/**`, `capabilities.json`; A6-11). This ADR defines them as a design contract for future milestones; it does not claim they are implemented.
- **C4/C5 out of scope:** `target-architecture.md:66-69` targets "reliable broker-API execution and medium-frequency research, not exchange-colocated HFT," and requires workload measurements and a decision record before C++/SBE/kdb+/q/Kafka/Redis/K8s adoption. The `low-latency` and `hft` rows below are **placeholders, not authorized profiles — no implementation, optimization, hardware assumption or performance claim is authorized by them** (challenger Section D.2; see M1-RUN-INV-019).
- **Amendment resolution:** challenger **A-1** (in-process vs separate-process authority) aligned in D-007.4 / M1-RUN-INV-021 / U-007-e: "in-process" is the **M1 design posture only**; separate-process/workload isolation is an **unimplemented deployment precondition** from ADR-002 §3.4, not built or required to be built by M1, and recorded as a **future ADR-gated owner decision**, not forbidden. **Section D.2** low-latency/hft non-authorization stated above. **Section C** §9 owner/milestone and precondition labelling applied.

---

## 1. Problem

M0 found that runtime behaviour is real but undocumented and unbudgeted:

1. **No profiles or budgets exist.** No workload class, profile, latency target, throughput SLO, operating envelope or benchmark harness is repo-defined (A6-08/09/11). `anti-patterns.md:307-432,910-923` is prescriptive ("how to set budgets") with no numbers; `target-architecture.md:66-69` is a qualitative posture.

2. **Runtime behaviour is implicit and inconsistent.** Rust capture uses the default tokio multi-thread runtime but performs all frame work — Base64 decode, SHA-256, JSON, blocking `sync_data()` — inline in a single task (`upstox_recorder.rs:221-294,470-509`; A6-01/03). FYERS uses three app threads plus vendor threads, a bounded queue (16,384) and gap accounting (`fyers_recorder/recorder.py:34,141-201,243-307`; A6-04). Research is single-threaded with deterministic output and flock-serialized publication (`A4 §7`; `point_in_time.py:559-563`). The paper engine is a synchronous library with `BTreeMap` state and per-event fsync (`A6-10`; `account-journal/src/lib.rs:503-509`).

3. **Workloads must map cleanly.** A6 §6 maps C0 batch (decode export, research, PIT), C1 streaming (Upstox/FYERS recorders), C2 interactive (cost-cli, kite-ops), C3 event-driven quant (paper-engine, simulators, normalizers); C4/C5 absent/excluded. Without declared profiles, a C3 correctness-oriented structure could be mistaken for a latency capability, and a C1 recorder could inherit inappropriate durability/backpressure assumptions.

4. **Budgets must come from workloads, not aspiration.** A6 §7 defines W1–W6 measurements (raw append, end-to-end capture, decode, recovery, FYERS overload, account soak) with percentiles p50–p99.99, CPU/RSS, queue depth, drops, fsync count, recovery time — but budgets must be defined **before** optimization (A6-08). The design must therefore express budgets as *named placeholders filled by measurement*, never invented numbers.

This ADR declares the six logical contracts, maps C0–C3 to them, and fixes each profile's failure/authorization posture so no downstream milestone can smuggle in a remote dependency or a latency claim without a gate.

---

## 2. Alternatives considered (with rejection reasons)

- **Define only the four implemented-workload profiles (standard/streaming/research/execution) and omit low-latency/hft.** Rejected. The programme and owner directive name all six; omitting the excluded ones hides the C4/C5 scope decision and would let an unqualified HFT claim appear later. Better to define them explicitly as out-of-scope placeholders.

- **Publish numeric budgets (e.g. target frames/s, p99 µs) now.** Rejected. No workload measurement or threshold harness exists at baseline (A6-08/09); numbers would be aspiration, violating the owner directive and A6-08's "budgets first" rule. The ~43 µs/append proxy (A3-03) and 16,384 FYERS queue are *observed baseline facts*, not budgets.

- **Make profiles enforced configs/build flavors in M1.** Rejected. No profile registry exists (A6-11); enforcement would be new infrastructure requiring an ADR and measured requirement (ADR-0001). M1 is design-only.

- **Merge `streaming` and `execution` into one "realtime" profile.** Rejected. Streaming is a capture/I/O profile with gap-accounted loss semantics; execution is a correctness-critical state profile with fail-closed risk. Conflating them would let a bounded-queue drop policy leak into account state.

- **Let the execution profile call a remote risk service / KMS / broker port to "strengthen" authorization.** Rejected. This is R1 Q4 / §14 #10/#11: it couples a hard-disabled local system (A5 F-A5-00) to KMS, an mTLS daemon, eight missing collectors and broker connectivity, creating new SPOFs with no measured requirement. Owner challenge 1 explicitly requires resolving the importable-token boundary **without** assuming remote service is the answer.

---

## 3. Decision

Declare six logical profiles. Each is a *contract namespace* with nine policy dimensions. Values marked `Q_<name>`, `B_<name>`, `L_<name>` are named parameters to be filled from measured workloads (A6 §7 W1–W6) and approved by ADR — **no literal numeric budget is asserted here**.

### D-007.1 — Profile matrix (nine policy dimensions)

| Dimension | **standard** | **streaming** | **research** | **execution** | **low-latency** (out of scope) | **hft** (out of scope) |
| --- | --- | --- | --- | --- | --- | --- |
| **Threading** | Single/default; no dedicated IO threads (C2 cost-cli/kite-ops) | Reader isolated from blocking durable writer; dedicated writer thread (FYERS reference, A6-04); Rust inline single-task is *insufficient for this contract* (A6-03) | Single-threaded; publishers serialized by flock (`point_in_time.py:559-563`) | Single serial account authority (ADR-006 D-006.3); no shared mutable state across threads | Dedicated pinned threads, IO isolated — design intent only | Colocated, CPU-isolated — placeholder only |
| **Async** | None required (synchronous calls) | tokio `select!` stop/socket; bounded channel reader→writer (A6-03 gap) | None | None (synchronous library) | No allocation/no syscall on hot path; event/busy-poll — intent only | Same, qualified hardware — placeholder |
| **Allocation** | General allocator; no pooling requirement | Reuse buffers; frame size cap exists (`recorder.rs:32`, 4 MiB — observed, not a budget) | General; canonical JSON strict settings (`market.py:53-55`) | `BTreeMap`/clone-heavy, correctness-oriented (`A6-10`) | Pre-allocated pools, no per-event malloc; SBE gated (target-arch:66-69) | Same — placeholder |
| **Queue** | None (finite input) | **Bounded** capacity `Q_stream` (from W2/W5); overflow → gap record + drop count + non-research-grade (A6-04); no silent drop | None | External intent queue bounded `Q_exec`; no internal queue | Bounded SPSC `Q_ll`, explicit overload signal | Bounded, measured — placeholder |
| **Serialization** | JSON/JSONL; canonical JSON where hashes recorded | Canonical JSON per frame; `payload_sha256` preserved; never split frames (ADR-006) | One canonical JSON encoder (`ensure_ascii=False, allow_nan=False, sort_keys`) — unify `report_sha256` divergence (A4-006) | Journal JSON with per-event fsync; BLAKE3 chain (`account-journal`) | Binary (SBE candidate) **only** with measured justification + ADR | Same, gated — placeholder |
| **Persistence** | None for CLI; PIT publication flock+fsync+atomic rename (`point_in_time.py:646-662`) | Per-frame durable append, ACK = `sync_data()` returned (`journal.rs:47-53`); no group commit without ADR-008 | Immutable, no-overwrite, content-hashed dataset (`point_in_time.py`) | Per-event durable append; replay re-derives state; P&L recomputed from fills+marks or excluded (`state.rs:573-577`) — ADR-009 | Async durability with defined ACK level + RPO/RTO from workload (ADR-008) — not defined here | Placeholder |
| **Failure** | Fail closed on validation error; non-zero exit; no duplicate side-effect retry | Gap/reset → discontinuity flag + `Resyncing`, fail closed until fresh snapshot (ADR-006) | Fail closed on determinism/timezone errors; runtime fingerprint required for numerical claims (A4-006) | Fail closed: UNKNOWN, stale, missing permit, persistence failure, reconciliation mismatch, kill switch (`target-arch.md:61-63`; ADR-003) | Deterministic overload behaviour; fail closed | Fail closed — placeholder |
| **Scheduling** | On-demand / batch / cron; no latency SLA | Continuous long-lived; backpressure required | Batch/on-demand; deterministic across `PYTHONHASHSEED` (A4 E-evidence) | Event-driven; logical time, no wall-clock in decisions | Budgeted `L_ll` from W1–W3, p50–p99.99 reporting (A6 §7), no number here | Qualified environment only |
| **Observability** | Exit status + quality JSON (`cli.py:205-218`) | Queue depth, drop/gap count, fsync count, reconnect count, `capture_sequence` — names defined, values from W2/W5 | Dataset/report hashes, quality report, runtime fingerprint (design requirement) | Decision capsule (`engine.rs:900-902`), account view, risk results | Full latency percentiles, queue depth, drops; hardware/toolchain captured | Hardware certification — placeholder |

### D-007.2 — Workload class → profile mapping

| Class | Workloads (baseline paths) | Profile(s) | Status |
| --- | --- | --- | --- |
| **C0 batch/scheduled** | `upstox_decode_export.rs`, `point_in_time.py`, `quantlab/{backtest,portfolio_backtest,alpha_search,experiments}.py`, `.local/extras/**` | `research` (and `standard` for pure CLI transforms) | Present as workload; profile contract is design-only |
| **C1 streaming** | `upstox_recorder.rs`, `upstox_smoke.rs`, `fyers_recorder.py` | `streaming` | Present as workload; bounded/backpressure contract partly unmet at baseline (A6-03) |
| **C2 interactive** | `cost-cli`, `kite-ops`, `ops/market_session.py` | `standard` | Present as workload |
| **C3 event-driven quant** | `paper-engine`, `risk-engine`, `execution-policy`, `account-journal`, Python simulators, `providers.py` | `execution` (state/risk) + `research` (Python replay) | Present as workload; live routing hard-disabled (A5 F-A5-00) |
| **C4 low-latency** | none implemented | `low-latency` | **Out of scope** — placeholder (target-arch:66-69) |
| **C5 HFT** | none; explicitly excluded | `hft` | **Out of scope** — placeholder (target-arch:66-69, ADR-0001/0003) |

### D-007.3 — Budget discipline

- Each profile's `Q_*`, `B_*`, `L_*` parameters are defined by name only. They are populated from A6 §7 workloads W1–W6 (p50–p99.99, max, CPU/RSS, queue depth, drops, fsync count, recovery time), on fixed hardware/toolchain with recorded commit SHA, before any optimization.
- No profile may assert a numeric throughput/latency target in M1. The ~43 µs/append fsync proxy (A3-03), 16,384 FYERS queue and 4 MiB frame cap are **observed facts about the baseline**, explicitly not budgets or targets.
- `low-latency`/`hft` carry no budget at all until a separate decision and qualified environment exist (owner directive; A6 posture).

### D-007.4 — Owner-challenge constraints carried by profiles (interfaces only)

- **Challenge 1 — Authorization (no remote service assumed; A-1 alignment).** The **M1 design posture** for the `execution` profile (and its excluded descendants) is a **single in-process authority with no implicit remote dependency** at the risk-token boundary (A5 F-A5-03: importable `_GATE_SIGNING_CAPABILITY` in `risk_service.py:24-26,192-204`). This posture is **not an implementation requirement and does not foreclose the future isolation path**: ADR-002 §3.4 identifies a separately administered process/workload identity, signer-key policy and audit retention as the deployment precondition for CLM-010 to be literally true. That isolation is **unimplemented, unowned, and not built or required to be built by M1**; whether the future live authority remains in-process or moves to a separate workload is a **future, ADR-gated owner decision** (U-007-e), recorded here rather than resolved by silence. Nothing in this ADR forbids that decision. No profile licenses building a KMS/mTLS/8-collector call in M1; doing so requires an ADR + measured requirement (ADR-M1-002; R1 Q4). `standard`/`streaming`/`research` profiles own no authorization authority. This is a profile posture, not an implementation or a deployment architecture.
- **Challenge 2 — OMS.** The `execution` profile owns one durable order lifecycle and owner, including UNKNOWN and reconciliation, fail-closed (ADR-M1-003); interface implication only.
- **Challenge 4 — Durability.** Each profile declares a persistence/ACK posture by reference to ADR-008 (streaming = per-frame ACK, execution = per-event ACK, research = immutable dataset, low-latency/hft = ACK level + RPO/RTO from workload). Off-host anchoring is ADR-010; envelope implications only. This profile ADR fixes no ACK meaning itself.
- **Challenge 5 — Replay.** `research` and `execution` profiles must recompute P&L from fills+marks or exclude it; copied-forward P&L (`state.rs:573-577`, `engine.rs:971`) never qualifies as verified equality (ADR-009).
- **Challenge 6 — Evidence.** Every guarantee in this matrix is `designed`, not `verified`; claims fail closed when evidence is absent. Profile existence is an audit classification, not an implemented capability (A6-11).

---

## 4. Ownership

- **Decision owner:** Runtime Design Group (this ADR); owner-level acceptance recorded in `M1-RECONCILIATION.md`.
- **Profile definitions (design-time state):** owned by the Runtime Design Group for M1. No runtime state is created.
- **Per-profile runtime state owners (proposed, largely unassigned at baseline):** `streaming` → capture/adapter owner; `research` → research-tooling owner (CAP-015); `execution` → paper-engine/account-journal owner (currently **unowned** per F-A1-07 — gap recorded); `standard` → tooling owner; `low-latency`/`hft` → **no owner** (out of scope).
- **Budget parameters (`Q_*`, `B_*`, `L_*`):** proposed owner = performance/verification owner; values require measurement (A6 §7) and owner approval. M0 records no such owner/budget (A6-08) — **unresolved**, gap recorded.
- **No persistent state is created by this ADR.**

---

## 5. Invariants (explicit IDs)

- **M1-RUN-INV-017 (design-only status):** All six profiles are logical contracts; none is an implemented, enforced config in M1.
- **M1-RUN-INV-018 (budget provenance):** No numeric latency/throughput/capacity value appears in a profile unless traced to a named workload measurement and approved by ADR.
- **M1-RUN-INV-019 (C4/C5 exclusion):** `low-latency` and `hft` are out of scope; no implementation, hardware assumption, latency claim, optimization or claim attaches to them in M1 (`target-arch.md:66-69`; challenger Section D.2).
- **M1-RUN-INV-020 (scope isolation):** `streaming` loss/drop semantics never apply to `execution` account state; `execution` fail-closed semantics are never weakened by `streaming` overload policy.
- **M1-RUN-INV-021 (local authorization posture — A-1 aligned, not a blanket ban):** No profile authorizes **building or wiring** a remote or isolated authorization/anchor dependency during M1; any such change requires an ADR + measured requirement (owner challenge 1). The M1 design posture is in-process with no implicit remote dependency, but a **separate-process/workload isolation path (ADR-002 §3.4) is a future ADR-gated owner decision and is explicitly not forbidden here** (U-007-e).
- **M1-RUN-INV-022 (P&L recompute):** `research` and `execution` profiles never accept copied-forward P&L as verified equality.
- **M1-RUN-INV-023 (observability completeness):** Each profile declares all nine policy dimensions; profiles with undeclared dimensions are incomplete and fail the acceptance check.

---

## 6. Failure cases (concrete, including adversarial)

1. **Profile laundering (primary).** A downstream claim says "the low-latency profile meets p99 X" when no low-latency workload, budget or implementation exists. M1-RUN-INV-017/019 forbid; claims fail closed (challenge 6). **Adversarial:** a stakeholder points to the matrix as evidence of capability; the status column and A6-11 disprove it.

2. **Aspirational budget injection.** An unconstrained milestone inserts "target 100k frames/s" into a profile without W1–W5 measurement. M1-RUN-INV-018 rejects; budgets come from workloads.

3. **Backpressure policy bleed.** A streaming overload setting is applied to the execution profile, dropping an account event under load and corrupting state. M1-RUN-INV-020 forbids.

4. **Remote-coupling creep (challenge 1).** The execution profile is amended to call KMS/mTLS/eight collectors to "verify" permits, creating SPOFs and violating ADR-0001. M1-RUN-INV-021 blocks without an ADR + measured requirement; baseline live-disable stays local (A5 F-A5-00).

5. **Copied P&L green compare (challenge 5).** Research/execution replay reports verified-equal P&L copied via `RiskStateObserved`. M1-RUN-INV-022 forbids; ADR-009 owns recompute.

6. **Async misapplication.** Tokio multi-thread runtime assumed to give parallelism while all work is inline in one task with blocking fsync (A6-03). The streaming row explicitly declares reader/writer isolation as a *contract requirement*, not an achieved property.

7. **Unowned profile state.** Execution profile's account state has no named owner (F-A1-07); an incident has no accountable party. Ownership §4 records the gap; owner acceptance must assign it.

---

## 7. Dependencies

- **ADR-M1-006 (Ordering/partitions):** execution/streaming profiles inherit ordering scopes, account authority and backpressure semantics; stream profile capacity depends on its contract.
- **ADR-M1-005 (Event identity):** identity contract fields are the envelope carried by all profiles.
- **ADR-M1-002 / ADR-M1-003 (Authorization / OMS):** execution-profile authorization and OMS constraints (challenges 1–2) are owned there.
- **ADR-M1-008 / ADR-M1-009 / ADR-M1-010 (Durability / Replay / Anchoring):** persistence, P&L recompute and anchoring implications referenced by profile rows.
- **Evidence dependencies:** A6-01/03/04/08/09/10/11; A3-03 (fsync proxy, non-budget); A4 §7 (single-thread concurrency), A4-006 (runtime fingerprint); A5 F-A5-00/F-A5-03; `target-architecture.md:66-69`; Amendment A §A3 (C4/C5 exclusion, no partitioning severity downgrade).
- **External capability dependencies:** none. No service, broker, hardware or benchmark harness required in M1.

**Explicitly unresolved disagreements (recorded, not guessed):**
- **U-007-a:** Baseline §13.11 records C4/C5 excluded by target architecture, while the programme's M8/M9 ambition points toward HFT. This ADR keeps C4/C5 out of scope for M1 and does not resolve the later ambition; a separate decision/environment is required.
- **U-007-b:** Whether profiles should ever become enforced configs (A6-11 says none exist). Decision here = design-only; enforcement is gated behind a measured requirement.
- **U-007-c:** Budget parameter values and their owner — deferred to A6 §7 measurement; no values or owner assumed.
- **U-007-d:** Whether the Rust inline-single-task model (A6-03) must be restructured to meet the streaming contract. Declared as a required contract property, but restructuring is implementation, out of scope for M1; unresolved until a measured requirement exists.
- **U-007-e — referred to owner/future ADR (challenger A-1):** whether the future live authorization authority is in-process or a separately administered process/workload. ADR-002 §3.4 requires separate-process isolation as a deployment precondition for literal CLM-010 truth; ADR-007 records in-process as the **M1 design posture only** and does **not** decide or forbid the isolation path. That isolation is unimplemented and not built by M1; the choice is an owner decision to be made in a future ADR and must appear in `M1-RECONCILIATION.md` (Section F.2).

---

## 8. Rejection conditions

This decision is overturned or revised if committed evidence shows:

1. A repo-defined profile registry/enforcement is approved with a measured requirement (would revise M1-RUN-INV-017 and D-007.1 status).
2. `target-architecture.md:66-69` posture is formally changed to include low-latency/HFT, with qualified hardware and a decision record (would revise M1-RUN-INV-019).
3. Measured workloads (A6 §7) produce approved numeric budgets (would populate, not invalidate, `Q_*`/`B_*`/`L_*` parameters and D-007.3).
4. A profile is shown to require a remote authorization/anchor dependency **during M1** for correctness (would revise the M1 posture in M1-RUN-INV-021/D-007.4). This does **not** prejudice the future ADR-gated isolation decision (U-007-e), which remains an owner choice.
5. Evidence shows `streaming` drop semantics are safe for account state (would revise M1-RUN-INV-020).

---

## 9. Measurable acceptance requirement (design only — not executed)

**Test name (proposed):** `test_runtime_profile_contract_completeness`
**Command (proposed, a documentation/contract lint run only in a gated follow-on milestone):**
`python3 scripts/check_profile_contract.py --adr adr/ADR-M1-007-runtime-profiles.md --strict`
- **Owner:** performance / verification owner (proposed; **unassigned at baseline** — A6-11, F-A1-07). Ratify at exit gate.
- **Target milestone:** M2 (future, separately authorized); **not an M1 deliverable**.
- **PRECONDITION — not an M1 gate.** No profile registry, config or checker script exists (A6-11); no owner is assigned. The 54/54 cell check is sound once scripted, but the script must first be created by the named owner in the target milestone. This requirement is a specification, **not** evidence that any profile is implemented or verified.

Specification and thresholds (design only):

- Parse the D-007.1 matrix and assert **6/6 profiles × 9/9 policy dimensions** are populated (M1-RUN-INV-023). Threshold: 54/54 cells non-empty.
- Assert each of `low-latency` and `hft` carries status `out_of_scope` citing `target-architecture.md:66-69`. Threshold: 2/2 marked out of scope; 0 implementation claims.
- Assert **0 unlinked numeric budgets**: every `Q_*`/`B_*`/`L_*` parameter is a named placeholder mapped to an A6 §7 workload (W1–W6); no literal latency/throughput/capacity target appears in a profile (M1-RUN-INV-018). Threshold: 0 numeric budget literals.
- Assert the C0–C5 → profile mapping in D-007.2 covers C0–C3 with a workload path and marks C4/C5 out of scope. Threshold: 4/4 present classes mapped; 2/2 excluded.
- Assert the four challenge constraints (local authorization, OMS, durability-by-reference, P&L recompute) appear in D-007.4; no row **requires or authorizes building** a remote risk service/KMS/broker dependency in M1 (M1-RUN-INV-021); and the future ADR-gated isolation path (U-007-e) is recorded, not forbidden. Threshold: 0 required/authorized M1 remote dependencies; 1 recorded future-isolation decision.

**Falsification note:** a profile that fails any threshold is incomplete and must be rejected per the README's "an ADR without all nine sections is incomplete" rule and the exit-gate condition "measurable requirements defined for every decision."

**Baseline status:** no profile registry, config or checker exists (A6-11); no owner or milestone is committed. The checker and all thresholds are **PRECONDITION — not an M1 gate**, to be implemented and executed only after owner authorization. They are **not** evidence that any profile is implemented or verified.
