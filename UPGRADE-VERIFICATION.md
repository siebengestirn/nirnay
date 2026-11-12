# Nirnay Quant upgrade and verification report

**Report date:** 2026-11-12 (initial); 2026-11-13 (release-hardening rerun); 2026-11-13 (r3 identity and packaging gates)  
**Repository:** Nirnay Quant, India-first research-to-paper trading workspace  
**Delivery scope:** Data normalization and reconciliation, bitemporal research datasets, event-driven paper simulation, bounded alpha research, OMS/risk reference contracts, C++ numeric/order-book oracle, extras promotion audit.

## Executive result

The supplied V4 candidate has been upgraded with a runnable Python research/data path, active contracts, synthetic fixtures, test coverage, CI changes and a promoted C++ oracle. The repository is materially more complete for reproducible research and paper simulation. It is **not** a production brokerage execution platform, a certified algo-trading system, a complete multi-asset backtester or evidence of profitable alpha.

Live order submission remains hard-disabled. The OMS module is a broker-neutral reference lifecycle implementation and adapter protocol, not a production adapter to Upstox, FYERS, Kite or another broker. The signed permit verifier verifies an externally rooted approval/evidence chain, but there is no deployed independent HSM/KMS risk service, key ceremony, broker certification or live control plane in this repository.

## Release-hardening rerun — 2026-11-13

**Environment:** macOS 27.0.1 arm64; AppleClang 21.0.0.21000334; Homebrew GCC 16.2.0; CMake 4.4.3; pinned Rust 1.99.1; Python 3.14.8; pytest 9.2.0; PyArrow 26.0.1; clang-format 21.1.8.  
**Revision basis:** the delivery has no VCS metadata. The bundle is identified by the adjacent `.sha256` sidecar and the `SOURCE-MANIFEST.sha256` content digest recorded in `BUILD-RESULTS.md`; a previous archive hash is not evidence about this bundle.

| Gate | Result |
|---|---|
| `cargo fmt` (root and `extras/`) | **PASS** |
| `cargo test --workspace --all-targets --locked` | **PASS — 278 tests, 0 failed** |
| `cargo test --manifest-path extras/Cargo.toml --locked` | **PASS — 96 tests, 0 failed** |
| `cargo clippy -D warnings` (root and `extras/`) | **PASS** |
| Root C++ oracle, AppleClang 21 | **PASS — 1/1** |
| Root C++ oracle, GCC 16.2.0 | **PASS — 1/1** |
| Extras C++ probes, AppleClang 21 | **PASS — 1/1** |
| Extras C++ probes, GCC 16.2.0 | **PASS — 1/1** |
| Plain root `python3 -m pytest -q` | **PASS — 156 passed, 0 skipped, 2 subtests** |
| PyArrow Parquet round-trip and semantic tamper rejection | **PASS — PyArrow 26.0.1** |
| Python `compileall` (research, apps, scripts, ops) | **PASS** |
| `scripts/check_repo.py check` with Cargo present | **PASS** (no static fallback) |
| `scripts/validate_contracts.py` | **PASS — 42 JSON, 27 TOML, 17 Draft 2020-12 schemas** |
| `clang-format 21.1.8 --dry-run --Werror` | **PASS — all 10 C/C++ files** |
| Artifact hashes and bundle integrity | **PASS — 8/8 artifacts; ZIP and sidecar hashes recorded separately** |
| `scripts/supply_chain.py pins` | **PASS — 43 workflow action references pinned to commit SHAs** (30 at r2; 13 added by the packaging, macOS and release jobs) |
| `scripts/supply_chain.py constraints` | **PASS — 3 constraint sets, 79 exact pins, every declared dependency covered** |
| `scripts/supply_chain.py sbom` | **PASS — 342 components, deterministic (identical sha256 across runs)** |
| `pip-audit` (research, kite-ops, fyers-recorder) | **PASS — 0 undocumented advisories; 51 inherited FYERS advisories waived with owner and expiry** |

