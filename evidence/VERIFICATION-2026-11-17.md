# Verification record — Nirnay Quant research-to-paper upgrade

**Prepared:** 2026-11-12  
**Updated:** 2026-11-13 (release-hardening rerun; authoritative section below)  
**Original environment:** Linux x86_64, Python 3.13.5, pytest 9.0.2, CMake 3.31.6, GCC 14.2.0.  
**Scope:** Python research/adapter tests, repository policy checks, CLI smoke tests, point-in-time publication verification, the isolated C++ numeric/order-book oracle, and the separate `extras/` workspace.  
**Not a certification:** This is reproducible local engineering evidence, not broker, exchange, market-data entitlement, legal, profitability or production-readiness certification.

## 2026-11-13 release-hardening rerun (authoritative for this bundle)

**Environment:** macOS 27.0.1 arm64; AppleClang 21.0.0.21000334; Homebrew GCC 16.2.0; CMake 4.4.3; pinned Rust 1.99.1; Python 3.14.8; pytest 9.2.0; PyArrow 26.0.1; clang-format 21.1.8.  
**Revision basis:** no VCS metadata is present in the delivery. The bundle is identified by the adjacent `.sha256` sidecar and the `SOURCE-MANIFEST.sha256` digest recorded in `BUILD-RESULTS.md`.

| Check | Result |
|---|---|
| `cargo fmt` (root and `extras/`) | **PASS** |
| `cargo test --workspace --all-targets --locked` | **PASS — 278 tests, 0 failed** |
| `cargo test --manifest-path extras/Cargo.toml --locked` | **PASS — 96 tests, 0 failed** |
| `cargo clippy -D warnings` (root and `extras/`) | **PASS** |
| Root C++ oracle, AppleClang 21 and GCC 16 | **PASS — 1/1 each** |
| Extras C++ probes, AppleClang 21 and GCC 16 | **PASS — 1/1 each** |
| `python3 -m pytest -q` | **PASS — 156 passed, 0 skipped, 2 subtests** |
| PyArrow Parquet round-trip and semantic tamper rejection | **PASS — PyArrow 26.0.1** |
| Python `compileall` | **PASS** |
| `scripts/check_repo.py check` with Cargo present | **PASS** (no static fallback) |
| `scripts/validate_contracts.py` | **PASS — 42 JSON, 27 TOML, 17 Draft 2020-12 schemas** |
| `clang-format --dry-run --Werror` | **PASS — all 10 C/C++ files** |
| Artifact hashes | **PASS — 8/8 against `artifacts/SHA256SUMS`** |
| `scripts/supply_chain.py pins` | **PASS — 43 action references pinned to commit SHAs** (30 at r2, 13 added with the packaging, macOS and release jobs) |
| `scripts/supply_chain.py constraints` | **PASS — 3 constraint sets, 79 pins, full declared coverage** |
| `scripts/supply_chain.py sbom` | **PASS — 342 components; byte-identical across two runs** |
| `pip-audit` via `scripts/supply_chain.py audit` | **PASS — research 0, kite-ops 0, fyers-recorder 51 waived (dated waivers)** |
| Installed Kite wheel smoke test | **PASS — `keychain.swift` present, `pip check` clean, CLI entry point runs** |
| Deterministic `quantlab demo` | **PASS — two runs, identical SHA-256 `2a214de59eee15cbe5d4b7e3c698463d43df48b36076d1bea7e3ba3b47e16084`** |

CI now runs the root Rust workspace, the separate `extras/` Rust workspace, both C++ targets under GCC, the pinned C++ formatter, schema validation, the required Parquet test, and isolated Kite/FYERS environments. The `provider-boundary` canary fails if the documented `fyers-apiv3`/`requests` conflict silently disappears. The hosted run is not claimed here until it executes on a pushed commit; the table records the equivalent commands run locally.

## 2026-11-13 r3 — release identity, packaging and provenance gates

