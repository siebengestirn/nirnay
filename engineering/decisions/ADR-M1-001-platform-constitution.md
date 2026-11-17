# ADR-M1-001 — Platform constitution: domains, ownership, non-responsibilities and dependency policy

- **Status:** Proposed (M1 design only — not implementation authority)
- **Owner group:** Architecture Design Group, M1 Platform Constitution
- **Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9` (`main`), read via `git show 01eb382e:<path>`
- **Governing inputs:** M0 `CONSOLIDATED-BASELINE.md` §2, §4, §8, §11, §13, §14 and **Amendment A (GOVERNING)** A3/A5/A6/A9; `engineering/target-architecture.md`; `engineering/decisions/ADR-0001-modular-monolith.md`; `architecture/layers.toml`; A1 (F-A1-01/03/05/07); R1 (Q1a/Q1b/Q3/Q4/Q5); R2 (C1/C2)
- **Evidence levels (M0 convention):** A documentation · B source · C test · D CI · E execution · F operational. M0 highest evidence in this scope is **B + C**; D is CI wiring only; E/F are absent. No claim of platform readiness is made here.
- **Amendment 1 (2026-11-17):** Independent-challenger disposition recorded in §9 (subsection "Amendment-1 challenger disposition"). Invariant-ID collisions (challenger A-0) are **RESOLVED**: the coordinator assigned the `M1-ARCH-INV-*` namespace and published `INVARIANT-REGISTRY.md` (zero duplicate definitions). Cross-references in this ADR were updated accordingly.

---

## 1. Problem

M0 established a well-factored research/paper codebase but **no platform constitution**: responsibilities overlap, three OMS implementations own the same lifecycle state (A1 F-A1-03; baseline §4/§13.7), three authorization owners are unconnected (A5 §7; baseline §8), and the two most safety-critical control crates — `risk-engine` (pre-trade risk) and `execution-policy` (signed permits) — have **no declared owner** (A1 F-A1-07, S4 with S1 exposure; baseline §2 "ownership gaps"). The declaration "hexagonal architecture enforced by layers.toml + check_repo.py" is only nominally true (baseline §2; A1 F-A1-01/04/05/08).

The specific problems this ADR must settle:

1. **No authoritative map of domains** stating, per domain, its purpose, non-responsibilities, owned state, public interfaces, permitted/forbidden dependencies, failure behaviour and operational owner. Without it, "which component owns this fact?" is unanswerable (baseline §4 state-ownership matrix shows this ambiguity for order lifecycle, authorization, P&L and event identity).
2. **Ownership vacuum at the control boundary.** `capabilities.json` names 9+ owners but none names `risk-engine`, `execution-policy`, `xtask` or the four foundation/domain crates (A1 §7, F-A1-07). A defect or waiver in the independent risk authority has no accountable reviewer.
3. **Boundary confusion.** Conflating the *target* hexagonal design (ports/adapters) with the *current* modular-monolith contract (crate boundaries + data formats + process boundaries) produced two opposite errors: A1/R1 treating "no ports" as a code defect, and R1 arguing ports are not programme-required at all. Amendment A3 ruled that the governing charter **does** require hexagonal ports/adapters, that this is a gap relative to the target constitution, and that **no port implementation is authorized in M0/M1**; M1 must decide by ADR with a measured requirement (ADR-M1-004).

This ADR defines the constitution. **It authorizes no port, no service, no schema file, no crate, no optimization, no feature, no broker connection and no live-routing change** (challenger Section D / finding A-5 charter-fit). Whether the mandatory identity-field surface defined in ADR-M1-005 (Runtime group) constitutes a forbidden "schema change" at the design level is **not this ADR's ruling**; it is recorded as an owner charter-fit question in §3.5 and §8.7.

---

## 2. Alternatives considered (with rejection reasons)

| # | Alternative | Rejection reason |
|---|---|---|
| A | Declare a constitution that **includes** port interfaces as current, enforced contracts | Rejected. No production `dyn` port trait exists (A1 §3.3; only `CheckedPaise` numeric trait at `trading-core/src/money.rs:17`); enforcing a non-existent contract is the "nominally true" defect Amendment A3/A5 forbids. Ports are a *target*, gated in ADR-M1-004. |
| B | Adopt R1's position that ports are not programme-required and reduce this to a documentation reword only | Rejected. Amendment A3 explicitly **rejects** R1's premise: the governing charter requires "hexagonal architecture, explicit ports and adapters." R1's valid nuance (not a violation of current ADR-0001; no implementation authorized) is adopted, but its "no requirement" premise is not. |
| C | Unify all overlapping responsibilities by merging crates/languages into one OMS/authority component now | Rejected. M0/ADR-0004 forbid feature/implementation work in M1; A5 §7 explicitly declines to force Python and Rust into one language. This ADR assigns *authority* and *ownership* by decision (ADR-M1-002/003), not by code merge. |
| D | Leave `risk-engine`/`execution-policy` ownerless pending an organizational decision | Rejected. An unowned critical control is itself an S1-exposure finding (F-A1-07). The constitution must name an accountable owner role and make ratification an exit condition, not defer it. |
| E | Treat every crate in `layers.toml` as a separate "domain" (one owner per crate) | Rejected. Over-fragments the model; contradicts ADR-0001 ("a crate boundary must correspond to a stable contract, independent ownership, meaningful dependency rule or separate verification need"). Domains are coalesced around stable contracts and state, with crates mapped into them. |

---

## 3. Decision

### 3.1 Foundational rule

**Crate boundaries are the current modular-monolith contract.** Per ADR-0001 (`engineering/decisions/ADR-0001-modular-monolith.md:1-12`), the stable contract today is the crate boundary, the data format (JSONL, NRJ1 frame, raw-frame schema) and the process boundary — not a port trait. `architecture/layers.toml:1-3` (strictly-downward ranks: foundation 0 → domain 1 → storage 2 → services 3 → engines 4 → strategies 5 (empty) → adapters 6 → apps 7 (leaf)) correctly forbids outward edges (affirmed by R1 Q1a and the withdrawal of PRE-002, baseline §2), but a rank scheme alone does not create dependency inversion (R1 Q1a nuance adopted). **New ports, services, schemas or infrastructure require a measured requirement and a separate ADR** (ADR-0001; Amendment A5; ADR-M1-004). Nothing in this constitution authorizes implementing them. Stated positively for the exit record (challenger finding A-5, Section D): **ADR-M1-001 and ADR-M1-004 authorize no port, no service, no schema file, no optimization, no feature implementation, no broker connection and no live-routing change; no platform-readiness claim; no automatic start of implementation.** The mandatory field surface in ADR-M1-005 is a Runtime-group design contract and is not authorized or ruled on here.

### 3.2 Architectural domains

Nine domains. Each row states purpose, non-responsibilities, owned state, public interface, permitted/forbidden dependencies, failure behaviour and operational owner role. Crates map into domains; a crate may not implement a responsibility owned by another domain.

| Domain | Purpose | Non-responsibilities | Owned state | Public interface | Permitted deps / Forbidden deps | Failure behaviour | Operational owner (role) |
|---|---|---|---|---|---|---|---|
| **D1 Foundation & Domain** | Exact money, instruments, effective-dated terms, venue-neutral order/book contracts (`market-types`, `india-domain`, `trading-core`, `market-book`) | No filesystem, network, DB, time source, strategy or broker logic | Value types only; no mutable lifecycle state | Rust types (`Paise`, `OrderStatus`, `Order`, `OrderBook`) | Permitted: none internal (foundation)/lower domain only. Forbidden: all infrastructure (A1 §3.1) | Pure functions; invalid values cannot be constructed; no runtime recovery | *Foundation/Domain owner* (role — to be named by repository owner; currently only generic "domain" in `capabilities.json`) |
| **D2 Storage & Durability** | Append-only journals, hash chains, replayable frames (`account-journal`, `market-archive`) | No order decisions, no risk verdicts, no order lifecycle transitions | Journal bytes, chain heads, continuity/fence metadata, frame integrity | Journal append/verify APIs; NRJ1 frame format; manifest | Permitted: D1. Forbidden: D3/D4/D5/D6 (no upward edge, `layers.toml` storage=2) | Fail-closed on torn/partial frames; poison state required; ACK meaning defined in ADR-M1-008/010 | *Storage owner* (`capabilities.json` CAP-003/account-journal, CAP-005/market-archive) |
| **D3 Risk & Authorization** *(single authority — see ADR-M1-002)* | Independent verification of exact order terms; pre-trade risk evaluation; signed runtime evidence (`risk-engine`, `execution-policy`, Python `live_auth`/`risk_evidence`/`risk_service`) | Does not issue live orders, own order lifecycle, or route; does not synthesize control observations | Trust registry, policy, one-time permit ledger, verification verdict, capability state | `evaluate_risk`, `verify_live_permit`, verified-sources gate | Permitted: D1. Forbidden: D5 strategy internals, D6 broker adapters, D7 OMS internals (`layers.toml` deny edges `risk-*→strategy-*`) | Deny/abstain on any unverified, stale or caller-forged input; **authority boundary fixed in ADR-M1-002** | *Risk & Authorization owner* (**new role — closes F-A1-07**; to be named by repository owner; acceptance conditional on ratification) |
| **D4 Simulation** | Deterministic event-sourced paper replay and accounting projection (`paper-engine`) | Not authoritative for live order lifecycle; not an authorization authority; not a broker | Projection (cash/positions/fees — recomputed from fills); observed `daily_pnl` flagged non-authoritative (challenge 5) | Engine event API, `account_view()`, `ReconcileInput` | Permitted: D1/D2/D3. Forbidden: adapters/broker SDK (engines=4 < adapters=6) | Deterministic, fail-closed; P&L must be recomputed or excluded from "verified-equal" (ADR-M1-009) | *Simulation owner* (`capabilities.json` CAP-004/paper-engine) |
| **D5 Strategy** *(reserved, empty)* | Pure intent/decision logic | No broker/OMS/account/risk internals | Proposed intent only | (future) intent emission | Forbidden: D3, D6, D7 (`layers.toml` deny edges) | Reject any dependency on execution internals | *Strategy owner* (role; crate layer currently empty — `layers.toml:24-26`) |
| **D6 Capture & Adapters** | Provider capture, codecs, network boundary (`upstox-tools`, `upstox-feed-proto`, FYERS recorder) | Does not decide orders, own lifecycle, or authorize | Raw frames, capture manifests, codec state | Raw frame schema, decoder, composition-root binaries | Permitted: D1/D2; network I/O confined here and in apps. `upstox-feed-proto` is a **codec**, not a network adapter (ADR-M1-004) | Accounted, fail-closed capture loss; unsigned manifest digest is an integrity gap (challenge 4 → ADR-M1-010) | *Market-data owner* (`capabilities.json` CAP-006/012/013) |
| **D7 Order Lifecycle / OMS** | Durable order intent, idempotency, outbox, UNKNOWN quarantine (Python `durable_oms`) | Does not authorize orders, run strategy, or query broker autonomously | Order lifecycle state, outbox, idempotency keys, event chain (authority fixed in ADR-M1-003) | `accept_intent`, outbox dispatcher, `apply_broker_event` | Permitted: D1/D3 verdict (as data). Forbidden: self-authorization, direct broker network I/O | Fail-closed; UNKNOWN remains UNKNOWN until authenticated reconciliation (ADR-M1-003) | *Execution / OMS owner* (`capabilities.json` CAP-011/CAP-016 = "execution") |
| **D8 Research** | Python feature/experiment/statistics lab (`quantlab`) | Must not reimplement authoritative money/fees (ADR-0003; F-A1-02) | Canonical events, datasets, experiment records (non-authoritative financial state) | Python modules/CLI | Permitted: imports of D1-equivalent value semantics via ADR-0003 bridge. Forbidden: independent fee arithmetic, broker SDK in domain | No live side effects; no platform-readiness claims | *Research / research-data owner* (`capabilities.json` CAP-008/010/015/018) |
| **D9 Verification & Operations** | Architecture/claims gates, CI, release provenance (`xtask`, `scripts/`, workflows) | Not a runtime domain; does not grant itself authority to verify claims by editing JSON | Gate config, claims/evidence records | `cargo xtask check`, `check_repo.py`, CI | Permitted: read all domains. Forbidden: mutating source; claiming `verified` without executed evidence (challenge 6 → ADR-M1-011) | Fails closed when evidence absent; deny patterns must be non-vacuous (ADR-M1-004) | *Verification / release-engineering owner* (`capabilities.json` CAP-014/019); `xtask` owner gap closed here |

### 3.3 Ownership reconciliation (M0 fact: `risk-engine` and `execution-policy` unowned)

The constitution closes F-A1-07 by assigning both unowned control crates to **D3 Risk & Authorization**, governed by a single named owner role (*Risk & Authorization owner*). Because the repository owner has not yet populated that role, its naming is an **M1 exit-gate condition**, not something this design group may invent. Until ratified, D3 is *owned by role* and its authority decisions in ADR-M1-002 are conditionally binding. The same mechanism closes the `xtask` gap via D9. Foundation/domain crates (D1) receive a concrete owner role rather than the generic "domain" label.

### 3.4 Cross-cutting invariants of the constitution

Domains never share mutable lifecycle state across boundaries without an authority decision. Where M0 showed shared/ambiguous state — order lifecycle (A1 F-A1-03), authorization (A5 F-A5-02), P&L (A5 F-A5-08), event identity (A2 F-01/F-02) — this constitution assigns *one* owner per fact through ADR-M1-002/003 and the Runtime/Governance ADRs, and forbids a second owner from silently arising.

### 3.5 Owner-challenge coverage across the four Architecture ADRs (cross-reference by number)

| Challenge | Where addressed |
|---|---|
| **1. Authorization** — importable risk-token boundary without assuming remote service | ADR-M1-002 (whole ADR; decision + negative test). Constitution maps it to D3. |
| **2. OMS** — one lifecycle/owner, UNKNOWN + reconciliation | ADR-M1-003 (whole ADR). Constitution maps it to D7. |
| **3. Events & partitions** — observation vs business identity; multi-instrument frames; account-global risk | ADR-M1-004 §6 (dependency rule: no partition of account-global risk state; multi-instrument frames unkeyable, `upstox_recorder.rs:238`, `engine.rs:955-971`). Detailed contract deferred to Runtime ADR-M1-005/006 (cross-reference). Constitution assigns capture to D6, aggregate risk to D3/D4. |
| **4. Durability** — ACK meaning, fault survival, trusted off-host anchor | ADR-M1-003 §6 (outbox/journal ACK semantics, UNKNOWN fault survival) and ADR-M1-001 D2 failure behaviour. Detailed ACK/RPO/anchor contract deferred to Governance ADR-M1-008/010 (cross-reference). |
| **5. Replay** — no copied-forward P&L as verified equality | ADR-M1-001 D4 failure behaviour and ADR-M1-003 §6 (authoritative order state vs observed P&L). Detailed replay/compare contract deferred to Governance ADR-M1-009 (cross-reference; `state.rs:573-577`, `engine.rs:971`). |
| **6. Evidence** — maturity vs verification status; fail closed without evidence | ADR-M1-001 D9 failure behaviour, and the Rejection conditions / Acceptance sections of all four ADRs. Detailed evidence model deferred to Governance ADR-M1-011 (cross-reference; A7-001/002). |

**Owner charter-fit question (challenger A-5 / Section D.1):** the challenger asks whether ADR-M1-005's mandatory new identity-field surface (`capture_frame_id`, `observation_id`, `business_event_id`, `origin_class`, `schema_version`) is a forbidden "schema change" at the design level. This is **not in the Architecture group's scope to decide**: ADR-M1-005 belongs to the Runtime group, and the Architecture group neither authors nor rules on that field surface. Record here for the owner ruling at M1 exit; if ruled a schema change, it must be routed through the measured-requirement gate in ADR-M1-004 §3.2 (§8.7 of this ADR).

**Challenger findings outside this ADR's edit scope** (recorded, not silently accepted or rejected): A-2 (ADR-M1-005 false dedup), A-4/B-5 (ADR-M1-009 `pnl_basis` binding), B-3 (ADR-M1-005/006 serial authority), B-4 (ADR-M1-008/010 anchor authority), B-6 (ADR-M1-011 evidence gate), and Section D.2 (ADR-M1-007 optimization design intent). The Architecture group offers no contest on these; they require their owning groups' and the owner's disposition in `M1-RECONCILIATION.md`.

---

## 4. Ownership

- **Decision owner:** Architecture Design Group (M1 Platform Constitution), ratifiable only by the repository owner at the M1 exit gate. This ADR creates no source, crate, schema or service state.
- **State created by this decision:** the domain register and owner-role assignments in §3.2/§3.3. The register is a *design artifact* in this workspace; it is not a mutation of `capabilities.json` or any repository file (M1 is design-only; the four user-owned uncommitted files are untouched).

**Owner role for every state this ADR creates (challenger Section F.8):**

| State created | Owner role | Person |
|---|---|---|
| Domain register and per-domain non-responsibility map (§3.2) | *Architecture / Platform owner* (D9 boundary for gate policy; D1 for port location) | **Not named — owner-ratification condition at M1 exit gate** |
| Trust registry, policy, permit ledger, verification verdict, signing-capability state (D3, owned via ADR-M1-002) | *Risk & Authorization owner* (D3 role; new, closes F-A1-07) | **Not named — owner-ratification condition at M1 exit gate** |
| Order lifecycle, outbox, idempotency and event-chain state (D7, owned via ADR-M1-003) | *Execution / OMS owner* (D7 role; `capabilities.json` CAP-011/CAP-016 = "execution") | **Not named — owner-ratification condition at M1 exit gate** |
| Gate config, claims/evidence records, layer/deny policy (D9) | *Verification / release-engineering owner* (D9 role; `capabilities.json` CAP-014/019) | **Not named — owner-ratification condition at M1 exit gate** |

- **Ratification dependency:** naming the actual natural persons behind **D1, D3, D5 and D9** owner roles is a **repository-owner action at the M1 exit gate**, not a design output and not something this group may invent. Absence of ratification is a recorded rejection condition (§8.3), not a silent assumption.

---

## 5. Invariants

Group-scoped invariant IDs `M1-ARCH-INV-001..028` used within ADR-M1-001..004. The original cross-group collision set (challenger A-0) is **RESOLVED** by the coordinator's renumbering: Runtime uses `M1-RUN-INV-*`, governance ADRs use their own prefixes, and `INVARIANT-REGISTRY.md` is the authoritative registry with zero duplicate definitions. All cross-references and §9 gate citations in this ADR resolve unambiguously to a single definition.

- **M1-ARCH-INV-001 — Single owner per fact.** Every piece of state named in M0 §4 has exactly one domain owner. No two domains may claim the same lifecycle/authorization/P&L/event-identity fact.
- **M1-ARCH-INV-002 — Crate boundary is the current contract.** Until a measured-requirement ADR introduces a port, inter-domain contracts are crate boundaries, data formats and process boundaries only; no port trait is asserted to exist.
- **M1-ARCH-INV-003 — Downward dependencies only.** A domain may depend only on lower-or-equal-ranked domains per `architecture/layers.toml:1-3`; no upward or cyclic edge (A1 §2.2 DAG).
- **M1-ARCH-INV-004 — Control crates are owned.** `risk-engine` and `execution-policy` are owned by D3; an unowned critical control crate is a constitution violation.
- **M1-ARCH-INV-005 — No speculative infrastructure.** No port, service, crate, schema or network/broker change without a measured requirement and a separate ADR (ADR-0001; Amendment A5).
- **M1-ARCH-INV-006 — Owners are non-responsibility-enforcing.** A domain must refuse work that belongs to another domain (e.g., D8 must not reimplement fees; D7 must not self-authorize; D3 must not route).
- **M1-ARCH-INV-007 — Fail-closed defaults.** Ambiguous authentication, reconciliation, durability or evidence states resolve to "deny/unknown/unverified", never to an assumed-success state.
- **M1-ARCH-INV-008 — Maturity ≠ verification.** A capability's implementation maturity is independent of its verification status; absence of executed evidence means unverified (challenge 6; A7-001).

---

## 6. Failure cases (concrete, including adversarial)

1. **Two owners claim order state (recurrence of F-A1-03).** A client uses Python `durable_oms` for intent acceptance while simulation uses Rust `paper-engine`; identical `client_order_id` values get different dedup semantics and no canonical store exists. *Constitution response:* D7 is sole lifecycle owner (ADR-M1-003); D4 is explicitly demoted. Fails the exit gate if a second domain is observed persisting lifecycle state.
2. **Unowned control crate defect.** A waiver in `execution-policy` (whose `OrderRequest` binds no side/venue — `execution-policy/src/lib.rs:71-78`) is merged with no accountable reviewer. *Response:* M1-ARCH-INV-004 assigns D3; absent ratification is a rejection condition.
3. **Boundary laundering via "hexagonal" label.** A reviewer asserts ports/adapters are enforced because `layers.toml` ranks pass, while no port trait exists (A1 F-A1-01/05). *Response:* M1-ARCH-INV-002/005; ADR-M1-004 fixes adapter semantics.
4. **Adversarial — domain overreach.** D5 strategy code imports broker SDK or OMS internals. *Response:* forbidden deps in D5; `layers.toml` deny edges (`strategy-*→broker-*`, `strategy-*→oms*`) must be non-vacuous (ADR-M1-004 acceptance).
5. **Aggregate risk lost by partitioning (challenge 3).** Two-instrument replay partitioned by instrument checks exposure locally; both pass while aggregate `gross_exposure_paise`/`daily_pnl` is breached (`engine.rs:955-971`, `state.rs:113-116`). *Response:* account-global risk stays in D3/D4; ADR-M1-004 forbids partition-local risk authority; detailed rule in ADR-M1-006.
6. **Evidence laundering (challenge 6).** A claim is marked `verified` by editing `claims/ledger.json` without executing the cited command (`check_claims` at `check_repo.py:176-187` checks fields only). *Response:* M1-ARCH-INV-008; ADR-M1-011.
7. **P&L laundered into "verified" (challenge 5).** A replay compare accepts a copied-forward `daily_pnl` (`state.rs:573-577`, `engine.rs:971`) as verified equality. *Response:* D4 rule; ADR-M1-009.
8. **Unsigned off-host anchor absent (challenge 4).** Three internal chains pass while a host compromise rewrites all records (F-A5-06). *Response:* D2 failure behaviour; ADR-M1-010.

---

## 7. Dependencies

- **Other ADRs:** ADR-M1-002 (D3 authority), ADR-M1-003 (D7 lifecycle), ADR-M1-004 (dependency/port rules); Runtime ADR-M1-005/006 (challenge 3); Governance ADR-M1-008/009/010/011 (challenges 4/5/6).
- **Evidence (M0, pinned to `01eb382e`):** baseline §2/§4/§8/§11/§13 + Amendment A3/A5/A6/A9; A1 F-A1-01/03/05/07; A5 §7; R1 Q1a/Q1b/Q3/Q4/Q5; R2 C1/C2.
- **External capabilities:** repository-owner ratification of owner roles (D1/D3/D5/D9). No external service, KMS, mTLS, collector or broker capability is required by this ADR.
- **Governing documents:** `ADR-0001-modular-monolith.md`, `ADR-0004-live-disabled.md`, `target-architecture.md`, `layers.toml`, `capabilities.json`.

---

## 8. Rejection conditions

This decision is overturned or must be revised if new evidence shows any of:

1. The governing charter is amended to **remove** the hexagonal ports/adapters requirement — then §3.1/ADR-M1-004's target-vs-current framing changes (Amendment A3 premise removed).
2. A measured requirement is approved that a domain boundary cannot satisfy (e.g., a second feed/store with a defined acceptance test) — then a port/service decision is required and this constitution's "no ports now" stance is revisited through ADR-M1-004.
3. The repository owner declines to ratify owner roles for D3/D9, or assigns D3 authority to a different domain — §3.3 and the D3 column are void and the affected ADRs (002/003) must be re-based.
4. Source at `01eb382e` is shown to contain a production port trait or a fourth OMS/authority owner — the inventory in §3.2 is inaccurate and the domain map must be redrawn.
5. A domain is demonstrated to own two conflicting statefacts in a way that cannot be separated by authority (e.g., cash and exposure proven both account-global and partition-local with no reconciliation) — M1-ARCH-INV-001 must be refined with Runtime ADR-M1-006.
6. Any executed (E/F) evidence contradicting the B/C-level findings that motivated the domain split (A1 F-A1-03, A5 §7).
7. **Owner charter-fit ruling (challenger A-5 / Section D.1):** if the owner rules that ADR-M1-005's mandatory identity-field surface is a forbidden schema change, it must be routed through the ADR-M1-004 §3.2 measured-requirement gate; this ADR's D8/D1 mapping is unchanged but the field surface is not authorized. This is an **owner ruling**, not a decision the Architecture group makes (§3.5).
8. **Global invariant registry (challenger A-0) — RESOLVED.** `INVARIANT-REGISTRY.md` is authoritative; namespace allocation and cross-reference updates are complete, with zero duplicate definitions. Acceptance citations in §6/§9 are unambiguous. Any future ID reassignment requires a registry update under the same process.

---

## 9. Measurable acceptance requirement (design only — not executed in M1)

These are **future gate specifications, not delivered properties and not executed in M1**. Every named artifact has an owner role and target milestone (challenger Section C); where an external capability is absent it is labelled **PRECONDITION — not an M1 gate**.

| ID | Named test / command (future) | Threshold | Owner (role) | Target milestone | Status |
|---|---|---|---|---|---|
| AC-001-1 | `python3 scripts/check_repo.py arch` (subcommand already exists at `01eb382e:scripts/check_repo.py:210-241`) plus a **new** domain-ownership assertion test over `capabilities.json` | 0 unowned control crates; `risk-engine`, `execution-policy`, `xtask` each map to a named owner role (closes F-A1-07) | *Verification / release-engineering owner* (D9) for gate; *owner-ratification* for the names | M2 (assertion test); owner names at M1 exit | Future gate spec — the new assertion test does not exist at baseline |
| AC-001-2 | Static domain-map conformance test; detector spec must define "fee math" (positive + negative fixtures) and broker-SDK import pattern before threshold is measurable | 0 cross-domain responsibility violations, measured by the specified detector | *Verification owner* (D9) + *Research owner* (D8) for fee-math spec | M2 | Future gate spec — detector undefined at baseline |
| AC-001-3 | `git show 01eb382e:crates/...` inventory test: no production `dyn` port trait in `crates/**` (A1 FT-D) | 0 production port traits asserted as current contract | *Verification owner* (D9) | M2 | Future gate spec |
| AC-001-4 | Owner-ratification checklist at the M1 exit gate | D1/D3/D5/D9 owner roles named by repository owner; otherwise §8.3 rejects this ADR | **Repository owner** (not a design-group gate) | **M1 exit gate (owner action)** | **Owner-ratification condition — not falsifiable by the design group** |

### Amendment-1 challenger disposition (ADR-M1-001)

- **A-5 / Section D (charter-fit) — ACCEPTED.** Exact text changes: §1 closing paragraph now states the ADR authorizes "no port, no service, no schema file, no crate, no optimization, no feature, no broker connection and no live-routing change"; §3.1 gained an explicit positive statement for the exit record; §3.5 records the ADR-005 field-surface question as an owner ruling; §8.7 records the corresponding rejection condition. No schema or field surface is defined here.
- **A-0 (invariant collisions) — RESOLVED by coordinator renumbering (2026-11-17).** §5 now states the group-scoped namespace; the authoritative registry is `INVARIANT-REGISTRY.md`; automated verification found zero duplicate definitions and zero unresolved citations. This ADR cites no IDs from other groups' ADRs.
- **Section C (untestable §9) — ACCEPTED.** Exact text change: the §9 table gained `Owner (role)`, `Target milestone` and `Status` columns; AC-001-4 is labelled an owner-ratification condition, not a design-group gate.
- **CONTEST — challenger Section C representative row for AC-001-1 claims "No `arch` subcommand exists at baseline."** This is factually incorrect. `01eb382e:scripts/check_repo.py:210-241` defines `run(command)` with `if command in {"check", "arch"}:` and a usage string listing `check|arch|waivers|claims`; A1 §3.4 also records `arch` dispatch. The *domain-ownership assertion test* is new and unowned, which is the valid part of the finding; the `arch` subcommand itself exists. Evidence cited above; no change made to the command name.

*End of ADR-M1-001.*