CI now covers the root Rust workspace, the separate `extras/` Rust workspace, both C++ targets under GCC, the pinned C++ formatter, schema validation, the required Parquet test, and isolated Kite/FYERS environments. A `provider-boundary` canary confirms the known `fyers-apiv3`/`requests` conflict is still present, so isolation cannot silently rot. The hosted CI run itself is not evidence in this local report until it executes on the pushed commit; the table above records the equivalent commands run locally.

### Release-hardening change set (r2, 2026-11-13)

Dependency integrity moved from "future work" to enforced gates:

- **Workflow actions are commit-SHA pinned** in `ci.yml`, `nightly.yml` and `release.yml` (43 references), each with a `# vX` provenance comment. `taiki-e/install-action` selects its tool through `tool:` because pinning its per-tool tags by hash is discouraged upstream.
- **Python resolution is pinned** in `constraints/` — 37 pins for research, 10 for Kite, 32 for FYERS — compiled with `uv` against the Python 3.12 target CI uses. CI installs from the constraint file and then installs the local package with `--no-deps`; all three environments were rebuilt that way locally and passed `pip check` and their test suites (research 156, Kite 21, FYERS 7).
- **`pip-audit` gates every Python job**, with owner-assigned, expiring waivers in `constraints/pip-audit-allowlist.toml`. Research and Kite report zero advisories. FYERS reports 51, all inherited from `fyers-apiv3 3.2.1` pinning `aiohttp==3.9.3`, `requests==2.31.0` and `setuptools==68.0.0`; this is recorded as accepted residual risk, not silently waived.
- **A CycloneDX 1.5 SBOM** is generated deterministically from `Cargo.lock` plus the constraint sets (`scripts/supply_chain.py sbom`, 342 components).
- **Wheel packaging defect fixed:** a stale `src/**/__pycache__` was being shipped as package data in all three wheels. `include-package-data = false` is now set, wheels are rebuilt clean, and the Kite wheel still carries `kite_ops/native/keychain.swift` (verified by installing the wheel in a fresh environment).
- **Artifacts rebuilt** in this environment (three wheels, three Rust binaries, the C++ oracle and the Swift helper) and re-hashed; `artifacts/SHA256SUMS` verifies 8/8.

Gate 0 exit condition is met for CI coverage: every required gate runs in CI, and no gate is excluded for an undocumented platform reason. The remaining Gap 0 caveat is unchanged and cannot be closed by repository code — **the workflow changes have not executed on a pushed commit**; only the equivalent commands were run locally.


### Release identity and packaging gates (r3, 2026-11-13)

An independent review of the r2 archive found two identity defects. Both are fixed, and both are now enforced by a gate rather than by care.

| Property | r2 | r3 |
|---|---|---|
| Archive hash in the build report | stale value copied from a superseded build | sidecar is written from the archive itself by `scripts/make_bundle.py`; the report quotes that value |
| Root SBOM filename | archive `SBOM.cdx.json`, manifest `sbom.cdx.json` | one name, `sbom.cdx.json`, in source, manifest and archive |
| Manifest and archive | separate ad-hoc `zip` invocation, so member names could diverge | generated from one file list; member list asserted equal to the manifest |
| Case-sensitive verification | none — APFS resolved the mismatched name, a Linux runner would not | `scripts/verify_bundle.py` compares exact strings and needs no case-sensitive filesystem |

| Gate | Result |
|---|---|
| `python3 scripts/check_wheels.py --dist dist` | **PASS — 3 wheels, zero cache entries, each installed and ran its console script in a fresh venv** |
| Negative control: a `__pycache__` entry injected into a wheel | **FAILS as intended** — two named errors |
| `python3 scripts/verify_bundle.py --bundle <r3 tree> --zip <r3 archive>` | **PASS — every manifest entry, archive member and member hash** |
| Negative control: the same verifier against the r2 archive | **FAILS as intended** — four named errors, including `sbom.cdx.json` versus `SBOM.cdx.json` |
| `xcrun swiftc -typecheck` and `-O` on the bundled Keychain helper | **PASS — Swift 6.4, Mach-O arm64** |
| Workflow inventory | `ci.yml` 9 jobs, `nightly.yml` 5 jobs, `release.yml` 2 jobs; all YAML parses |