An independent review of the r2 archive found that the build report quoted a stale ZIP hash, and that the archive stored `SBOM.cdx.json` while `SOURCE-MANIFEST.sha256` listed `sbom.cdx.json`. The second defect was invisible here because APFS is case-insensitive and `shasum -c` resolved the name; it would have failed on the Ubuntu runner.

Both are fixed structurally, and both fixes have a negative control showing the gate can fail.

| Check | Command | Result |
|---|---|---|
| Wheel contents and clean installs | `python3 scripts/check_wheels.py --dist dist` | **PASS — 3 wheels, 0 cache entries, each installed and ran `--help` in a fresh venv** |
| Negative control, dirty wheel | same, with a `__pycache__` member injected | **FAILS as intended — 2 named errors** |
| Bundle assembly | `python3 scripts/make_bundle.py --source-dir . --artifacts ../artifacts --output …r3.zip` | **PASS — manifest and archive written from one file list** |
| Case-sensitive bundle verification | `python3 scripts/verify_bundle.py --bundle <tree> --zip <archive>` | **PASS — every manifest path, archive member and member hash** |
| Negative control, r2 archive | same verifier, r2 inputs | **FAILS as intended — 4 named errors, incl. `sbom.cdx.json` vs `SBOM.cdx.json`** |
| Swift helper | `xcrun swiftc -typecheck` and `-O` | **PASS — Swift 6.4, Mach-O arm64** |
| Workflow syntax and inventory | YAML parse | **PASS — `ci.yml` 9 jobs, `nightly.yml` 5, `release.yml` 2** |
| Action pinning across every workflow | `python3 scripts/supply_chain.py pins` | **PASS — 43 references** |

The release workflow (`release.yml`, `v*` tags) rebuilds wheels and binaries from the tagged commit, re-verifies the bundle, and attests assets with `actions/attest-build-provenance@4d101475…` (v4.2.2). No hosted run and no attestation exists yet.

## 2026-11-13 r0 — executable modes and release gating

Two defects survived r3: the bundler wrote a constant `0o644` for every archive member, so all five compiled binaries extracted non-executable; and `release.yml` ran independently of `ci.yml`, so a tag could publish while the gates were red, with both release jobs racing to create the same release.

Executable bits now come from the staged file (`0o755` when any execute bit is set), the verifier rejects ELF/Mach-O payloads stored without one, and `--self-test` clears a bit in a copy of the archive and requires the check to fail. `ci.yml` gained `workflow_call`; `release.yml` opens with a `quality` job that calls it, `bundle` needs `quality`, and `macos-assets` needs `bundle`.

| Check | Command | Result |
|---|---|---|
| Executable members preserved | `make_bundle.py`, then archive mode dump | **PASS — 7 members at `0o755`, `unzip` restores them executable** |
| Bundle verification, modes included | `python3 scripts/verify_bundle.py --bundle <tree> --zip <archive>` | **PASS — 343 entries, 345 members, per-member hashes, modes** |
| Negative control, cleared bit | same verifier, `--self-test` | **PASS — clearing the bit fails verification** |
| Negative control, binary `chmod 644` before bundling | build and verify | **FAILS as intended — 2 named errors** |
| Negative control, extracted file loses its bit | verifier on the extracted tree | **FAILS as intended — 1 named error** |
| Release chain | YAML parse of `release.yml` | **PASS — `quality` → `bundle` → `macos-assets`** |

Workflow inventory: `ci.yml` 9 job definitions (10 executions), `nightly.yml` 5, `release.yml` 3; 43 remote action references pinned.

The remaining tables below are retained as historical records for 2026-11-12. Their counts, environment notes and Parquet skip are superseded by this section.

## Executed checks — original 2026-11-12 Linux environment (historical)

