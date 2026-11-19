# ADR-M1-003 — One authoritative OMS lifecycle and owner (UNKNOWN + reconciliation)

- **Status:** Proposed (M1 design only — not implementation authority)
- **Owner group:** Architecture Design Group, M1 Platform Constitution
- **Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9`, read via `git show 01eb382e:<path>`
- **Governing inputs:** M0 baseline §4/§8 + Amendment A4.2/A4.3/A5; A1 F-A1-03; A5 F-A5-04/05/06/07/08; R1 §2.3 (F-A1-03 affirmed); R2 Q3/Q6; `target-architecture.md:56-63`; `ADR-0004-live-disabled.md`
- **Challenge addressed:** **Owner challenge 2 — one lifecycle and one owner, including UNKNOWN outcomes and reconciliation (no broker-query adapter exists today).** Also carries challenges 4 (ACK/fault survival) and 5 (P&L not authoritative for replay) at the OMS boundary.
- **Amendment 1 (2026-11-17):** Challenger A-3 and B-2 disposition recorded in §3.2.1, §3.3 and §9. `submission_attempt_id` is fully specified as a **future contract**; authenticated reconciliation and its tests are **PRECONDITIONS, not M1 controls**; the forged-`FILLED` bypass is **not claimed closed**. Invariant-ID collisions (A-0) RESOLVED: `M1-ARCH-INV-*` namespace and authoritative `INVARIANT-REGISTRY.md` applied; cross-references updated.

---

## 1. Problem

M0 found **three overlapping order-lifecycle owners with no shared contract** (A1 F-A1-03; baseline §4/§8; A5 §7):

1. **`research/python/src/quantlab/oms.py`** — "deterministic state-machine/reference OMS": `OrderProjection` with a JSONL `HashChainJournal`, states `working/partially_filled/filled/pending_cancel/cancelled/rejected/expired/unknown` (`oms.py:361`), idempotency keys `client_order_id`, `broker_order_id`, `cancel_request_id`, `event_id`/`execution_id`. Live submission raises unconditionally (`oms.py:242-247`).
2. **`research/python/src/quantlab/durable_oms.py`** — `DurableOMSStore`: SQLite WAL/FULL, transactional outbox, event hash chain, leases, UNKNOWN quarantine; 9 order states (`:132`) and 6 outbox states (`:130`). Owns the more complete durable lifecycle. Live dispatcher raises unconditionally (`:608-609`).
3. **`crates/paper-engine` + `crates/account-journal`** — Rust event-sourced matcher with its own
   7-state `OrderStatus` (`trading-core/src/model.rs:257-318`, including `OutcomeUnknown` which is
   resumable) and a BLAKE3-chained accounting projection.

They share **no schema, DB, process, event type, submission-attempt ID or vocabulary** (A5 §7, "SHARED CONTRACTS: NONE"). Consequences (baseline §12, A5 F-A5-04/05, R2 Q6):

- Identical `client_order_id` values get different dedup semantics depending on which store is used; there is no rule for which store is canonical.
- **UNKNOWN is operationally stuck**: after an ambiguous SUBMIT timeout (`mark_unknown`, `durable_oms.py:397-421`), no component queries the broker order book; resolution occurs only if a caller invokes `apply_broker_event` (`:456-550`). `request_cancel` is refused while UNKNOWN (`:283-284`). No broker-query adapter exists.
- **R2 Q6:** `apply_broker_event` is the *only* method that sets terminal order state and is entirely caller-supplied. It rigorously validates shape (strictly increasing sequence, unique event_id/execution_id, transition table, overfill check at `:480-539`), but performs **no authenticity check** — a caller knowing `broker_order_id`/`client_order_id` can drive any working order to FILLED with a well-formed sequence, and the resulting `BROKER_EVENT_APPLIED` events are chained, so the audit trail attests the fabrication as if genuine.
- No **per-submission-attempt ID** exists (A5 F-A5-05): broker-side idempotency rests solely on `client_order_id`, so an operator resubmission after UNKNOWN reconciliation could double-fill at a broker that dedupes differently.
- Audit chains are internal and unsigned (A5 F-A5-06; challenge 4 → ADR-M1-010).

---

## 2. Alternatives considered (with rejection reasons)

| # | Alternative | Rejection reason |
|---|---|---|
| A | Make Rust `paper-engine`/`account-journal` the authoritative OMS | Rejected. It is a deterministic *simulator*: no durable intent/outbox, no client-order idempotency store, no UNKNOWN reconciliation protocol for live state, and no broker-query authority. Its `OrderStatus` is a projection view, not a lifecycle ledger. Demote to simulation. |
| B | Make Python `oms.py` (reference OMS) the authority | Rejected. It has a JSONL chain but no transactional outbox/lease/unknown-quarantine discipline; `durable_oms.py` is the richer durable implementation (A5 §7). Retain as readable spec, not authority. |
| C | Choose Python `durable_oms` as authority but let `apply_broker_event` remain the reconciliation authority as-is | Rejected. R2 Q6: caller-supplied events with no authenticity check cannot be a reconciliation *authority*; they can forge terminal state and the chain attests it. Reconciliation authority must be separated and provenance-checked. |
| D | Define a broker-query adapter and networked reconciliation now | Rejected. No broker adapter exists; ADR-0001/Amendment A5 forbid new services/ports without a measured requirement; R1 Q4 warns of SPOFs. Reconciliation authority is **defined** here; the adapter is a future, separately-authorized capability. |
| E | Force all three into one language/component during M1 | Rejected. M1 is design-only; A5 §7 explicitly declines to unify languages. Authority is assigned by decision, not code merge. |

---

## 3. Decision

**The single authoritative lifecycle owner is the Python `durable_oms.DurableOMSStore`.** It owns order lifecycle state, idempotency and the outbox for the whole platform design. `oms.py` and the Rust `paper-engine`/`account-journal` are explicitly **demoted** (§3.4). This ADR defines the *contract*; it implements nothing.

### 3.1 Complete order state machine (authoritative vocabulary)

Order states (from `durable_oms.py:132`, aligned with `oms.py:361` and `model.rs:257-267`):

`PENDING_NEW` → `WORKING` → `PARTIALLY_FILLED` → `FILLED` (terminal)
with `CANCEL_PENDING` → `CANCELLED` (terminal), `REJECTED` (terminal), `EXPIRED` (terminal), and `UNKNOWN` (non-terminal, resumable).

Normative transitions (authoritative source: `durable_oms.py:493-534` transition table, mirrored by `oms.py:350-413` and `model.rs:permits()`):

| From | Permitted next states |
|---|---|
| PENDING_NEW | WORKING, PARTIALLY_FILLED, FILLED, CANCELLED, REJECTED, EXPIRED, UNKNOWN |
| WORKING | WORKING, PARTIALLY_FILLED, FILLED, CANCEL_PENDING, CANCELLED, REJECTED, EXPIRED, UNKNOWN |
| PARTIALLY_FILLED | PARTIALLY_FILLED, FILLED, CANCEL_PENDING, CANCELLED, EXPIRED, UNKNOWN |
| CANCEL_PENDING | CANCEL_PENDING, WORKING, PARTIALLY_FILLED, FILLED, CANCELLED, UNKNOWN |
| UNKNOWN | WORKING, PARTIALLY_FILLED, FILLED, CANCEL_PENDING, CANCELLED, REJECTED, EXPIRED, UNKNOWN |
| FILLED / CANCELLED / REJECTED / EXPIRED | terminal — no outgoing transitions |

**UNKNOWN semantics (normative):** an ambiguous submission or cancel outcome after dispatch transitions the order (and its outbox command) to UNKNOWN (`durable_oms.py:397-421`, `:238-256`). UNKNOWN is **never auto-retried**: a DISPATCHING command whose lease expires is converted to UNKNOWN, not requeued (`:236-256`); `PaperOutboxDispatcher` marks UNKNOWN on any exception after the broker call may have started (`:643-648`). A late cancel timeout must not overwrite a terminal fill/cancel (`:405-411`). UNKNOWN is resumable only via authenticated reconciliation (§3.3). Terminal states are immutable.

Outbox command states (authoritative: `durable_oms.py:130`): `PENDING → CLAIMED → DISPATCHING → ACKNOWLEDGED`, or `DISPATCHING/CLAIMED → UNKNOWN`, or `→ REJECTED`. CLAIMED (no side effect yet) is safely reclaimable on lease expiry (`:236-237`); DISPATCHING (side effect may have escaped) is never reclaimed (A5 F-A5-07).

### 3.2 Idempotency keys (normative)

| Key | Meaning | Rule |
|---|---|---|
| `client_order_id` | Stable client-supplied logical order key | Unique; reuse with a different intent/broker → `IdempotencyConflict` (`oms.py:239-241`; `durable_oms.py` FK). Broker idempotency anchor today. |
| `broker_order_id` | Broker-assigned id on submit ACK | Unique per broker; immutable once bound; mismatch → conflict (`oms.py:266-270`; `durable_oms.py:390-393`). |
| `event_id` / `execution_id` | Deduplication of broker events / fills | Strictly increasing sequence + unique ids; replay with different content → `IdempotencyConflict` (`durable_oms.py:480-539`). |
| `outbox_id` | Stable per-command key | One outbox command per (client_order_id, command_type); lease/attempts tracked (`durable_oms.py:130-145`). |
| **`submission_attempt_id`** | **NEW token to be defined in a future milestone** — distinct per broker submission attempt | Full specification in §3.2.1. **Absent at baseline; this is a contract requirement, not an M1 implementation and not an M1 schema file** (charter §3.6). Required before any live broker adapter; its broker idempotency assumption is a **PRECONDITION**. |

### 3.2.1 `submission_attempt_id` — full specification (future contract; challenger A-3/B-2)

All fields below are a **design specification for a future milestone**, not an M1 schema file, code path or capability (challenger A-5 charter-fit; see ADR-M1-001 §3.5/§8.7).

| Aspect | Specification |
|---|---|
| **Field name** | `submission_attempt_id` (string). Distinct from `client_order_id`, `broker_order_id`, `outbox_id` and `event_id`. |
| **Format / generation rule** | Opaque 128-bit random token (UUIDv4 or `os.urandom(16)` hex). **Never derived from** `client_order_id`, `broker_order_id`, `attempts` counter or any local sequence; uniqueness, not monotonicity, is required. A new value is minted for **each** adapter submission/cancel invocation; a retry of the same physical attempt reuses the same value. |
| **Persistence location** | A column `submission_attempt_id TEXT` on the outbox row (`oms_outbox`), plus the value recorded in the chained `oms_events` body of `SUBMISSION_ATTEMPT_STARTED`, so the audit chain binds attempt id → outbox id → worker. (This column/event-body addition is a future schema change, not an M1 change.) |
| **Mint point relative to `mark_dispatch_started`** | `claim_next` (CLAIMED) → **mint `submission_attempt_id` and persist it atomically with `mark_dispatch_started` in the same `BEGIN IMMEDIATE` transaction** (DISPATCHING) → **only then** call `adapter.submit(..., submission_attempt_id=<that value>)`. Persisting before the external side effect means a crash after mint but before the call leaves a recorded, unresolvable attempt that DISPATCHING→UNKNOWN already refuses to blind-retry. This also bounds the CLAIMED→DISPATCHING window of A5 F-A5-07: no adapter side effect before DISPATCHING is committed. |
| **Broker idempotency contract assumed** | The certified broker adapter must accept a caller-supplied idempotency key and deduplicate on `(broker, submission_attempt_id)`: a resend with the **same** attempt id is deduplicated (no second order), while a **new** attempt id after authenticated reconciliation is a distinct, new submission. Baseline has **no such contract** — `broker-capabilities.v1.json` declares no idempotency field and no live broker adapter exists (A5 F-A5-05). **This is a PRECONDITION on the future certified adapter, not a property of M1.** |

**Reconciliation dependency:** the "resubmission after UNKNOWN reconciliation" scenario changes from *impossible today* to *possible only after* the authenticated-reconciliation precondition (§3.3) exists. Until then the post-reconciliation resubmission case is unreachable; AC-003-5 is labelled accordingly (§9).

### 3.3 Reconciliation authority (explicit; no broker-query adapter exists today)

- **Baseline fact (stated plainly, challenger B-2):** `apply_broker_event` (`durable_oms.py:456-550`) has **no authenticity mechanism at all**. It validates shape, strictly increasing `sequence`, unique `event_id`/`execution_id`, the transition table and overfill only; there is no signature parameter, verifier, attestation field or broker-query adapter. A caller knowing `broker_order_id`/`client_order_id` can drive any WORKING order to FILLED with a well-formed sequence, and the resulting `BROKER_EVENT_APPLIED` events are chained, attesting the fabrication as if genuine (R2 Q6). **M1 does not close this bypass.** M1-ARCH-INV-018 below is a **requirement, not an implemented control**; no text here claims forged FILLED is prevented today.
- **Authenticity requirement (future, PRECONDITION):** before `apply_broker_event` may set terminal state in authoritative mode, it must validate provenance — a signature or trusted-adapter attestation binding the event to the broker, in addition to the existing checks. A caller-supplied unauthenticated event must then be rejected (accepted only in explicitly non-authoritative paper mode) and must never drive a working order to FILLED. **No signature field, verifier or broker adapter exists at baseline.**
- **Only component permitted to resolve UNKNOWN** (future): an authenticated reconciliation input — a signed broker-query result or signed broker event from a trusted broker adapter. `reconcile.py:1-40` is market-data only and **not** the reconciliation authority; Rust `engine.rs:759-828` compares against a caller-supplied `ReconcileInput` and is a simulator path, not the authority.
- **Fail-closed today:** because no broker-query adapter is present, UNKNOWN orders remain UNKNOWN indefinitely; no automatic resolution, no retry, no cancel. This is acceptable *only* under the hard-disabled live gate (`ADR-0004`).

**PRECONDITION — not an M1 control (challenger A-3/B-2):** the authenticated-reconciliation capability, its broker adapter, the `submission_attempt_id` broker contract (§3.2.1) and the chain anchor require (a) a **certified broker-adapter / reconciliation owner role** (new; to be named by the repository owner) and (b) a **measured-requirement ADR authorizing broker connectivity** — target **M3 or the first broker-connectivity milestone, currently unscheduled**. Until both exist, UNKNOWN is terminal-by-omission; the post-reconciliation resubmission case in AC-003-5 is unreachable and is labelled PRECONDITION in §9.

- Gap detection (future requirement): a missing broker event must be detectable (current code cannot detect a gap until a later higher-sequence event, `durable_oms.py:491-492`).

### 3.4 Explicit demotions

- **`oms.py`** → *reference/specification state machine*. Its transition table and `BrokerAdapter` Protocol (including `query_order`, `oms.py:80-86`) are the readable contract; it does **not** own persistent lifecycle state. No second persistent lifecycle store may be introduced from it.
- **`paper-engine` / `account-journal`** → *deterministic simulation and accounting projection*. `OrderStatus`/`OutcomeUnknown` (`model.rs:257-318`) is a simulator view with no broker authority; cash/positions/fees are recomputed from `FillBooked`, and `daily_pnl` is an observed value (`state.rs:573-577`, `engine.rs:971`) that is **not authoritative for order state or verified P&L** (challenge 5 → ADR-M1-009).

### 3.5 Boundary with other challenges

- **Challenge 4 (durability):** acknowledgement meaning is *`sync_data()` returned* for journal appends and *commit returned* for the transactional intent+outbox+event write (`durable_oms.py:192-226`); DISPATCHING means "a side effect may have escaped". These survive a crash as defined in `DurableOMSStore`; system-level RPO/RTO/fault model and off-host chain anchor are Governance ADR-M1-008/010.
- **Challenge 5 (replay):** the authoritative OMS chain records order events; P&L is not derived there and must not be copied into a "verified-equal" replay result (ADR-M1-009).

---

## 4. Ownership

- **Decision owner:** Architecture Design Group (M1 Platform Constitution), ratifiable by repository owner.

**Owner role for every state this ADR creates (challenger Sections C/F.8):**

| State created | Owner role | Person |
|---|---|---|
| Order lifecycle state, outbox state, idempotency keys, event chain, `submission_attempt_id` contract (D7) | *Execution / OMS owner* (D7 role; `capabilities.json` CAP-011/CAP-016 = "execution") | **Not named — owner-ratification condition at M1 exit gate** |
| Authenticity attestation of reconciliation inputs (D3 boundary) | *Risk & Authorization owner* (D3 role) | **Not named — owner-ratification condition at M1 exit gate** |
| Certified broker-query/adapter + reconciliation owner (needed to make §3.3 PRECONDITION satisfiable) | **New role — does not exist at M1** | **Not named; requires repository-owner ratification + measured-requirement ADR (M3)** |
| Future outbox column / chained event body for `submission_attempt_id` (§3.2.1) | *Execution / OMS owner* (D7) at the future milestone | **Not named — owner-ratification condition; no schema file created in M1** |

- **Ratification condition:** naming the natural persons for the D7 and D3 roles is a **repository-owner action at the M1 exit gate**, not a design output and not a person this group may invent. Absent ratification, §8.6 rejects this ADR.
- **State created by this decision:** the authoritative-lifecycle designation, state machine and attempt-ID contract (design record in this workspace). No repository file, DB, schema or service is created in M1.

---

## 5. Invariants

These are group-scoped IDs within ADR-M1-001..004 (`M1-ARCH-INV-*`). Challenger A-0 is RESOLVED: the coordinator's global registry (`INVARIANT-REGISTRY.md`) assigns unique namespaces across all 11 ADRs with zero duplicate definitions; all citations here resolve unambiguously.

- **M1-ARCH-INV-015 — Single lifecycle owner.** `DurableOMSStore` is the sole authoritative order-lifecycle and idempotency owner; no second persistent lifecycle store.
- **M1-ARCH-INV-016 — Terminal immutability.** FILLED/CANCELLED/REJECTED/EXPIRED never transition; UNKNOWN is the only resumable non-terminal outcome.
- **M1-ARCH-INV-017 — No blind retry.** A DISPATCHING or UNKNOWN command is never automatically resent; only authenticated reconciliation may resolve UNKNOWN (A5 CLM-009).
- **M1-ARCH-INV-018 — Authenticated reconciliation (REQUIREMENT / unimplemented precondition, NOT an M1 control).** No caller-supplied event may set terminal state without broker authenticity attestation in authoritative mode (R2 Q6). **No such mechanism exists at baseline; `apply_broker_event` has no authenticity check and the forged-FILLED bypass is open. This invariant is aspirational until §3.3's precondition is met.**
- **M1-ARCH-INV-019 — Distinct submission attempt (future contract / PRECONDITION).** Every broker submission attempt has a distinct persisted `submission_attempt_id` used as the broker idempotency key. **Full spec in §3.2.1; not implemented at baseline and not reachable without the reconciliation precondition.**
- **M1-ARCH-INV-020 — Observe-then-dispatch order.** An adapter side effect may occur only after `mark_dispatch_started`; the CLAIMED→DISPATCHING window is not store-enforced today and must be governed by protocol (A5 F-A5-07).
- **M1-ARCH-INV-021 — P&L not lifecycle truth.** Observed `daily_pnl` is not authoritative order state and is excluded from verified-equality claims (challenge 5).

---

## 6. Failure cases (concrete, including adversarial)

1. **SUBMIT timeout → UNKNOWN.** Broker call raises after dispatch begins; `mark_unknown` sets order and outbox UNKNOWN (`durable_oms.py:397-421`, `:643-648`). No auto-retry; `request_cancel` refused (`:283-284`); order stuck until authenticated reconciliation. *Response:* M1-ARCH-INV-016/017; reconciliation precondition §3.3.
2. **Adversarial — forged terminal state (R2 Q6).** A caller invokes `apply_broker_event` with a well-formed WORKING→PARTIALLY_FILLED→FILLED sequence for a known `broker_order_id`, no signature. Current code accepts it and chains `BROKER_EVENT_APPLIED`, attesting the fabrication. **M1 does not close this bypass** (§3.3). *Response:* M1-ARCH-INV-018 is a requirement, not an implemented control; AC-003-4 is a FUTURE gate spec requiring the unimplemented authenticity mechanism. A "non-authoritative paper mode" escape hatch is not a property of the code and is not accepted as mitigation.
3. **Duplicate resubmission after UNKNOWN (F-A5-05).** Operator concludes the order was never placed and resubmits with the same `client_order_id`; a broker that does not dedupe on that field accepts two orders. **This scenario is unreachable today because UNKNOWN cannot be reconciled (§3.3).** *Response:* M1-ARCH-INV-019 / §3.2.1 define the future contract; AC-003-5 is labelled PRECONDITION.
4. **Lease expiry in CLAIMED vs DISPATCHING.** CLAIMED expired → safely reclaimed (`durable_oms.py:236-237`); DISPATCHING expired → UNKNOWN (`:238-256`). A buggy worker performs a side effect between `claim_next` and `mark_dispatch_started`, crashes, and the reclaimed CLAIMED command is redispatched → duplicate order (A5 F-A5-07). *Response:* M1-ARCH-INV-020; dispatch-token requirement.
5. **Late cancel timeout overwrites a fill.** A cancel timeout arrives after an authoritative FILLED; current code guards with a terminal-status check (`durable_oms.py:405-411`, `:246-250`). *Response:* M1-ARCH-INV-016; regression test AC-003-3.
6. **Three-owner divergence (F-A1-03).** Mixed use of `durable_oms` and `paper-engine` gives different dedup semantics with no canonical store. *Response:* M1-ARCH-INV-015; demotions §3.4.
7. **Unsigned-chain rewrite (challenge 4).** A host compromise rewrites all order events with recomputed hashes; `verify_event_chain()` returns `verified_internal_integrity` with `signature:False` (`durable_oms.py:562-578`). *Response:* off-host anchor in ADR-M1-010.
8. **P&L laundering (challenge 5).** A replay compare reports a `daily_pnl` copied from `RiskStateObserved` as verified-equal (`state.rs:573-577`, `engine.rs:971`). *Response:* M1-ARCH-INV-021; ADR-M1-009.

---

## 7. Dependencies

- **Other ADRs:** ADR-M1-001 (D7 owner; D3/D9); ADR-M1-002 (authorization verdict consumed as data, never self-authorization); ADR-M1-004 (no broker/port built now); Governance ADR-M1-008 (ACK/fault model), ADR-M1-009 (no copied-forward P&L), ADR-M1-010 (off-host chain anchor), ADR-M1-011 (evidence status). Runtime ADR-M1-005/006 (order/event identity and partition semantics).
- **Evidence (`01eb382e`):** `durable_oms.py:130-145,192-271,283-291,390-421,456-550,562-578,592-649`; `oms.py:32,66,80-86,242-247,350-413`; `trading-core/src/model.rs:257-318`; `paper-engine/src/engine.rs:354-368,520-540,755-830,955-971`; `state.rs:113-116,573-577`; `reconcile.py:1-40`; A1 F-A1-03; A5 F-A5-04/05/06/07/08; R2 Q3/Q6.
- **External capabilities (preconditions, not authorized):** a certified broker-query/adapter with authenticity attestation; a broker idempotency contract; off-host anchor and signing key. **None exists at baseline.**
- **Governing docs:** `target-architecture.md:56-63` (OMS owns lifecycle/idempotency; timeout = unknown; reconcile before retry; mismatches fail closed); `ADR-0004-live-disabled.md`.

---

## 8. Rejection conditions

This decision is overturned or revised if new evidence shows:

1. `durable_oms` is demonstrated to lose structural atomicity under the crash profile of interest (no CLM-009 crash-injection test exists today, A3-06; challenge 4) — then another store or an added durability contract is required.
2. A measured requirement mandates a distributed/multi-node OMS that a single-node SQLite store cannot satisfy — then the authority choice is revisited (and a new ADR decides consensus/replication).
3. Source at `01eb382e` is shown to contain a fourth lifecycle owner or a working broker-query adapter — the inventory (§1/§3.4) is inaccurate.
4. R2 Q6 is refuted by executed evidence that `apply_broker_event` already authenticates broker provenance — M1-ARCH-INV-018 is relaxed.
5. A certified broker adapter and reconciliation authority are delivered under a separate ADR with a different owner — §3.3 is promoted from precondition to implementation and re-adjudicated.
6. Owner assignment of D7 is refused or reassigned — §4 is void.
7. A measured-requirement ADR authorizes broker connectivity and names a certified broker-adapter / reconciliation owner — then §3.3's PRECONDITION (and AC-003-4/5) is promoted to an implementation milestone and re-adjudicated; M1 still claims no such control.
8. A charter-fit ruling declares the §3.2.1 field/column/event-body additions a forbidden schema change — then the attempt-ID contract is withdrawn from this ADR and routed through the ADR-M1-004 §3.2 gate (owner ruling, challenger A-5).
9. Executed evidence shows `apply_broker_event` already enforces authenticity at baseline (refuting R2 Q6) — then the §3.3 baseline-fact paragraph and M1-ARCH-INV-018's "requirement only" status are revised.

---

## 9. Measurable acceptance requirement (design only — not executed in M1)

These are **future gate specifications, not delivered properties and not executed in M1**. Every named artifact has an owner role and target milestone (challenger Section C); requirements depending on nonexistent external capabilities are labelled **PRECONDITION — not an M1 gate**.

| ID | Named test / command (future) | Threshold | Owner (role) | Target milestone | Status |
|---|---|---|---|---|---|
| AC-003-1 | `python -m pytest -q research/python/tests/test_durable_oms.py -k "unknown or reclaim or dispatch"` — require ≥1 test executed (`-k` must not deselect all); assert UNKNOWN after timeout, no auto-requeue, cancel refused | 0 automatic retries of DISPATCHING/UNKNOWN; 0 cancellations accepted while UNKNOWN | *Execution / OMS owner* (D7) | M2 | Future gate spec; parts are already source-true but not re-run here |
| AC-003-2 | Static ownership test: `grep` for persistent lifecycle writes outside `durable_oms.py` | 1 authoritative lifecycle store; `oms.py`/`paper-engine` write 0 | *Execution / OMS owner* (D7) + *Verification owner* (D9) | M2 | Future gate spec |
| AC-003-3 | Terminal-immutability and late-timeout regression (mirrors `durable_oms.py:405-411`) | 0 transitions out of terminal states; 0 terminal states overwritten by a late cancel | *Execution / OMS owner* (D7) | M2 | Future gate spec |
| AC-003-4 | Adversarial negative test (R2 Q6): unauthenticated well-formed `apply_broker_event` driving WORKING→FILLED | When implemented: 0 unauthorized terminal transitions; event rejected | *Execution / OMS owner* (D7) + *Risk & Authorization owner* (D3) | M3 | **PRECONDITION — not an M1 gate** (requires unimplemented authenticity mechanism + certified adapter owner). At baseline the forged event is ACCEPTED; bypass open (§3.3) |
| AC-003-5 | Idempotency test: resubmit after UNKNOWN reconciliation, assert distinct `submission_attempt_id` (§3.2.1) passed as broker key | When implemented: 1 unique attempt id per submission; 0 duplicate orders | *Execution / OMS owner* (D7) + future broker-adapter owner | M3 | **PRECONDITION — not an M1 gate** (reconciliation unreachable today; no broker idempotency contract) |
| AC-003-6 | Dispatch-order test (A5 F-A5-07): side effect between CLAIMED and DISPATCHING, then crash/reclaim | 0 duplicate adapter `submit` calls for one `client_order_id` | *Execution / OMS owner* (D7) + *Verification owner* (D9) | M3 | Future gate spec — test seam not yet specified; depends on dispatch-token mechanism |

### Amendment-1 challenger disposition (ADR-M1-003)

- **A-3 (reconciliation impossible vs normative) — ACCEPTED.** Exact text changes: §3.2.1 fully specifies `submission_attempt_id` (field name, UUIDv4 format/generation, `oms_outbox` column + chained `SUBMISSION_ATTEMPT_STARTED` body persistence, mint atomically with `mark_dispatch_started` before any adapter call, `(broker, submission_attempt_id)` broker idempotency assumption); §3.3 and §9 label authenticated reconciliation and AC-003-4/5 **PRECONDITIONS** with owner role (new certified broker-adapter/reconciliation owner, to be ratified) and target M3 / first broker-connectivity milestone. The post-reconciliation resubmission case is now explicitly unreachable until that precondition exists.
- **B-2 (forged FILLED, unimplemented control) — ACCEPTED.** Exact text changes: §3.3 opens with a plain baseline-fact statement that `apply_broker_event` has no authenticity mechanism and that **M1 does not close the bypass**; M1-ARCH-INV-018 is annotated "REQUIREMENT / unimplemented precondition, NOT an M1 control"; §6 case 2 states the bypass is not closed and rejects the "non-authoritative paper mode" escape hatch; §9 AC-003-4 is a PRECONDITION with the baseline-failure disclosed. No claim of closure is made.
- **Section C (untestable §9) — ACCEPTED.** §9 gained `Owner (role)`, `Target milestone`, `Status` columns; AC-003-4/5 labelled PRECONDITION.
- **CONTEST (narrow):** the challenger's remediation "mark M1-ARCH-INV-019 and AC-003-5 as
  unimplemented preconditions with no acceptance claim" is adopted, but this ADR does **not**
  withdraw the *specification* of `submission_attempt_id`: the challenger's Section F.4 offers
  "fully specify" OR "mark as precondition"; this ADR does both — the spec is a design contract for
  M3, not an M1 acceptance claim. No external capability is assumed to exist.

*End of ADR-M1-003.*