New CI surface: `packaging` (clean rebuild, cache rejection, clean-environment install and CLI smoke), `macos` (Apple silicon Rust workspace, AppleClang for both C++ targets, Swift typecheck and compile, macOS wheel build and install), and a tag-triggered `release.yml` that rebuilds from the tagged commit, re-verifies the bundle as a consumer would, and attests the archive, wheels, binaries and SBOM with `actions/attest-build-provenance` pinned to `4d101475…` (v4.2.2).

Not established: no new job has run on a hosted runner, no attestation has been produced, and no release-signing identity exists.

## Implemented in the active repository

### Point-in-time datasets and corporate actions

- Bitemporal provider listings, universe membership and corporate-action records; valid-time and knowledge-time selection are distinct.
- Listing and action revisions are immutable supersession chains. Late-published reference data cannot be applied to an earlier observation's knowledge state.
- Canonical instrument/venue resolution is checked at dataset admission; a provider listing venue mismatch is rejected.
- Corporate-action source hashes, announcement/retrieval/recorded timestamps, ex/effective, record and payable dates, revision lineage, exact split/bonus ratios and action status are retained.
- Raw source prices are not overwritten. Split/bonus adjustment is a separate exact rational view; dividend is a cash-flow event. Rights, mergers, demergers, delistings and other unimplemented transforms fail closed.
- Dataset rows retain event, raw capture-frame, listing, universe-membership and corporate-action lineage hashes. The manifest verifier recomputes the published hash sets.
- Publication uses a lock, staging, fsync and atomic directory rename; it refuses an existing destination.

### Upstox and FYERS

- Upstox V3 normalization preserves `currentTs` and `ltpc.ltt` as different timestamp semantics, and maps full quote/depth data only from decoded full Protobuf messages.
- A Rust `upstox-decode-export` binary was added to verify raw-frame hashes and export full decoded Protobuf envelopes for Python normalization.
- FYERS Data Socket SDK callback records are normalized separately from original wire frames. The normalizer validates the recorder hash chain when present and rejects explicit capture gaps by default.
- Both providers map through explicit provider-instrument-to-canonical-ID maps. Last-price observations are not promoted into synthetic unique trade prints or aggressor-side classifications.
- Cross-feed reconciliation compares fresh comparable quote/depth state. It does not average prices, sum duplicate liquidity or certify sequence continuity.

### Event-driven backtesting

- Deterministic event replay uses a single explicitly selected provider and one canonical instrument.
- Order activation and cancellation include configured latency; time-in-force, partial fills, participation caps, maker/taker fees and square-root taker impact are represented.
- A displayed L2 queue-ahead approximation is used; it is not FIFO truth. Passive queue consumption requires a real `trade` event with known compatible aggressor side. LTP snapshot changes do not qualify.
- Repeated identical snapshots cannot be reused as fresh liquidity. Displayed-size increases are the only newly displayed liquidity tranche accepted by the current model.
- Execution cash/P&L includes fill prices and fees. Impact is itemized but not double-debited. Signed spread cost is measured against the last fresh two-sided midpoint; maker improvement may be negative. Gross/net attribution is regression-tested.
- The simulator is intentionally single-instrument, long-only, one-active-order cash equity. It does not implement exchange FIFO, multi-asset portfolio/risk, short borrow, derivatives, auctions, settlement, corporate cash flows or calibrated real market impact.

### Strategy and alpha research

- A deterministic streaming EMA-cross strategy emits intents only; it has no broker or risk access.
- A bounded, declared 25-candidate family (momentum, mean-reversion and EMA cross) can be screened with costs, chronological train/validation/untouched holdout, one-return embargo at boundaries, flat-start/flat-end folds and Holm-adjusted validation statistics.
- The bar-close research screen applies configurable basis-point costs and explicit annualization cadence. It is not event-level execution simulation, arbitrary program synthesis or an alpha-certification framework.
- A positive synthetic holdout is labelled `positive_holdout_screen_passed_not_alpha_certified`; the checked-in bars are deterministic synthetic fixtures, not market data.