| Check | Result | Qualification |
|---|---|---|
| `PYTHONPATH=research/python/src:apps/fyers-recorder/src:apps/kite-ops/src:scripts python3 -m pytest -q research/python/tests apps/fyers-recorder/tests apps/kite-ops/tests scripts/test_check_repo.py` | **PASS — 104 passed, 2 subtests passed** | Local fixtures and fake SDK/HTTP clients; no provider account was accessed. |
| `python3 -m compileall -q research/python/src apps/fyers-recorder/src apps/kite-ops/src ops scripts` | **PASS** | Python syntax/bytecode compilation. |
| `python3 scripts/check_repo.py check` | **PASS** | The tool reported Cargo unavailable and used its documented static Cargo-manifest dependency-graph fallback. It did not run `cargo metadata`. |
| CMake configure/build for `verification/cpp-oracle` | **PASS** | C++20 verification-only target, GCC 14.2.0, `-Wall -Wextra -Werror`. |
| `ctest --test-dir /tmp/nirnay-cpp-oracle --output-on-failure` | **PASS — 1/1** | One deterministic numeric/order-book oracle executable. This is not a Rust/C++ differential golden-corpus run. |
| `quantlab demo` | **PASS** | Deterministic synthetic EMA-cross/backtest path; output declares `not_real_market_data=true` and `not_alpha_evidence=true`. |
| PIT dataset CLI `build-dataset` + `verify-dataset` | **PASS — 3 synthetic rows** | JSONL dataset published and internal artifact/data digests recomputed. Verifier reports `cryptographic_signature=false` and `quality_or_entitlement_certified=false`. |
| `quantlab alpha-search` on checked-in synthetic CSV | **PASS — 25-candidate screen completed** | Output status is `positive_holdout_screen_passed_not_alpha_certified`; all input data are synthetic. Not profitability evidence. |
| `quantlab --help` and FYERS recorder CLI help | **PASS** | CLI parser smoke tests only. |
| JSON Schema validation | **PASS — 6 schemas** | All six active `*.schema.json` documents pass Draft 2020-12 schema self-validation. |
| Deterministic demo reproducibility | **PASS** | Two independent demo runs produced byte-identical JSON output (SHA-256 `2a214de59eee15cbe5d4b7e3c698463d43df48b36076d1bea7e3ba3b47e16084`). |
| Repository data/config parsing | **PASS** | 121 JSON files and 45 TOML files parsed successfully. |
| Optional Parquet writer | **NOT RUN** | PyArrow is not installed in this environment; JSONL publication/verification passed. Install `research` extras to exercise Parquet. |
| Rust formatting/check/Clippy/tests/release build | **PASS in owner rerun** | The package as first built failed these gates. Defects were corrected and the gates now pass; see the owner completion rerun below. |
| Live Upstox/FYERS/Kite connection, order, fill or account reconciliation | **NOT RUN** | No live credentials or provider account calls were used. |

## Owner completion rerun — 2026-11-12

**Environment:** macOS 27.0.1 arm64, AppleClang 21.0.0.21000334, CMake 4.4.3, pinned Rust 1.99.1, Python 3.14.8, pytest 9.2.0, `cryptography` 50.0.2, `cargo-deny` 0.20.2.

The delivered ZIP was first audited as received. ZIP integrity passed, but the source tree did **not** pass the announced Rust gates. The following defects were found and corrected before the final package was rebuilt:

1. `Cargo.lock` was stale: both `cargo metadata --locked` and `cargo check --locked` refused it. The lock file was regenerated from the pinned manifests.
2. `apps/cost-cli/src/lib.rs` failed `--all-targets` compilation because the test deserialized `ChargesOutput` without a `Deserialize` derive. Its test inputs also omitted the opening quote on `turnover_paise`, so the cost-model test failed at runtime when the suite was finally executed.
3. `crates/execution-policy/tests/execution_policy.rs` imported the undeclared `base16` crate and contained a literal NUL byte inside a source string (line 588). The import was replaced with a local test-only hex helper and the byte was converted to the `\x00` escape.
4. `crates/paper-engine/tests/venue_matching.rs` duplicated a crate-level `#![allow(...)]` attribute, which fails Clippy under `-D warnings`.
5. Rustfmt drift existed across 12 further source files; `cargo fmt --all` normalized them.

