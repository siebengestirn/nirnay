# ADR-M1-008 — Durability acknowledgement levels, fault model and RPO/RTO requirements

**Status:** PROPOSED — Governance Design Group, M1 Platform Constitution (2026-11-17)
**Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9` (read-only reference)
**Scope:** DESIGN ONLY. This ADR defines engineering classes and measurable requirements. It authorises no code, storage engine, batching/group-commit change, service, network, broker or live-routing change. No acceptance test named below has been executed by M1.
**Authority:** M1 does not implement. Adoption requires repository-owner approval (M1 exit gate).
**Amendment 2 — challenger disposition:** Resolved finding **B-4** (D1 zero-loss premises made explicit; raw path not claimable) and **§9** (named owners, target milestones, PRECONDITION labels per challenger §C). Charter-fit reaffirmed (challenger §D). Finding **A-0** (invariant-ID collision) is out of scope for this file: the challenger itself lists `M1-DUR-INV-` as a separate scheme (`CHALLENGER.md:43`); this ADR uses only that namespace and does not renumber.

---

## 1. Problem

At baseline the only durability acknowledgement boundary in the repository is "the per-append `sync_data()` returned." In `crates/market-archive/src/journal.rs:47-53` the raw writer performs `write_all(line)` → `write_all(b"\n")` → `sync_data()?` (line 51); `create_new()` also `sync_all()`s the parent directory (`journal.rs:30-32`). The account journal does the same per record (`crates/account-journal/src/lib.rs:503-509`). There is **no system-level acknowledgement contract, no fault model, and no RPO/RTO target** (`CONSOLIDATED-BASELINE.md:136,259`; A3 §6, A3-07).

Specific defects that make a level vocabulary necessary:

- **Payload-only integrity with no continuity in the verify path.** `RawFrame::validate()` checks only non-empty fields, `schema_version`, and `payload_sha256` over decoded `payload_base64` bytes (`frame.rs:75-97`); `verify_journal`/`visit_journal` (`journal.rs:70-110`) reject torn tails and bad payloads but accept sequence/timestamp edits and whole-record deletion (A3-01 E results, `A3-durability.md:167-171`; re-filed S2 by R2 B1 and Amendment A §A1). The recorder's whole-file SHA-256 in `manifest.json` is written but never re-verified and is unsigned (`upstox_recorder.rs:302-356`).
- **No poison/unknown state in the raw writer.** `JournalWriter { file: File }` has no state flag (`journal.rs:17-19`); a failed append leaves the writer reusable and can append behind a partial line (A3-02). The shipped caller aborts (`upstox_recorder.rs:246-248`), which mitigates but does not fix the library contract.
- **Divergent and silent torn-tail policy.** Raw archive refuses and leaves bytes unchanged (`journal.rs:36-45,89-91`); account journal silently truncates the final partial frame via `repair_tail()` = `set_len()` + `sync_all()`, reporting only `repaired_tail_bytes` (`lib.rs:378-399,538-545`; A3-04). An uncertain tail is therefore represented as *evidence destruction* in one subsystem and *manual refusal* in another.
- **No backup/restore/replication/retention** in scope (A3-07), no crash/fault-injection CI gate (A3-10), and per-append sync is a measured throughput ceiling (~43 µs/append, ≥11.7× write cost; A3-03) that must **not** be "fixed" by group commit before an acknowledgement contract and a measured requirement exist (Amendment A §A5; A6-01).

The system currently reaches at best **D1** (local persisted journal with verification); D2 exists only as test-only `open_append` recovery verification; D3/D4 are absent (`CONSOLIDATED-BASELINE.md:136`).

---

## 2. Alternatives considered (with rejection reasons)

| Alternative | Rejection reason |
| --- | --- |
| **"Durability" = one binary flag / `sync_data()` returned** | Collapses five distinct failure domains (process, OS, host/disk, site, hostile host) into one claim. It cannot express what a crash after the ACK loses, and it hides that no RPO/RTO is defined (§1). Rejected. |
| **Universal "zero-loss" guarantee for all writes** | No fault model or boundary is defined; a host/disk loss destroys the sole copy (A3-07) and a host compromise rewrites all three unanchored chains (F-A5-06). A universal zero-loss claim is unsupportable and would be false assurance. Explicitly rejected (see M1-DUR-INV-001). |
| **Replace per-frame `sync_data()` with group commit now** | A3-03 measured only a proxy ceiling; no frame-rate envelope or durability/latency budget exists. A6-01/A3-03 and Amendment A §A5 require a measured requirement and decision record first. Rejected as a present change; a future change would need its own ADR. |
| **Introduce a new storage engine / database for all journals** | Same measured-requirement gate; would also create a new SPOF and couple a hard-disabled research system to infrastructure (Amendment A §A5, R1 D2). Rejected. |
| **Treat the account journal's silent `repair_tail()` as the recovery standard** | It destroys forensic evidence on every uncertain tail with only a byte count (A3-04). An uncertain outcome must be represented explicitly, not silently repaired. Rejected. |
| **Use a standalone per-record hash chain as the durability/integrity answer** | A same-writer chain establishes only self-consistency against a host compromise; R2 D1 rejected exactly this remedy as false assurance. Rejected here; see ADR-M1-010 for the anchored form only. |
| **Define RPO/RTO as current achieved numbers** | No measurement, backup, restore drill or replication exists (A3-07, A7 §7 item 10). Numbers would be fabricated. Rejected; requirements are stated as targets to be measured, not achievements. |

---

## 3. Decision

### 3.1 Terminology (decoupled from any level)

These terms are defined so that an acknowledgement sentence is unambiguous:

- **accepted** — the application has taken the object into in-process state; no stability claim.
- **committed** — the store's own record/transaction semantics declare the write final (e.g. the newline commit marker plus `sync_data()` at `journal.rs:50-51`, or the single `BEGIN IMMEDIATE` commit at `durable_oms.py:222`).
- **persisted** — bytes are on stable *local* media (a `sync_data`/`fsync` returned).
- **acknowledged** — the durability boundary for the claimed level returned success to the caller; an ACK is always relative to a named level and fault model, never absolute.
- **persisted-and-replicated** — persisted and present on at least one independent failure domain under the level's policy.
- **uncertain outcome** — a write/sync/submit whose result is unknown; must be represented by an explicit state and handled fail-closed, never as success and never by silent truncation.

### 3.2 The five engineering classes

**D0–D4 are engineering classes for reasoning and requirement-setting. They are NOT current guarantees. No subsystem is granted a level by this ADR; a level is earned only by the evidence named in §9.**

| Level | Meaning of ACK / "committed-persisted-replicated" | Faults survived | May be lost | RPO requirement (target, to be measured) | RTO requirement (target, to be measured) | Uncertain outcome representation |
| --- | --- | --- | --- | --- | --- | --- |
| **D0 — Volatile / unacknowledged** | Only *accepted*; no commit, no persist, no replicate. ACK is explicitly absent. | None. | Everything on process or OS failure. | Unbounded (no requirement met). | Not applicable (no durable state to restore). | "Accepted, durability not asserted"; must never be reported as persisted. |
| **D1 — Local persisted, single node, integrity-checked** | ACK = `sync_data()` returned on the local node (`journal.rs:51`; `lib.rs:505`; WAL/FULL at `durable_oms.py:79,101`). Committed + persisted locally. Not replicated. | Process crash and OS crash for every record whose ACK returned; single-writer exclusion via `fs2` lock (`journal.rs:29,39`; `lib.rs:197-203`). | The in-flight record between the last ACK and failure; host/disk loss (sole copy, A3-07); metadata/whole-record edits (A3-01); any tail silently truncated by `repair_tail()` (A3-04). | ≤ one in-flight, unacknowledged unit; exact bound must be measured and stated in the runtime envelope. | Time to reopen + stream `verify_journal` over the whole file (O(total archive), no partitioning — A6-06); must be measured and bounded before any claim. | Raw writer must adopt an explicit `CommitOutcomeUnknown`/poison state (contrast `lib.rs:93-96,507-509`); the writer is closed/reopened and the tail is refused or quarantined — not silently truncated. Until then D1 status cannot be claimed for the raw path. |
| **D2 — Local persisted + demonstrated recovery** | ACK = D1 ACK plus a verified recovery path exercised on open (reopen + chain/digest verification), equivalent to a productionised form of the test-only `open_append` recovery (`CONSOLIDATED-BASELINE.md:136`). | All D1 faults, plus detection of a truncated/corrupt tail through a defined, tested repair-or-quarantine procedure. | Same as D1 for unacked tail and host loss; corruption that the recovery check cannot detect remains possible without the anchor of D4/ADR-M1-010. | Same as D1, plus a documented maximum recovery verification time. | Bounded and measured recovery-time target; currently undefined and therefore D2 is **not** currently met. | Torn tail must be surfaced as a reportable incident (`repaired_tail_bytes` alone is insufficient — A3-04), with quarantine of repaired bytes pending review. |
| **D3 — Replicated across independent failure domains** | ACK = persisted locally **and** replicated to ≥1 independent failure domain under a defined policy; "replicated" becomes meaningful only here. | Single host/disk/site loss in addition to D2 faults. | Data not yet replicated (bounded by the replication policy); data destroyed concurrently on all domains. | Replication lag bound (explicitly defined and measured; **no value exists today**). | Failover/restore time bound (explicitly defined and measured; **no value exists today**). | Replica divergence, quorum loss and partial replication must be explicit states; no silent promotion. No service is authorised by this ADR. |
| **D4 — Replicated + externally anchored, compromise-evident** | ACK = D3 ACK plus the signed chain head / content commitment recorded in a trusted off-host anchor (contract in ADR-M1-010). | Adds hostile-host compromise: a host that rewrites every local record/chain cannot forge the anchor. | Anything not yet anchored; anchor-domain compromise (out of scope of the local fault model and must be stated). | Anchor cadence bound (defined + measured). | Anchor-verification/restore-time bound (defined + measured). | Anchor unavailable = fail closed (no "verified" status); anchor disagreement = integrity incident. No anchor exists by M1 (ADR-M1-010 §3). |

### 3.3 Zero-loss rule (unstated premises made explicit — challenger B-4)

**Universal zero-loss is rejected.** Zero-loss may be asserted only as the conjunction of (a) a named level, (b) a named fault model, (c) a named acknowledgement boundary, **and (d) satisfaction of that level's stated preconditions**, and only when the evidence of §9 exists.

The narrow admissible form is **retained** (the challenger affirmed it) but is now stated with its previously unstated premises:

> "Within D1, for the fault set {process crash, OS crash} and the boundary `sync_data()` returned at `journal.rs:51`, no acknowledged frame is lost — **provided (i) the writer maintains a defined poisoned/unknown-outcome state after any failed write or sync (A3-02; contrast the account journal's `CommitOutcomeUnknown`/`Poisoned` at `lib.rs:93-96,507-509`), and (ii) the journal file's directory entry was made durable when the file was created (parent-directory `sync_all()` at `journal.rs:30-32`, executed in `create_new`). Appends to an already-created file do not change a directory entry, so no per-append parent-directory sync is required; a parent-directory sync is required again after any rename, rotation, or other directory-entry change.**"

**Current attainability: none for the raw path.** The raw `JournalWriter { file: File }` has no poison flag (`journal.rs:17-19,47-53`; A3-02). The create-time parent-directory `sync_all()` (`journal.rs:30-32`) already protects the directory entry; the absence of a per-append directory sync is **not** a defect and is not a D1 premise. Therefore **D1 cannot be claimed for the raw path today** because the poison/unknown-outcome state is absent; the admissible sentence is a conditional requirement, not a present guarantee. It becomes assertable only after the poison premise is met and evidenced under §9, with the create-time directory sync verified. Any broader phrasing — or use of this sentence without its premise — is prohibited.

### 3.3.1 Platform-specific `sync_data` contract (source-verified 2026-11-17)

The platform contract was verified against the **exact pinned Rust 1.99.1 standard-library source** (`library/std/src/sys/fs/unix.rs`, `File::datasync` / `os_datasync`), not assumed:

- **Apple targets:** `os_datasync` calls `libc::fcntl(fd, libc::F_FULLFSYNC)` (lines 1282-1285). This is a stronger device-cache flush than plain `fsync` on Apple platforms, so the existing `sync_data()` call already provides that stronger flush there.
- **Linux and the listed Unix targets:** `os_datasync` calls `libc::fdatasync(fd)` (lines 1286-1301), which does not itself flush the device write cache.

Consequences for the fault model:

- D1's `{process crash, OS crash}` fault set is the scope of the narrow zero-loss sentence. A **power-loss guarantee is not claimed**; it depends on device and filesystem behaviour beyond `fdatasync` on Linux.
- No custom flush path is introduced by this ADR. Adding one requires a measured requirement, a platform-specific design and its own decision.
- The directory-sync requirement applies at file creation (`journal.rs:30-32`) and at any future rename or rotation; it does not apply to every append to an existing file.

### 3.4 Out-of-scope non-decisions (explicit)

- No group commit, batching, or new storage engine is prescribed; any such proposal requires a measured requirement and a separate ADR (A3-03, A6-01, Amendment A §A5).
- No backup/restore/replication service, schedule, or infrastructure is designed here; D3/D4 are requirement classes only.
- This ADR does not promote any baseline claim to `verified` (see ADR-M1-011).

### 3.5 The six owner challenges — implications carried by this ADR

Challenges 4 (durability) is primary here. The other challenges are owned by other groups; their **durability implications** are stated here and are not re-decided:

1. **Authorization (risk-token boundary).** The importable token at `risk_service.py:24-26,192-204` can sign arbitrary `RiskFacts` in-process. No durability level confers an authorization boundary: a durably persisted, D1–D4 "signed" runtime-evidence artifact is still forgeable if its signer is co-located with OMS/strategy. Therefore the *evidence* of a signed decision carries no authority unless the authority precondition (Architecture ADR-M1-002) is met; and the anchor key (ADR-M1-010) must not be available on the producing host. Primary decision: Architecture ADR-002.
2. **OMS lifecycle / owner, UNKNOWN and reconciliation.** The durable OMS is a D1 local store (WAL/FULL, `durable_oms.py:79,101`). An UNKNOWN order after a submit timeout (`durable_oms.py:238-256,397-421`) is precisely an *uncertain submission outcome* — durability cannot resolve it, and no broker-query path exists (F-A5-04). Requirement: UNKNOWN/quarantine and reconciliation events must be recorded as first-class uncertain states and must not be represented as verified recovery without a reconciliation drill; CLM-009's crash falsifier is untested (A3-06). Primary owner decision: Architecture ADR-003.
3. **Events and partitions.** Every raw frame is stamped `provider_instrument_id="__multi_instrument_frame__"` (`upstox_recorder.rs:238`) and cash/exposure/`daily_pnl` are account-global (`state.rs:113-116`, `engine.rs:955-971`). A durability level must be asserted over the **frame/payload** boundary (the `payload_sha256` of one frame), not over a derived per-instrument event; partition-local D1/D2 claims cannot be summed into an account-global claim (M1-DUR-INV-007). Primary decisions: Runtime ADR-005/006.
4. **Durability (primary here).** §3.2 defines each level's ACK meaning, survived faults, possible losses, RPO/RTO targets and uncertain-outcome representation; zero-loss is scoped by M1-DUR-INV-001/002. The trusted off-host anchor required for D4 is contracted in ADR-M1-010.
5. **Replay.** A frame ACK at `journal.rs:51` is an input to the replay checkpoint, not proof that the session is complete (manifest written only at `finish()` — R2 C2); replay comparison discipline is defined in ADR-M1-009.
6. **Evidence.** No durability level is `verified` at baseline. A level claim becomes verifiable only with the measured fault-injection/RPO/RTO evidence of §9 under the two-field model of ADR-M1-011.

**Charter-fit reaffirmation (challenger Section D).** M1 authorises no anchor service, storage engine, schema change, registry edit or gate code; no implementation and no platform-readiness claim; design documents (including this ADR) cannot self-certify; and nightly run `43792611015` is unverified by owner ruling and is not CI or release evidence (`CONSOLIDATED-BASELINE.md:25`, Amendment A §A9). Every §9 item is a future-milestone specification, not an M1 gate. The separate question of whether a mandatory field surface is a forbidden "schema change" (challenger A-5/§D.1) is referred to owner ruling, not decided here.

---

## 4. Ownership

- **Decision author:** Governance Design Group (NIRNAY M1). **Approver:** repository owner (M1 exit gate); M1 alone does not adopt.
- **State created by the decision:** the level register (which subsystem is claimed at which level, under which fault model) and the RPO/RTO requirement register. **Operational owner is UNSET in the baseline:** capability owners are team labels, not named accountable individuals (A7 §9), and no runbook exists for journals/OMS (A3-07, A3-10). Design requirement: each claimed level must name an accountable owner and an on-call reconstruction procedure before a level may be asserted; until then the owner is "unassigned — claim not adoptable." This gap is recorded, not silently filled.

---

## 5. Invariants (explicit IDs)

- **M1-DUR-INV-001 (Zero-loss scope).** No "zero-loss" statement is admissible unless it names a level, a fault set, and an ACK boundary. Universal zero-loss is false by construction (sole copy + unanchored chains).
- **M1-DUR-INV-002 (ACK relativity).** Every acceptance/acknowledgement claim is qualified by a level from §3.2. Unqualified "durable" is not a valid status.
- **M1-DUR-INV-003 (No silent tail repair).** An uncertain tail must be surfaced as an explicit state/incident; silent truncation (`repair_tail()` at `lib.rs:378-399`) and silent writer reuse (`journal.rs:17-19,47-53`) do not satisfy any level ≥ D1.
- **M1-DUR-INV-004 (Failure domain separation).** A level's faults must be enumerated; D1 does not survive host loss, D3 requires an independent failure domain, D4 requires a trust-independent anchor.
- **M1-DUR-INV-005 (Measured targets only).** RPO/RTO values must be measured by the commands in §9 before being stated; absent measurement the field is "undefined," never a number.
- **M1-DUR-INV-006 (No unauthorised optimisation).** No group commit or storage-engine change is implied by adopting these classes; such change requires a measured requirement and its own ADR.
- **M1-DUR-INV-007 (Global-vs-partition).** A durability or recovery claim for a partition is not a claim for the account-global state (`engine.rs:955-971`); aggregate coverage must be asserted separately.
- **M1-DUR-INV-008 (Authority independence).** A durability level or persisted signature is not evidence of authorization; the risk-token boundary (Challenge 1) and anchor key separation (ADR-M1-010) are preconditions, not consequences.
- **M1-DUR-INV-009 (D1 premise must hold before any D1 claim).** A D1 / narrow-zero-loss assertion is admissible only if the writer has an explicit poisoned/unknown-outcome state (A3-02). The file's directory entry must have been synchronized at creation (`create_new`, `journal.rs:30-32`); a per-append parent-directory sync is **not** required, but any rename or rotation that changes a directory entry must be followed by a directory sync. Until the poison premise holds, D1 is a requirement class, not a claimable level for the raw path (challenger B-4, corrected by the owner durability review).

---

## 6. Failure cases (concrete, adversarial)

1. **Disk full mid-append, writer reused (raw).** `write_all(line)` succeeds, `write_all(b"\n")` or `sync_data()` fails (A3-02). Because `JournalWriter` has no poison flag, a future caller retries and appends behind a partial line; a valid-looking merged JSON record could pass `from_json_line`. Expected behaviour under this ADR: the write is an *uncertain outcome*, writer poisoned, close/reopen, tail refused or quarantined; no D1 ACK is issued.
2. **Crash after ACK but before session manifest.** All frames ACKed at `sync_data()`; crash before `finish()` (`upstox_recorder.rs:302-356`) loses the session definition, not the frame bytes (R2 C2). A recovery that assumes frame ACK = complete-session proof is wrong; D1 ACK covers frames only.
3. **Host compromise rewrites every record with recomputed hashes.** For the raw payload-only journal (A3-01) and the three unanchored chains (F-A5-06), `verify_event_chain()` still returns `verified_internal_integrity` (`durable_oms.py:562-578`, `signature: False`). Under this ADR such a state is at most D1 self-consistency; **D4 requires the off-host anchor of ADR-M1-010** to detect it. Without the anchor no level above D2 is claimable.
4. **Adversarial torn tail masquerading as a repairable crash.** An attacker appends a data-bearing partial frame to the account journal; `repair_tail()` silently discards it and reports only `repaired_tail_bytes` (A3-04). Under M1-DUR-INV-003 this is an incident with quarantined bytes, not an invisible truncation.
5. **UNKNOWN order treated as D1-committed.** After SUBMIT timeout the OMS marks DISPATCHING→UNKNOWN (`durable_oms.py:238-256`); no broker query exists (F-A5-04). Representing this as "committed" or "recovered" is forbidden; it is an uncertain outcome requiring reconciliation evidence (Challenge 2).
6. **Per-partition success summed into account-global durability.** Two partition replays each pass their local checks while aggregate `gross_exposure`/`daily_pnl` breach (`engine.rs:955-971`; R1-Q3/A4 R1-C). M1-DUR-INV-007 forbids deriving an account-global ACK from partition-local ones.
7. **Fabricated "verified" durability claim by JSON edit.** A claim is set to `status: verified`
  in `claims/ledger.json` without evidence; `check_repo.py:174-187` would police it but the
  baseline has 13/13 `implemented` and zero evidence (A7-001). ADR-M1-011 closes this; this ADR
  forbids lowering the level by registry edits (M1-DUR-INV-005).

---

## 7. Dependencies

- **On other ADRs:** ADR-M1-010 (off-host anchor) is a precondition for any D4 assertion; ADR-M1-011 (evidence model) governs how a level claim becomes `verified`; ADR-M1-009 (replay contract) consumes the ACK boundary for checkpoint coverage; Architecture ADR-M1-002/003 (authorization, OMS owner) and Runtime ADR-M1-005/006 (event identity, partitions) own Challenges 1–3 whose implications are recorded in §3.5.
- **On evidence:** A3-02/03/04/06/07/10, F-A5-04/06, R2 §C2, Amendment A §§A1/A5/A6. All are B/C/E-proxy findings pinned to `01eb382e`; none is F-level.
- **On external capabilities (absent):** backup/restore, replication, crash/fault-injection infrastructure, RTO/RPO measurement, off-host anchor and key management. Their absence is why no level above D1 is currently claimable.
- **Constraint:** no port, service, storage engine, network/broker or live change is authorised; dependencies on external capabilities are *requirements to be decided later*, not work items here.

---

## 8. Rejection conditions

This decision is overturned or revised if: (a) a measured frame-rate/throughput envelope plus a group-commit durability/latency study shows the per-append ACK boundary is untenable (then a separate ADR is required); (b) a real backup/restore/drill produces measurable RPO/RTO that make a level's targets concrete; (c) an implemented off-host anchor (ADR-M1-010) changes what D4 can assert; (d) a chosen OMS/event/partition owner (Challenges 2–3) relocates the acknowledgement boundary; (e) an owner ruling changes the permitted fault model.

**Recorded unresolved disagreements (not resolved by vote):** R1 argued "do not authorize M1" while R2 argued "authorize on acceptance" — resolved by Amendment A §A6 to scope M1 to ADR/design work, with R1's concern absorbed; no implementation is authorised. The baseline §13 items 9 (no durability targets/ACK boundary), 10 (whether D/E/F evidence is attainable on the current account) and 11 (C4/C5 scope) remain open owner decisions. A3-03's E-proxy ceiling and the "do not replace with group commit" caution are agreed; the *value* of any future envelope is not. These disagreements are recorded rather than resolved here.

---

## 9. Measurable acceptance requirement (DESIGN ONLY — not executed by M1)

Each requirement is a future-gate specification with a named command, a threshold, an **Owner (role)** and a **Target milestone**. Per challenger §C, anything depending on a capability absent at baseline is labelled **PRECONDITION — not an M1 gate**. Roles are not named persons: baseline "owners" are team labels (A7 §9) and assignment is itself an owner action. M1 defines these; it runs none of them.

1. **ACK-boundary / D1 premises (D0→D1).** Named command (proposed): `cargo test -p market-archive --offline torn_tail_is_reported_and_not_repaired`, plus a new fault-injection test (A3 M4) injecting a failing `write_all`/`sync_data`, plus a directory-fsync assertion. **Owner:** `market-archive` crate owner (unassigned per A7 §9 — assignment required). **Target milestone:** M2 (owner-gated). **Status:** PRECONDITION — not an M1 gate (fault-injection harness does not exist). **Threshold:** failed append → poisoned writer; second append refused; 0 lines accepted after the error; no D1 ACK on an uncertain outcome; create-time parent-directory sync verified; rename/rotation directory sync verified where rotation occurs. No per-append directory sync is required. Current: poison test absent → D1 not demonstrated (M1-DUR-INV-009).
2. **Metadata / whole-record integrity (D1 entry).** Named command: market-archive falsifier harness (A3-01 E) extended with sequence/timestamp edits and whole-line deletion. **Owner:** `market-archive` crate owner + verification-tooling owner (unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (harness exists only as an out-of-repo scratch E artifact, not committed). **Threshold:** 100% of metadata edits and deletions detected, or the level is downgraded and the claim reworded (R2 B1 / Amendment A §A1). Current E result: 3/3 accepted → raw path fails.
3. **Recovery-time measurement (D2 entry).** Named command (proposed): timed `open_append`/`verify_journal` over a fixed-size baseline-built journal with recorded environment. **Owner:** `market-archive` crate owner. **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate. **Threshold:** a *numeric* p99 bound measured from the workload (W4/W6), not "bounded O(archive)" — an asymptotic claim is not a threshold (challenger §C); no number exists today → D2 not met.
4. **RPO/RTO crash + restore drill (D3 entry).** Named command (proposed): subprocess `SIGKILL` mid-`accept_intent` then reopen (A3 M9), plus a restore drill (M10). **Owner:** authoritative-OMS owner (Architecture ADR-003; unassigned) + release/operations owner. **Target milestone:** M3+ (owner-gated). **Status:** PRECONDITION — not an M1 gate (no backup/restore/replication; no harness). **Threshold:** 0 committed transactions lost; 0 intent-without-outbox; numeric RPO/RTO within bounds defined *from* the drill. Crash falsifier currently untested (A3-06).
5. **Anchor-detection test (D4 entry).** Named command (proposed): rewrite all chain heads with recomputed hashes, then run an anchor-backed verifier (ADR-M1-010 §9). **Owner:** independent anchor authority — UNSET and unimplemented (ADR-M1-010 §4). **Target milestone:** undetermined; no M1 path. **Status:** PRECONDITION — not an M1 gate; D4 is a requirement class only, unfalsifiable in M1. **Threshold:** 100% of full-chain rewrites rejected.
6. **Registry non-vacuity (level claims).** Named command: `python3 scripts/check_repo.py claims` after the gate extension specified in ADR-M1-011. **Owner:** registry/gate owner (unassigned). **Target milestone:** M2. **Status:** PRECONDITION — not an M1 gate (gate extension not built in M1). **Threshold:** a level/verification claim without its named evidence fails; currently vacuous (A7-001).

*Design only. No command above has been executed by M1; thresholds are requirements for M2+ gates. Items 1–6 are preconditions, not M1 acceptance gates.*