### OMS and live-risk reference boundary

- The local OMS has stable client identity/idempotency, broker order identity, a hash-chained journal, partial fill accounting, duplicate broker-event detection, explicit unknown submit outcomes, cancel intent and restart reconstruction.
- A broker adapter protocol defines the intended boundary. No production broker adapter is implemented or certified.
- The signed permit verifier checks a root-signed trust registry, issuer signature, distinct approver roles, an independent risk-service signature over runtime evidence, commit/strategy/policy hashes, order/account/broker scope, limit bounds and validity windows. Permit consumption is recorded once-only.
- **The OMS refuses all live submissions even when a test permit verifies.** Production use requires separately deployed keys and signing services, hardened durable state, adapter conformance/recovery tests, exchange/broker entitlement and legal/compliance approval.

### C++ oracle and extras promotion

- Selected predecessor C++ numeric/order-book code is promoted under `verification/cpp-oracle/` as an isolated C++20 build/test target. It is not linked to or authoritative over production Rust.
- The original active Upstox decoder/schema remain in the Rust workspace. The predecessor's bitemporal/corporate-action and provenance concepts are reimplemented behind current Python contracts rather than copied wholesale.
- The old challenge monolith, prediction-market integrations, q tickerplant experiments, old deployments, archived measurements and predecessor lockfiles are not promoted into runtime dependencies. On 2026-11-13 the useful subset was trimmed from 325 to 61 reference files and promoted from `legacy/` to `extras/` (India ingestion/durable-store crate, C++ SBE/conformance probes and synthetic fixtures); the full pre-cleanup tree is preserved externally at `../legacy-snapshot-2026-11-13/legacy-full-backup-2026-11-13.tar.gz` (SHA-256 `a49e864308a7e58d667a86049dd80e8ff2c631cd762e50a74aff60007464f9b1`). `extras/**` is the separate `nirnay-extras` workspace, excluded from active builds; its 96 Rust tests and C++ probe pass.

## Verification performed in this delivery environment

| Gate | Result |
|---|---|
| Python tests (`research/python`, FYERS recorder, Kite ops, repository gate tests) | **104 passed, 2 subtests passed** |
| Python `compileall` over research, apps, ops and scripts | **Passed** |
| `python3 scripts/check_repo.py check` | **Passed** in the original 2026-11-12 environment; Cargo was unavailable there and the static manifest fallback was used. The 2026-11-13 rerun used real Cargo metadata. |
| Six JSON Schema definitions (`Draft 2020-12`) | **All schema definitions valid** |
| All repository JSON files | **Parsed successfully** |
| `.github/workflows/ci.yml` and `nightly.yml` YAML parsing | **Passed** |
| C++20 numeric/order-book oracle CMake build | **Passed** |
| C++ oracle CTest | **1/1 passed** |
| Deterministic `quantlab demo` | **Completed**; one synthetic fill; output marks `not_real_market_data=true` and `not_alpha_evidence=true` |
| Synthetic PIT CLI build | **Published 3 rows**, no warnings |
| Synthetic PIT CLI verify | **`verified_internal_integrity`**; no cryptographic source signature or quality/entitlement certification claimed |
| Synthetic alpha-search CLI | **Completed**; 25-candidate family; synthetic fixture only; never promoted as validated alpha |

The C++ oracle was built and tested using CMake/CTest. The Python suite and policy gate were rerun after the final venue-consistency and spread-accounting regression tests.

## Not run / not established

