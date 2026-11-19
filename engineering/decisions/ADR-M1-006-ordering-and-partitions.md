# ADR-M1-006 — Ordering scopes and partition semantics

- **Status:** PROPOSED (designed, not implemented). M1 design work only.
- **Group:** Runtime Design Group
- **Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9` (all citations `01eb382e:`)
- **Scope:** ordering/partition *contract semantics* only. No partitioning code, no service, no numeric budget. Budgets come from measured workloads (ADR-007, A6 §7), not aspiration.
- **Evidence discipline (owner challenge 6):** partition/ordering facts are level B (source) and R1-C-verified; no performance claim is `verified`. C4/C5 and horizontal scaling are explicitly out of scope by `target-architecture.md:66-69`.
- **Amendment resolution:** challenger **B-3** (serial authority asserted, not enforced) resolved by an explicit residual-risk statement in D-006.3 / §6 case 1 / U-006-a: the account-global rule is **affirmed as a design requirement but is not a working control at baseline**. **A-2 alignment:** dedup is scoped per ADR-005 option (b). **Section C** §9 owner/milestone and precondition labelling applied.

---

## 1. Problem

M0/R1 established that ordering and partitioning cannot be treated as a free, uniform layer:

1. **Raw frames cannot be keyed by instrument.** Every Upstox raw frame is stamped `provider_instrument_id="__multi_instrument_frame__"` (`upstox_recorder.rs:238`; `RawFrame::new` at `frame.rs:46-72`), and `payload_sha256` covers the whole decoded payload (`frame.rs:71,79`). Per-instrument partitioning at the journal layer either collapses to one partition or requires splitting the payload, which invalidates the integrity hash and the one-frame→two-event shared `raw_sha256` identity (A2 F-02; R1-C Q3). Decode already imposes a **global** deterministic key sort — `keys.sort()` / `feeds.sort_by()` (`decode.rs:64-85`, "no ordering implied by arrival position").

2. **Account-global state is lost under per-instrument partitioning.** `paper-engine::state.rs` holds cash, positions, charges, `daily_pnl`, positions/orders/intents/fills in one projection (`state.rs:108-119`). `risk_context()` builds one aggregate `gross_exposure_paise` and one account-level `daily_pnl` (`engine.rs:940-971`), consumed by `RiskCheck::GrossExposure` and `RiskCheck::DailyLossLimit` (`risk-engine/src/lib.rs:154-157,273,282`). R1-C Q3's falsification: partition a two-instrument replay so each starts from capital C and checks its own book; each order individually fits `max_gross_exposure_paise`, combined exceeds it — **neither partition fires the exposure or daily-loss check**, and `daily_pnl` is a single external observation copied verbatim (`state.rs:573-577`).

3. **Ordering metadata is richer than the live path uses.** `ContinuityTracker` detects `Initial/InOrder/Gap/Duplicate/OutOfOrder` (`continuity.rs:3-21,56-105`) and `EventMeta` carries `connection_epoch`/`receive_seq` fences (`market-types/src/model.rs:620-664`), but both are unwired (A2 F-04/F-05). The live normalizer sees no gap signal for Upstox (A2 F-07/FM-7).

4. **Backpressure is asymmetric and partly absent.** The Rust recorder has no bounded queue, no drop counter, no overflow semantics; work runs inline on the tokio task (`upstox_recorder.rs:470-509`; A6-03). FYERS has a bounded queue (16,384) with gap records, dropped counts and downstream fail-closed (`fyers_recorder/recorder.py:34,75,141-201`; A6-04).

5. **There is no measured scaling requirement.** No workload class, profile, budget, SLO or benchmark harness exists (`A6-08/09/11`; `target-architecture.md:66-69` excludes C4/C5). R1 downgraded A6-06 (no partitioning) to S4: the honest current-state contract is a single-node envelope, not horizontal scaling.

The problem is therefore not "add partitioning." It is to define which ordering scopes matter,
  where, and to prevent a partition scheme from silently breaking account-level correctness or
  integrity.

---

## 2. Alternatives considered (with rejection reasons)

- **Partition raw journal by `provider_instrument_id` at capture.** Rejected. Frames are multi-instrument (`upstox_recorder.rs:238`); splitting invalidates `payload_sha256` (`frame.rs:71`) and the shared `raw_sha256` identity (A2 F-02; R1-C Q3). It also cannot be done without a schema/wire change, which M1 forbids.

- **One global total order for everything, always.** Rejected as unattainable and unnecessary. There is no single authoritative exchange sequence for the feeds (Upstox `currentTs` is a feed envelope, FYERS `exch_feed_time` a callback value — `providers.py:108-110,205,233`), and strict global ordering across sources conflicts with horizontal scaling (R1/A6). Only the (excluded) C4/C5 or cross-account netting would need it.

- **Per-instrument partition-local risk checks (scale-out by instrument).** Rejected — this is the R1-C Q3 failure: aggregate `gross_exposure_paise` and account `daily_pnl` are breached while every local check passes. State.rs/engine.rs hold one account projection; it must be evaluated as one authority.

- **Treat `connection_epoch`+`receive_seq` as a global order and embed it in identity.** Rejected. It is a per-connection receive fence, not a cross-source order; embedding it in identity recreates the reconnect-instability defect (ADR-005 F-01). Valid for ordering only.

- **Adopt horizontal scaling now with a coordination protocol.** Rejected as speculative: no measured requirement, no budget, no harness (A6-08/09), ADR-0001 forbids new infrastructure without one, and C4/C5 are excluded (`target-architecture.md:66-69`). R1 flagged the baseline's "M5 prerequisite absent" assertion as unsupported — recorded as unresolved, not invented.

- **Leave the Rust recorder's implicit socket-buffer backpressure as "good enough."** Rejected. It is unbounded, unmeasured, and unobservable (A6-03); it contradicts the repo's own `STA-010` policy. FYERS' bounded+gap-accounted model is the correct reference semantics.

---

## 3. Decision

### D-006.1 — Five ordering scopes and where each is required

| Scope | Definition | Authority | Required where |
| --- | --- | --- | --- |
| **Wire / frame (source-local, epoch-scoped)** | Order bytes were read from one connection | Recorder (`capture_sequence`, `upstox_recorder.rs:232-249`) | Capture journal append; reconnect fence; gap detection. The only order raw capture can guarantee. |
| **Decode (frame-global, deterministic)** | Sorted feed keys within a frame, by key not arrival | `decode.rs:64-85` | Producing a deterministic, reproducible projection of a multi-instrument frame. Explicitly NOT a temporal order. |
| **Instrument / series (per `provider_instrument_id`)** | Order of one instrument's observations by source/fence time | Derived from ordered raw frames + identity (ADR-005) | Book reconstruction, last-trade edge detection (`providers.py:46-64`), PIT/temporal joins. A *derived projection*, not a raw partition key. |
| **Entity / account (per `account_id`)** | Total order of all events mutating one account's cash/positions/orders/P&L | Single account authority | All risk checks (`gross_exposure_paise`, `daily_pnl`, kill switch — `engine.rs:940-971`; `state.rs:108-119`) and OMS lifecycle (ADR-003). This is the correctness-critical scope. |
| **Global (cross-source / cross-account)** | Single total order across all sources and accounts | None today | **Not required** for C0–C3. Only for excluded C4/C5 or future cross-account netting. Not free; gated (D-006.5). |

**Rule:** the strongest scope required by a computation wins; weaker scopes may not silently
  substitute. In particular, an instrument-scoped order may never stand in for entity-scoped risk
  state.

### D-006.2 — Partition key decision (raw layer vs derived layer)

- **Raw capture partition key = the durable frame / connection epoch** (`capture_frame_id` from ADR-005, or the `(source, connection_epoch)` pair). This preserves `payload_sha256`, the multi-instrument frame, and the one-frame→two-event shared `raw_sha256` identity. Instrument is **not** a raw-layer key (D-006.1 evidence).
- **Derived per-instrument streams** may be partitioned by `provider_instrument_id`, but only as a *downstream projection* over a single ordered raw frame source. The raw layer remains the integrity authority; a derived partition never rewrites or re-hashes payload bytes.
- Partition ownership (key → worker) is **not defined in M1** — no lease/epoch/reassignment mechanism exists at baseline (baseline §4: "Partition ownership: None"). Any future scheme requires a measured requirement and an ADR.

### D-006.3 — Coordination rule for account-global state

- All events that mutate account-global state (cash, gross exposure, `daily_pnl`, open orders, kill switch — `state.rs:108-119`, `engine.rs:955-977`) are serialized through **one account authority** that owns the aggregate checks. A per-instrument partition may compute local quantities but **must not** decide `GrossExposure`, `DailyLossLimit`, or kill-switch state on a partition-local projection.
- **Default (C0–C3): single-threaded serial account authority.** The merged account projection is the only input to `risk_context()` (`engine.rs:918-979`). This matches the current paper-engine design and the single-threaded research pipeline (A4 §7 concurrency).
- **Gated alternative:** if a measured requirement ever justifies partitioned replay, a **two-phase aggregation** is required — local deltas are merged into a canonical account state with a monotonic account version, and no risk decision is evaluated until all preceding deltas are included. This is a design placeholder, not an implementation, and requires an ADR + measured requirement.
- `daily_pnl` is one account-level external observation (`state.rs:573-577`), never partition-local. Per owner challenge 5, it must be **recomputed from fills+marks or excluded** from any verified-equal replay field; a coordinated-but-copied value is still not a verified figure.

**Residual risk / enforcement status (challenger B-3 — required, not withdrawn): the account-global rule above is a DESIGN REQUIREMENT, not a working control at baseline.** Concretely:
- Partition ownership (key → worker), leases, epochs, and any enforcement seam that would force a single account authority are **undefined at baseline** (U-006-a; baseline §4 "Partition ownership: None"; D-006.2). There is no structural mechanism in M1 that prevents an implementer from constructing `risk_context()` (`engine.rs:918-979`) from a partial/partition-local projection.
- The baseline API takes a **caller-supplied projection**; the sole current caller derives its own `feed_health` from its replayed projection (`engine.rs:934-939`, R2 B3). Nothing in the contract makes the merged-projection requirement executable or testable in M1; M1-RUN-INV-011 states intent only.
- The two-instrument falsifier in §9 (R1-C Q3) is **source-verified (B) but UNEXECUTED**, and this design adds no seam that would make it pass without future implementation. It is a specification, not evidence that the breach is prevented.
- Therefore any downstream claim that "account-global limits cannot be bypassed" is **not supportable from M1 artifacts**; it becomes supportable only when a named owner implements and executes the enforcement seam and the §9 falsifier (owner/milestone labelling applies).

### D-006.4 — Sequence gap / duplicate / reconnect semantics

- On each connection epoch the local receive sequence starts fresh. A decreasing counter is a **reconnect boundary** (new epoch), not a duplicate; identity is unaffected (ADR-005 D-005.2).
- Within an epoch, apply the `ContinuityTracker` observation model (`continuity.rs:3-21,56-105`): `Initial`, `InOrder`, `Gap{missing}`, `Duplicate`, `OutOfOrder{highest_seen}`.
  - **Gap** → emit gap flag/marker, transition affected book to `ContinuityState::Resyncing` (`market-types/src/model.rs:708-724`), and fail closed until a fresh authoritative snapshot. FYERS already produces explicit gap records (`recorder.py:141-168`); Upstox has none (A2 F-05/FM-7) — this is a required contract, not an implemented change.
  - **Duplicate** → within the known-sufficient context (re-delivery/replay of a captured payload), dedup on `observation_id` (ADR-005 option b) and count; do not apply twice to mutable state. For two genuine distinct observations with identical bytes, ADR-005 option (b) does **not** authorize collapse — treat the residual risk (understated occurrence count/exposure/P&L) as recorded there, not as a gap-detection fix.
  - **OutOfOrder** → buffer or flag; never silently apply to mutable book state.
- Provider sequence/time is advisory where no authoritative exchange sequence exists (`providers.py:108-110,205,233`); the local receive fence is authoritative for **ordering only**, never for identity or an exchange-FIFO claim. The existing quality disclaimers state this (`cli.py:210-214`).

### D-006.5 — Backpressure and the scaling/ordering constraint

- Every producer→consumer boundary must declare four things: bounded capacity, overflow policy, drop/gap accounting, and acknowledgement meaning. FYERS is the reference semantics (bounded queue, gap record, `dropped_callback_count`, downstream fail-closed — `recorder.py:34,141-201,359-390`; A6-04). Rust recorder currently has none (A6-03) → must be declared as a required contract, not implemented here.
- No silent drop: any overflow produces a gap/discontinuity signal and marks downstream data non-research-grade.
- **Numeric capacities are NOT set here.** They are parameters to be filled by workload measurement (A6 §7 W2/W5, ADR-007). The 16,384 FYERS queue and 4 MiB frame cap are *observed baseline facts*, not budgets.
- **Explicit rejection of "horizontal scaling and strict global ordering are free":** they are not. Instrument partitioning breaks account-global risk (§1.2), raw frames cannot be keyed by instrument (§1.1), and global ordering has no authoritative source. Any horizontal scaling requires (a) a measured requirement, (b) the two-phase account coordination above, and (c) an ADR. Until then the contract is a **single-node envelope**. R1's "M5 prerequisite" assertion and C4/C5 exclusion are recorded as unresolved (§7), not resolved by assumption.

### D-006.6 — Owner-challenge implications (interfaces only)

- **Challenge 3 (events and partitions, primary):** this ADR owns the multi-instrument-frame constraint (D-006.2, M1-RUN-INV-010) and account-global risk semantics (D-006.3, M1-RUN-INV-011/012); the observation-vs-business identity half is owned by ADR-M1-005. Together the two ADRs satisfy challenge 3 by number.
- **Challenge 1 (authorization, no remote service assumed):** the risk permit's context must be built from the merged account projection (D-006.3), not a partition-local view; adding a remote authority is not required by this decision and is gated (ADR-M1-002/R1 Q4).
- **Challenge 2 (OMS):** order-lifecycle and UNKNOWN/reconciliation events must be totally ordered in the **entity/account scope** and carry stable identity (ADR-005) — interface implication only; lifecycle authority is ADR-M1-003.
- **Challenge 4 (durability), envelope implication only:** the frame append ACK is connection-scoped (`sync_data()` returned, `journal.rs:47-53`); it is not a statement of replay coverage. Cross-reconnect continuity relies on the epoch fence; ACK semantics/anchoring are ADR-008/010.

---

## 4. Ownership

- **Decision owner:** Runtime Design Group (this ADR); owner-level acceptance recorded in `M1-RECONCILIATION.md`.
- **Ordering-scope state (design-time):**
  - Wire/frame order + epoch fence — proposed owner: capture/adapter layer (`apps/upstox-tools`, `apps/fyers-recorder`).
  - Decode order — proposed owner: proto decoder owner (`crates/upstox-feed-proto`).
  - Instrument/series order + gap flags — proposed owner: normalizer/quality owner (`providers.py`, `cli.py`).
  - Entity/account order + aggregate risk checks — proposed owner: paper-engine/account-journal owner (`crates/paper-engine`, `crates/account-journal`). M0 records these crates as unowned (baseline F-A1-07) — **unresolved**, gap recorded.
- **Partition ownership:** none exists at baseline; D-006.2 defines the raw-layer key but assigns no worker/lease owner. Explicitly unresolved.
- **No persistent state is created by this ADR.**

---

## 5. Invariants (explicit IDs)

- **M1-RUN-INV-009 (scope precedence):** No weaker ordering scope substitutes for a stronger required scope; instrument-scoped order never satisfies entity-scoped risk.
- **M1-RUN-INV-010 (raw key):** The raw-layer partition/identity key is the frame (or `(source, connection_epoch)`), never `provider_instrument_id`; `payload_sha256` and multi-instrument frames are never split.
- **M1-RUN-INV-011 (account authority — DESIGN REQUIREMENT only, not an enforced control; see D-006.3 residual risk):** Every `GrossExposure`, `DailyLossLimit`, and kill-switch decision is to be evaluated on the merged account projection, not a partition-local projection. No enforcement seam or partition-ownership mechanism exists at baseline to make this executable in M1.
- **M1-RUN-INV-012 (daily-P&L scope):** `daily_pnl` is one account-level observation; it is never partition-local and never accepted as copied-forward verified equality (challenge 5).
- **M1-RUN-INV-013 (gap fail-closed):** Any detected gap or reconnect moves affected book state to `Resyncing`/non-live until a fresh authoritative snapshot; no silent recovery.
- **M1-RUN-INV-014 (no silent drop):** Every dropped/overloaded item produces a gap record and a non-research-grade mark.
- **M1-RUN-INV-015 (no free global order):** No claim of strict global ordering or horizontal scaling without a measured requirement and an approved ADR; default is single-node.
- **M1-RUN-INV-016 (budget provenance):** No numeric capacity/latency value in this contract; all budgets trace to a named workload measurement.

---

## 6. Failure cases (concrete, including adversarial)

1. **Partition-local limit bypass (R1-C Q3) — design requirement, not enforced.** Two instruments, starting capital C; each partition buys C and passes its local `max_gross_exposure_paise`; aggregate 2C breaches. The design *requires* the account authority to reject the second order (D-006.3/M1-RUN-INV-011); **nothing in M1 structurally enforces this** — no partition ownership, lease, or enforcement seam exists (U-006-a), and the falsifier is unexecuted. **Adversarial:** an operator configures per-instrument partitions "for throughput" and omits the aggregate authority; M1-RUN-INV-011 forbids the topology on paper, but the baseline offers no mechanism that would detect or prevent it. This case is the primary motivation for the residual-risk statement in D-006.3, not evidence that the control works.

2. **Splitting a multi-instrument frame.** A practitioner partitions by instrument at the journal layer, splitting the payload; `payload_sha256` (`frame.rs:71`) no longer matches and the shared `raw_sha256` identity is destroyed (A2 F-02). D-006.2/M1-RUN-INV-010 forbid it.

3. **Decode order mistaken for temporal order.** A consumer treats `feeds.sort_by` order (`decode.rs:84-85`) as event chronology. The source comment explicitly denies arrival-position ordering; M1-RUN-INV-009 prevents substituting it for instrument/account order.

4. **Reconnect misread as duplicate.** A reset counter is treated as a duplicate sequence within an epoch, suppressing a genuine gap. D-006.4 treats it as an epoch boundary; identity stays stable (ADR-005).

5. **Unbounded overload (Rust recorder).** Sustained feed > fsync service rate; socket buffers grow, no drop counter, silent loss (A6-03). D-006.5/M1-RUN-INV-014 require bounded capacity + gap accounting (not implemented in M1).

6. **Copied P&L laundering (challenge 5).** A coordinated replay copies `daily_pnl` (`state.rs:573-577`, `engine.rs:971`) and reports a green compare. M1-RUN-INV-012 forbids treating it as verified; ADR-009 owns the fix.

7. **Adversarial epoch forgery.** A compromised producer stamps a new `connection_epoch` to evade gap detection. The fence is provenance metadata, not trust (ADR-010 anchoring is required for authenticity); this ADR records the dependency, does not claim to solve it.

---

## 7. Dependencies

- **ADR-M1-005 (Event identity):** supplies `capture_frame_id`, `observation_id` and the rule that epoch/counter are not identity components. Must remain consistent (no epoch in identity).
- **ADR-M1-009 (Replay contract):** owns "recompute P&L from fills+marks or exclude"; D-006.3/M1-RUN-INV-012 depend on it.
- **ADR-M1-003 (OMS):** owns entity-scoped lifecycle and UNKNOWN/reconciliation ordering (challenge 2).
- **ADR-M1-002 (Authorization):** risk permit must consume the merged account projection; no remote coupling assumed.
- **ADR-M1-007 (Runtime profiles):** execution/streaming profiles inherit this ordering contract; backpressure capacity parameters come from its workload budgets.
- **ADR-M1-008 / ADR-M1-010 (Durability / anchoring):** connection-scoped ACK and epoch provenance; envelope implications only.
- **Evidence dependencies:** A2 F-02/F-04/F-05/FM-7; R1-C Q3, R1 Q3 downgrade of A6-06 to S4; A6-03/04/06/08/09/11; Amendment A §A3/§A4.
- **External capability dependencies:** none. No partition/lease infrastructure required.

**Explicitly unresolved disagreements (recorded, not guessed):**
- **U-006-a:** Baseline §4 asserts "Partition ownership: none — M5 prerequisite absent," but R1-C Q3 found **no M5 charter text** establishing that prerequisite, and §13.11/A6 exclude C4/C5. Unresolved contradiction; this ADR does not invent M5 scope. **Amended for B-3:** this is also the explicit residual risk for the account-global rule — partition ownership, the enforcement seam, and the owner that would implement them are all undefined at baseline, so M1-RUN-INV-011 is intent only. Must appear in `M1-RECONCILIATION.md` (challenger Section F.9).
- **U-006-b:** Single serial authority (default) vs two-phase aggregated coordination (gated). Decision picks serial for C0–C3; the scaled path is a placeholder pending measured requirement.
- **U-006-c:** Numeric queue/overflow capacities — deferred to workload measurement; no value guessed here.

---

## 8. Rejection conditions

This decision is overturned or revised if committed evidence shows:

1. A certified authoritative exchange sequence exists for the feeds, enabling a true global order (would revise D-006.1/D-006.5).
2. A measured scaling requirement makes single-node envelope insufficient, with an approved ADR for two-phase account coordination and a partition-ownership/lease scheme (would revise D-006.2/D-006.3).
3. A split-frame scheme preserves `payload_sha256` and the shared `raw_sha256` identity (would revise M1-RUN-INV-010).
4. Account-global checks are shown to be genuinely instrument-local (contradicts `engine.rs:940-971`, `state.rs:108-119`) — would revise M1-RUN-INV-011.
5. A gap/reconnect can be proven benign without fail-closed resync (would revise M1-RUN-INV-013).

---

## 9. Measurable acceptance requirement (design only — not executed)

**Test names (proposed):**
- `test_partitioned_replay_does_not_bypass_account_global_limits`
  - **Owner:** paper-engine / account-journal owner (proposed; **unassigned at baseline** — F-A1-07). Ratify at exit gate.
  - **Target milestone:** M2 (future, separately authorized).
  - **PRECONDITION — not an M1 gate.** Test does not exist; the partition-ownership/enforcement seam it presumes is undefined (U-006-a). The scenario is source-verified (B) but **UNEXECUTED**; no M1 artifact makes it pass (B-3).
- `test_raw_frames_are_not_split_by_instrument_partition`
  - **Owner:** capture/storage owner (proposed; unassigned at baseline). Ratify at exit gate.
  - **Target milestone:** M2 (future, separately authorized).
  - **PRECONDITION — not an M1 gate.** Test does not exist; the raw-frame integrity reasoning is source-verified (B), not executed.

**Commands (proposed, to exist only after owner authorization):**
`python3 -m pytest -q research/python/tests/test_ordering_partitions.py` and
`cargo test -p paper-engine --locked test_partition_local_exposure_does_not_mask_breach`

**Command-level precondition (challenger Section C):** the backpressure-accounting scenario below requires a bounded queue in the Rust recorder, which does **not** exist at baseline (A6-03). It is **PRECONDITION — not an M1 gate** until that queue contract is implemented in a future, separately authorized milestone with a named owner.

Concrete falsification scenarios and thresholds (specification only):

- **Aggregate-limit falsifier (primary, from R1-C Q3):** two instruments A and B; starting cash C; `max_gross_exposure_paise = C` (aggregate). A partition-local replay buys C of A on partition A and C of B on partition B; each local exposure = C (passes locally), combined = 2C (breaches).
  - **Threshold:** the account authority evaluating `gross_exposure_paise` on merged state must reject or flag **≥1** of the two orders such that **0** aggregate breaches are admitted. If both pass and the aggregate exceeds the limit, the decision is **falsified**.
  - **Adversarial variant:** inject `observe_risk_state(daily_pnl=0)` while fills imply a large loss; assert the daily-loss check on merged, recomputed state is not bypassed (threshold: 0 passes on a real breach; cross-ref ADR-009 challenge 5).
- **Gap/reconnect semantic:** inject a sequence skip within an epoch and a counter reset across epochs.
  - **Threshold:** 100% of skips raise a gap flag and transition the book to `Resyncing`; 100% of resets are classified as epoch boundaries (not duplicates).
- **Raw-frame integrity:** take a 2-instrument Upstox frame with `provider_instrument_id="__multi_instrument_frame__"` (`upstox_recorder.rs:238`) and assert no partition scheme changes `payload_sha256` or produces per-instrument frame hashes.
  - **Threshold:** 1 frame hash preserved; 0 split payloads.
- **Backpressure accounting (design check):** for a bounded queue overflow test double, assert 100% of drops produce a gap record and `non_research_grade` mark.

**Baseline status:** these tests do not exist; no owner or milestone is committed. The aggregate-limit scenario is a source-verified (B), **unexecuted** falsifier from R1-C Q3; the backpressure gap behaviour exists only in FYERS (A6-04 C-level) and the Rust recorder lacks the bounded queue the scenario needs. All four requirements are **PRECONDITION — not an M1 gate**, to be implemented and executed only after owner authorization. They are specifications, **not** evidence that the account-global rule is enforced or that any breach is prevented (B-3).
