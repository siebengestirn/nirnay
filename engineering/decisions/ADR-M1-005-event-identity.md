# ADR-M1-005 — Canonical event identity: observation identity vs business identity

- **Status:** PROPOSED (designed, not implemented). M1 design work only.
- **Group:** Runtime Design Group
- **Baseline:** `01eb382e1c51d7ec0c672083b2a8cbe7da77f4a9` (all citations `01eb382e:`)
- **Scope:** identity *contract semantics* only. This ADR creates **no schema FILE, no code, no service, and authorizes no schema change**. The field surface it names (`capture_frame_id`, `observation_id`, `business_event_id`, `origin_class`, `schema_version`, source hashes) is a **semantic contract for a future, separately-authorized schema change** — not an implemented or authorized change to `schemas/market-data/canonical-event.v1.schema.json` or any other schema. Representation is deferred to a gated follow-on ADR.
- **Open exit-gate condition (owner charter-fit ruling — challenger A-5 / Section D.1):** whether defining this mandatory semantic field surface constitutes a forbidden "schema change" at the design level is **referred to the owner for a ruling** and must be recorded in `M1-RECONCILIATION.md` before the exit gate is signed. This ADR does not claim such a change is authorized.
- **Evidence discipline (owner challenge 6):** every claim below is level B (source) or E (reproduced by A2) at baseline; nothing here is `verified`. Implementation-maturity and verification-status remain separate fields.
- **Amendment resolution:** challenger **A-2** (false dedup) resolved by option (b) below; **A-5** / Section D.1 (charter-fit) referred to owner ruling as an open exit-gate condition; **Section C** §9 owner/milestone and precondition labelling applied.

---

## 1. Problem

M0 established an E-reproduced identity defect and three structural gaps, all pinned to baseline:

1. **Identity is bound to a reset-prone local counter.** `event_id` is `f"upstox:{local_sequence if local_sequence is not None else received_utc_ns}:{provider_key}:{digest[:16]}:book"` (`providers.py:134`, and `:151`, `:227`, `:244` for the trade/FYERS variants). `local_sequence` is caller-supplied and mapped from `capture_sequence` (`upstox_recorder.rs:232-249`), a recorder-local frame counter with no exchange authority (`providers.py:5-8`). A2 executed: identical payload with `local_sequence=7` vs `8` → different `event_id` sharing one payload hash (`A2-data-semantics.md` F-01). Recorder restart resets `frame_count` (`upstox_recorder.rs:232-235`), so replayed payloads receive new IDs. Every downstream de-duplicator keys solely on `event_id` (`point_in_time.py:410-412`; `backtest.py:281-283`; `portfolio_backtest.py:133-136`). Consequence: duplicate observations are admitted to research data.

2. **Observation and business event are not separated.** One decoded feed entry emits a `book` state event and, when the last-trade tuple advances (`providers.py:46-64,148,241`), a `last_trade_observation` event. Both share the same `raw_sha256` (`:144,160` / `:237,253`); only the `:book`/`:trade` suffix differs (`market.py:193-194`; canonical schema `:35-38,156-159`). `digest[:16]` is a truncated 64-bit prefix (`providers.py:130`). A consumer joining on `raw_sha256` merges distinct observations; two payloads can collide in the prefix (A2 F-02).

3. **A second, disjoint ordering model exists but is dead.** Rust `EventMeta` uses **microseconds** with `connection_epoch`, `receive_seq`, `has_receive_fence()` (`market-types/src/model.rs:620-664`), plus `ContinuityState`, `DiscontinuityReason`, `ExchangeEvent`, `Control::Recover` (`:700-863`). Grep shows definitions/tests only — no non-test caller (A2 F-04). The live Python model uses integer **nanoseconds** (`market.py:95-100`). Two clock/ordering models coexist; the richer one is unwired scaffolding that creates false assurance.

4. **The canonical envelope omits per-event availability/record time.** `MarketEvent` carries receive/source/last-trade times (`market.py:86-112`) but no availability-time or record/knowledge-time field; those exist only in the dataset manifest (`manifest.rs:14-38`) and PIT rows, and in a Rust-only `EventTimes` (`engine.rs:904-916`). Freshness is computed from receive time (`reconcile.py:48-54`). Amendment A rules this S2 (target doc is "destination, not current implementation"), but replay/freshness labelling depends on it.

