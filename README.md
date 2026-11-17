# Nirnay Quant — research-to-paper trading workspace

**Scope:** India-first systematic research, market-data capture, point-in-time data construction, event-driven paper simulation and controlled execution contracts.

**Release status:** research/paper engineering candidate. This repository is not a certified broker-connected trading system, not a SEBI/exchange-approved algo product, and not evidence of profitable alpha. Live order routing is hard-disabled. Every capability statement below distinguishes code present from external verification still required.

## What is actually implemented

| Capability | Active implementation | Boundary / non-claim |
|---|---|---|
| Durable raw journal | `crates/market-archive` | A single-writer, newline-delimited journal with `sync_data()` per append, poisoned-writer behaviour after an uncertain commit, and a bounded incremental scanner. Serialized records are capped at 1 MiB (512 KiB decoded payload); oversized, torn or corrupt records fail closed. |
| Raw Upstox V3 capture and Protobuf decode | `crates/upstox-feed-proto`, `apps/upstox-tools`, `schemas/upstox/v3/` | `upstox-decode-export` verifies raw-frame hashes and exports full decoded message envelopes. It now reads the journal through the shared bounded `JournalReader`, so record limits, torn-tail detection and EOF semantics match the archive; a failure after several records publishes no output. The current local gate passes 410 Rust test cases on pinned Rust 1.99.1; live provider capture remains unverified. |
| FYERS Data Socket capture | `apps/fyers-recorder` | Captures decoded SDK callback JSON, not original wire packets. Local sequence/gap detection does not establish exchange sequence continuity. |
| Canonical Upstox/FYERS market events | `research/python/src/quantlab/market.py`, `providers.py` | Preserves provider IDs, timestamp semantics, local sequence, source payload and capture-frame hashes. LTP changes are observations, not invented trade prints or aggressor-side labels. |
| Cross-feed reconciliation | `quantlab/reconcile.py` | Compares fresh, comparable provider state for one canonical instrument. It never merges/sums duplicate liquidity or constructs an authoritative consolidated tape. |
| Point-in-time reference data and datasets | `quantlab/point_in_time.py` | Bitemporal listing/universe membership, immutable corporate-action revision lineage, exact split/bonus view, source/frame/event lineage, atomic immutable JSONL publication, optional Zstandard Parquet, semantic Parquet verification and optional detached Ed25519 manifest signature through AWS KMS. A valid signature authenticates the publisher key, not the upstream data source or its completeness. |
| Event-driven backtest | `quantlab/backtest.py`, `quantlab/portfolio_backtest.py` | `EventDrivenBacktester` is single-instrument; `MultiInstrumentPortfolioBacktester` is a separate multi-instrument, single-provider, long-only NSE cash portfolio model with independent per-symbol orders, cash/exposure reservations, latency, partial fills, participation caps, fees, spread/impact and conservative L2 market-by-price queue approximation. Neither is an exchange matching engine; queue/impact parameters need calibration. |
| Strategy and alpha research | `quantlab/strategies.py`, `alpha_search.py`, `validation.py` | Streaming deterministic EMA-cross fixture and bounded predeclared candidate screen with train/validation/untouched holdout, embargo, flat segment portfolios, explicit net costs, Holm/HAC screening, CSCV/PBO and Deflated Sharpe diagnostics. This is a research screen, not arbitrary code generation, full execution-cost proof or validated alpha. |
| Order management and live authorization contracts | `quantlab/oms.py`, `quantlab/durable_oms.py`, `risk_evidence.py`, `risk_service.py`, `risk_gateway.py`, `risk_daemon.py`, `kms_signing.py`, `live_auth.py` | Local SQLite WAL/FULL transactional intent/outbox store, hash-chained journal, leases, idempotent broker-event ingestion, partial fills, cancel/unknown-outcome handling and paper-only dispatcher. A separately runnable HTTPS/mTLS risk-evidence daemon obtains eight independently signed control observations and signs scoped runtime evidence via AWS KMS Ed25519. The daemon and KMS adapter have tests but no deployed KMS key, collectors or cloud service; no broker OMS adapter is certified and live mode always rejects. |
| C++ numeric/book oracle | `verification/cpp-oracle/` | Isolated C++20 verification target, not linked to production. A shared Rust/C++ golden corpus is still required before claiming full differential equivalence. |
| Broker operations | `apps/kite-ops` | macOS Keychain-backed local Kite OAuth login and read-only profile/orders/positions/instrument operations are implemented. Order submission is unconditionally hard-blocked in this build; this adapter is not connected to the durable OMS and is not a production broker adapter. OAuth must be completed by the account owner. |