These defects are consistent with the original delivery never having compiled the Rust test targets or run `cargo test`.

| Owner gate | Result |
|---|---|
| `cargo fmt --all -- --check` | **PASS** |
| `cargo check --workspace --all-targets --locked` | **PASS** |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | **PASS** |
| `cargo test --workspace --all-targets --locked` | **PASS — 278 tests, 0 failed** |
| `cargo build --release -p upstox-tools --bins --locked` | **PASS** |
| `cargo xtask check` | **PASS** (`PASS: check repository gates`) |
| `cargo deny check` | **PASS** (`advisories ok, bans ok, licenses ok, sources ok`; duplicate-crate warnings only) |
| Python suite | **PASS — 104 passed, 2 subtests passed** |
| `python3 scripts/check_repo.py check` | **PASS** with real `cargo metadata` (no static fallback) |
| C++20 CMake build + CTest | **PASS — 1/1**, CMake 4.4.3 / AppleClang 21 |
| JSON Schema self-validation | **PASS — 6 Draft 2020-12 schemas** |
| Deterministic demo | **PASS** — byte-identical, SHA-256 `2a214de59eee15cbe5d4b7e3c698463d43df48b36076d1bea7e3ba3b47e16084` |
| PIT `build-dataset` + `verify-dataset` | **PASS — 3 synthetic rows**, `verified_internal_integrity` |
| Alpha screen | **PASS — 25 candidates over 420 synthetic observations**; status `positive_holdout_screen_passed_not_alpha_certified` |

The 102/104 difference in earlier reports is explained by the repository-gate
tests: `scripts/test_check_repo.py` contributes 2 tests. The
application/research suites contribute 102; the full command documented in
this file and `Makefile` contributes 104.
contributes 104.

The final package was rebuilt from the corrected tree and re-verified from a fresh extraction. Its SHA-256 is recorded in the adjacent `SHA256SUMS` file. No live provider session, broker order, entitlement check or Parquet run was added by this rerun.

## Current capability and evidence boundaries

### Point-in-time data

The active Python data path includes bitemporal listing and universe membership, immutable corporate-action revisions, source IDs/retrieval times/hashes, exact split/bonus ratios, separate adjustment views, immutable JSONL publication and a verifier that recomputes internal lineage digests. The sample fixture demonstrates the workflow only. It is not an official exchange/security master or complete corporate-action history. Dividends are cash-flow events; unsupported complex action types fail closed. Delistings, symbol changes, mergers/demergers, rights issues, taxes and full exchange adjustment policy still require authoritative effective-dated data and approved transformations before broad production research.

### Providers

- **Upstox:** current official V3 docs describe Protobuf WebSocket frames, binary subscription requests, mode-specific limits and `market_info`/snapshot/live-feed sequencing. The active raw recorder keeps the original frame and local receive sequence. The decode-export utility is Rust and remains owner-build verification. The active normalizer does not claim Full D30 support or a complete closing-auction/pre-open state model. It preserves `currentTs` separately from per-instrument `ltpc.ltt` and does not infer aggressor side.
- **FYERS:** current official docs distinguish Data Socket (quote/index/five-level depth) from the separate Protobuf TBT socket (50-level NSE equity/NFO depth). Standard/Prime Data Socket ceilings are documented as 100/5,000 symbols per user; TBT has separate limits. The current recorder is Data Socket only. `--max-symbols` defaults to 100 and is an operator-declared bound, not remote entitlement verification. The journal captures decoded SDK callbacks, not raw wire packets; its sequence is local, and its manifest remains `research_grade=false`.
- **Cross-feed reconciliation:** canonical normalizers preserve provider identity and timestamp semantics. Reconciliation is a diagnostic comparison only; provider sizes are not summed and one provider must be selected for replay. Neither feed is independently certified as lossless or complete by this verification run.