5. **Origin classification has no first-class field.** The system is deliberately conservative in prose and flags — `provider` enum, `source_timestamp_semantics`, `not_a_confirmed_trade_print`, `aggressor_side_unknown` (`providers.py:161,254`), quality-limitation strings (`cli.py:210-214`), refusal to treat SDK callbacks as exchange ticks (`market.py:1-5`; `providers.py:1-9`; `reconcile.py:33-34`). But there is **no structured origin field**: "exchange event", "SDK callback", "raw wire frame", "local receive", and "reconstructed" are encoded in free-form strings and field naming (A2 Critical Question: "Partial yes"). Upstox `currentTs` is a feed envelope time, not exchange time (`providers.py:108-110`); FYERS `exch_feed_time` is a callback value preserved verbatim (`:205,233`).

No single identity contract currently lets a consumer (a) deduplicate across reconnect, (b) join an inferred observation to its source payload without collapsing them, (c) tell what kind of thing produced an event, or (d) reproduce a replay without re-deriving identity. This ADR decides that contract at the semantic level.

---

## 2. Alternatives considered (with rejection reasons)

- **A2-alt: Keep `event_id` as-is and rely on downstream dedup on `raw_sha256` instead.** Rejected. `raw_sha256` is shared by the two events from one payload (F-02), so dedup on it collapses a quote and its inferred trade observation. It is also a per-instrument hash over `{"key","wrapped"}` (`providers.py:130`), not the durable frame hash, so it cannot anchor capture-level provenance.

- **A2-alt: Make `event_id` = `payload_sha256` of the whole raw frame.** Rejected. Every Upstox frame is stamped `provider_instrument_id="__multi_instrument_frame__"` (`upstox_recorder.rs:238`) and `payload_sha256` covers the whole decoded payload (`frame.rs:71,79`). Many instruments share one frame hash; it cannot identify a single instrument's observation, and splitting the frame would invalidate the integrity hash (R1-C Q3, ADR-006).

- **A2-alt: Add `connection_epoch` (and/or `received_utc_ns`, `local_sequence`) into the identity key.** Rejected. This recreates F-01: a reconnect or replay changes the epoch/counter/receive time and yields a new ID, so duplicates are admitted. All locally assigned, per-delivery metadata must be excluded from identity. The Rust fence concept is valid for **ordering** (ADR-006) but not for identity.

- **A2-alt: Wire the Rust `EventMeta`/µs model as the single authoritative model and convert Python to it.** Rejected. µs is lower precision than the live ns fields; ns→µs conversion is lossy, so µs cannot be authoritative. There is no consumer of the Rust types (F-04) and no measured cross-stack bridge requirement (A4 §2.1; ADR-0001 measured-requirement rule). Wiring it would be speculative infrastructure (R1 Q2).

- **A2-alt: Propose a universal event schema covering both Python and Rust now.** Rejected. R1 independently affirmed A2's restraint (R1 Q2): with two disjoint models and no measured bridge, a universal schema is exactly the speculative architecture ADR-0001 forbids. No producer/consumer exists for a cross-stack artifact (`A4-replay-research.md` §2.1).

- **A2-alt: Leave origin classification in free-form flags and prose.** Rejected. Downstream code cannot branch on a string convention reliably; the FYERS `exch_feed_time` name invites mislabelling as exchange time (A2 Critical Question). A constrained, fail-closed field is required to prevent over-claiming.

---

## 3. Decision

Adopt a **content-addressed, layer-split identity contract**. It defines three identity classes and a reconnect rule. It is a semantic contract, not a schema.

### D-005.1 — Three identity classes

1. **Capture frame identity (`capture_frame_id`).** Content hash of the whole decoded provider payload = the existing `payload_sha256` (`frame.rs:71`), plus the journal record hash `source_frame_sha256` where one exists (`market.py:110-112`). Scope: one raw frame, all instruments in it. This is the durable capture artifact identity; it is unchanged by reconnects because it is over payload bytes.