## Quick start

Python 3.12+ is required. Install the research package and test dependencies:

```bash
python3 -m pip install -e 'research/python[dev,research,security]'
python3 -m pip install -e 'apps/fyers-recorder[test]'
python3 -m pip install -e 'apps/kite-ops[test]'
```

Run the complete local gate (formatting, style checks, Rust and Python tests,
Clippy, dependency policy and contract validation):

```bash
make check
```

Run the local verification suite (Python tests, compile/policy gates, active JSON/TOML and schema validation, and the isolated C++ oracle):

```bash
make verify-local
```

Style is enforced on added lines for Rust, Python, C/C++, Shell, Makefile,
JSON/JSONL and Markdown; see `engineering/style-policy.md`.

The root `pytest.ini` collects only the active test suites and excludes `extras/`, `target/`, virtual environments and generated caches. The Parquet round-trip runs when PyArrow is installed from the `research` extra; CI installs it and fails if the test is skipped.

Check dependency integrity — workflow SHA pins, resolved Python constraints and the SBOM:

```bash
make supply-chain
```

Regenerate the constraint sets with `make constraints` after a deliberate dependency change. Known provider-SDK advisories are waived with an owner and expiry in `constraints/pip-audit-allowlist.toml`; see `engineering/supply-chain.md`.

Check the packaging contract — three clean wheels, no cache entries, each installed and executed in a fresh environment:

```bash
make packaging-test
```

Build a release bundle and verify it case-sensitively the way a Linux consumer would:

```bash
make bundle BUNDLE_OUT=../nirnay-quant-bundle.zip BUNDLE_ARTIFACTS=../artifacts
mkdir -p /tmp/unpacked && cd /tmp/unpacked && unzip -q /path/to/bundle.zip
python3 /path/to/repo/scripts/verify_bundle.py --bundle . --zip /path/to/bundle.zip
```

`scripts/make_bundle.py` writes the manifest and the archive from one file
list, so a manifest path and an archive member name cannot diverge; both are
compared as exact strings, because a case-insensitive filesystem cannot tell
`sbom.cdx.json` from `SBOM.cdx.json`. The bundle job stages compiled output
under `$RUNNER_TEMP/release`, outside the checkout, and the bundler rejects an
artifacts directory inside the source tree so wheels and binaries cannot be
packed twice.

Run the deterministic synthetic strategy/backtest smoke test:

```bash
quantlab demo
```

The demo reports `not_real_market_data=true` and `not_alpha_evidence=true`. It is a deterministic systems fixture, not a performance claim.

## Research pipeline

### 1. Capture and preserve provider data

Upstox V3 captures original binary Protobuf payloads in `raw-frames.jsonl`. The
companion diagnostic projection is useful for inspection but is not treated as
complete depth/quote data. The decoder uses the bounded `JournalReader` from
`market-archive`; a record without a final newline is a torn tail and a failure
anywhere in the journal prevents publication of decoded output. Capture-manifest
verification against the decoded journal is not yet implemented, so this path
establishes file-to-record integrity but not that the archive matches a
completed capture session. Decode the raw frame journal before normalization:

```bash
cargo run --release -p upstox-tools --bin upstox-decode-export -- \
  --input data/market-sessions/upstox/<session>/raw-frames.jsonl \
  --output data/market-sessions/upstox/<session>/decoded-full.jsonl
```

Copy and edit the example maps using an independently reconciled instrument master (do not treat the example entries as live mappings):

```bash
cp configs/providers/upstox-instrument-map.example.json data/upstox-instrument-map.json
cp configs/providers/upstox-venue-map.example.json data/upstox-venue-map.json
```

Normalize the full decoded export using an explicit mapping from provider instrument IDs to canonical security IDs:

```bash
quantlab normalize --provider upstox \
  --input data/market-sessions/upstox/<session>/decoded-full.jsonl \
  --instrument-map data/upstox-instrument-map.json \
  --venue-map data/upstox-venue-map.json \
  --output data/normalized/upstox/<session>.jsonl
```

Prepare the FYERS map separately:

```bash
cp configs/providers/fyers-instrument-map.example.json data/fyers-instrument-map.json
```

The FYERS Data Socket recorder defaults to a conservative 100-symbol cap. Current official documentation lists 100 symbols/user for Standard and 5,000 for Prime across Data Socket connections. The recorder cannot verify your account tier; raise the cap only after confirming the entitlement:

```bash
python3 -m fyers_recorder.cli \
  --watchlist configs/providers/fyers-watchlist.txt \
  --max-symbols 5000 \
  --output data/market-sessions/fyers
```

This path captures `SymbolUpdate` and five-level `DepthUpdate`. FYERS separately documents a 50-level Protobuf Tick-by-Tick (TBT) socket with different limits; the TBT socket is not implemented by this recorder. The active archive is SDK callback JSON, not raw WebSocket bytes.

FYERS callbacks are SDK-decoded JSON, not WebSocket frame bytes:

```bash
quantlab normalize --provider fyers \
  --input data/market-sessions/fyers/<session>/events.jsonl \
  --instrument-map data/fyers-instrument-map.json \
  --output data/normalized/fyers/<session>.jsonl
```

By default, explicit FYERS journal gaps stop normalization. `--allow-gaps` is an explicit degraded mode; resulting quality metadata marks the output as unsuitable for a continuous/research-grade tape. Keep provider instrument maps versioned and dated; do not map on ticker text alone.

Cross-check comparable Upstox/FYERS quote or depth state:

```bash
cat data/normalized/upstox/<session>.jsonl data/normalized/fyers/<session>.jsonl \
  > /tmp/provider-events.jsonl
quantlab reconcile --events /tmp/provider-events.jsonl \
  --now-utc-ns <trusted-current-utc-nanoseconds>
```

Reconciliation is diagnostic. A quote-state comparison does not establish that both providers delivered every event, nor does it justify summing their displayed size. Use a single selected provider for any backtest replay unless an independently designed feed arbitration layer selects one authoritative path.

### 2. Build a point-in-time dataset

Reference data must provide bitemporal provider listings, immutable corporate-action source records and (for historical universe analysis) dated universe-membership records. The repository includes illustrative synthetic examples:

```bash
quantlab build-dataset \
  --events research/python/fixtures/canonical-events.example.jsonl \
  --reference-data research/python/fixtures/reference-data.example.json \
  --dataset-id local-pit-example \
  --created-at-utc-ns <trusted-utc-nanoseconds> \
  --universe-id fixture-universe \
  --output data/research/local-pit-example

quantlab verify-dataset data/research/local-pit-example
```

The fixture's hashes are fixture provenance IDs, not official exchange source hashes. Outputs are immutable and the destination must not already exist. Optional Parquet output requires PyArrow. Optional detached manifest signing requires an AWS KMS Ed25519 key and workload credentials with narrowly scoped `kms:DescribeKey`, `kms:GetPublicKey` and `kms:Sign` permissions:

```bash
quantlab build-dataset ... --parquet

# Optional: sign the canonical manifest digest with a customer-managed AWS KMS Ed25519 key.
quantlab build-dataset ... --kms-key-id arn:aws:kms:REGION:ACCOUNT:key/KEY_ID --kms-region ap-south-1

# Trust the independently distributed public key when verifying a signed release.
quantlab verify-dataset data/research/local-pit-example \
  --trusted-key dataset-release-2026=BASE64_ED25519_PUBLIC_KEY
```

`verify-dataset` establishes internal manifest/artifact/hash consistency; with a trusted key, it additionally verifies the publisher's detached Ed25519 signature over a domain-separated manifest digest. It does not establish source authenticity, entitlements, or source completeness. It cannot certify the source was official, complete, licensed, correct, or independently captured. The builder preserves raw prices and emits exact split/bonus adjustment factors as a separate point-in-time view. Dividends remain cash-flow events; rights, mergers, demergers, delistings and other complex actions fail closed until a reviewed transform exists. If the selected universe membership source is incomplete, survivorship bias remains possible and the manifest does not claim otherwise.

### 3. Run a bounded alpha-search screen

The CSV interface requires strictly increasing `timestamp_ns` and positive `close` columns. The checked-in series is deterministic and synthetic:

```bash
quantlab alpha-search research/python/fixtures/alpha-bars.synthetic.csv \
  --periods-per-year 252 \
  --brokerage-bps 2 --exchange-regulatory-bps 0.5 --taxes-bps 0.5 \
  --spread-bps 2 --slippage-bps 1 --market-impact-bps 1 \
  --output /tmp/alpha-screen.json
```

The search family is finite and predeclared. It selects on validation only, applies Holm correction across the candidate family, then reports one untouched chronological holdout. Each segment starts flat and pays exit/liquidation turnover cost; one return interval at each fold boundary is embargoed. The report additionally includes development-only CSCV/PBO and validation-only Deflated Sharpe diagnostics. Sharpe annualisation is controlled by the explicit `--periods-per-year`; it is not inferred. The screen uses bar closes and simplified basis-point costs, not full event-level fills or all exchange charges. A positive holdout is a research screen, not validated alpha. Use licensed point-in-time data and the event simulator before any strategy promotion.

## Market and execution model caveats

- Upstox `currentTs` is retained as feed-envelope/received-tick time; `ltpc.ltt` is represented separately as last-trade time. A change in an LTP snapshot is not evidence of a discrete trade print with aggressor side.
- FYERS symbol/depth callbacks are decoded SDK messages. Their local callback order is not an exchange sequence. The current recorder captures the Data Socket five-level depth, not the separate 50-level Protobuf TBT socket. Official Data Socket Standard/Prime symbol ceilings are 100/5,000; the configured cap is operator-declared and broker entitlement remains unverified.
- Upstox V3 subscription limits vary by mode and plan; the official docs distinguish LTPC, Option Greeks, Full and Plus Full D30. Do not assume published ceilings apply to the connected account. The current normalizer is not a full CAS/auction-session model.
- Aggregate L2 depth does not expose individual queue priority. The backtester models a conservative queue ahead and only allows passive queue consumption from an explicitly classified trade event with known aggressor side. Current normalizers do not manufacture those trade events from LTP-only updates.
- Taker spread is embedded in crossing the selected book; square-root impact is a separate modeled price displacement. It is not calibrated without real venue-specific data. Impact is an attribution of cost and is not debited twice.
- `quantlab.backtest.EventDrivenBacktester` is single-instrument. The separate `quantlab.portfolio_backtest.MultiInstrumentPortfolioBacktester` supports multiple NSE cash instruments but remains long-only, single-provider, one-active-order-per-instrument and cash-funded. Neither models margin, short borrow, derivatives, auctions, corporate-action cash settlement, multi-venue routing, broker throttles, exact order-by-order priority or exchange-certified matching.
- The strategy emits order intents only. It never calls brokers, risk authorities or OMS mutation methods.

## OMS, live risk and regulatory boundary

The active OMS components are `quantlab.oms` (domain state machine) and `quantlab.durable_oms.DurableOMSStore` (single-node SQLite WAL/FULL transactional store with atomic intent+outbox acceptance, leases, hash-chain event journal, idempotent broker event/execution IDs, monotonic broker sequence, partial fills, cancel request/ack separation and unknown-outcome quarantine). `PaperOutboxDispatcher` admits only paper adapters. SQLite is not replicated HA or distributed exactly-once delivery. There is no certified Upstox/FYERS/Kite execution adapter behind this OMS; Kite operations are read-only and its submission method is hard-blocked.

The permit verifier expects an externally root-signed trust registry, independent issuer signature, distinct approver roles, separate risk-service runtime attestation, bounded scope/time and one-time consumption. Runtime evidence can only be constructed from eight fresh, independently signed source observations (account risk, kill switch, market data, reconciliation, broker entitlement, session, clock and deployed artifact); direct caller-supplied `RiskFacts` signing is rejected. `AwsKmsEd25519Signer` supports a customer-managed AWS KMS Ed25519 key, but no key or separate risk service is deployed here. Private keys must stay in separately administered KMS/HSM/offline services, not in Git, fixtures or the OMS process. Test signing keys are ephemeral. The OMS rejects every live submission, even when a test permit verifies.

Before live use, the organization needs (at minimum) legally reviewed broker/exchange/SEBI applicability, account and data entitlements, certified broker adapters, independent real-time risk service, operational kill switch, static-IP/session checks where applicable, clock/feed health, reconciliation, key management, audit retention, deployment controls, controlled canaries and incident/restore drills. A signed permit is necessary policy evidence, not a substitute for those controls. Consult `engineering/market-compliance-and-entitlements.md` and qualified compliance counsel; this README is not legal advice.