1. **Rust compilation, formatting, Clippy or Rust tests:** Cargo/Rust were not available in the original preparation runtime. In the owner completion rerun (macOS arm64, pinned Rust 1.99.1), the package as first built failed: a stale `Cargo.lock`, a missing `Deserialize` derive and malformed test JSON in `cost-cli`, an undeclared `base16` test import plus a literal NUL byte in `execution-policy`, a duplicated Clippy allow attribute in `paper-engine`, and rustfmt drift. All were corrected; `cargo fmt`, `cargo check`, `cargo clippy -D warnings`, `cargo test` (278 tests), the release `upstox-tools` build, `cargo deny check` and `cargo xtask check` now pass. See `VERIFICATION.md`.
2. **Live provider connectivity:** No Upstox/FYERS credentials, paid-plan entitlements, live sockets or production exchange sessions were used.
3. **Broker execution:** No broker order was submitted. No production Upstox/FYERS/Kite adapter is provided by the new broker-neutral OMS protocol.
4. **Parquet:** resolved on 2026-11-13. PyArrow 26.0.1 is installed; the semantic round-trip and tamper-rejection test pass. Real-data schema evolution, partitioning, row-group sizing and long-run compatibility remain external.
5. **Market-data entitlement and legal provenance:** Synthetic fixture hashes establish test reproducibility only. The source does not independently authenticate exchange/broker data rights, completeness, licensing or legal permission.
6. **Strategy profitability:** No real licensed point-in-time dataset, calibrated fees/impact, capacity analysis, independent holdout or paper/live track record was supplied. No profitability claim is made.
7. **Execution calibration:** L2 queue, latency and market-impact assumptions require venue/account-specific empirical calibration. The current provider normalizers do not produce verified unique aggressor-labelled trade prints for passive fill proof.
8. **Institutional controls:** No HSM/KMS key ceremony, independent risk-service deployment, kill-switch drill, disaster-recovery exercise, broker certification, static-IP proof or regulatory approval is represented as complete.

## Repository-owner verification commands