2. **Observation identity (`observation_id`).** Content hash over `(provider, source_scope, provider_instrument_id, instrument-local canonical payload bytes)`. For Upstox the existing `raw_hash({"key": provider_key, "wrapped": wrapped})` (`providers.py:130`) already computes the instrument-local part; for FYERS, `raw_hash(message)` (`:223`). `observation_id` is **reconnect-stable and sequence-reset-stable**: it excludes `local_sequence`, `capture_sequence`, `connection_epoch`, `received_utc_ns`, `received_monotonic_ns`, `availability_time`, `record_time`, and `source_frame_sha256`. Two deliveries of byte-identical canonical payload content therefore receive the same ID. **This is an identity-stability property only — it is NOT, by itself, a safe deduplication rule.** Whether equal IDs may be collapsed is scoped in D-005.2 (dedup sufficiency); F-01 is fixed by stability plus that scoped rule, not by stability alone.

3. **Business-event identity (`business_event_id`).** Content hash over `(observation_id, event_kind, domain_discriminator)`. It is distinct per domain event derived from one observation. A `book`/`quote` state and an inferred `last_trade_observation` from the same payload share `observation_id` but have different `business_event_id`; each carries an explicit back-reference `observation_id`. This replaces the `:book`/`:trade` suffix with a first-class relationship and removes reliance on a truncated 64-bit prefix for differentiation.

### D-005.2 — Reconnect / sequence-reset semantics

- `connection_epoch` and local `receive_seq` are **provenance/ordering metadata only** (produced by the mechanism described in ADR-006); they are never identity components. A reconnect advances the epoch but **must not** change any `observation_id`/`business_event_id` for identical payload content.
- A sequence counter that decreases is interpreted as a **connection boundary**, not a duplicate and not an identity change. Within one epoch, gap/duplicate/out-of-order detection follows ADR-006 using the `ContinuityTracker` observation vocabulary (`continuity.rs:3-21,56-105`), currently unwired.
- **Dedup sufficiency — RESOLUTION of challenger A-2 (option b, demote the guarantee).** Same content yields the same `observation_id` in all cases (M1-RUN-INV-001), but downstream **collapse is authorized only where payload identity is known sufficient**, i.e. where identical canonical bytes are known to denote the same logical observation because the item is a **re-delivery/replay of an already-captured payload** (journal re-read, reconnect resend, checkpoint replay). In that context dedup on `observation_id` (replacing `event_id`) collapses duplicates and fixes F-01; that is the scoped cross-reconnect duplicate-detection guarantee.
- **Explicitly NOT covered:** two genuine, distinct real-world observations that happen to produce byte-identical canonical payloads (e.g. two separate fills at the same price, quantity and timestamp precision). The feeds supply no exchange sequence or event identity to distinguish them (`providers.py:5-8`; Upstox `currentTs` is a feed-envelope time at `:108-110`; FYERS `exch_feed_time` is a callback value at `:205,233`). The design does **not** guarantee their separation. Residual risk: such a pair collapses to one `observation_id`, the second is dropped by downstream dedup, and exposure/P&L derived from the deduplicated stream is **understated** — feeding directly into the account-global and replay guarantees (ADR-M1-006/009). No discriminator is invented here (option a rejected: none is available at baseline and inventing one would be speculative); overturning this requires a measured requirement and a future ADR. See §6 case 7, U-005-c, §8 item 1 and §9.

### D-005.3 — Python ns vs Rust µs scaffolding

- **Python integer-nanoseconds is the authoritative time representation** at the canonical boundary (`market.py:95-100`). All canonical time fields remain ns.
- **Rust `EventMeta` (µs) and its continuity types are designated non-authoritative roadmap scaffolding**, not a second model. They contribute *concepts* to this contract — `connection_epoch`/`receive_seq` as an ordering fence (ADR-006), `ContinuityState` as fail-closed book state (ADR-006) — but no field of `EventMeta` is an identity component and µs is never a canonical time unit.
- No conversion, no wiring, no schema change is authorized. This designation exists to remove false assurance (A2 F-04 / Amendment A S2), not to build a bridge.

