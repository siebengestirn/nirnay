# Risk controls and recovery drill — recorded local implementation evidence

Date: 2026-11-21. Worktree `/Users/tourist/Documents/nirnay-riskctl`, branch
`feat/risk-controls`, base `ec0b216f07e7b7e653e8f784b18c6517d6335696`. No commit
was created by this work: the change set is uncommitted on the named branch, so
the commit field below records the base and the uncommitted state rather than a
new SHA. Status is **Implemented**, not Verified: no independent reviewer, CI
run or production environment is claimed.

## Commit and state

- `base_commit`: `ec0b216f07e7b7e653e8f784b18c6517d6335696`
- `head_commit`: same as base (no commit created; uncommitted worktree change)
- `branch`: `feat/risk-controls`
- `working_tree`: modified tracked files plus new source/test files listed below

## Commands and counts

- `cargo fmt --all` — exit 0.
- `cargo test -p trading-core --lib` — 13 passed, 0 failed (one new digest test).
- `cargo test -p risk-engine` — 27 passed, 0 failed.
- `cargo test -p risk-authority --test risk_controls` — 5 passed, 0 failed.
- `cargo test -p authoritative-oms --test permit_binding` — 4 passed, 0 failed.
- `cargo test -p paper-runner --test recovery_drill` — 1 passed, 0 failed; the
  single test runs eight boundary/fault cases with four journal fault kinds.
- `cargo test --workspace` — full Rust suite, 0 failed.
- `make check` — full deterministic gate; the final exit status is recorded in
  the completion report, not promoted to an independent verification claim.

New falsifier tests total 17 including the trading-core digest test. They cover:
a tampered intent, a rewritten policy version or snapshot revision, a spent
permit with a new client id, an identical replay after restart, two intents under
an order cap but over the strategy cap, a distinct strategy bucket, an over-cap
or unlabelled sector, a configured stress-loss breach, over-participation and
missing depth, over-limit or missing derivative greeks, a short without borrow or
above cost, and one integrated drill over every OMS and account-journal append
boundary under both OMS faults and four account-journal faults.

## Drill result

The integrated drill walks append boundaries 0 through 3 with OMS
`CommitFault::PartialWrite` and `CommitFault::SyncFailure`, and injects the
account-journal faults `BeforeHeader`, `MidBody`, `AfterBodyBeforeSync` and
`AfterSync` at the same boundaries. At every boundary it asserts the committed
prefix of both journals, that a poisoned writer refuses every later mutation,
that an engaged persisted kill switch survives a fresh authority and denies new
risk, that a replay digest mismatch latches the reconciliation block past a
later matched view, and that `Projector::verify_replay` succeeds on the committed
ledger prefix. Torn OMS and journal tails are refused or repaired to the prefix;
the After* journal faults write a complete frame that replays as one additional
record. Independently reopened OMS stores report identical state digests.

## Source digests (SHA-256)

- `crates/trading-core/src/risk.rs`: `fb1587755dd62b682e4b48ea659bc3c21599d4c8bbe0550a9def2ef76cdac5cd`
- `crates/risk-engine/src/lib.rs`: `7c0eb780b1997a172f388d1f8b76b0df1623d426e6f2bf7a9de340159c69c937`
- `crates/risk-authority/src/authority.rs`: `22f63113a147615e2ddfa0f2da9b28f734ce7916cac039c0eec4c3fe83ee95c6`
- `crates/authoritative-oms/src/store.rs`: `fccb1f83f9f9b8d1cd2b536fa79bf63657fc284fea337e41c8bf684692811369`
- `apps/paper-runner/tests/recovery_drill.rs`: `49357d9b0885207f450a3651bf5037921dffe29a0e9ace69b1432c5de8a8d463`

## Boundaries and blocked items

The permit digest is an unkeyed integrity binding over canonical order terms.
It is not a signature. Real KMS keys, signed permits, production identity, a
deployed independent risk service and collectors remain blocked by owner KMS/IAM
decisions. Sector, stress, liquidity, Greeks and borrow controls are typed,
synthetic and uncalibrated; a real sector master, ADV/depth history, stress
calibration, vol/strike surfaces, borrow/margin/funding rates and derivatives
product expansion remain blocked by data rights and an owner/ADR decision. No
new `Product` variant, pricing library, database, service, live endpoint or
credential was added. The recovery drill is in-process; independent
certification, HA or distributed failover, multi-region recovery and certified
broker-delivery semantics remain blocked and are not claimed. No profit,
live-readiness or platform-readiness claim follows from this record.

## Capability and claim records

CAP-050 through CAP-053 and CLM-045 through CLM-048 carry `status: implemented`
with `evidence_status` and `falsifier` text that state the boundaries above.