### Backtesting and alpha

The event-driven simulator is a deterministic single-instrument, long-only research model with configured latency, participation limits, partial fills, conservative aggregate-L2 queue-ahead approximation, explicit aggressor-side trade fixtures for passive fills, fees and modeled taker impact. It is not FIFO/exchange matching. Real vendor LTP changes are observations, not aggressor-tagged unique prints; they cannot consume the passive queue. Impact and queue parameters are not calibrated to venue data. Portfolio accounting, broad product/derivative semantics, exchange auction rules, rejects, settlement, borrow, financing and broker routing are not certified.

The alpha pipeline searches a finite, declared 25-candidate family. It applies chronological train/validation/untouched-holdout splits, embargo, flat-start/flat-end segment portfolios, declared bps costs and Holm/HAC-style screening. Its input fixture is synthetic and close-based; the result cannot be interpreted as evidence of alpha. It is not unrestricted arbitrary code generation. Strategy selection on licensed point-in-time data and full event-level execution remains required.

### OMS and risk authorization

`quantlab.oms` is a broker-neutral lifecycle reference state machine with durable hash-chain journaling, idempotency, partial fills, update sequencing, cancellation intent and unknown-submit handling. `quantlab.live_auth` verifies a root-signed trust registry, independent issuer/approver roles, an independent risk-service signature over exact runtime evidence, time/scope/limit constraints and one-time permit consumption. These are authorization primitives, not a production OMS. No broker adapter implementation is certified and **live routing is hard-disabled even when a test permit verifies**. Do not enable a live path without separate broker certification, KMS/HSM integration, operational risk service, independent controls, regulatory review and tested recovery/reconciliation.

### Extras migration

The active workspace does not compile or import the predecessor monolith. On 2026-11-13 the useful subset was reduced from 325 to 61 files and promoted from `legacy/` to `extras/` as the separate `nirnay-extras` workspace: the India ingestion/durable-store crate and its two test suites, the C++ SBE/conformance probes, synthetic fixtures and challenge/internal schemas. The full pre-cleanup tree is preserved externally at `../legacy-snapshot-2026-11-13/legacy-full-backup-2026-11-13.tar.gz` (SHA-256 `a49e864308a7e58d667a86049dd80e8ff2c631cd762e50a74aff60007464f9b1`), with the removed-file manifest at `../legacy-snapshot-2026-11-13/legacy-removed-manifest-2026-11-13.txt`. The extras crate passes 96 tests against the active `india-domain`; the C++ probes pass 1/1. Selected concepts were reimplemented behind current contracts; the C++ numeric/order-book oracle remains isolated under `verification/cpp-oracle`. See `engineering/extras-promotion-audit.md`, `engineering/legacy-audit-2026-11-13.md` and `engineering/migration-map.csv`. Promotion of the retained India code into the active workspace still requires a new crate, ADR, owner and tests.

## Current official API references checked on 2026-11-12

- Upstox Market Data Feed V3: <https://upstox.com/developer/api-documentation/v3/get-market-data-feed/>. The page documents Protobuf transport, binary subscription requests, feed modes, individual/combined limits, market status/snapshot/live messages and CAS fields.
- FYERS WebSocket overview: <https://fyers.in/api-docs/v3/api/websockets/websocket-overview/>. The page was last updated 2026-10-14 and distinguishes Data Socket, TBT and General WebSocket; it lists the relevant Standard/Prime subscription limits.
- FYERS Data Socket reference: <https://fyers.in/api-docs/v3/api/websockets/data-socket/>.

Published API ceilings do not prove an account's entitlement. Check the account contract/plan and applicable exchange/data licences before recording or using data.

## Owner-run Rust gates