## Build and verification

The Rust workspace pins Rust 1.99.1 in `rust-toolchain.toml`, edition 2024. Run with the pinned toolchain:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo deny check
cargo xtask check
```

Run the promoted C++ verification oracle independently:

```bash
cmake -S verification/cpp-oracle -B /tmp/nirnay-cpp-oracle -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/nirnay-cpp-oracle --parallel
ctest --test-dir /tmp/nirnay-cpp-oracle --output-on-failure
```

The oracle is not wired into production Rust. On 2026-11-18 the pinned Rust
1.99.1 gates were rerun on `main` at `c91ad3f`: local `make check` passed with
410 Rust test cases across the workspace and extras targets, 156 Python tests
and 2 subtests, plus formatting, style, Clippy `-D warnings` for both
workspaces, `cargo deny check`, `cargo xtask check` and contract validation.
Hosted CI run `44199935228` passed 11/11 jobs. CI also builds both C++ targets
under GCC and runs schema validation and the required Parquet round-trip. The
Parquet test executes rather than skips. Live Upstox/FYERS/Kite accounts were
not accessed; provider tests use deterministic fixtures and fake clients/SDK
objects. Earlier 2026-11-13 counts (278 root tests, 96 extras tests) are
historical. See `BUILD-RESULTS.md`, `VERIFICATION.md` and
`engineering/provider-environments.md`.

## Extras policy

`extras/**` is the promoted reference workspace (`nirnay-extras`) and is
excluded from all active build/runtime dependencies. On 2026-11-13 the useful
subset was trimmed from 325 to 61 files and promoted from `legacy/` to
`extras/`: the India ingestion/durable-store crate and tests, the C++
SBE/conformance probes, synthetic fixtures and challenge/internal schemas. The
old monolith, prediction-market sessions, q experiments, archived measurements,
historical registries, duplicate active-crate copies, old locks and old deploy
workflows were moved to the external snapshot
`../legacy-snapshot-2026-11-13/legacy-full-backup-2026-11-13.tar.gz` (SHA-256
`a49e864308a7e58d667a86049dd80e8ff2c631cd762e50a74aff60007464f9b1`). The
predecessor's useful Upstox decoder and protocol schema are active; selected
C++ numeric/book code is promoted as an isolated oracle; corporate-action and
provenance concepts are reimplemented behind current bitemporal contracts.
`cargo test --manifest-path extras/Cargo.toml` and the extras C++ probes are
part of the active gates. Historical counts from 2026-11-13 and 2026-11-17 are
retained in the dated verification documents; the current result is the
2026-11-18 gate run recorded above. See `engineering/extras-promotion-audit.md`,
`engineering/legacy-audit-2026-11-13.md` and `engineering/migration-map.csv`.
Promotion of the retained India code into the active workspace still requires a
new crate, ADR, owner and tests.

## Engineering standards and evidence

- `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`
- `engineering/style-policy.md` (binding style gates for every source language)
- `engineering/anti-patterns.md`
- `engineering/research/point-in-time-data.md`
- `engineering/research/provider-normalization.md`
- `engineering/research/backtesting-and-alpha.md`
- `engineering/research/oms-and-live-risk.md`
- `engineering/production-readiness-matrix.md`
- `engineering/deployment/aws-kms-risk-service.md`
- `engineering/market-compliance-and-entitlements.md`
- `engineering/extras-promotion-audit.md`
- `UPGRADE-VERIFICATION.md` (current implementation inventory, exact verification results and owner-run Rust/live acceptance gates)
- `VERIFICATION.md` (executed checks, including the owner completion rerun)

**Governing principle:** a capability is admitted by contract, executable tests and external evidence—not by a successful demo, strategy backtest, architecture diagram or a green check from a different commit.

### 2026-11-18 current-state note

The verification for the current `main` commit `c91ad3f` is recorded in this
note and in hosted CI run `44199935228`, which passed 11/11 jobs; the
pinned-toolchain local `make check` passed all project gates (410 Rust test
cases, 156 Python tests, 2 subtests). `VERIFICATION.md` still contains the
older dated 2026-11-17 record; older counts and commit hashes in dated
documents are historical. These results verify the named commit only and do
not grant M1 exit approval, M2 authorization, platform readiness or live
trading. Capture-session verification and the raw-to-paper vertical slice are
still outstanding.
