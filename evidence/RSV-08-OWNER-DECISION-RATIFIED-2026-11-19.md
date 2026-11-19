# RSV-08 owner decision — ratification — 2026-11-19

**Status: RATIFIED — Option B.** The owner selected Option B: supersede the
frozen multi-stage crash-boundary requirement with SQLite transaction atomicity
plus immutable revisions. Earlier records remain immutable: the PREPARED pack
(`evidence/RSV-08-OWNER-DECISION-2026-11-19.md`) and its independent review still
report the condition as initially not confirmed; this record is the decision.

## Decision

- The publication protocol for `DurableReferenceStore` is one `BEGIN IMMEDIATE`
  SQLite transaction containing the immutable revision row and the singleton
  head, with resolution by publication key on retry. The archive object write
  and manifest append remain outside publication, as implemented.
- RSV-08 is **superseded** under this decision. It is not marked `passed`, and
  the frozen contract's multi-stage crash tests are not relabelled as passed.
- No publication journal was implemented. The owner did not select Option A.

## Basis recorded at decision time

- Decision pack: `evidence/RSV-08-OWNER-DECISION-2026-11-19.md`.
- Independent review: CONDITION NOT CONFIRMED on the original record, with four
  uncovered objectives (`.local/nirnay-review-archive/
  rsv08-owner-decision-independent-review.md`).
- Evidence gaps closed afterwards: `evidence/RSV-08-ACTIVE-EVIDENCE-2026-11-19.md`
  (PR #56), with child-process termination at during-transaction,
  after-insert-before-commit, before-commit and after-commit boundaries,
  uncertain-commit poison/reopen resolution, and injected full-disk plus
  read-only-path failures. PR #56 review verdict: APPROVE WITH NON-BLOCKING
  COMMENTS; exact-head CI 11/11 on that PR's head.

## Non-equivalence acknowledgement (carried forward)

The frozen multi-stage crash tests and the transactional tests are **not
equivalent**. Two frozen kill points (archive object write, manifest append) do
not exist in the active protocol. The surviving boundaries are governed by one
SQLite commit, and uncertain outcome resolves by publication key rather than by
a staged journal. Supersession accepts the replacement's objective-level
guarantees; it does not assert procedure-level equivalence.

## Scope

Decision only. No contract edit, capability or claim promotion, journal, code
change, `extras/` change, live path or trading component.

Owner signature: **recorded in this session** (owner-directed selection of
Option B). Decision date: 2026-11-19.