With the repository's pinned Rust 1.99.1 toolchain and components installed, run from the repository root:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo build --release -p upstox-tools --bins --locked
cargo deny check
cargo xtask check
```

Do not report a Rust gate as passed until these exact commands pass on the delivered commit. Keep all live execution controls disabled during initial build and provider-capture validation.


## Deep Python/research follow-up — 2026-11-12 (historical)

> Superseded by the 2026-11-13 section above for test counts, Cargo availability and Parquet status.

After the owner Rust verification recorded above, a Python-only systems follow-up added a root pytest configuration, a synthetic research-to-paper integration test, contract validation script and `make verify-local` target. Plain root test execution now excludes `extras/` by default and resolves active source packages without an environment-specific `PYTHONPATH`.

Most recent checks from the follow-up environment:

- `python3 -m pytest -q`: **155 passed, 1 skipped, 2 subtests passed**. The one skip is the optional Parquet round-trip because PyArrow is absent.
- Python `compileall`: passed for research, FYERS, Kite, scripts and ops.
- `python3 scripts/check_repo.py check`: passed using static Cargo manifest graph validation because Cargo is unavailable in this runtime.
- `python3 scripts/validate_contracts.py`: 40 active JSON files parsed, 25 TOML files parsed and 17 Draft 2020-12 schema definitions self-validated.
- C++20 numeric/order-book oracle reconfigured and rebuilt; CTest passed 1/1.
- The new integration test composed synthetic PIT dataset publication/verification, one-provider portfolio replay, the 25-candidate net-of-cost alpha screen and durable OMS paper outbox/broker-event processing.

These are local synthetic/structural checks. The 2026-11-13 rerun installed PyArrow and executed the Parquet semantics; actual broker APIs and exchange sessions, AWS KMS provisioning, separate risk-service deployment and production operational controls remain unverified. Rust was rerun on this tree with pinned 1.99.1 and is now a CI gate. Live routing remains hard-disabled.

### Relocatable package verification correction — 2026-11-12

A clean-extraction run initially found that `make cpp-test` reused a fixed `/tmp/nirnay-cpp-oracle` CMake cache created from the original source path. The Makefile now derives `CPP_BUILD_DIR` from the current absolute repository path, so independent worktrees and freshly extracted archives cannot accidentally reuse another source tree's CMake cache. The `make verify-local` result above was rerun after this fix. The final archive is also verified by extracting it into a new directory and rerunning `make verify-local` there.

## 2026-11-17 post-merge verification (authoritative for `f832789`)

**Revision verified:** `f8327892c373931e097b52893f4f2395bdc5fb57` (PR #7 sweep remediation, squash merge).
**Hosted CI:** run `44019753876` on `main` — 10/10 jobs SUCCESS (`rust`, `extras-rust`, `cpp`, `macos`, `python-research`, `python-provider (fyers)`, `python-provider (kite)`, `packaging`, `provider-boundary`, `supply-chain`).
**Local rerun (macOS arm64, pinned Rust 1.99.1):** `make check` exit 0; root Rust 278 tests; extras Rust 92 passed, 0 failed, 4 ignored; Python 156 passed, 2 subtests; schemas, C++ builds/tests/format, supply-chain pins/constraints/SBOM/pip-audit, clean wheel builds/installs, bundle verification with negative self-test, Swift keychain typecheck/compile, and Git policy all PASSED.
**Extras count semantics:** the earlier `96 tests` figure is the pre-sweep 2026-11-13 total. The current invocation reports **92 passed, 0 failed, 4 ignored**; the four ignored items are child-process-only workers invoked explicitly by parent tests with `--ignored`. Historical totals above remain records of their dated runs and are not current results.
**Scope of this evidence:** engineering verification for the named commit only. It is not platform-readiness certification, M1 exit-gate approval, M2 implementation authorization, broker/KMS/provider/Keychain runtime verification, or live-trading authorization.