### D-005.4 — General envelope + domain payloads: gated, not adopted

- A **universal event schema is NOT adopted.** No new schema file is proposed in M1, and no existing schema file is edited.
- A **minimal identity contract IS specified now** — exactly the fields above (`capture_frame_id`, `observation_id`, `business_event_id`, `origin_class`, `schema_version`, source hashes, time-domain labels) — because it is the prerequisite to fix F-01/F-02 and to support ADR-009 replay. **This is a semantic contract for a future, separately-authorized schema change, not an implementation or authorization of one.** M1 creates no schema file, emits no producer/consumer field changes, and does not amend `canonical-event.v1.schema.json` or any registry. Whether this field surface is nevertheless a forbidden "schema change" at the design level is an **open exit-gate condition referred to the owner** (challenger A-5 / Section D.1). Domain payload content remains the existing Python `MarketEvent` v1 (`market.py:86-112`) and the separate Rust types; the identity contract references them, it does not replace them.
- A full **general envelope + domain payload** schema is warranted only when **all** gates are met: (a) a measured requirement exists (e.g. a second live collector in Rust, or a cross-stack replay producer/consumer — none exists at baseline, A4 §2.1); (b) an ADR with a named owner and acceptance tests is approved (ADR-0001 rule); (c) the ns↔µs precision question is resolved losslessly. Until then the answer is "not warranted."

### D-005.5 — Origin classification (first-class, fail-closed)

Define a constrained `origin_class` with the vocabulary: `exchange_event` | `sdk_callback` | `raw_wire_frame` | `local_receive` | `reconstructed`. Rules:

- **No current source produces `exchange_event`.** Upstox `currentTs` is a feed envelope time (`providers.py:108-110`) and FYERS `exch_feed_time` is a callback value (`:205,233`). A normalizer must never emit `origin_class=exchange_event` from these; the quality-limitation strings already state this (`cli.py:210-214`). This is a fail-closed rule supporting owner challenge 6.
- `sdk_callback` is the correct class for Upstox `fullFeed`/`ltpc` and FYERS `sf`/`dp`/`if` observations. `raw_wire_frame` is the capture-layer class (frame hash). `local_receive` labels the receive act (currently only timestamps, `market.py:95-96`). `reconstructed` labels inferred events (the `last_trade_observation` at `providers.py:148,241`), which must carry `observation_id` of the source and retain the existing conservative flags (`not_a_confirmed_trade_print`, `aggressor_side_unknown`).

### D-005.6 — Owner-challenge interface implications (no implementation)

- **Challenge 3 (events and partitions, primary):** this ADR supplies the observation-vs-business identity split (D-005.1) and the reconnect-stable cross-reconnect dedup rule (D-005.2). The multi-instrument-frame and account-global-risk halves of challenge 3 are owned by ADR-M1-006; together the two ADRs satisfy challenge 3 by number.
- **Challenge 4 (durability), envelope implication only:** `capture_frame_id`, `observation_id`, `business_event_id`, and `source_frame_sha256` are the fields an acknowledgement and a provenance anchor will reference. ACK meaning and anchoring are owned by ADR-008/010; this ADR only guarantees the fields are stable and content-addressed.
- **Challenge 5 (replay), event-data requirement:** identity stability is necessary but not sufficient for a non-tautological replay. Replay must recompute P&L from fills+marks or exclude it; the identity contract guarantees observations deduplicate across replay and that `reconstructed` events are distinguishable from confirmed prints. Cross-ref ADR-009.
- **Challenge 1 (authorization) / Challenge 2 (OMS), interface implication only:** any order/broker/UNKNOWN/reconciliation events (ADR-003) must use content-addressed identity, never a local reset-prone counter, so replay and reconciliation do not admit duplicates. No remote service is assumed.

---

## 4. Ownership

