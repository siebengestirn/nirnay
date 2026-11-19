# Replay P&L invariant — source verification — 2026-11-19

**Scope:** verification of a factual status claim in
`engineering/decisions/ADR-M1-009-replay-contract.md`. This record changes no
code and no decision. The accepted ADR is left intact.

## ADR claim under check

ADR-M1-009 §9 item 2 states, under a design falsifier for copied-forward P&L:
"Current: P&L included tautologically and no machine check exists → fails."
The ADR's §3.1 and §5 and invariant M1-REP-INV-001 require that P&L never be
reported `verified-equal` unless recomputed from an anchored fill stream.

## Verified source state

Direct inspection of the active code at merged `main` `da090f0`:

| Path | Lines | Actual behavior |
|---|---|---|
| `crates/paper-engine/src/state.rs` | 573–582 | `RiskStateObserved` copies `daily_pnl` into the projection (`self.daily_pnl = *daily_pnl`). No comparison occurs. |
| `crates/paper-engine/src/engine.rs` | 971 | `risk_context` reads `self.projection.daily_pnl`. No comparison occurs. |
| `crates/paper-engine/src/bin/paper_run.rs` | 705–712 | Replay-vs-recorded report compares exactly four fields: `dataset_sha256`, `journal_records`, `journal_sha256`, `verification`. No P&L field. |
| `crates/account-journal/src/lib.rs` | 65–71 | `VerificationReport` fields are `record_count`, `segment_count`, `head_hash_hex`, `repaired_tail_bytes`. No P&L field. |
| `crates/paper-engine/src/engine.rs` | 759–828 | `reconcile` compares positions and active order IDs. No P&L field. |
| `crates/paper-engine/src/state.rs` | 527–563 | `ReconciliationObserved` re-checks expected positions and active orders. No P&L field. |

The replay "verified-equal" field set, taken from the actual comparisons, is:
fill price, fees, cash delta, derived position and order state, reconciliation
positions and active orders, plus journal/run report hashes. `daily_pnl` is
**not in that set**.

## Conclusion

The ADR's current-state claim is stale, not a description of this code. There is
no tautological P&L compare to remove: P&L is carried as an unverified
observation and never asserted equal by replay. M1-REP-INV-001 is therefore
**satisfied by construction** in the active code, not violated.

No code was changed. Changing a correct comparison to match an inaccurate
sentence would weaken financial evidence and is rejected.

## Residual risk recorded, not claimed fixed

No machine check currently prevents a future change from adding a P&L field to
the replay-verified set. A guard that asserts the compared field list contains
no P&L term would convert this document finding into an executable invariant.
That is a separate, reviewed change; this record does not claim it exists.

## Boundaries

Verification record only. No decision, capability, claim, code, owner-owned file
or production behavior changed.
