# ADR-M1-011 — Evidence model: maturity vs verification status, fail-closed claims

**Status:** PROPOSED — Governance Design Group, M1 Platform Constitution (2026-11-17)
**Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9` (read-only reference)
**Scope:** DESIGN ONLY. Defines the claim/evidence vocabulary and a design requirement to make the registry gate non-vacuous. Authorises no registry edit, gate code, CI change or live change. No acceptance command below is executed by M1.
**Authority:** M1 does not implement. Adoption requires repository-owner approval.
**Amendment 2 — challenger disposition:** Resolved finding **B-6** (evidence sufficiency now requires declared test relevance and attested execution, and the process-isolation precondition is a structured flag, not free-text `limitations`) and **§C/§9** (named owners, milestones, PRECONDITION labels). Two-field model and nightly exclusion (challenger §E) retained unchanged. Enforcement timeline (no gate code in M1) is referred to owner ruling. Finding **A-0** is resolved: this ADR's `M1-EV-INV-` namespace was already unique; the coordinator's authoritative `INVARIANT-REGISTRY.md` verifies zero duplicate definitions across all 11 ADRs.

---

## 1. Problem

Two machine-readable registries cannot express verification status (A7-001, S1). `engineering/capabilities.json` uses `status ∈ {implemented, partial}` for all 19 entries; the string `verified` never appears, and the free-text `evidence_status` (e.g. `:15`) is unvalidated. `engineering/verification-policy.md:6-12` defines **Proposed / Implemented / Verified / Falsified** and states "No capability becomes `verified` by changing JSON" and that a verified claim needs a named command, named tests and a recorded date (`:14-15`). `claims/ledger.json` uses that vocabulary but has **13/13 claims `implemented`, zero `verified`, zero `evidence` blocks** (`:7,19,27,...,144`). A third vocabulary exists in `production-readiness-matrix.md:69-80` (`implemented / locally_verified / provider_verified / data_certified / operationally_certified / live_authorized`).

The enforcement code is therefore **vacuous**: `scripts/check_repo.py:174-187` (`check_claims`) requires `command/verified_at/tests` only when `status == "verified"`; with no verified claim the gate is green without checking anything (A7-001). `scripts/validate_contracts.py:40-58` only self-validates schema documents, never instances. Evidence levels A–F (`CONSOLIDATED-BASELINE.md:8`) show **highest committed evidence B (source) + C (unit tests); D/E/F absent** (A7-002), and every "PASS / 278 tests" figure is level-A documentation.

Owner ruling: the cited nightly run `actions/runs/43792611015` returns 404 unauthenticated and no run id is committed; it is **DOWNGRADED TO UNVERIFIED** and must not count as CI or release evidence (`CONSOLIDATED-BASELINE.md:25`, Amendment A §A9 condition 5).

---

## 2. Alternatives considered (with rejection reasons)

| Alternative | Rejection reason |
| --- | --- |
| **Keep the single `status` field and add more labels** | Conflates "code exists" (maturity) with "independently verified" (evidence); any reader of `implemented` over-infers verification (A7-001). Rejected. |
| **Set some claims to `verified` to make the gate fire** | Would be a fabricated verification — policy forbids JSON-only promotion and no named command/date/environment evidence exists (A7-002). Rejected. |
| **Count the nightly run / local rerun as `Verified`** | Owner ruling downgraded it to UNVERIFIED; no hosted run id or attestation is committed (§1). Rejected. |
| **Keep the third `production-readiness-matrix.md` vocabulary as equally authoritative** | Its own `locally_verified` criterion is unmet (`evidence/VERIFICATION-2026-11-17.md:12` says no VCS metadata present), producing contradiction (A7 §3.1). Rejected as a status source. |
| **Trust design documents / this ADR as certification** | Design work has no certifying authority; owner approval is required and M1 authorises no implementation (README §15, Amendment A §A6). Rejected (M1-EV-INV-007). |
| **Make the gate non-vacuous by code changes during M1** | M1 is ADR/design only; gate code belongs to a future milestone. Rejected as present work; stated as a design requirement instead (§3.3). |

---

## 3. Decision

### 3.1 Two-field model

Every claim/capability record must carry two independent fields:

- **`implementation_maturity`** ∈ `{proposed, partial, implemented}` — describes code/design existence only. Mapped from the current `capabilities.json` `implemented`/`partial` labels and `verification-policy.md` Proposed/Implemented.
- **`verification_status`** ∈ `{unverified, locally_verified, provider_verified, operationally_certified, verified, falsified}` — describes evidence strength, with `unverified` the default. Mapped from verification-policy Verified/Falsified and the third vocabulary's tiers, collapsed into one authoritative ladder.

Rules:
- The two fields are independent: `implemented` maturity with `unverified` status is the normal baseline state; maturity never implies verification.
- The nightly-run claim is `unverified` by owner ruling and remains so until a committed run id/attestation bound to a SHA exists (§1).
- `falsified` requires the recorded falsifier to have failed; a code-traced concern stays
  `unverified`, not `falsified` (R2 B4 evidence-language correction).

### 3.2 Structured evidence object (required for every non-unverified state)

A record whose `verification_status ≠ unverified` must carry a structured evidence object with, at minimum:

- `command` — the exact named command executed (e.g. a `cargo test`/`pytest`/named gate command). A plausible string is a claim, not proof of execution.
- `verified_at` — recorded date/time. Self-declared; insufficient on its own (see `execution_attestation`).
- `tests[]` — named tests referenced as `path::test_name` (reusing the citation format in `check_repo.py:180-187`), each of which must exist **and be declared relevant**.
- `test_relevance[]` — a **declared mapping from the claim to each cited test**: for every `tests[]` entry, an explicit statement of which part of the claim's falsifier that test exercises. A real but unrelated `path::test_name` fails this requirement (challenger B-6).
- `execution_attestation` — structured proof that the command ran: a named hosted `run_id` **or**
  a recorded command-output reference, plus `exit_status`, `test_counts` and the `commit` it ran
  against. This attests execution; `command` + `verified_at` alone do not (challenger B-6).
- `environment` — toolchain/versions/target (the baseline `verification-policy.md:15` asks for commit SHA, toolchain, target triple, command, exit status, test counts, artifact hashes).
- `limitations` — narrative scope/limits (e.g. absence of anchor/real data, ADR-M1-010). This is **not** the enforcement point for the process-isolation precondition.
- `isolation_attested` — a **structured flag** for the process-isolation precondition (Challenge 1), replacing free-text prose as the machine-checkable field: `isolation_posture ∈ {not_applicable, in_process, separate_process_administered}` plus `isolation_attested ∈ {true, false}`. An authorization claim whose precondition is a separately administered process with its own signer key policy is `unverified` unless `isolation_attested = true` with `isolation_posture = separate_process_administered`; `in_process` cannot satisfy CLM-010's independence claim.

**Residual (challenger re-verification N-3):** the flag and posture are asserted by the claimant;
  no field in this evidence object independently attests that a separate workload identity
  existed. `execution_attestation` proves a command ran, not that isolation was real. A determined
  claimant can set `isolation_attested=true, isolation_posture=separate_process_administered` with
  no external proof. Required future control: bind the posture to an independently attested source
  (anchor/authenticated run metadata from the isolated workload). Until that exists the flag is a
  **self-declaration with residual forgery risk** (M1-EV-INV-011); no M1 claim may call isolation
  machine-enforced.
- `commit` — the SHA the evidence was produced against.

Absent any required element — including a missing relevance mapping, missing execution attestation, or an unsatisfied structured isolation flag — the record is treated as `unverified` (fail closed, §3.3).

### 3.3 Fail-closed rule and non-vacuous gate (design requirement)

- **Fail closed:** if evidence is absent, incomplete, cites a non-existent test, cites a test with no declared relevance, lacks execution attestation, or leaves the structured isolation precondition unsatisfied, `verification_status` must be forced to `unverified`; a non-unverified label without complete evidence is invalid and the gate must reject it. No claim silently inherits verification from a sibling record or a design document.
- **Make the gate non-vacuous (design requirement, no code now):** the future gate must (a) require the structured evidence object for **any** non-unverified state, not only `verified` (fixing `check_repo.py:174-187`); (b) validate test **existence** (reuse `check_repo.py:180-187`); (c) validate test **relevance** against the declared `test_relevance[]` mapping — a real but unrelated test is a rejection (challenger B-6); (d) validate **execution attestation** — a named run id or recorded command output with exit status/test counts bound to the cited commit — so a plausible `command` plus a fabricated `verified_at` cannot pass (challenger B-6); (e) validate the structured `isolation_attested`/`isolation_posture` flag rather than parsing prose `limitations`; (f) include committed negative-control fixtures covering **semantic sufficiency failures** (irrelevant test, missing attestation, false isolation flag), not only a malformed shape, so the gate demonstrably *can* fail across those classes (A7-004/A7-005; challenger B-6); (g) keep `validate_contracts.py` instance-conformance honest — schema self-validation alone (`scripts/validate_contracts.py:40-58`) is not evidence of artifact conformance.
- **Enforcement timeline:** no gate code, registry edit or CI change is built in M1; until the future gate exists, "editing a registry cannot raise status" (M1-EV-INV-004) is a design rule, not an enforced control. The milestone and accountable owner for that gate are referred to owner ruling (challenger §F.6/§F.8). M1 changes no registry, gate or CI file.

### 3.4 The six owner challenges — evidence implications carried by this ADR

Challenge 6 (evidence) is primary here.

1. **Authorization (risk-token boundary).** CLM-010's "independent risk authority" is conditionally true only under process isolation; the importable token (`risk_service.py:24-26,192-204`) signs arbitrary `RiskFacts` in-process. Evidence implication: no authorization claim may be `verified` while the signer is co-located with OMS/strategy; the process-isolation precondition must be expressed by the structured `isolation_attested`/`isolation_posture` fields of §3.2 (not by prose `limitations`), and no remote service is assumed as the answer (Architecture ADR-M1-002 decides the boundary).
2. **OMS lifecycle / UNKNOWN / reconciliation.** CLM-009's crash falsifier has no crash-injection test (A3-06); no broker reconciliation exists (F-A5-04). Evidence implication: OMS atomicity/UNKNOWN-recovery claims remain `implemented`/`unverified` until a crash test and reconciliation drill are recorded; UNKNOWN/quarantine states must appear in evidence scope, not be omitted.
3. **Events and partitions.** Source/replay claims depend on frame-level anchoring (ADR-M1-010 INV-006/007) and aggregate coverage (`engine.rs:955-971`, `__multi_instrument_frame__` at `upstox_recorder.rs:238`). Evidence implication: a partition-local verification cannot be cited as evidence for an account-global claim; limitations must state the identity/partition model assumed (Runtime ADR-005/006).
4. **Durability.** A level claim (ADR-M1-008) becomes `verified` only with its named fault-injection/RPO/RTO command in evidence; no durability claim is verified today (A3-10).
5. **Replay.** A replay result is `verified` only with the fresh-process command, runtime fingerprint and anchor receipt as evidence; a `verified_internal_integrity` dataset (`point_in_time.py:726-762`) is not that (ADR-M1-009).
6. **Evidence / self-certification.** This ADR and all M1 documents are proposals; they cannot certify themselves. Owner approval is required (README §73-80, Amendment A §A9), and no claim is promoted by this document.

**Charter-fit reaffirmation (challenger Section D).** M1 authorises no anchor service, storage engine, schema change, registry edit or gate code; no implementation and no platform-readiness claim; design documents (including this ADR) cannot self-certify; and nightly run `43792611015` is unverified by owner ruling and is not CI or release evidence (`CONSOLIDATED-BASELINE.md:25`, Amendment A §A9). The gate in §3.3 is a future-milestone specification, not an M1 gate, and this ADR edits no registry or gate file.

---

## 4. Ownership

- **Decision author:** Governance Design Group (M1). **Approver:** repository owner; owner approval is required for any claim promotion and for adoption of this model.
- **State created:** the two-field claim model, the structured evidence object, the fail-closed/non-vacuous-gate requirements. **Operational owner of the registries and gate is UNSET in the baseline:** "owner" fields are team labels, not named individuals (A7 §9), and `claims/ledger.json`/`capabilities.json` have no accountable person recorded. Requirement: name a registry/gate owner before the model is adoptable; the verification-policy authority is the owner's ruling, not self-granted. Gap recorded, not silently filled.

---

## 5. Invariants (explicit IDs)

- **M1-EV-INV-001 (Two fields).** `implementation_maturity` and `verification_status` are separate; neither is derivable from the other.
- **M1-EV-INV-002 (Unverified default).** Absent complete structured evidence, `verification_status = unverified`.
- **M1-EV-INV-003 (Structured evidence required).** Every non-unverified state carries `command`, `verified_at`, `tests[]`, `environment`, `limitations`, `commit`; cited tests must exist.
- **M1-EV-INV-004 (No JSON-only promotion).** Editing a registry cannot raise verification status (verification-policy.md:14-15).
- **M1-EV-INV-005 (Non-vacuous gate).** The gate must be demonstrably capable of failing a malformed non-unverified claim via a committed negative-control fixture.
- **M1-EV-INV-006 (No false falsification).** A concern that is code-traced with an unexecuted falsifier is `unverified`, not `falsified` (R2 B4/A7 correction).
- **M1-EV-INV-007 (No self-certification).** Design documents, including this ADR, cannot certify claims; only owner approval plus recorded evidence can promote a claim.
- **M1-EV-INV-008 (Scope fidelity).** Evidence cited for partition-local or paper-only behaviour cannot be presented as evidence for account-global or live authority (Challenges 1–4).
- **M1-EV-INV-009 (Semantic evidence sufficiency).** Evidence is sufficient only if each cited test has a declared relevance to the claim's falsifier **and** execution is attested by a named run id or recorded command output with exit status/test counts bound to the cited commit. Test existence alone is insufficient (challenger B-6).
- **M1-EV-INV-010 (Structured isolation flag).** The process-isolation precondition is represented by structured `isolation_attested`/`isolation_posture`, not free-text `limitations`; an authorization claim remains `unverified` unless the flag attests a separately administered process (challenger B-6/C).
- **M1-EV-INV-011 (Isolation-flag residual).** `isolation_attested`/`isolation_posture` are claimant-supplied and not independently attested by this evidence object; until posture binding to an external authority exists, the flag is a self-declaration with residual forgery risk and no M1 claim may call isolation machine-enforced (challenger re-verification N-3).

---

## 6. Failure cases (concrete, adversarial)

1. **Maturity read as verification.** A consumer reads `status: implemented` for CAP-001/008/019 and infers a verified capability; policy explicitly denies this. M1-EV-INV-001/002 force the two-field reading with `unverified` default.
2. **Vacuous green gate.** With 13/13 claims `implemented` and zero `verified`, `check_repo.py:174-187` never runs its evidence checks (A7-001). M1-EV-INV-005 requires a negative-control fixture that makes the gate fail on a malformed claim; current gate cannot.
3. **Nightly claim counted as evidence.** An operator cites run 43792611015 as CI verification; it is 404/uncommitted and owner-downgraded. M1-EV-INV-002/004 keep it `unverified`.
4. **Self-certifying design.** This ADR or a capability entry declares "verified by M1 design." M1-EV-INV-007 forbids; only owner approval + recorded evidence promote.
5. **Co-located risk signer claimed as independent authority.** CLM-010 set to `verified` while `_GATE_SIGNING_CAPABILITY` remains importable (`risk_service.py:24-26,192-204`). M1-EV-INV-010 requires the structured isolation flag (`isolation_attested`/`isolation_posture`); prose `limitations` has no enforcing force, and without the flag the claim fails closed to `unverified` (Challenge 1; challenger re-verification N-2).
6. **OMS crash claim without a crash test.** CLM-009 promoted to verified although no SIGKILL test exists (A3-06). M1-EV-INV-003/006 keep it `implemented`/`unverified` until the crash drill and reconciliation evidence exist (Challenge 2).
7. **Code-traced concern labelled `falsified`.** A researcher marks a claim falsified because a proposed test "would" fail. M1-EV-INV-006 forbids; only an executed falsifier qualifies.
8. **Partition evidence cited for global claim.** A per-instrument replay test is attached to an account-global risk claim; aggregate breach (`engine.rs:955-971`) unnoticed. M1-EV-INV-008 rejects the citation (Challenge 3).
9. **Real but irrelevant test + fabricated date (challenger B-6).** A claim is promoted to `verified` with a complete-looking evidence object citing a real `path::test_name` that tests an unrelated property, a plausible `command`, and a made-up `verified_at`. An existence-only gate passes; under M1-EV-INV-009 the missing `test_relevance[]` mapping and `execution_attestation` force `unverified`. No gate is built in M1, so this remains a design rule until the future milestone (owner ruling).
10. **Isolation asserted in prose, not structured (challenger C).** A CLM-010-style claim writes "independently isolated" in free-text `limitations` while running in-process with the importable token (`risk_service.py:24-26,192-204`). Under M1-EV-INV-010 the prose has no force: `isolation_attested` must be `true` with `isolation_posture = separate_process_administered`, else the claim fails closed to `unverified`.

---

## 7. Dependencies

- **On other ADRs:** ADR-M1-008 (durability level claims need measured evidence), ADR-M1-009 (replay claims need fresh-process evidence), ADR-M1-010 (provenance claims need an anchor receipt and cannot be self-certified); Architecture ADR-M1-002/003 and Runtime ADR-005/006 own Challenges 1–3 whose evidence implications are recorded in §3.4.
- **On evidence:** A7-001/002/003/004/005, R2 B4/§E, `CONSOLIDATED-BASELINE.md:25,74-92,250-255,368-373`, `verification-policy.md:6-15`, `check_repo.py:158-188`, `validate_contracts.py:40-58`.
- **On external capabilities (absent):** hosted CI run/attestation bound to a SHA, applied rulesets, named accountable registry owner. Their absence is why all claims remain at most `implemented`/`unverified` today (owner decision, baseline §13 item 10).
- **Constraint:** no registry/gate/CI change is authorised by M1; §9 commands are future gate specifications.

---

## 8. Rejection conditions

This decision is revised if: (a) the owner adopts a different verification vocabulary or assigns a
  different authoritative registry; (b) hosted CI/attestation becomes attainable, changing which
  states are reachable (owner decision, baseline §13 item 10); (c) an independent owner rules that
  a specific claim's falsifier has failed, moving it to `falsified`; (d) a security/authority
  decision (Architecture ADR-M1-002) changes what counts as sufficient isolation evidence.

**Recorded unresolved disagreements (not resolved by vote):** Three vocabularies coexist (`capabilities.json`, `verification-policy.md`, `production-readiness-matrix.md`); this ADR proposes one collapse but does not delete the others (A7-001/§3.1). The nightly run is downgraded by owner ruling, not by evidence analysis (§1, Amendment A §A9). R2's epistemic challenge — that "reproducible scenarios" often means B/C-traced with proposed tests, not E-reproduced (R2 §E, Amendment A §A7) — is adopted as the `unverified`-by-default rule; whether D/E/F is attainable on the current account remains an open owner decision (baseline §13 item 10). The challenger's B-6 sufficiency defect is resolved at the design level (§3.2/§3.3, M1-EV-INV-009/010), but **the enforcement timeline — who owns and when the future gate is built — is referred to owner ruling** (challenger §F.6/§F.8); until then M1-EV-INV-004 is a design rule, not an enforced control. These are recorded, not resolved by vote.

---

## 9. Measurable acceptance requirement (DESIGN ONLY — not executed by M1)

Each item carries a named command/test/check, a threshold, an **Owner (role)** and a **Target milestone**. Per challenger §C, items depending on absent capabilities (registry/gate extension, negative-control fixture) are **PRECONDITION — not an M1 gate**. Roles are not named persons (A7 §9); assignment is an owner action. M1 defines these; it runs none.

1. **Two-field presence.** Named command (proposed): a registry lint asserting every claim/capability record has both `implementation_maturity` and `verification_status` with valid vocabularies. **Owner:** registry/gate owner (unassigned; §4). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (lint does not exist). **Threshold:** 100% of records carry both fields; today 0% (single `status` only).
2. **Semantic evidence sufficiency for non-unverified (challenger B-6).** Named command: extended `python3 scripts/check_repo.py claims`. **Owner:** registry/gate owner (unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (gate extension not built). **Threshold:** 100% of non-unverified records carry the complete evidence object including `test_relevance[]`, `execution_attestation` (run id or recorded output + exit status/test counts + commit), and structured `isolation_attested`/`isolation_posture`; a real but unrelated test, a plausible command with a fabricated `verified_at`, or an unattested isolation flag must fail. Current gate checks only `verified` existence (A7-001).
3. **Non-vacuous negative control (semantic classes).** Named test (proposed): committed fixtures
   with (a) a malformed non-unverified claim, (b) a real but irrelevant cited test, (c) a missing
   execution attestation, (d) an in-process claim with `isolation_attested=true`. **Owner:**
   registry/gate owner (unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1
   gate (fixtures and gate absent). **Threshold:** gate returns non-zero for all four classes; today
   no fixture exists and the gate is vacuously green.
4. **Nightly claim status.** Named check (design): assert the nightly-run claim is `unverified` with no CI/release evidence attached. **Owner:** repository owner (ruling) + registry/gate owner (M2 enforcement). **Target milestone:** owner ruling now; M2 gate. **Status:** owner-ruling condition, not an M1 gate. **Threshold:** 0 uses of run 43792611015 as evidence (`CONSOLIDATED-BASELINE.md:25`).
5. **No self-certification.** Named check (design): assert no M1 design document or registry entry sets any `verification_status` above `unverified` without recorded owner approval plus evidence. **Owner:** repository owner. **Target milestone:** M1 exit gate. **Status:** owner-ruling condition; design documents cannot self-certify. **Threshold:** 0 self-certified claims; all 13 baseline claims remain `implemented`/`unverified` until evidenced.
6. **Structured isolation precondition (challenger C).** Named check (design): any authorization claim above `unverified` must carry `isolation_attested=true` with `isolation_posture=separate_process_administered`, not prose in `limitations`. **Owner:** registry/gate owner + authorization owner (Architecture ADR-002; unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (gate not built). **Threshold:** 100% of such claims carry the structured flag; CLM-010 currently lacks enforceable isolation (Challenge 1).

*Design only. No command above has been executed by M1; thresholds are requirements for M2+ gates. Items 1, 2, 3, 6 are preconditions; 4 and 5 are owner-ruling conditions. Design documents cannot self-certify these results.*