- **Decision owner:** Runtime Design Group (this ADR). Acceptance of the identity contract is an owner-level M1 decision, recorded in `M1-RECONCILIATION.md`; the group cannot self-authorize it.
- **State created by the contract (design-time only):**
  - `observation_id`, `business_event_id`, `capture_frame_id` — proposed owner: the canonical-model owner (currently the Python research package, `research/python/src/quantlab/market.py`, outside the Cargo graph per A2 §7). Assignment of a production owner is **unresolved** (M0 baseline §4: "Market event identity — no explicit owner"); this ADR records the gap rather than filling it by assumption.
  - `origin_class` — proposed owner: provider-normalization owner (`providers.py` normalizers), because only the normalizer knows whether a payload is a callback, a reconstructed observation, or a wire frame.
  - `connection_epoch`/`receive_seq` — ordering metadata owner is the capture/collector layer (ADR-006); this ADR explicitly does **not** make it identity state.
- **Persistent state created at runtime by any future implementation:** none authorized. This ADR creates no durable state.

---

## 5. Invariants (explicit IDs)

- **M1-RUN-INV-001 (identity stability, scoped — amended per challenger A-2):** For identical `(provider, source_scope, provider_instrument_id, canonical payload bytes)`, `observation_id` is identical regardless of `local_sequence`, `capture_sequence`, `connection_epoch`, receive time, or availability/record time. **This asserts ID stability, not safe collapse.** Downstream collapse is authorized only under the dedup-sufficiency condition in D-005.2 (known re-delivery/replay of a captured payload); equal IDs from two genuine distinct observations are a recorded residual risk, not a guaranteed-safe dedup.
- **M1-RUN-INV-002 (no local counter in identity):** No identity component is derived from a reset-prone local counter or from any wall-clock/monotonic receive timestamp.
- **M1-RUN-INV-003 (observation/business separation):** One observation may yield N ≥ 1 business events; all share `observation_id`; all `business_event_id` values are distinct; each business event back-references its observation.
- **M1-RUN-INV-004 (no hash-prefix identity):** Identity uses full 256-bit digests; the truncated `digest[:16]` prefix (`providers.py:130`) is not an identity authority.
- **M1-RUN-INV-005 (reconnect invariance):** Advancing `connection_epoch` or resetting the local sequence counter never changes an existing `observation_id` or `business_event_id`.
- **M1-RUN-INV-006 (origin fail-closed):** No `exchange_event` class is emitted from a source that supplies only a feed-envelope or SDK callback timestamp. `reconstructed` events retain the `not_a_confirmed_trade_print` and `aggressor_side_unknown` flags.
- **M1-RUN-INV-007 (single time authority):** Canonical time is integer ns; µs `EventMeta` is not authoritative and contributes no identity field.
- **M1-RUN-INV-008 (no speculative schema, charter-fit qualified):** No universal event schema is created in M1 and no schema file is created or schema change authorized; the minimal identity contract is a semantic contract only — the sole new semantic surface — for a future, separately-authorized schema change. Whether defining it is a forbidden "schema change" is an open owner exit-gate ruling (A-5 / Section D.1).

---

## 6. Failure cases (concrete, including adversarial)

1. **Recorder restart / sequence reset (the E-proven case).** `frame_count` resets to 0 (`upstox_recorder.rs:232-235`); the same payload is replayed with `local_sequence` 7 then 8. Baseline emits two IDs (A2 F-01) and double-counts. Under D-005.1/005.2 both map to one `observation_id` → deduplicated. **Adversarial:** an attacker deliberately resets the counter to flood research data with duplicates; content-addressing makes the flood collapse to one observation.

2. **Frame hash vs per-instrument hash confusion.** A two-instrument frame has one `payload_sha256` (`frame.rs:71`) but two per-instrument `raw_sha256` values (`providers.py:130`). A consumer using the frame hash as observation identity would merge both instruments. D-005.1 keeps the levels separate.

3. **64-bit prefix collision.** Two distinct payloads whose `digest[:16]` prefixes collide (`providers.py:130,151`). Baseline conflates or mis-differentiates; D-005.4/M1-RUN-INV-004 use full 256-bit digests for business identity.

4. **Origin laundering (adversarial).** A normalizer labels an FYERS `exch_feed_time` callback as `exchange_event`, giving it false authority and bypassing conservative reconciliation (`reconcile.py:33-34` refuses only inferred *trade* events, not falsely-labelled exchange events). D-005.5/M1-RUN-INV-006 fail closed; acceptance test asserts zero `exchange_event` emissions from current sources.

