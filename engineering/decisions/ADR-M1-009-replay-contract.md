# ADR-M1-009 — Replay and verification-equal contract (no copied-forward P&L)

**Status:** PROPOSED — Governance Design Group, M1 Platform Constitution (2026-11-17)
**Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9` (read-only reference)
**Scope:** DESIGN ONLY. Defines a replay record contract and a comparison discipline. Authorises no CLI code, schema, service, network or live change. No acceptance command below is executed by M1.
**Authority:** M1 does not implement. Adoption requires repository-owner approval.
**Amendment 2 — challenger disposition:** Resolved findings **A-4 / B-5** (`pnl_basis` is no longer accepted as a self-declaration; it is verifier-derived, with the residual bypass stated explicitly), **§C/§9** (named owners, milestones, PRECONDITION labels), and reaffirmed charter-fit (§D). Referred to owner ruling: the A-5/§D.1 question of whether the experiment-record unification and mandatory field surface are forbidden "schema changes." Finding **A-0** is out of scope: this ADR uses only the `M1-REP-INV-` namespace, which the challenger lists as a separate scheme (`CHALLENGER.md:44`); no renumbering.

---

## 1. Problem

There is **no source-to-replay driver** at baseline. The CLI registers only `demo`, `build-dataset`, `verify-dataset`, `normalize`, `reconcile`, `alpha-search` (`research/python/src/quantlab/cli.py:318-375`); the sole "E2E" test is in-process, starts from already-canonical fixtures, uses disconnected per-stage inputs and never replays-and-compares a recorded run (`research/python/tests/test_quant_pipeline_e2e.py:64-171`; A4-001, S1). Per-component determinism is strong (fresh-process `demo`/`alpha-search`/`build-dataset` byte-identical across `PYTHONHASHSEED=1|2|12345|random`, A4 §10) but does not compose into a system guarantee.

The experiment-record contract is orphaned: `experiments.py:16-160` defines fields but no producer exists, `code_revision` is free-form, two mutually incompatible schemas coexist (`schemas/research/experiment-record.v1.schema.json` and `...v1.json`), and `scripts/validate_contracts.py:44-58` only self-validates schema documents (A4-002). Float statistics carry **no runtime fingerprint** — no Python version, dependency lock digest, platform or libm field (A4-006).

written").

---

## 2. Alternatives considered (with rejection reasons)

| Alternative | Rejection reason |
| --- | --- |
| **Include P&L in the verified-equal field set** (replay compare copies P&L forward) | Tautological: the value is copied, not recomputed (`engine.rs:354-368`, `state.rs:573-577`, `engine.rs:971`). Green compare attests an unverified figure; R1 D1 and Amendment A §A5 explicitly reject it. Rejected. |
| **Trust the existing in-process E2E test as system replay** | Starts from canonical fixtures, not source/raw; disconnected stage inputs; no recorded checkpoint or compare artifact (A4-001). Rejected. |
| **Rely on per-component `PYTHONHASHSEED` determinism as replay proof** | Necessary but not sufficient: no shared checkpoint/compare, float statistics are host/version-relative and have no fingerprint (R2 §2.2, A4-006). Rejected as the system property. |
| **"Persist rejected trials" / add a runtime fingerprint field only** | Records an environment without enforcing numerical equivalence and allows silent overwrite of rejected trials; R2 D2 requires a no-overwrite store plus cross-environment byte-compare. Rejected as a standalone fix. |
| **Add a remote replay service / broker-connected driver** | New SPOF, couples a hard-disabled local system to infrastructure; no measured requirement (Amendment A §A5, R1 D2). Rejected. |
| **Synthesize missing final marks so P&L is always computable** | Documented policy forbids synthesizing missing marks (`backtesting-and-alpha.md:43`); R2 B2 upheld `net_pnl=-100` as policy-conformant. Rejected; use an explicit unmarked-residual signal instead (A2/Amendment A §A2). |

---

## 3. Decision

### 3.1 P&L rule (mandatory, primary)

**Copied-forward P&L must never qualify as verified equality.** A replay comparison may treat P&L as `verified-equal` only in one of two admissible modes:

- **(A) Recompute mode** — P&L is recomputed *inside the replay* from the fill stream and marks that the replay itself derives (fills + marks, with an explicit unmarked-residual policy), and the recomputed value is compared to an independently recorded reference. The externally supplied `RiskStateObserved.daily_pnl` (`state.rs:573-577`, `engine.rs:354-368`) is **not** that reference.
re-derived.

Default for M1 design: **mode (B) for any value sourced from `RiskStateObserved`**, with mode (A) permitted only when the recomputation and its mark policy are explicitly defined and evidenced. No compare may silently include an observed P&L as equal.

#### 3.1.1 Binding of `pnl_basis` and the verified-equal field set (challenger A-4 / B-5)

The earlier wording — "a machine-readable `pnl_basis` discriminator must accompany every replay record" — is **withdrawn**. A producer-authored discriminator carried in the same record it is meant to police is exactly the self-declared-flag pattern this ADR criticises in `point_in_time.py:726-762`; a producer can set `pnl_basis=excluded` (or `recomputed`) while the P&L value is copied from `RiskStateObserved` (`engine.rs:354-368`→`state.rs:573-577`→`engine.rs:971`), and a consumer cannot tell the lie. That is the **residual bypass** and it is recorded as unclosed here, not claimed to be fixed by a field.

The required design contract is therefore that `pnl_basis` and the verified-equal field set are **derived by the verifier, not asserted by the producer**:

- **Verified-equal field set (mode B).** The set of fields eligible for verified equality is fixed by the *replay contract itself* — the declared list of fields the replay re-derives — not by any value in the producer's record. A field is excluded by construction if it is not in that re-derived set; `RiskStateObserved.daily_pnl` is never in it. The "declaration" is a property of the verifier's field-set definition, so a producer cannot widen it.
- **Recompute admission (mode A).** A P&L field is admitted as verified-equal only if the verifier can recompute it from the record's fill stream and marks and the recomputed value matches. The fill stream must itself be committed in the ADR-M1-010 anchor receipt, so a producer cannot substitute a favourable fill stream.
- **Anchor binding.** Both the replay-derived field-set definition and the fill-stream commitment are included in the anchor subject (ADR-M1-010 **§3.1** "What is anchored", fourth bullet — corrected from the erroneous §3.2 citation by challenger re-verification N-1). A receipt that does not commit to them cannot support a P&L equality claim.
- **Fail closed.** If the verifier cannot perform the above — no anchor, no replay driver, no recomputable fill stream — P&L is excluded and the claim is not `verified` (ADR-M1-011).

**Current-state truth (equivalent to challenger option b, stated explicitly):** because no off-host anchor exists (ADR-M1-010 §4) and no `replay-run` driver exists (A4-001), **none of the above is machine-enforceable today**. Until those preconditions exist, treating P&L as excluded is a **documentation convention with residual forgery risk**: a producer can mislabel or silently include a copied P&L and no committed check detects it. This is a known, unclosed bypass, not a guarantee; §9 test 2 is specified accordingly and is a PRECONDITION, not an M1 gate.

### 3.2 Replay record contents (information contract, not a schema)

A replay record must make the following information classes explicit and content-addressed (no-overwrite, immutable); it must be sufficient to re-execute the run in a fresh process:

1. **Inputs** — references plus content hashes for the raw source frames (the `payload_sha256`/frame identity of ADR-M1-010), the canonical events, and the dataset manifest hash; external observation inputs identified separately.
2. **Ordering** — sequence/order keys and the clock domain for each input (source/received/available/record times, ns semantics), so a reorder is detectable.
3. **Versions** — one authoritative git commit SHA (not a free-form `code_revision`), schema versions, and config/strategy artifact hashes; the two experiment-record schemas are unified into one record identity (A4-002).
4. **Initial state** — the starting state hash and the state from which the replay begins.
5. **External observations** — `RiskStateObserved`, broker-report observations and any caller-supplied risk context, tagged as *observed/attested* and excluded from verified-equal unless recomputed (§3.1).
controlled.
7. **Numerical environment (runtime fingerprint)** — Python version, dependency/constraints digest, platform/OS, libm and float mode; required because float statistics (`alpha_search.py:80-177`, `metrics.py`, `validation.py`, A4-006) are environment-relative.
8. **Outputs** — decisions, fills, fees, cash, positions, and P&L, plus an unmarked-residual validity signal (A4-003/Amendment A §A2: every nonzero position must have a mark or be flagged). Any `pnl_basis` value is **verifier-derived per §3.1.1**, never a producer assertion carried as evidence.
9. **State hashes** — canonical hashes of each stage's output state for byte-compare.
10. **Provenance anchor reference** — the ADR-M1-010 anchor receipt committing the source inputs **and** (per §3.1.1) the replay-derived verified-equal field-set definition and the fill-stream commitment on which any P&L equality depends; without it the record cannot claim source authenticity or P&L equality. The anchor is an unimplemented, unowned precondition (ADR-M1-010 §4), not a present control.

### 3.3 Determinism / nondeterminism handling

- Verified equality is claimed only when a fresh process, varying the recorded seeds and the numerical environment, reproduces every `verified-equal` field byte-for-byte.
- Known nondeterminism sources are controlled or disqualifying: wall-clock/PRNG/builtin `hash()`/`HashMap`/`HashSet` were absent in the audited Python pipeline (A4 §10) and must remain absent in replay roots; float statistics require the runtime fingerprint and a cross-version byte-compare (A4-006); market-data corrections have no supersession model (A4-005) and must be represented as new immutable records, never silently mixed.
- If numerical equivalence cannot be demonstrated across the declared environment set, the affected fields are demoted from `verified-equal` — not silently proclaimed deterministic.

### 3.4 No-overwrite persistence

Replay/experiment records are append-only and content-addressed; rejected trials are preserved and cannot be overwritten (R2 D2, AGENTS.md:9 "preserve failed experiments" is currently unenforced). This is a contract requirement; the concrete store is out of scope for M1.

### 3.5 The six owner challenges — implications carried by this ADR

Challenge 5 (replay) is primary here.

claim.
2. **OMS.** An UNKNOWN order (`durable_oms.py:397-421`) is an observed state, not a recomputable output; replay may verify the *state-transition trace* but cannot verify a broker outcome that was never observed. Reconciliation evidence (F-A5-04) must be an external observation class in the record.
3. **Events and partitions.** The one-frame→two-event identity problem (`__multi_instrument_frame__`, `upstox_recorder.rs:238`) means the replay record keys source hashes at the frame boundary, not only per derived event. Account-global P&L/exposure (`state.rs:113-116`, `engine.rs:955-971`) must be recomputed/verified at the aggregate level; partition-local replays cannot each claim verified equality for a global figure (R1-Q3, R1-C). Primary decisions: Runtime ADR-005/006.
alone.
5. **Replay (primary here).** §3.1 prohibits copied-forward P&L as verified equality (M1-REP-INV-001/002); §3.2–3.4 define the record, fresh-process determinism and no-overwrite persistence; this is the full contract of this ADR.
6. **Evidence.** A replay claim only becomes `verified` under ADR-M1-011 with a named fresh-process command, environment and limitations; a recorded `verified_internal_integrity` dataset (point_in_time.py:726-762) is not that.

**Charter-fit reaffirmation (challenger Section D).** M1 authorises no anchor service, storage engine, schema change, registry edit or gate code; no implementation and no platform-readiness claim; design documents (including this ADR) cannot self-certify; and nightly run `43792611015` is unverified by owner ruling and is not CI or release evidence (`CONSOLIDATED-BASELINE.md:25`, Amendment A §A9). The "information contract" in §3.2 creates no schema file; the unification of the two experiment-record schemas and the mandatory field surface are future contract requirements, and whether they constitute a forbidden "schema change" is referred to owner ruling (challenger A-5/§D.1). Every §9 item is a future-milestone specification, not an M1 gate.

---

## 4. Ownership

- **Decision author:** Governance Design Group (M1). **Approver:** repository owner.
- **State created:** the replay-record contract, the `verified-equal` field classification, the no-overwrite record store requirement and the anchor-reference binding. **Operational owner UNSET in baseline:** CAP-015 (research-tooling) is a team label, not a named individual (A7 §9); the experiment record currently has no producer (A4-002). Requirement: a named owner for the record store and the compare driver must be assigned before any replay claim is adoptable; recorded here as an open ownership gap, not filled.

---

## 5. Invariants (explicit IDs)

- **M1-REP-INV-001 (No tautological P&L).** A field copied from `RiskStateObserved` into the replay (`engine.rs:354-368`→`state.rs:573-577`→`engine.rs:971`) is never `verified-equal`, unless recomputed from fills+marks under mode (A).
- **M1-REP-INV-002 (Verified-equal is re-derived).** Only fields the fresh-process replay itself recomputes may be verified-equal; external observations are labelled and excluded by default.
- **M1-REP-INV-003 (Fresh process).** System replay equality is claimed only from a fresh process with recorded seeds and numerical fingerprint; in-process composition tests do not count.
- **M1-REP-INV-004 (Fingerprint or demote).** Without a runtime fingerprint and cross-environment byte-compare, float-derived fields are demoted from verified-equal.
- **M1-REP-INV-005 (No overwrite).** Experiment/replay records and rejected trials are immutable and content-addressed; overwriting a trial invalidates its claim.
- **M1-REP-INV-006 (Aggregate coverage).** Account-global outputs are verified at aggregate level; pass of every partition-local replay does not imply an account-global verified-equal result.
- **M1-REP-INV-007 (Source anchor required).** A replay record without an ADR-M1-010 anchor receipt for its source inputs cannot assert source authenticity (verification laundering, R2 C1).
- **M1-REP-INV-008 (Mark validity).** P&L in a verified comparison must be accompanied by an unmarked-residual signal; a nonzero position without a mark is flagged, never silently valued at zero (A2/Amendment A §A2).
- **M1-REP-INV-009 (No self-certifying `pnl_basis`).** `pnl_basis` and the verified-equal field set are verifier-derived (§3.1.1), not producer assertions; a field is excluded unless in the replay-derived set or verifier-recomputed from an anchored fill stream. A producer-set discriminator has no evidentiary force (challenger A-4/B-5).
- **M1-REP-INV-010 (Residual-bypass disclosure).** Until an anchor and a fresh-process driver exist, P&L exclusion is a documentation convention with residual forgery risk; this bypass is recorded as unclosed and no M1 claim may assert machine-enforced exclusion.

---

## 6. Failure cases (concrete, adversarial)

1. **Tautological P&L compare green on a large loss.** A caller calls `observe_risk_state(daily_pnl=0, ...)` (`engine.rs:354-368`) while fills imply a large loss; replay reads `daily_pnl` at `engine.rs:971` and "verifies" it. Under M1-REP-INV-001 the P&L field is excluded (mode B) or recomputed (mode A); the compare cannot pass on the copied value.
2. **Cross-version float drift misattributed.** Same code and data under Python 3.12 (CI pin) vs 3.14 (local) produce a different `report_sha256` (A4-006). Without the fingerprint (M1-REP-INV-004) the drift is unattributable; with it, the field is demoted until a cross-version byte-compare passes.
3. **Fabricated provenance laundered into a "verified" replay.** A canonical row carries an arbitrary valid 64-hex `raw_sha256` matching no captured payload; `verify-dataset` (`point_in_time.py:726-762`) checks hex shape only, returns `verified_internal_integrity`, and the replay "verifies" a dataset built on fabricated provenance (R2 C1). M1-REP-INV-007 requires the anchor receipt to refute it.
4. **Unmarked residual valued at zero.** A one-sided-book fill leaves `marks={}`, `net_pnl=-100` (A4-003 E, policy-conformant per R2 B2). An automated consumer treats it as authoritative. M1-REP-INV-008 flags the residual; mode (A) recompute must apply the documented mark policy.
5. **Rejected trial overwritten.** Two alpha screens produce the same artifact key; the losing trial is overwritten (R2 D2). M1-REP-INV-005 forbids overwriting; no-overwrite store required.
6. **Partition-local passes hide account-global breach.** Multi-instrument frames split for replay; each partition's risk check passes while aggregate exposure breaches (`engine.rs:955-971`; R1-Q3). M1-REP-INV-006 forbids the global claim.
7. **Reconciliation fabricated as replay output.** A malicious caller supplies `apply_broker_event` values (`durable_oms.py:456-550`) that drive an order to FILLED; replay treats the state as recomputed. M1-REP-INV-002 classifies it as an external observation; authenticity is a provenance/authority concern (ADR-M1-010, Architecture ADR-M1-002).
8. **Caller lies with `pnl_basis` (challenger B-5).** A producer copies `RiskStateObserved.daily_pnl` (`engine.rs:354-368`→`state.rs:573-577`→`engine.rs:971`) but stamps `pnl_basis=excluded` (or `recomputed`) in its own record; the compare reports verified-equal. Under M1-REP-INV-009 the discriminator has no force: only the verifier's field-set definition or an anchored fill-stream recomputation admits P&L. **Residual (M1-REP-INV-010):** with no anchor and no driver today, this forgery is undetectable by any committed check — the exclusion is a documentation convention, not a control.

---

## 7. Dependencies

- **On other ADRs:** ADR-M1-010 (anchor receipt for source inputs, §3.2 item 10); ADR-M1-008 (ACK/session-completeness boundary for checkpoint coverage); ADR-M1-011 (evidence for any `verified` replay claim); Architecture ADR-M1-002/003 (authority/OMS state classes); Runtime ADR-M1-005/006 (event identity, partition/aggregate semantics).
- **On evidence:** A4-001/002/003/005/006/007/008; R1-C, R1 D1, R2 §§2/C1/D2; Amendment A §§A2/A4/A5; `CONSOLIDATED-BASELINE.md:140-148,256-258,278`.
- **On external capabilities (absent):** unified experiment record producer, no-overwrite store, cross-version numerical environment, real provider raw archives (CAP-005/012), off-host anchor.
- **Constraint:** no new CLI driver, schema, service or live path is authorised by M1; §9 commands are future gate specifications.

---

## 8. Rejection conditions

This decision is revised if: (a) a future design proves P&L can be recomputed from fills+marks with a specified mark policy and an independent reference, allowing mode (A) to become default; (b) an authoritative event/partition model (Runtime ADR-005/006) changes where the source-hash boundary lies; (c) a cross-environment study shows the numerical fingerprint is insufficient and fields must remain permanently demoted; (d) an anchor (ADR-M1-010) becomes available and changes the provenance requirement.

**Recorded unresolved disagreements (not resolved by vote):** R1 D1 (reject a P&L-inclusive compare) was accepted by Amendment A §A5, but the baseline records **both** allowed remedies — "recompute or exclude" (`CONSOLIDATED-BASELINE.md:278`) — and the choice between mode (A) and mode (B) is left to the implementing ADR with evidence; this ADR sets exclusion as the safe default, not a settled universal. The challenger's A-4/B-5 self-certification defect is resolved by verifier-derived binding (§3.1.1) **and** the residual bypass is recorded as unclosed until an anchor and driver exist (M1-REP-INV-010); no machine-enforceable exclusion is claimed at M1. The two incompatible experiment record schemas are not reconciled here (A4-002); whether that unification is a forbidden schema change is referred to owner ruling (challenger A-5/§D.1). Cross-version float divergence is code-traced, not executed (A4-006; R2 §B4). Baseline §13 items 6 (authoritative event model) and 7 (authoritative OMS/authorization) remain open.

---

## 9. Measurable acceptance requirement (DESIGN ONLY — not executed by M1)

Each item carries a named command/test, a threshold, an **Owner (role)** and a **Target milestone**. Per challenger §C, items depending on absent capabilities are **PRECONDITION — not an M1 gate**. Roles are not named persons (baseline "owners" are team labels, A7 §9); assignment is an owner action. M1 defines these; it runs none.

1. **Fresh-process compare driver.** Named command (proposed): `quantlab replay-run --run <checkpoint> --compare` via `subprocess` across `PYTHONHASHSEED=1|2|12345|random` (A4-001/MT-01). **Owner:** research-tooling owner (CAP-015; unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (command does not exist; A4-001). **Threshold:** byte-identical verifier-derived `verified-equal` fields and state hashes across all seeds; 0 mismatches; in-process tests do not qualify.
2. **No copied-forward P&L / no self-certifying `pnl_basis` (challenger A-4/B-5).** Named falsifier (design): set `RiskStateObserved.daily_pnl` to a value contradicting fill notional (`engine.rs:354-368`, `state.rs:573-577`) **and stamp `pnl_basis=excluded` (or `recomputed`) in the producer record**; assert the compare still refuses P&L equality unless the verifier recomputes from an anchored fill stream. **Owner:** replay-contract owner + anchor authority (both unassigned/unimplemented). **Target milestone:** M3 (after anchor + driver). **Status:** PRECONDITION — not an M1 gate (no anchor, no driver; M1-REP-INV-010 records the unclosed bypass). **Threshold:** 100% of copied contradictions rejected even when mislabelled; a green compare on a copied value is a failure. Current: P&L included tautologically and no machine check exists → fails.
3. **Runtime fingerprint present.** Named command (proposed): `quantlab alpha-search <fixture>` then inspect the emitted record (MT-02). **Owner:** research owner (CAP-010/CAP-015; unassigned). **Target milestone:** M2. **Status:** gateable once the record producer exists; named owner required. **Threshold:** 100% of records carry Python version, constraints digest, platform and numerical fingerprint; today 0% (A4-006).
4. **Cross-environment byte-compare.** Named command (proposed): run `alpha-search` under two supported Python versions and byte-compare reports (MT-06). **Owner:** CI/research-tooling owner (unassigned). **Target milestone:** M2+. **Status:** PRECONDITION — not an M1 gate (only Python 3.14 available at baseline; second pinned environment absent). **Threshold:** 0 differing `verified-equal` fields, else affected fields demoted (M1-REP-INV-004).
5. **No-overwrite persistence.** Named test (proposed): re-register a rejected trial under the same key (R2 D2). **Owner:** research-tooling owner (unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (no record store exists). **Threshold:** overwrite refused; rejected trial preserved; 100% of prior trials present.
6. **Mark-validity signal.** Named test (proposed): one-sided-book fill scenario of A4-003 (MT-03/MT-09). **Owner:** research owner (CAP-009; unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (no unmarked-residual signal exists). **Threshold:** every nonzero position marked or flagged; 0 silently zero-valued residuals in a verified comparison.
7. **Source-anchor binding.** Named falsifier (design): inject an arbitrary valid 64-hex `raw_sha256` with no captured payload, build dataset, `verify-dataset` (R2 C1). **Owner:** anchor authority (UNSET/unimplemented, ADR-M1-010 §4). **Target milestone:** undetermined; no M1 path. **Status:** PRECONDITION — not an M1 gate. **Threshold:** rejected when no anchor receipt validates it; current behaviour returns `verified_internal_integrity`, exit 0 → fails.

*Design only. No command above has been executed by M1; thresholds are requirements for M2+ gates. Items 1, 2, 4, 5, 6, 7 are preconditions, not M1 acceptance gates.*
