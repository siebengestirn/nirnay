# ADR-M1-010 — Signed off-host provenance anchor and source-trust boundary

**Status:** PROPOSED — Governance Design Group, M1 Platform Constitution (2026-11-17)
**Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9` (read-only reference)
**Scope:** DESIGN ONLY. Defines a trust/anchoring contract. Authorises no anchor service, key infrastructure, network, storage engine, schema or live change. **No anchor is implemented by M1.** No acceptance command below is executed by M1.
**Authority:** M1 does not implement. Adoption requires repository-owner approval.
**Amendment 2 — challenger disposition:** Resolved finding **B-4** (the D4 anchor authority is labelled an UNIMPLEMENTED, UNOWNED PRECONDITION with no M1 path to satisfaction; D4 is a requirement class, not a reachable level) and **§C/§9** (named owners, milestones, PRECONDITION labels). Charter-fit reaffirmed (§D). Finding **A-0** is out of scope: this ADR uses only the `M1-PROV-INV-` namespace, a separate scheme per `CHALLENGER.md:45`; no renumbering.

---

## 1. Problem

M0/R2 found **verification laundering**: every verification layer is internally consistent but unanchored to its source, so a fully "verified" dataset and deterministic P&L can rest on fabricated provenance (R2 C1, S1; Amendment A §A4 item 1). The chain is concrete:

- **Durability layer:** the raw journal checks only `payload_sha256` over decoded base64 (`frame.rs:75-97`); metadata edits and whole-record deletion are accepted (A3-01 E, `A3-durability.md:167-171`). The recorder's whole-file SHA-256 is written to `manifest.json` but **never re-verified and unsigned** (`upstox_recorder.rs:302-356`).
- **Canonical layer:** `MarketEvent.raw_sha256`/`source_frame_sha256` are **self-reported string fields** (`research/python/src/quantlab/market.py:87-112`), produced from decoded JSON/FYERS journal, never recomputed against an authoritative raw archive.
- **Replay/verification layer:** `verify_published_dataset` (`point_in_time.py:726-762`) checks the two hashes are well-formed hex (`_valid_sha256`) and that the dataset's stored hash *sets* match recomputed row hash sets; it reads **no raw archive** and echoes the builder's own `raw_prices_mutated=False`/`point_in_time=True` flags (`:506,524` set; `:734` checks). Falsifier: inject an arbitrary valid 64-hex `raw_sha256` corresponding to no captured payload → `verified_internal_integrity`, exit 0 (R2 C1).
- **Audit chains:** three independent internal chains (Python `oms.py`, Python `durable_oms.py`, Rust `account-journal`) have no external anchor and are not cross-linked; `verify_event_chain()` returns `{"signature": False}` (`durable_oms.py:562-578`; F-A5-06, S1).

A host with write access can rewrite every record with recomputed hashes and pass every local check. **R2 D1 proved that a standalone per-record chain creates false assurance** (Amendment A §A5): a chain and a manifest digest computed and re-verified by the same writer establish only self-consistency while adding a reassuring "verified" signal — a net *decrease* in real assurance. This ADR must not repeat that design.

---

## 2. Alternatives considered (with rejection reasons)

| Alternative | Rejection reason |
| --- | --- |
| **Standalone per-record digest + chain and manifest re-verification by the same recorder** | R2 D1 / Amendment A §A5: same-writer chain and same-writer manifest digest prove only self-consistency against a host compromise; adds false assurance. Explicitly rejected — must not be implemented as the fix. |
| **Trust self-reported `raw_sha256` in `MarketEvent` as provenance** | It is a caller-supplied string checked only for hex shape; arbitrary valid hashes pass (`point_in_time.py:726-762`). Rejected. |
| **Sign everything with a key held on the recorder/OMS host** | A host compromise obtains the signing key and can forge receipts; no trust boundary is gained. Rejected (contradicts the "key not on recorder host" requirement of R2 D1/Amendment A §14 #12). |
| **Make the anchor a synchronously-required remote service in every verification call** | Couples a hard-disabled local system to network infrastructure, creating a new SPOF; no measured requirement (Amendment A §A5 R1 D2). Rejected as a *mandatory online dependency*; the contract instead fails closed when the anchor is unavailable, without prescribing a service. |
| **Cross-link the three internal chains to each other** | Still three same-host chains; a single host compromise rewrites all consistently (F-A5-06). Useful only *in addition to* an external anchor. Rejected as a standalone answer. |
| **Implement the anchor during M1** | M1 is ADR/design only; no service, key management or network changes are authorised (README §10-15, Amendment A §A6). Rejected. |

---

## 3. Decision

### 3.1 The anchoring contract (design only; unimplemented)

An **anchor** is a signed, timestamped commitment to a chain head or content digest, written to a store that is *not writable by the producing host* and verifiable with a public key that is *not present on that host*. This ADR defines the contract, not an implementation. No anchor exists by M1.

**What is anchored:**
- The head of each of the three audit chains (Python `oms.py` `HashChainJournal`, Python `durable_oms.py` `verify_event_chain`, Rust `account-journal` BLAKE3 chain), so a full-chain rewrite can be detected against an external truth.
- The source-provenance commitment for capture/dataset lineage: the raw-frame `payload_sha256`/frame identity (the one-frame boundary of Challenge 3) and the dataset content digest, so `verify-dataset` can consult an external record rather than echoing self-reported hashes.
- A cross-chain consistency statement once a single OMS/authority owner is chosen (Architecture ADR-M1-002/003); the three chains are not silently assumed consistent today.
- **Replay P&L subjects (challenger re-verification N-1).** The verifier-derived **replay-equal field-set definition** and the **fill-stream commitment** on which P&L recomputation depends (ADR-M1-009 §3.1.1) are anchored subjects. Without committing to these, an anchor receipt cannot authenticate a P&L equality claim, and the `pnl_basis` remedy is not a closed loop. Both are future contract requirements; no anchor exists by M1.

**By whom / key management (preconditions, not built here):**
- The **anchor authority** is a distinct trust principal from the recorder/OMS host. It alone holds the signing key; the producer never possesses it.
- Key lifecycle (generation, storage, rotation, revocation) is off-host and independently controlled. The verifier holds only the public key / trust root.
- Key separation from the risk-signing authority and the importable capability token (`risk_service.py:24-26,192-204`) is mandatory: the anchor key must not be usable to forge authorization evidence (Challenge 1).

> **B-4 STATUS LABEL — the anchor authority is an UNIMPLEMENTED, UNOWNED PRECONDITION.** No anchor authority, off-host store, signing key, key lifecycle, trust-root distribution path or accountable owner exists at M1 (§4; F-A5-09; A7 §7 items 4–5). Creating one requires exactly the service/key/network infrastructure M1 forbids and for which no measured requirement exists (Amendment A §A5; challenger B-4). **There is no M1 path to satisfaction.** Consequently **D4 (ADR-M1-008 §3.2) is a requirement class, not a reachable level today**: it is unfalsifiable in M1 (no anchor to verify against), and no source-provenance or chain claim may be presented as anchored, verified, or platform-ready on the strength of this contract. The contract below is a future-milestone requirement, not a control.

**How verification consults the anchor:**
- A verifier must retrieve the anchor receipt (signed head/digest plus timestamp and the subject it commits to), verify its signature against the trust root, and compare the locally computed head/digest to the anchored value.
- A verification result may be `verified` **only** when this comparison succeeds. Internal consistency alone (`verified_internal_integrity`, `point_in_time.py:734`; `durable_oms.py:577`) is, at most, a *self-consistency* status and must be labelled as such.
- The trust boundary is exact: **the producing host, its journals, its manifest digests, its self-reported hashes and its internal chains are untrusted for integrity; only the off-host anchor and its key are trusted.** Anything derived solely from the producer is admissible as a claim, never as proof.

**Fails closed when the anchor is unavailable:**
- If the anchor cannot be retrieved, its signature is invalid, its key is untrusted/revoked, or the local value disagrees: verification must **not** return `verified`/`verified_internal_integrity`; it returns an explicit `anchor_unavailable` / `anchor_mismatch` outcome and the downstream claim fails closed (no promotion, no release evidence).
- Unavailability is never silently treated as "no anchor needed" and never downgraded to a green self-check.

### 3.2 What this ADR explicitly does NOT do

- Does not implement, purchase or configure an anchor service, KMS, network path or storage; does not define a concrete receipt format/schema.
- Does not prescribe a standalone per-record chain (R2 D1). A per-record chain is permissible only as an internal consistency aid whose head is anchored here; without the anchor it confers no verified status.
- Does not change any live/verification/release pipeline. It is a precondition contract for a future level (ADR-M1-008 D4) and replay authenticity (ADR-M1-009 INV-007).
- Does **not** make D4 a reachable level or any claim verifiable: the anchor authority is an unimplemented, unowned precondition (B-4 status label above). Until an anchor exists and an independent owner is named, all provenance claims remain below `verified` and D4 remains a requirement class only.

### 3.3 The six owner challenges — implications carried by this ADR

Challenge 4's "trusted off-host anchor" is primary here (see also ADR-M1-008 D4).

1. **Authorization (risk-token boundary).** Anchoring does not fix the importable token (`risk_service.py:24-26,192-204`) — a co-located signer can still forge runtime evidence before it is anchored. Evidence implication: an anchor receipt only authenticates *what was committed*, not the authority of the committer; the process-isolation precondition (Architecture ADR-M1-002) must be evidenced separately through the structured `isolation_attested`/`isolation_posture` fields of ADR-M1-011 §3.2; `limitations` is narrative only and has no enforcing force (challenger re-verification N-2). Anchor key ≠ risk-signing key.
2. **OMS lifecycle / UNKNOWN / reconciliation.** The OMS chain (`durable_oms.py:562-578`, `signature: False`) must be among the anchored heads; UNKNOWN/quarantine and reconciliation events must be within the committed subject so a full rewrite is detectable (F-A5-06). The anchor cannot resolve an UNKNOWN order (no broker query, F-A5-04) — it only makes the recorded history tamper-evident. Owner for the lifecycle: Architecture ADR-M1-003.
3. **Events and partitions.** The anchor must bind the **raw frame boundary** (`payload_sha256` of the `__multi_instrument_frame__` frame, `upstox_recorder.rs:238`), not only per-derived-instrument canonical events; otherwise splitting one frame into two events breaks the identity and a tampered multi-instrument frame could pass (R1-Q3). Account-global state (`state.rs:113-116`, `engine.rs:955-971`) must be within the committed subject; partition-local digests alone cannot attest the aggregate. Replay implication: ADR-M1-009 INV-007/006. Primary decisions: Runtime ADR-005/006.
4. **Durability.** D4 of ADR-M1-008 is defined by this anchor; without it, a rewritten chain still "verifies," so no level above D2 self-consistency is claimable. Per-frame ACK (`journal.rs:51`) is not an anchor and does not prove session completeness (`finish()` manifest, R2 C2).
5. **Replay.** `verify-dataset`'s hex-shape check (`point_in_time.py:726-762`) must consult the anchor; a replay record without an anchor receipt cannot assert source authenticity (ADR-M1-009 INV-007).
6. **Evidence.** An "anchored" claim is `verified` only with the anchor receipt as evidence under ADR-M1-011; design documents (including this ADR) cannot self-certify that an anchor exists.

**Charter-fit reaffirmation (challenger Section D).** M1 authorises no anchor service, key infrastructure, storage engine, schema change, registry edit or gate code; no implementation and no platform-readiness claim; design documents (including this ADR) cannot self-certify; and nightly run `43792611015` is unverified by owner ruling and is not CI or release evidence (`CONSOLIDATED-BASELINE.md:25`, Amendment A §A9). The broker/anchor services referenced in §3.1/§3.3 are future preconditions, not acts authorised here. Every §9 item is a future-milestone specification, not an M1 gate.

---

## 4. Ownership

- **Decision author:** Governance Design Group (M1). **Approver:** repository owner.
- **State created:** the anchor contract (trusted principal, key-separation preconditions, fail-closed semantics) and the list of anchored subjects. **Operational owner of the anchor service and key is UNSET and does not exist at M1** — no service, IAM/KMS key or collector exists (F-A5-09, A7 §7 items 4-5). Requirement: before any anchored claim is adoptable, an independent owner (not the recorder/OMS host operator) must be named for the anchor store and signing key. Recorded as an open gap; M1 does not fill it.

---

## 5. Invariants (explicit IDs)

- **M1-PROV-INV-001 (External trust root).** Integrity is relative to an off-host anchor and its key; no on-host chain, digest, manifest or signature is a trust root.
- **M1-PROV-INV-002 (No standalone chain).** A per-record chain or same-writer manifest digest, without an anchored head under an off-host key, confers no verified status (R2 D1).
- **M1-PROV-INV-003 (Fail closed).** Anchor unavailable, invalid, untrusted or mismatched ⇒ verification must not return `verified`; explicit `anchor_unavailable`/`anchor_mismatch` and no promotion.
- **M1-PROV-INV-004 (Self-reported ≠ provenance).** Self-declared `raw_sha256`/`source_frame_sha256` and manifest flags (`point_in_time.py:506,524,734`) are claims, not proof; verification must consult an anchor to authenticate them.
- **M1-PROV-INV-005 (Key separation).** The anchor key is distinct from the risk-signing capability token and is never present on the producing host.
- **M1-PROV-INV-006 (Frame-boundary binding).** The anchored raw-frame commitment binds the multi-instrument frame identity; partition-local digests cannot replace it (Challenge 3).
- **M1-PROV-INV-007 (Aggregate coverage).** The committed subject includes account-global state; anchored partition digests do not attest the aggregate.
- **M1-PROV-INV-008 (No implementation by M1).** This ADR creates a contract only; no anchor, service, network or key artifact is authorised or claimed to exist.
- **M1-PROV-INV-009 (D4 unattainable at M1).** The anchor authority is an UNIMPLEMENTED, UNOWNED PRECONDITION with no M1 path to satisfaction; D4 is a requirement class, not a reachable or verifiable level. No provenance claim may be presented as anchored/verified until an independent anchor authority exists and is evidenced (challenger B-4).

---

## 6. Failure cases (concrete, adversarial)

1. **Arbitrary valid hex hash laundered into "verified".** A canonical row gets a 64-hex `raw_sha256` matching no captured payload; `verify-dataset` passes (`point_in_time.py:726-762`, R2 C1 falsifier). Under M1-PROV-INV-004 verification must consult the anchor and reject; current behaviour returns `verified_internal_integrity`, exit 0.
2. **Full-chain rewrite by host compromise.** An attacker recomputes every record hash in `durable_oms.py` or rewrites the BLAKE3 `account-journal`; `verify_event_chain()` still returns `verified_internal_integrity` (`signature: False`). M1-PROV-INV-001/002 require the anchored head to detect; without an anchor this is undetectable (F-A5-06).
3. **Same-writer per-record chain as "fix" (R2 D1 replayed).** A well-meaning implementation adds a chain and re-verifies the recorder's own manifest digest. Operators trust it more while it detects nothing against host compromise. M1-PROV-INV-002 forbids granting verified status; the design must not present this as the solution.
4. **Anchor offline during verification.** The anchor store is unreachable or its receipt is absent. M1-PROV-INV-003: return `anchor_unavailable`, fail closed — never silently fall back to `verified_internal_integrity`.
5. **Anchor key compromised / signer co-located.** If the anchor private key is on the recorder host, a compromise forges receipts (M1-PROV-INV-005). If the risk-signing token is reused as the anchor key, forged authorization evidence is anchored as authentic (Challenge 1).
6. **Multi-instrument frame split defeats binding.** Two canonical events derived from one `__multi_instrument_frame__` frame are anchored individually; an attacker tampers one derived frame hash with no anchor match to the original payload. M1-PROV-INV-006 requires the frame-level commitment.
7. **Partition-local anchors attested as account-global.** Per-instrument digests are anchored while aggregate `daily_pnl` is tampered (`engine.rs:955-971`). M1-PROV-INV-007 requires the aggregate in the committed subject.
8. **Stale anchor replay.** An old valid receipt is presented for changed content; the digest comparison must detect mismatch (M1-PROV-INV-003 subject-binding). This is a requirement; no mechanism is implemented by M1.

---

## 7. Dependencies

- **On other ADRs:** ADR-M1-008 (D4 is defined by this anchor); ADR-M1-009 (anchor receipt required for replay source authenticity); ADR-M1-011 (an anchored claim becomes `verified` only with receipt evidence, and design docs cannot self-certify); Architecture ADR-M1-002/003 (authority/OMS owner and key separation); Runtime ADR-M1-005/006 (frame/partition identity).
- **On evidence:** R2 C1 and D1 (S1, governing), F-A5-06, A3-01/A3-05, A7-002/003; Amendment A §§A4 item 1, A5 row for §14 #6/#12, A9 condition 4; `CONSOLIDATED-BASELINE.md:158,206,276`.
- **On external capabilities (absent, all out of scope for M1):** an independently controlled off-host store, a signing key with off-host lifecycle, a trust-root distribution path, real provider raw archives to anchor against (CAP-005/012), and a reconciliation/authority owner. Their absence is why no anchor exists by M1.
- **Constraint:** no service, network, storage engine, KMS or schema change is authorised; the trust boundary is a contract only.

---

## 8. Rejection conditions

This decision is overturned or revised if: (a) a threat model shows an off-host anchor is unattainable on the current account — then all source-provenance claims must remain below `verified` and be explicitly downgraded (owner decision on D/E/F attainability, baseline §13 item 10); (b) an independent authority choice (Architecture ADR-M1-002/003) relocates which principal must hold the anchor key; (c) a single canonical event/frame identity (Runtime ADR-005/006) changes the anchoring subject; (d) operational evidence (F-level) shows a different trust boundary is sufficient.

**Recorded unresolved disagreements (not resolved by vote):** R2 D1 rejects the standalone per-record chain; Amendment A §A5 accepts a chain only when bundled with a signed off-host-anchored head (§14 #6 + #12) — this ADR follows that ruling, but no owner has yet decided whether an anchor is attainable. The baseline §13 item 10 (D/E/F evidence attainability) and item 6 (authoritative event model) remain open. F-A5-06 is a control gap (S1) while its realized scenario is S2 today; the anchor requirement is a future-wiring prerequisite, not a present incident. These are recorded, not resolved here.

---

## 9. Measurable acceptance requirement (DESIGN ONLY — not executed by M1)

Each item carries a named command/test, a threshold, an **Owner (role)** and a **Target milestone**. Per challenger §C, every anchor-dependent item is **PRECONDITION — not an M1 gate** (no anchor authority exists; B-4 status label in §3.1). Roles are not named persons (A7 §9); assignment is an owner action. M1 defines these; it runs none.

1. **Fabricated-provenance falsifier.** Named command (proposed): inject an arbitrary valid 64-hex `raw_sha256` with no captured payload, run `build-dataset` then `verify-dataset` (R2 C1). **Owner:** independent anchor authority (UNSET/unimplemented, §4). **Target milestone:** undetermined; no M1 path. **Status:** PRECONDITION — not an M1 gate. **Threshold:** exit non-zero with `anchor_mismatch`/absent-anchor outcome; current baseline returns `verified_internal_integrity`, exit 0 → fails.
2. **Full-chain-rewrite detection.** Named test (proposed): rewrite all rows of `durable_oms.py` event chain and the BLAKE3 `account-journal` with recomputed hashes, then run anchored verification (F-A5-06 missing test). **Owner:** independent anchor authority + authoritative-OMS owner (both unassigned). **Target milestone:** undetermined. **Status:** PRECONDITION — not an M1 gate; D4 unfalsifiable in M1. **Threshold:** anchor comparison rejects 100% of rewrites.
3. **Fail-closed on anchor outage.** Named test (proposed): run verification with the anchor store unreachable. **Owner:** verification/release-engineering owner (unassigned). **Target milestone:** M3+ after anchor exists. **Status:** PRECONDITION — not an M1 gate. **Threshold:** 0 `verified` results; outcome `anchor_unavailable`; no fallback to self-consistency green.
4. **Key-separation check.** Named test (proposed): assert the anchor verification path cannot be satisfied by the importable `_GATE_SIGNING_CAPABILITY` token (`risk_service.py:24-26`). **Owner:** independent anchor authority + authorization owner (Architecture ADR-002; unassigned). **Target milestone:** M3+. **Status:** PRECONDITION — not an M1 gate (no anchor key exists). **Threshold:** 0 forgeries accepted; token and anchor key distinct.
5. **Frame-boundary binding.** Named falsifier (design): tamper one derived event of a `__multi_instrument_frame__` frame (`upstox_recorder.rs:238`) and verify against the anchored frame commitment. **Owner:** anchor authority + Runtime event-identity owner (ADR-005; unassigned). **Target milestone:** M3+. **Status:** PRECONDITION — not an M1 gate. **Threshold:** mismatch detected; partition-only digests insufficient.
6. **No self-certification.** Named check (design): confirm no repository ADR or registry grants `verified` to a source-provenance claim without an anchor receipt under ADR-M1-011. **Owner:** registry/gate owner (unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (ADR-011 gate extension not built in M1). **Threshold:** 0 self-certified provenance claims; design documents carry no certifying authority.
7. **P&L subject binding (challenger re-verification N-1).** Named falsifier (design): with an anchor available, present a receipt that commits the source inputs and chain heads but **omits** the replay-equal field-set definition and fill-stream commitment; assert the verifier refuses any P&L `verified-equal` claim. **Owner:** independent anchor authority + replay-contract owner (both unassigned). **Target milestone:** M3+ (after anchor and driver). **Status:** PRECONDITION — not an M1 gate. **Threshold:** 100% of receipts lacking the two P&L subjects rejected for P&L equality; a green P&L compare on such a receipt is a failure.

*Design only. No anchor exists by M1 and no command above has been executed; thresholds are requirements for M2+ gates. All seven items are preconditions, not M1 acceptance gates.*