5. **µs false assurance.** An engineer assumes reconnect fences are enforced because `EventMeta` exists and is unit-tested (`model.rs:620-664`; A2 F-04). D-005.3 designates it non-authoritative; the contract forbids treating its fields as identity or live ordering.

6. **Replay tautology (challenge 5).** A replay compare reports success because P&L was copied through `RiskStateObserved` (`state.rs:573-577`, `engine.rs:971`) rather than recomputed. Identity stability alone does not fix this; D-005.6 records the event-data dependency on ADR-009. Flagged, not silently absorbed.

7. **Byte-identical repeat payloads — RESIDUAL RISK ACCEPTED (option b).** Two genuine distinct observations may have identical canonical bytes (e.g. two separate fills at identical price/qty/timestamp precision). Content-addressing gives them one `observation_id`; wherever downstream dedup runs, the second is dropped. **Concrete consequence:** the stream under-reports occurrence count, and exposure/P&L derived from it is understated — the same class of harm the account-global and replay guarantees are meant to prevent (ADR-M1-006/009). This is **not solved in M1** and is **not** presented as an enforced guarantee either way: dedup is scoped to known re-delivery/replay contexts (D-005.2) and this distinct-observation case is explicitly excluded from that scope. No non-counter discriminator exists in the feeds to fix it without a speculative field; overturning the accepted risk requires a measured requirement and a future ADR (U-005-c, §8 item 1). Under option (a) this case would instead carry a source-provided event identity — rejected: none is available at baseline.

---

## 7. Dependencies

- **ADR-M1-006 (Ordering and partitions):** supplies `connection_epoch`/`receive_seq` fence semantics and gap/duplicate vocabulary; identity deliberately excludes them. Must stay consistent (no epoch in identity).
- **ADR-M1-009 (Replay contract):** consumes stable identity; owns the "recompute P&L from fills+marks or exclude" rule that this ADR cannot enforce.
- **ADR-M1-008 / ADR-M1-010 (Durability / provenance anchor):** consume `capture_frame_id`, `source_frame_sha256`, `observation_id` as the fields ACKs and anchors reference; envelope implication only.
- **Evidence dependencies:** A2 F-01/F-02/F-03/F-04 (F-01 E-reproduced); R1 Q2 (universal-schema restraint affirmed, F-04 downgraded S2); Amendment A §A3 (F-03→S2, F-04→S2); R1-C Q3 (multi-instrument frame constraint).
- **External capability dependencies:** none. No schema registry, no service, no broker connectivity required.

**Explicitly unresolved disagreements (recorded, not guessed):**
- **U-005-a:** Whether the Rust `EventMeta`/µs model should ever be wired. Decision here = demote to roadmap (no second model). R1 Q2 agrees on downgrade; baseline §13.6 lists "authoritative event model" as unresolved. If M5/collector work creates a measured requirement, a future ADR must resolve ns↔µs losslessly. Not resolved in M1.
- **U-005-b:** Placement of availability/record time — per-event vs manifest-only (A2 F-03, Amendment A S2). Identity contract can reference them as metadata when present; their authoritative placement is deferred to ADR-009.
- **U-005-c — RESOLVED in this amendment by option (b), residual risk accepted (challenger A-2).** Dedup is scoped to contexts where payload identity is known sufficient (re-delivery/replay of a captured payload); it is **not** claimed safe for two genuine distinct observations with identical bytes. The concrete residual risk (two distinct fills merging; exposure/P&L understated) is recorded in §6 case 7 and excluded from the §9 guarantee. This is a design scope decision, **not an owner ruling** and not a verified behaviour; it is referred to owner ratification at the exit gate (challenger Section F.3). Reopening requires either (a) a source-provided, non-counter discriminator, which no current feed supplies, under a future ADR with a measured requirement, or (b) evidence that the collapsed case cannot occur in the declared workload. At baseline neither exists.

---

## 8. Rejection conditions

This decision is overturned or revised if any of the following is established with committed evidence:

