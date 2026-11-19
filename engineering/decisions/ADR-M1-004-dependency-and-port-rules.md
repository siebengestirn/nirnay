# ADR-M1-004 — Dependency and port rules under the modular monolith

- **Status:** Proposed (M1 design only — **no port implementation authorized**)
- **Owner group:** Architecture Design Group, M1 Platform Constitution
- **Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9`, read via `git show 01eb382e:<path>`
- **Governing inputs:** M0 baseline §2 + Amendment **A3** (F-A1-01 reframed; R1's "no ports required" premise rejected; no port implementation in M0/M1) and **A5** (gated remedies); A1 F-A1-01/04/05/08; R1 Q1a/Q1b/Q3/Q5; `ADR-0001-modular-monolith.md`; `target-architecture.md:28-39,34-36,66-69`; `architecture/layers.toml`
- **Challenge addressed:** **Owner challenge 3 (dependency/ownership portion) — preserve multi-instrument frames and account-global risk semantics; no partition of shared risk state by instrument.** Full event/partition contract is Runtime ADR-M1-005/006 (cross-referenced).
- **Amendment 1 (2026-11-17):** Challenger A-5 charter-fit and Section C disposition recorded in §3.1/§8/§9. **This ADR authorizes no port, service, schema file, optimization, feature, broker connection or live-routing change.** The mandatory identity-field surface in ADR-M1-005 is not in this group's scope to rule on (owner charter-fit question). Invariant-ID collisions (A-0) RESOLVED: `M1-ARCH-INV-*` namespace and authoritative `INVARIANT-REGISTRY.md` applied; cross-references updated.

---

## 1. Problem

M0 reached apparently conflicting conclusions about ports:

- **A1:** "no ports / dependency inversion in Rust" (F-A1-01, S1); the `adapters` layer holds a codec, not a network adapter (F-A1-05); deny edges are vacuous (F-A1-08); the layer gate has blind spots (F-A1-04). Only `CheckedPaise` is a trait (`trading-core/src/money.rs:17`); no production `dyn` port object exists.
- **R1 Q1b/Q5:** the programme mandate never required ports, ADR-0001 makes crate boundaries the contract, and ports are speculative (S4, reword the doc). R1 Q1a affirmed the rank scheme is a correct *dependency-direction* policy.
- **Coordinator ruling (Amendment A3):** R1's premise is **rejected** — the governing charter explicitly requires "hexagonal architecture, explicit ports and adapters" and M1.2 "Adopt hexagonal architecture" with inbound/outbound ports. F-A1-01 is a genuine gap **relative to the target constitution**, but *not* a violation of current ADR-0001, and **no port implementation is authorized during M0/M1**; M1 must decide by ADR with a measured requirement.

Separately, **R1 Q3 / Amendment A4.4** showed two partition-semantics blockers that constrain any future adapter/store dependency design:

1. Every Upstox raw frame is stamped `provider_instrument_id="__multi_instrument_frame__"` (`upstox_recorder.rs:238`) and `payload_sha256` covers the whole decoded payload (`frame.rs:70`). Per-instrument partitioning at the journal layer either collapses to one partition or requires splitting the payload — invalidating the integrity hash and the one-frame→two-event shared-`raw_sha256` identity (A2 F-02).
2. Cash, positions, charges and aggregate risk are **account-global** (`paper-engine/src/state.rs:113-116`, `engine.rs:955-971`): `gross_exposure_paise` and `daily_pnl` are single account-level observations feeding `GrossExposure`/`DailyLossLimit` checks (`risk-engine/src/lib.rs:154-157,273,282`). Per-instrument partition-local replays can each pass their local checks while the aggregate is breached (R1 Q3 falsification).

The problem: decide the port question and dependency rules coherently, without authorizing speculative implementation, and in a way that does not break aggregate semantics.

---

## 2. Alternatives considered (with rejection reasons)

| # | Alternative | Rejection reason |
|---|---|---|
| A | Implement ports now (define inner traits, inject adapters, move network code out of apps) | Rejected. Amendment A3/A5: no port implementation in M0/M1; ADR-0001 forbids new infra without a measured requirement; no second Rust feed/store or perf budget exists. M1 is design-only. |
| B | Adopt R1 Q1b fully: ports are not programme-required; only reword the doc | Rejected. Amendment A3 explicitly rejects the premise — the charter requires hexagonal ports/adapters. R1's valid nuance (not a current ADR-0001 violation; no build now) is adopted; its "no requirement" claim is not. |
| C | Rename the `adapters` layer to `codecs` and leave network I/O in apps permanently | Partially adopted (§3.3 taxonomy) but not a full solution: it documents current reality yet would erase the *target* port concept the charter requires. The target direction is kept and the taxonomy fixed without deleting the goal. |
| D | Let each instrument own a partition with its own risk/limit check | Rejected (R1 Q3 / Amendment A4.4): breaks aggregate cash/exposure/daily-loss semantics and conflicts with unkeyable multi-instrument frames. Forbidden by dependency rule §3.5. |
| E | Make `check_repo.py` enforce full transitive/Python/filesystem boundaries now | Not authorized as implementation; recorded as a measured requirement / gate in §9 and deferred to Verification (ADR-M1-011). This ADR states the rule, not the tooling change. |

---

## 3. Decision

### 3.1 Are ports required, and in what form?

**Yes as a target; no as a current implementation.** The governing charter requires hexagonal architecture with inbound (driving) and outbound (driven) ports and adapters. This ADR establishes the *target contract*:

- **Ports are owned by the inner layer** (domain/application). An outbound port is a trait/interface defined where it is consumed; an adapter in an outer layer implements it and is injected at a **composition root** (the apps, per `target-architecture.md:34-36`).
- **Dependency direction is inward.** Outer layers (adapters, apps) depend on inner ports; inner layers never depend on outer adapters or infrastructure. `layers.toml:1-3` ranks (foundation 0 → … → engines 4 → strategies 5 → adapters 6 → apps 7 leaf) already enforce the outward-edge prohibition (`check_repo.py:61-62`), and A1 §2.2 verified no upward edge exists. **The rank scheme enforces direction but does not create dependency inversion** (R1 Q1a nuance) — ports are the mechanism that will, when a measured requirement appears.
- **Forms (design vocabulary only):** inbound ports = use-case interfaces by which a driving actor (CLI/app) invokes application logic; outbound ports = interfaces for market-data source, durable store, broker/reconciliation, clock and signing authority. No concrete trait, signature or file is specified in M1.
- **Charter-fit, explicit (challenger A-5 / Section D):** **ADR-M1-001 and ADR-M1-004 authorize no port, no service, no schema file, no crate, no optimization, no feature implementation, no broker connection and no live-routing change**, and make no platform-readiness claim. This ADR defines rules and a measured-requirement gate only. Whether ADR-M1-005's mandatory identity-field surface is a forbidden "schema change" is an **owner charter-fit question** (recorded in ADR-M1-001 §3.5/§8.7), not a ruling this group makes; if so ruled, it is routed through §3.2.

### 3.2 Measured-requirement gate for introducing a port (no speculative abstraction)

A port/adapter pair may be introduced **only** when all of the following are satisfied, each evidenced:

Q5).
2. **Two implementations or a named test consumer** exist/will exist at introduction, so the abstraction is exercised, not speculative.
3. **Owner and acceptance tests** for the port and its swap are defined (ADR-0001: "a crate boundary must correspond to a stable contract, independent ownership, meaningful dependency rule or separate verification need").
4. **A new ADR** records the decision. No port is created by this ADR.

**Rule preventing speculative abstraction (M1-ARCH-INV-025):** no trait, crate, service or configuration is introduced "in case" a future consumer appears. `target-architecture.md:38-39`: "Do not create empty crates to represent roadmap work. Add a crate after its API, ownership boundary and acceptance tests are defined." The empty `strategies` layer (`layers.toml:24-26`) stays empty until such a requirement exists.

### 3.3 The adapter layer's true role (codec vs network adapter)

- **Current fact:** `upstox-feed-proto` is a pure prost **codec** — no network I/O, no internal dependents (`upstox-feed-proto/src/decode.rs`; Cargo deps prost/prost-build/protoc-bin-vendored only, `Cargo.toml:10,19-20`). The actual network boundary is the leaf app `upstox-tools` (`upstox_recorder.rs:18-26`: reqwest/tokio/websocket + journal + codec). This is **by design** under `target-architecture.md:34-36` (apps are composition roots) — R1 Q1b is right that network I/O in the app is not a dependency violation (Amendment A3 downgraded F-A1-05 to S4).
- **Decision:** classify `upstox-feed-proto` as a **codec/translation library**, not a port-implementing network adapter. The `adapters` layer is a *destination* for genuine port implementations; until those exist, apps are the real network boundary. This corrects the layer taxonomy **without** moving code (Amendment A5 rejected speculative remedies) and without deleting the target.
- Future genuine adapters implement inner ports (market-data source, broker/reconciliation, durable store, clock, signing authority) at the outer layer and are injected at composition roots — subject to §3.2.

### 3.4 Gate enforcement rule (state, not implementation)

The "hexagonal enforced" claim must be made honest before it is asserted: `check_repo.py` currently checks direct deps only (`--no-deps`, `:68-75`), ignores filesystem (`fs2`/`std::fs` not restricted), ignores Python, and its deny-edge patterns are vacuous for `strategy-*`/`broker-*`/`oms*` (A1 F-A1-04/08). The rule: **a deny-edge pattern that matches no crate must be explicitly marked "future" and must not be counted as active enforcement**; a claim of "hexagonal enforced" requires a non-vacuity self-test. This is a *gate requirement* (§9), not an implementation authorized here; it belongs to Verification/ADR-M1-011.

### 3.5 Dependency rule protecting aggregate semantics (challenge 3)

- **No partition of account-global risk state.** Cash, positions, exposure and `daily_pnl` are account-global (`state.rs:113-116`, `engine.rs:955-971`). No adapter, port or store design may make a per-instrument partition the authority for `GrossExposure`/`DailyLossLimit` or for cash/position correctness. Aggregate checks require an account-level authority (D3/D4 per ADR-M1-001); partition-local checks are insufficient and forbidden as authoritative.
- **Multi-instrument frames stay whole.** Because raw frames are unkeyable by instrument and `payload_sha256` covers the whole payload (`upstox_recorder.rs:238`, `frame.rs:70`), no adapter or port may split a frame for partitioning in a way that invalidates frame integrity or the one-frame→two-event shared-`raw_sha256` identity. Observation identity vs business identity and the full partition/ordering contract are **Runtime ADR-M1-005/006** (cross-referenced); this ADR only fixes the *dependency* consequence.
- **Dependency direction for any future partition** must point inward to the account-global authority, never outward to a partition-local store that owns risk checks.

---

## 4. Ownership

- **Decision owner:** Architecture Design Group (M1 Platform Constitution), ratifiable by repository owner.

**Owner role for every state this ADR creates (challenger Section F.8):**

| State created | Owner role | Person |
|---|---|---|
| Target port form, inward-dependency rule, measured-requirement gate, codec taxonomy, aggregate-semantics dependency rule | *Architecture / Platform owner* (D9-verification boundary for gate policy; D1 for port location) | **Not named — owner-ratification condition at M1 exit gate** |
| Layer/gate policy (`architecture/layers.toml`, `scripts/check_repo.py` policy, target-architecture boundary claims) | *Verification / release-engineering owner* (D9 role; `xtask` assigned to D9 in ADR-M1-001) | **Not named — owner-ratification condition at M1 exit gate** |

- **Ratification condition:** naming the natural persons for the D1/D9 roles is a **repository-owner action at the M1 exit gate**, not a design output and not a person this group may invent. Absent ratification, §8.6 rejects this ADR.
- **State created by this decision:** the design records listed above only. No trait, crate, layer entry, schema file or gate code is created or modified in M1.

---

## 5. Invariants

These are group-scoped IDs within ADR-M1-001..004 (`M1-ARCH-INV-*`). Challenger A-0 is RESOLVED: the coordinator's global registry (`INVARIANT-REGISTRY.md`) assigns unique namespaces across all 11 ADRs with zero duplicate definitions; all citations here resolve unambiguously.

- **M1-ARCH-INV-022 — Crate boundary is the present contract; port is the target.** Current inter-domain contracts are crate boundaries, data formats and process boundaries (ADR-0001); ports are a charter-required target, not implemented now.
- **M1-ARCH-INV-023 — Inward dependency direction.** Outer layers depend on inner ports; inner layers never depend on adapters/infrastructure. Strictly-downward ranks remain; no upward edge.
- **M1-ARCH-INV-024 — Composition roots are apps.** Network I/O and injection live at app boundaries; `upstox-feed-proto` is a codec, not a network adapter, until a real port-implementing adapter exists.
- **M1-ARCH-INV-025 — No speculative abstraction.** A port/crate/service requires a measured requirement, two implementations or a named test consumer, an owner, acceptance tests and a new ADR. (This restates the substance of M1-ARCH-INV-005 from ADR-M1-001 as a distinct, separately enforceable rule. Global ID uniqueness was established by the coordinator's renumbering pass; see `INVARIANT-REGISTRY.md`. No cross-group ID is cited.)
- **M1-ARCH-INV-026 — Deny edges must be non-vacuous or marked future.** A rule matching no crate is not enforcement and cannot support an "enforced" claim.
- **M1-ARCH-INV-027 — Account-global risk is indivisible by partition.** No per-instrument partition may be the authority for exposure/daily-loss/cash checks; multi-instrument frames remain whole for integrity (challenge 3).
- **M1-ARCH-INV-028 — Taxonomy honesty.** Layer names must describe what they contain (codec vs adapter); a mislabeled layer that produces false assurance is a defect to be documented, not coded around in M1.

---

## 6. Failure cases (concrete, including adversarial)

1. **Premise war (A1 vs R1).** A reviewer calls "no ports" either a code defect (A1) or a non-requirement (R1). *Response:* §3.1 follows Amendment A3 — target yes, implement no. Both extreme framings are rejected.
2. **Speculative port introduced "for the future."** A `MarketDataSource` trait and empty adapter are added without a second feed or test. *Response:* M1-ARCH-INV-025; AC-004-2.
3. **Transitive restricted dependency leaks.** A new bridge crate pulls `tokio` into a service; `check_repo.py` direct-only check stays green (A1 FT-C). *Response:* §3.4 gate requirement; AC-004-3 (deferred to Verification).
4. **Vacuous deny edges.** `strategy-*→broker-*` never fires because no such crates exist, yet "enforced" is claimed (A1 F-A1-08). *Response:* M1-ARCH-INV-026; AC-004-4.
5. **Adversarial — partition bypass of aggregate limits (challenge 3).** A two-instrument replay is split; each partition checks exposure against its own book and both pass while the combined exposure breaches `max_gross_exposure_paise` (`engine.rs:955-971`, R1 Q3). *Response:* M1-ARCH-INV-027; aggregate check stays account-level; cross-ref ADR-M1-006.
6. **Adversarial — frame-splitting invalidates integrity.** An adapter splits a `__multi_instrument_frame__` payload per instrument for partitioning; `payload_sha256` no longer matches and the shared `raw_sha256` identity breaks (`upstox_recorder.rs:238`, `frame.rs:70`). *Response:* M1-ARCH-INV-027; cross-ref ADR-M1-005/006.
7. **Mislabeled layer false assurance.** Reviewers trust "adapters" as proof of ports-and-adapters while it holds only a codec (F-A1-05). *Response:* M1-ARCH-INV-028; §3.3 taxonomy.
8. **Filesystem/Python outside enforcement.** A domain crate adds `std::fs` or a research module imports a broker SDK and the layer gate stays green (A1 F-A1-04). *Response:* §3.4 gate requirement (ADR-M1-011).

---

## 7. Dependencies

- **Other ADRs:** ADR-M1-001 (D1/D9 owners; crate-boundary rule); ADR-M1-002 (signing-authority port is a *future* outbound port, not built here); ADR-M1-003 (broker/reconciliation port is a future measured-requirement adapter, not built here); Runtime ADR-M1-005 (observation vs business identity) and ADR-M1-006 (ordering/partition semantics — full challenge-3 contract); Governance ADR-M1-011 (gate enforcement/evidence).
- **Evidence (`01eb382e`):** `architecture/layers.toml:1-3,20-35,40-76`; `scripts/check_repo.py:16-17,61-64,68-75,158-188`; `target-architecture.md:28-39,34-36,38-39,66-69`; `trading-core/src/money.rs:17`; `upstox-feed-proto/Cargo.toml:10,19-20`; `upstox_recorder.rs:18-26,238`; `frame.rs:70`; `state.rs:113-116`; `engine.rs:955-971`; A1 F-A1-01/04/05/08; R1 Q1a/Q1b/Q3/Q5; Amendment A3/A4.4/A5.
- **External capabilities:** none required by this ADR. A future port would require an owner, a second implementation/test consumer and a measured requirement (§3.2); none exists at baseline.
- **Governing docs:** `ADR-0001-modular-monolith.md` (crate boundary = stable contract; measured requirement), `ADR-0004-live-disabled.md`, `target-architecture.md` (excludes C4/C5/HFT at `:66-69`).

---

## 8. Rejection conditions

This decision is overturned or revised if new evidence shows:

1. The governing charter is amended to **remove** the hexagonal ports/adapters requirement (reversing Amendment A3) — then §3.1 target stance and M1-ARCH-INV-022 collapse to R1 Q1b's position.
2. A **measured requirement** is approved with an acceptance test for a specific second feed/store/test seam — then the gate in §3.2 is satisfied and a port decision is authorized by a new ADR (this ADR remains the rule, not the implementation).
3. Source at `01eb382e` is shown to contain real production port traits (contradicting A1 §3.3) — the "current contract" claim and codec taxonomy are inaccurate.
4. Executed evidence shows per-instrument partitioning **can** preserve aggregate exposure/daily-loss semantics (refuting R1 Q3 / Amendment A4.4) — M1-ARCH-INV-027 is relaxed and ADR-M1-006 is re-based.
5. `target-architecture.md:66-69` C4/C5 exclusion is lifted with a measured requirement — the performance posture changes and dependency rules may need revision.
6. Owner ratification of D1/D9 is refused with responsibilities reassigned — §4 is void.
7. **Owner charter-fit ruling (challenger A-5 / Section D.1):** if the owner rules that ADR-M1-005's mandatory identity-field surface is a forbidden schema change, it must be routed through the §3.2 measured-requirement gate; this ADR's rules are unchanged but no such field surface is authorized here. This is an owner ruling, not an Architecture-group decision.
8. **Global invariant registry (challenger A-0) — RESOLVED.** The coordinator's renumbering pass assigns distinct namespaces (`M1-ARCH-INV-*` for ADR-001..004, `M1-RUN-INV-*` for ADR-005..007, governance prefixes unchanged). `INVARIANT-REGISTRY.md` is the authoritative registry; acceptance citations resolve unambiguously. M1-ARCH-INV-005 and M1-ARCH-INV-025 remain distinct IDs with distinct enforcement scopes.

---

## 9. Measurable acceptance requirement (design only — not executed in M1)

These are **future gate specifications, not delivered properties and not executed in M1**. Every named artifact has an owner role and target milestone (challenger Section C); requirements depending on nonexistent external capabilities are labelled **PRECONDITION — not an M1 gate**.

| ID | Named test / command (future) | Threshold | Owner (role) | Target milestone | Status |
|---|---|---|---|---|---|
| AC-004-1 | `grep -rn 'pub trait\|dyn ' crates/` and gate assertion that no production port trait is claimed as the *current* contract (A1 FT-D) | 0 production `dyn` port objects counted as current contract; target ports documented but unimplemented | *Verification / release-engineering owner* (D9) | M2 | Future gate spec; command exists, assertion test does not |
| AC-004-2 | Port-gate review test: for any proposed port, assert §3.2 items (measured requirement + 2 implementations or named test consumer + owner + acceptance tests + new ADR) | 0 ports introduced without all five items; 0 speculative empty traits/crates | *Architecture / Platform owner* (D9 boundary) | M2 | Future gate spec |
| AC-004-3 | `cargo xtask check` with a transitive-dependency test (A1 FT-C) and filesystem/Python boundary scans | 0 restricted externals reachable transitively; 0 domain-crate `std::fs`; 0 broker SDK imports in research-domain package | *Verification / release-engineering owner* (D9; `xtask` owner gap closed in ADR-M1-001) | M3 | Future gate spec — gate/tooling changes **not authorized in M1**; owner must be ratified |
| AC-004-4 | Deny-edge non-vacuity self-test over `layers.toml`; **"future"-marked patterns must be excluded from the active-enforcement count** (no escape hatch) | every **active** deny pattern matches ≥1 crate; 0 vacuous active edges; `future`-marked patterns excluded from the enforced count | *Verification / release-engineering owner* (D9) | M2 | Future gate spec; threshold tightened per challenger Section C |
| AC-004-5 | Aggregate-semantics regression (challenge 3): two-instrument replay partitioned locally; compare partition-local vs account-global exposure/daily-loss (`engine.rs:955-971`) | account-global authority only may pass/fail `GrossExposure`/`DailyLossLimit`; 0 partition-local authoritative checks; frames remain whole (`__multi_instrument_frame__`, `payload_sha256` intact) | *Verification owner* (D9) + *Simulation owner* (D4) | M2 | Future gate spec; two-instrument falsifier is source-verified but unexecuted; the test itself does not exist at baseline |

### Amendment-1 challenger disposition (ADR-M1-004)

- **A-5 / Section D (charter-fit) — ACCEPTED.** Exact text changes: header and §3.1 now state explicitly that this ADR (and ADR-M1-001) authorize **no port, service, schema file, crate, optimization, feature, broker connection or live-routing change** and make no readiness claim; §3.1 and §8.7 record the ADR-M1-005 field-surface question as an **owner ruling**, not this group's decision.
- **A-0 (invariant collisions) — RESOLVED by coordinator renumbering (2026-11-17).** Exact text changes: §5 no longer implies global uniqueness; §3.2/§5 remove the "aliases" claim; §8.8 records resolution. The coordinator assigned `M1-ARCH-INV-*` / `M1-RUN-INV-*` namespaces across all 11 ADRs and verified zero duplicate definitions (`INVARIANT-REGISTRY.md`). No IDs from other groups' ADRs are cited.
- **Section C (untestable §9) — ACCEPTED.** §9 gained `Owner (role)`, `Target milestone`, `Status` columns; AC-004-4's "marked future" escape hatch is closed by requiring future-marked patterns be excluded from the active-enforcement count; AC-004-3 is scheduled at M3 with the D9 owner.
- **CONTEST (out of scope, with reason):** challenger Section D.2 flags optimization design intent
  (pinned threads, pre-allocated pools, SBE) in ADR-M1-007 (Runtime group). That text is not in this
  ADR; this ADR neither accepts nor rejects it, and record no contest — it requires the Runtime
  group's and the owner's disposition (Section D.2 itself calls it acceptable with a caveat). A-1
  (single vs separate authority) and A-3/B-2 (OMS reconciliation) are likewise not in this ADR's
  scope; they are resolved in ADR-M1-002 and ADR-M1-003 respectively.

*End of ADR-M1-004.*
