# M2 authorization — Nirnay trading kernel — 2026-11-20

**Status: SIGNED — owner authorization recorded.** Baseline
`f016ad4170912eb8fbca9705b994d003f4a99a82` (verified main, hosted CI run
`44591312481`, 12/12 success; local `make check` exit 0). This record is the
controlling M2 decision. The earlier draft that listed both signatures as
pending was a proposal, not a signed record.

## Owner decisions

- **M1 exit acceptance: accepted** by the owner on 2026-11-20.
- **M2 implementation: authorized** by the owner on 2026-11-20, bounded to the
  scope below.

Signature: recorded by explicit owner instruction in this session. The owner
selected authorization rather than keeping it pending or modifying scope.

## Objective

Build one complete NSE cash-equity trading vertical in paper and shadow modes:
market-data capture through strategy, portfolio, risk, OMS and paper execution.

## Authorized work

1. Canonical event identity and continuity (M2.1).
2. Rust market-data and market-state kernel (M2.2).
3. One primary market-data source.
4. One deterministic Rust strategy runtime.
5. Portfolio sizing and account-global exposure.
6. Stateful risk authorization and atomic exposure reservation.
7. One authoritative Rust OMS.
8. One broker-adapter contract and test harness.
9. Replay, reconciliation and operating controls.
10. Paper/shadow deployment on Linux.
11. Operational monitoring, backup and recovery tests.

## Restrictions

- Live order submission stays disabled.
- No multi-asset, international, distributed-microservice, colocated-HFT or
  FPGA work.
- No automatic production deployment.
- No public trading service or external distribution.
- No new infrastructure dependency without an accepted contract and measured
  requirement.
- Kalshi and Polymarket remain outside the active product.

## Acceptance (per milestone)

Source-to-paper path must pass exact identity and continuity tests, real-source
ingestion and recovery, deterministic fresh-process replay, portfolio and
accounting tests, risk reservation and concurrency tests, OMS crash and UNKNOWN
reconciliation tests, broker-adapter contract tests, full CI with independent
review, and deployment/backup/restore tests. Implementation and verification
statuses stay separate.

## M2.1 — first increment

The owner directed M2.1 (event identity and continuity) as the first
implementation, then one NSE cash vertical through reconciliation, before any
asset-universe expansion.

This authorization does not authorize live trading. A separate owner decision
is required for R5/R6.