1. A measured requirement shows that two byte-identical canonical payloads must remain **distinct** observations and a non-counter discriminator becomes available, overturning the accepted residual risk in D-005.2 / §6 case 7 / U-005-c (option b → option a). Note: M1-RUN-INV-001 asserts ID stability in either case; what changes is the dedup-sufficiency scope, not the hash. Absence of such a requirement is why option (b) was chosen.
2. A cross-stack producer/consumer (e.g. a Rust collector feeding Python replay) is approved with an ADR, making a universal envelope+domain-payload schema warranted and the µs↔ns question resolvable (would revise D-005.4/D-005.3).
3. A source is certified to emit a genuine exchange sequence/time with structured provenance, making `origin_class=exchange_event` attainable (would revise D-005.5 fail-closed rule).
4. Content-addressed identity is shown to break a required bitemporal supersession model for corrected market records (A4-005: no supersession model exists at baseline) — identity may need an explicit `supersedes` link.
5. Level-E evidence shows `observation_id` colliding (full 256-bit) on the declared canonical encoding.

---

## 9. Measurable acceptance requirement (design only — not executed)

**Test name (proposed):** `test_cross_reconnect_observation_identity_stability`
**Command (proposed, to exist only in a gated follow-on milestone):**
`PYTHONPATH=research/python/src python3 -m pytest -q research/python/tests/test_event_identity_contract.py`
- **Owner:** canonical-model / research-data owner (proposed; **unassigned at baseline** — M0 baseline §4 "Market event identity — no explicit owner"). Must be ratified at the exit gate.
- **Target milestone:** M2 (future, separately authorized); **not an M1 deliverable**.
- **PRECONDITION — not an M1 gate.** The test file does not exist at baseline (challenger Section C), no owner is assigned, and the baseline fails the first two thresholds (A2 F-01/F-02, E-reproduced). This requirement is a **specification only**, not falsifiable in M1 and not evidence of correctness.

- Feed the **same** decoded Upstox feed entry (fixed `{"key","wrapped"}` content) to the identity-contract derivation with `local_sequence ∈ {1, 7, 8, 1000}` and simulated `connection_epoch ∈ {0, 1, 2}` and varying `received_utc_ns`.
  - **Threshold:** exactly **1 distinct `observation_id`** across all ≥12 combinations (currently **fails at baseline**: A2 E-result gives different IDs for sequence 7 vs 8).
- For one payload that advances the last-trade tuple, assert the emitted `book` and `last_trade_observation` events have **1 shared `observation_id`** and **2 distinct `business_event_id`**, each back-referencing the observation.
  - **Threshold:** `observation_id` cardinality = 1; `business_event_id` cardinality = 2; both back-references present.
- Simulate a full journal replay after a counter reset with K = 20 **pre-existing captured payloads re-delivered** (the known-sufficient dedup context of D-005.2):
  - **Threshold:** **100%** of re-delivered payloads resolve to an existing `observation_id` (0 new IDs), and downstream dedup admits **0** duplicates. **Scope note:** this threshold certifies only re-delivery/replay dedup. It makes **no** claim for two genuine distinct observations with identical bytes.
- **Residual-risk characterization (challenger A-2, option b) — NOT a correctness gate:** feed two genuinely distinct observations with byte-identical canonical payloads and record the outcome. The design predicts collapse to **1** `observation_id` and loss of the second occurrence. This is recorded as a known limitation with its exposure/P&L impact (§6 case 7); the test documents behaviour, it does **not** assert the collapse is acceptable. A future option-(a) discriminator would change the expected value and this requirement.
- Origin fail-closed: normalize representative Upstox and FYERS payloads and assert `origin_class ∈ {sdk_callback, reconstructed, raw_wire_frame}`; assert **0** events carry `origin_class=exchange_event`.
  - **Threshold:** 0 mislabelled exchange events.

**Baseline status:** this test does not exist; no owner or milestone is committed; the baseline fails the first two thresholds (A2 F-01/F-02, E-reproduced). The third threshold is scoped by option (b) and the fourth is a characterization, not a gate. The whole requirement is **PRECONDITION — not an M1 gate**, to be implemented and executed only after owner authorization; it is **not** evidence of current correctness.
