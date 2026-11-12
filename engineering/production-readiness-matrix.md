# Production-readiness matrix — Nirnay Quant

**Assessment:** engineering candidate with deterministic research and paper components. **Not production-live authorized.**

This matrix is the release authority for claims about capability. `implemented` means source code and local tests exist. It does not mean a provider, data right, cloud key, independent control, operational recovery procedure, or production account has been verified.

## 1. Six requested systems

| System | Implemented in repository | Verified in this delivery | External/operational work still required | Release status |
|---|---|---|---|---|
| Broker-neutral OMS | SQLite WAL/FULL synchronous store; transactionally accepted intent + outbox; leases; unknown-outcome handling; idempotent broker event and execution IDs; monotonic broker sequence; partial fills; cancel request/ack separation; hash-chain audit; paper dispatcher | Python state, failure and replay tests | Actual broker-specific execution adapters; reconnectable order-update streams; authoritative broker snapshot reconciliation; idempotency/duplicate behavior proven per broker; PostgreSQL/HA deployment decision; backup/restore, failover, capacity, incident drills, independent operations review | **Reference/paper only** |
| Independent risk authorization | Ed25519 root-signed trust registry; distinct issuer/risk signer/approver/control-source identities; two or more distinct approval roles; eight independent source roles; strict payload schemas, freshness, expiry and scope; monotonic per-source observation sequence with atomic replay ledger; AWS KMS Ed25519 signing adapter; one-time permit-use ledger; standalone HTTPS/mTLS risk-evidence daemon and mTLS client for independently signed control sources | Local cryptographic, mTLS transport, daemon API and negative-path tests with ephemeral keys/fake KMS | Deploy separate risk service and eight real control collectors; provision customer-managed KMS key and IAM boundaries; off-host append-only audit; trusted clock and managed mTLS identity; independent root/issuer/approver key ceremony, revocation/rotation, restore and compromise drills; production approval workflow | **Runnable service code; no provisioned service/key/collectors or live authorization** |
| Event-driven backtesting | Multi-instrument, single-provider replay; long-only cash equity; event ordering; order/cancel latency; partial fills; participation caps; cash/exposure reservations; fees/spread/impact; conservative market-by-price queue approximation; passive fill requires explicit trade event and aggressor | Synthetic deterministic tests for multi-symbol accounting, liquidity caps, latency, queue and provider isolation | Real exchange L2/L3 or MBO data; calibrated venue/broker latency, queue and impact parameters; auctions, halts, circuit limits, order types, settlement, dividends, corporate actions, taxes/charges and market-specific behavior; out-of-sample fill calibration | **Research simulator, not exchange matching engine** |
| Alpha generation/validation | Predeclared finite candidate family; causal signal application; train/validation/untouched chronological holdout; embargoed boundaries; flat fold portfolios; configured net costs; Holm multiple-testing adjustment; approximate HAC inference; CSCV/PBO and Deflated Sharpe diagnostics | Deterministic synthetic screen and tests | Licensed point-in-time data; preregistered hypotheses; broader independent trials registry; stability/regime/capacity analysis; event-level execution backtest; prospective paper observation; independent investment/risk review | **Screening framework; no alpha claim** |
| PIT data and corporate actions | Bitemporal listing and universe-membership revisions; immutable source-hashed corporate actions; exact split/bonus factors as separate views; raw price preservation; source-frame/event/lineage hashes; atomic immutable JSONL release; optional detached Ed25519 manifest signature | Synthetic PIT builds, as-of tests, lineage hash recomputation, immutable publish and signed-manifest test | Authoritative, licensed NSE/BSE historical/reference and corporate-action feeds; delisted/suspended/merged instruments; correction/cancellation completeness; official calendars; source-level controls and reconciliation; independent licensing approval | **Framework implemented; source completeness unverified** |
| Parquet | Zstandard Parquet writer, versioned metadata, artifact SHA-256 and semantic row-by-row comparison against canonical JSONL; optional detached signature applies to publication manifest digest | Local PyArrow 26.0.1 run and the CI `python-research` job exercise the semantic round-trip and tamper-rejection test | Schema evolution, compression alternatives, row-group sizing and long-run compatibility against real licensed data on the deployment platform; object-store immutability and retention | **Implemented and locally verified on synthetic fixtures; real-data scale unverified** |

## 2. OMS admission and non-negotiable behavior

The active durable OMS is a **single-node SQLite implementation**. It is not a distributed transaction coordinator or replicated HA system. It makes the internal acceptance/outbox transaction atomic; it cannot make a remote broker side effect exactly once.

The key safety rule is: once a command enters `DISPATCHING`, any timeout, process crash, or expired lease becomes `UNKNOWN`; it is not automatically retried. Reconciliation must establish whether the broker accepted the action before another submission is considered. Cancel request acknowledgement is not cancel confirmation. A fill/cancel race is represented by later broker events, not inferred from an HTTP response.

The only admitted dispatcher is paper mode. `mode="live"` is a hard failure. Kite's current operations client is read-only; it does not provide an OMS execution adapter. There is no supported command that can turn live trading on by editing a config file.

## 3. Risk authority and key separation

A valid runtime evidence signature is not a live permit. The permit verifier additionally binds the exact commit, broker, account, strategy, instrument, side, order type, product, quantity, notional, policy hash, artifact hash and expiry, plus a release issuer and independent role-distinct approvals.

`SignedEvidenceRiskGate` builds runtime evidence only after validating the offline root-signed registry and independently signed observations for:

1. account risk/exposure;
2. kill switch;
3. market-data health and unresolved gaps;
4. broker/account reconciliation;
5. broker entitlement;
6. venue session;
7. clock synchronization;
8. deployed strategy artifact/commit.

The public `IndependentRiskService.evaluate_and_sign` rejects caller-assembled `RiskFacts`. The observation gate consumes strictly increasing control-source sequences through a transactionally updated replay ledger; the included SQLite ledger is single-node, not a clustered compare-and-set service. `quantlab.risk_daemon` provides a standalone HTTPS/mTLS evidence service; it obtains all eight observations itself through `quantlab.risk_gateway`, enforces client-certificate fingerprints and uses the KMS signer. The service code is runnable but has not been deployed. The internal signing capability is an application-layer guard only: production independence requires separately deployed processes, OS/cloud identities, key policies, networks and operators. Python module privacy is not a security boundary.

AWS KMS Ed25519 support is implemented as an adapter and tested with a fake KMS. No AWS key, key policy, IAM role, CloudTrail trail, private endpoint, service deployment or root trust ceremony was provisioned or exercised in this delivery. See `engineering/deployment/aws-kms-risk-service.md`.

## 4. Market data and research claims

- Upstox raw binary Protobuf capture and FYERS decoded SDK callback capture have different source fidelity. A FYERS callback journal is not original WebSocket bytes.
- Provider-local receive sequence is not an exchange sequence. `currentTs`/feed timestamps are not silently relabelled matching-engine timestamps.
- Two providers observing the same venue liquidity are not two independent pools. Reconciliation is a data-quality comparison; backtests select one provider stream.
- A last-trade snapshot change is not a confirmed trade print and does not identify aggressor side. The L2 passive queue simulator advances only on explicitly represented trade events with known aggressor side. If the source does not provide these, passive fills are unavailable rather than fabricated.
- L2 market-by-price quantity cannot establish FIFO order, hidden liquidity, cancellations ahead, or queue priority. Exact FIFO claims require suitable order-by-order/MBO data and validated venue semantics.
- Split/bonus adjustment is a separate exact-ratio view. Dividend cash-flow entitlement, tax, reinvestment, rights, merger consideration, demerger allocation, delisting recovery and settlement are not silently approximated as split ratios.
- PBO/DSR/HAC results are diagnostics. The candidate search is finite and close-based; it does not synthesize arbitrary strategy code or prove future profitability.

## 5. Production acceptance gates

No system moves from research/paper to live until all applicable gates have evidence attached to the exact release SHA:

1. **Data rights:** executed provider/exchange contracts and entitlement records for collection, retention, historical use, derived data and redistribution.
2. **Data quality:** real sessions from each source; gap/reconnect/duplicate/sequence audits; instrument master and corporate-action reconciliation including delistings and corrections; clock/market-calendar checks.
3. **Replay:** deterministic replay from raw capture; restart/rebuild and artifact verification; Parquet semantic verification on deployment version.
4. **Simulation calibration:** actual order/ack/fill timestamps and market data; measured latency distributions; venue-specific fee/tax schedule; queue/impact calibration; holdout periods not used for tuning.
5. **Broker conformance:** official broker API integration and account entitlements; read-only profile/order/position reconciliation first; duplicate, timeout, cancel/replace, partial fill, disconnect and unknown-outcome drills; no auto-retry of ambiguous non-idempotent actions.
6. **Risk independence:** separately deployed risk service; independent control collectors; separate KMS/IAM and release/approver roles; key rotation/revocation; audit evidence; kill-switch drills; stale-source and clock-skew tests.
7. **Operations/security:** threat model, penetration review, least privilege, secret rotation, signed release provenance/artifact attestation, alert ownership, runbooks, backups/restores, RTO/RPO and incident response. Dependency integrity is now enforced in-repository: workflow actions are commit-SHA pinned, each Python surface ships a resolved constraint set, `pip-audit` gates undocumented advisories, and a deterministic CycloneDX SBOM is generated per release (see `engineering/supply-chain.md`). Packaging is gated the same way: `scripts/check_wheels.py` rejects cache entries in a built wheel and installs each one in a fresh environment, and `scripts/make_bundle.py` plus `scripts/verify_bundle.py` generate the manifest and archive from one file list, preserve executable permissions, and verify them by exact string comparison, including a self-test that proves the mode check can fail. A tag-triggered `release.yml` wires build-provenance attestation through `actions/attest-build-provenance`, and cannot publish unless the reusable `ci.yml` gates pass first. Still external: that workflow has never run, so no artifact carries an attestation, and custody of a release-signing identity is undefined.
8. **Regulatory/legal:** applicability review for the exact strategy, users, broker arrangement and exchange/regulatory framework; required broker/exchange approvals and controls. This repository does not establish regulatory approval.
9. **Approval:** independent engineering, quantitative research, operations, security and risk sign-off. The developer/strategy author cannot approve their own live authorization.

## 6. Truthful capability vocabulary

Use these terms in PRs and documentation:

- `implemented`: source exists and is covered by repository-level tests;
- `locally_verified`: command and exact commit SHA recorded in `VERIFICATION.md`;
- `provider_verified`: real provider connection/entitlement and session evidence exists;
- `data_certified`: source rights and completeness have been independently reviewed;
- `operationally_certified`: runbooks, recovery, security and ownership gates passed;
- `live_authorized`: separate release permit and all legal/broker/operational conditions are true.

A green test suite can establish only the first two.