Use the exact pinned Rust toolchain from `rust-toolchain.toml` (Rust 1.99.1, edition 2024), not an older system package:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo deny check
cargo xtask check
```

Then verify the Python and C++ surfaces:

```bash
python3 -m pip install -e 'research/python[dev,research,security]'
python3 -m pip install -e 'apps/fyers-recorder[test]'
python3 -m pip install -e 'apps/kite-ops[test]'
make python-test
python3 scripts/check_repo.py check
cmake -S verification/cpp-oracle -B /tmp/nirnay-cpp-oracle -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/nirnay-cpp-oracle --parallel
ctest --test-dir /tmp/nirnay-cpp-oracle --output-on-failure
```

Do not enable live mode as a substitute for missing tests, entitlements or approvals. Keep `extras/**` out of active build/runtime graphs. A future deletion proposal should first preserve required provenance outside the deployable repository and pass a dependency/reference scan.

## Final status

This delivery establishes a testable research-to-paper foundation and stricter evidence boundaries. It does **not** establish a production algo-trading firm, complete OMS, certified feed, market-quality guarantee or profitable strategy. Those remain deliberate, named acceptance gates rather than implied by the existence of code.


## Deep systems follow-up — 2026-11-12 (this delivery)

The follow-up strengthens local acceptance and integration coverage without changing Rust source:

- Added root `pytest.ini`. A plain `python3 -m pytest -q` searches only `research/python/tests`, `apps/fyers-recorder/tests`, `apps/kite-ops/tests` and `scripts`; it adds the corresponding `src/` directories to `pythonpath` and excludes `extras/`, `target/`, `.venv` and caches. This prevents the separately built reference workspace from being collected by default.
- Added `research/python/tests/test_quant_pipeline_e2e.py`, a deterministic synthetic integration test that composes canonical Upstox fixture events, bitemporal PIT dataset build/publication/verification, the multi-instrument portfolio event simulator (one provider only), the 25-candidate chronological net-of-cost alpha screen, and durable SQLite OMS paper outbox + broker event replay. The test explicitly checks that fixture-based results retain their non-production/non-alpha labels.
- Added `scripts/validate_contracts.py` and Make targets `python-test`, `python-check`, `schema-check`, `cpp-test` and `verify-local` so the non-Rust gates are reproducible with one command.
- The `AuthorizationUseLedger`, risk evidence gate, AWS KMS Ed25519 adapter, standalone HTTPS/mTLS risk-evidence daemon/client, durable OMS, Kite Keychain/OAuth implementation, Parquet writer/verifier and the six-system readiness matrix are present in the active source tree. Their limits remain as stated in `engineering/production-readiness-matrix.md`.

### Follow-up verification executed in this environment

| Gate | Result |
|---|---|
| Plain root `python3 -m pytest -q` | **156 passed, 0 skipped, 2 subtests passed** |
| Skipped test | None; PyArrow 26.0.1 is installed and the Parquet round-trip executed |
| Python `compileall` (research, apps, scripts, ops) | **PASS** |
| `python3 scripts/check_repo.py check` | **PASS** with real `cargo metadata` under pinned Rust 1.99.1 |
| `python3 scripts/validate_contracts.py` | **PASS — 41 active JSON files, 26 active TOML files, 17 Draft 2020-12 schemas** |
| C++20 CMake configure/build | **PASS** |
| C++ oracle CTest | **PASS — 1/1** |
| Synthetic end-to-end pipeline test | **PASS** as part of the full Python suite |
| Deterministic demo | **PASS** — two byte-identical runs, SHA-256 `2a214de59eee15cbe5d4b7e3c698463d43df48b36076d1bea7e3ba3b47e16084` |
| PIT CLI build/verify | **PASS** — 3 synthetic rows, `verified_internal_integrity` |
| Alpha CLI | **PASS** — 25 candidates, explicitly `not_alpha_certified` |

The `make verify-local` target covers Python tests, compile/policy checks, schema validation and both C++ targets. `make check` adds the root and `extras/` Rust gates. CI runs the same workspaces with Python 3.12 and GCC, and requires PyArrow so the Parquet test cannot silently skip. The 2026-11-13 local rerun used pinned Rust 1.99.1. No real provider session, account entitlement, broker order, AWS KMS key, deployed independent risk service, or authoritative exchange/corporate-action source was exercised here.

### Operational blockers that cannot be closed by repository code alone

1. Install/configure and verify an actual AWS KMS Ed25519 asymmetric key, narrow IAM roles, CloudTrail, private service networking and the separate risk-service workload. Complete issuer/approver/source key ceremonies and revocation/compromise/restore drills.
2. Operate the eight distinct control collectors on independent principals and deploy the risk evaluator outside the OMS process. The Python capability token is an application-layer guard, not isolation.
3. Register official broker order-update streams and implement broker-specific execution adapters with vendor contract conformance. Run disconnect, duplicate, partial-fill, cancel/fill race, unknown-outcome and restart reconciliation drills per broker/account.
4. Obtain licensed authoritative historical/reference/corporate-action datasets and verify delisted/suspended/universe history, correction chains, official calendar and source rights. Synthetic fixtures do not meet this requirement.
5. Calibrate queue, latency, impact, fees/taxes, order rejects and market capacity on genuine event-level data. Aggregate L2 cannot prove FIFO; exact FIFO requires suitable order-by-order data and venue rules.
6. **Resolved in repository on 2026-11-13:** PyArrow 26.0.1 is installed and the semantic Parquet round-trip plus tamper-rejection test pass locally and in the `python-research` CI job. Remaining external work: schema evolution, compression alternatives, row-group sizing and large-file tests against real licensed data on the deployment platform.
7. Run the final Rust format/check/Clippy/test/release gates on the exact extracted ZIP with pinned Rust 1.99.1; the follow-up changes in this delivery do not alter Rust sources, but a previous archive passing is not a substitute for checking the final artifact.

**Release conclusion:** the repository is ready for rigorous local research, deterministic paper simulation, and continued integration engineering. It is not certified for live order routing or institutional production claims. Live order submission remains hard-disabled.

### Relocatable clean-extraction gate — 2026-11-12

The `cpp-test` Make target now uses a repository-path-specific temporary build directory. This fixes CMake cache reuse across the working tree and a fresh extraction. Final package verification is performed against the extracted ZIP, not only the mutable preparation tree.
