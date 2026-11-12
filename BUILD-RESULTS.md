# Build and compile results — r0

**Bundle:** `nirnay-quant-production-upgrade-2026-11-13-r0.zip`
**Bundle SHA-256:** published as `nirnay-quant-production-upgrade-2026-11-13-r0.zip.sha256` beside the archive, written by `scripts/make_bundle.py` from the archive itself. A file cannot contain its own archive hash, so this document does not quote one.
**Contents:** `nirnay-quant/`, `artifacts/`, `sbom.cdx.json`, `BUILD-RESULTS.md`, `SOURCE-MANIFEST.sha256`
**Source manifest:** `SOURCE-MANIFEST.sha256`, 329 file hashes over `nirnay-quant/`, `artifacts/` and `sbom.cdx.json`; digest `9665538e8ee8cd345643b09b269702aeb2c0018af07723699d55ef5e47ef2fc0`
**Date:** 2026-11-13
**Environment:** macOS 27.0.1 arm64; AppleClang 21.0.0.21000334; Apple Swift 6.4; Homebrew GCC 16.2.0; CMake 4.4.3; pinned Rust 1.99.1; Python 3.14.8; pytest 9.2.0; PyArrow 26.0.1; clang-format 21.1.8; uv 0.11.27.

## Identity and history

The project was developed locally through working labels r0, r1, r2 and r3. Those labels named local archives under review; they were never published as commits, and no attempt is made here to present them as history. What exists publicly is one commit on `main` that contains the final state of that work, identified as **r0** because it is the first published release identity.

Published identity is the commit, not the label:

| What | Where |
|---|---|
| Source of this release | `main`, single commit `feat: initial import of nirnay r0` |
| Bundle hash | the `.sha256` sidecar beside the archive |
| Manifest digest | quoted above, and outside the manifest so it can be true |
| Branch, merge and attribution rules | `engineering/git-policy.json`, `engineering/git-workflow.md` |

This record supersedes the r3 archive `c93b608c…` that closed findings A and B, and the r2 archive `11778a60…` before it. Those hashes identify reviewed local archives, not published history; results recorded against them are not evidence about this bundle.

Findings A and B were closed in that reviewed archive and are carried here unchanged. Findings C and D are the reason r0 differs from it.

## Findings closed

### A — the quoted archive hash was stale

An earlier build report quoted `1bc7d4d3…`, which belonged to a superseded archive of the same tree; the archive itself was `11778a60…` and its sidecar agreed with it.

**Fixed by construction:** `scripts/make_bundle.py` hashes the finished archive and writes the sidecar itself, and this document never restates that value. One writer of the archive hash, next to the archive.

### B — `SBOM.cdx.json` versus `sbom.cdx.json`

The r2 archive stored `SBOM.cdx.json` at its root while the manifest listed `sbom.cdx.json`. APFS is case-insensitive, so `shasum -c` resolved the name; a case-sensitive runner would have failed one of 321 entries.

**Fixed by construction:** the root SBOM is `sbom.cdx.json`, the manifest and the archive are written from the same file list, and `scripts/verify_bundle.py` compares paths and member names as exact strings.

### C — executable permissions were discarded

`make_bundle.py` applied one constant mode, `0o644 << 16`, to every archive member. Five compiled binaries shipped, and every one extracted non-executable:

| File | Archived mode before | Archived mode in r0 |
|---|---|---|
| `artifacts/nirnay-keychain-helper` | 644 | **755** |
| `artifacts/nirnay_numeric_oracle_test` | 644 | **755** |
| `artifacts/upstox-decode-export` | 644 | **755** |
| `artifacts/upstox-recorder` | 644 | **755** |
| `artifacts/upstox-smoke` | 644 | **755** |

**Fixed at the source of the metadata:** `archive_mode()` reads the staged file's own mode and returns `0o755` when any execute bit is set, `0o644` otherwise; the member is written as `(stat.S_IFREG | mode) << 16`. Seven members carry `0o755` — the five binaries plus `ops/market_session.py` and `scripts/validate_contracts.py`, which are executable in the source tree — and `unzip` restores them executable.

**Gated:** `verify_bundle.py` refuses an archive member or an extracted file whose bytes are ELF or Mach-O but whose mode lacks an execute bit, and reports the mode it found. `--self-test` clears one execute bit in a copy of the archive and requires verification to fail, so the check ships with its own negative control.

### C, continued — the bundle shipped build caches

Auditing the archive against the commit found 18 files the commit does not contain: `*.egg-info` metadata left behind by editable installs in all three Python packages. `make_bundle.py` excluded a literal `build/` directory but nothing that merely ends in `build`, and no packaging metadata at all.

**Fixed:** any path component ending in `.egg-info` is excluded from the bundle, and `verify_bundle.py` now refuses build-cache components in both the archive and the extracted tree — `__pycache__`, `.pytest_cache`, `.ruff_cache`, `.mypy_cache`, `.cache`, `.venv`, `venv`, `target`, `build`, `dist`, `node_modules`, `.git`, plus `*.egg-info`, `*.pyc` and `*.pyo`. The pre-fix archive is the negative control: it fails with 36 named errors, 18 in the archive and 18 in the extracted tree.

**Source modes too:** importing the source out of an archive whose members were all `0o644` recorded `100755` files as `100644` in Git. Both affected files are corrected in the r0 commit.

### D — release publication could bypass CI

`ci.yml` ran on push and pull request; `release.yml` ran independently on `v*` tags. A tag could therefore publish artifacts whose tests had failed, and the two release jobs both called `gh release create`, so they could race on the same release.

**Fixed:** `ci.yml` declares `workflow_call`, making it a reusable workflow, and `release.yml` opens with a `quality` job that calls it. `bundle` needs `quality`, `macos-assets` needs `bundle`:

```
tag
 └─ quality (ci.yml: rust, extras-rust, cpp, python-research, python-provider ×2,
             provider-boundary, supply-chain, packaging, macos)
     └─ bundle (linux: wheels, binaries, SBOM, bundle, verify, attest, publish)
         └─ macos-assets (arm64: binaries, swift helper, attest, publish)
```

Publication is downstream of the same ten checks that guard a pull request, and the two jobs that touch the release are serialised. `actions/attest-build-provenance` remains pinned at `4d101475…` (v4.2.2); the reusable-workflow reference is local (`./.github/workflows/ci.yml`) and is not a remote action to pin — 43 remote action references remain SHA-pinned.

| Gate | Result |
|---|---|
| `scripts/check_wheels.py --dist dist` | **PASS — 3 wheels, zero cache entries, each installed and ran its console script in a fresh venv** |
| Negative control: a `__pycache__` member injected into a wheel | **FAILS as intended — 2 named errors** |
| `scripts/verify_bundle.py --bundle … --zip …` | **PASS — 329 manifest entries, members, per-member hashes, executable modes** |
| `scripts/verify_bundle.py … --self-test` | **PASS — clearing an execute bit fails verification** |
| Negative control: one binary `chmod 644` before bundling | **FAILS as intended — 2 named errors**, archive and extracted mode |
| Negative control: verifier against the r2 archive | **FAILS as intended — 4 named errors**, including `sbom.cdx.json` versus `SBOM.cdx.json` |
| Negative control: pre-fix archive containing `*.egg-info` | **FAILS as intended — 36 named errors**, cache paths in archive and tree |
| `scripts/make_bundle.py` run twice on identical inputs | **PASS — identical archive SHA-256** |
| Workflow graph | **PASS — `quality` → `bundle` → `macos-assets`; `ci.yml` accepts `workflow_call`** |
| `python3 scripts/check_git_policy.py --strict` | **PASS — branch, subjects, tracked paths, inventory** |
| `xcrun swiftc -typecheck` and `-O` on the bundled Keychain helper | **PASS — Swift 6.4, Mach-O arm64** |

## 2026-11-13 verification

Every row was executed against this tree on 2026-11-13, in this environment. Counts are the actual output of the command shown.

| Gate | Command | Result |
|---|---|---|
| Rust format, root | `cargo fmt --all -- --check` | **PASS** |
| Rust format, extras | `cargo fmt --manifest-path extras/Cargo.toml -- --check` | **PASS** |
| Rust compile, root | `cargo check --workspace --all-targets --locked` | **PASS** |
| Rust compile, extras | `cargo check --manifest-path extras/Cargo.toml --all-targets --locked` | **PASS** |
| Rust tests, root | `cargo test --workspace --all-targets --locked` | **PASS — 278 tests, 0 failed, 0 ignored** |
| Rust tests, extras | `cargo test --manifest-path extras/Cargo.toml --locked` | **PASS — 96 tests, 0 failed** (5 unit, 42 ingestion, 49 durable-store) |
| Clippy, root | `cargo clippy --workspace --all-targets --locked -- -D warnings` | **PASS** |
| Clippy, extras | `cargo clippy --manifest-path extras/Cargo.toml --all-targets --locked -- -D warnings` | **PASS** |
| Repository gates | `cargo xtask check` | **PASS** |
| Dependency policy | `cargo deny check` | **PASS** — advisories, bans, licences, sources |
| Repository policy | `python3 scripts/check_repo.py check` | **PASS** with real `cargo metadata`; no static fallback |
| Schema validation | `python3 scripts/validate_contracts.py` | **PASS — 44 active JSON, 27 active TOML, 17 Draft 2020-12 schemas** |
| Python suite | `python3 -m pytest -q` | **PASS — 156 passed, 0 skipped, 2 subtests** (research environment plus `requests`, the Kite runtime dependency that `pytest.ini`'s app collection requires; run against the extracted bundle as well as the working tree) |
| Parquet round-trip | `python3 -m pytest -q research/python/tests/test_market_pipeline.py -k parquet` | **PASS** — executed, not skipped; PyArrow 26.0.1 |
| Python compile | `python3 -m compileall -q research/python apps scripts ops` | **PASS** |
| Research env from constraints | `pip install -r constraints/python-research.txt` + `pip install --no-deps -e research/python`; `pip check` | **PASS — `pip check` clean** |
| Kite isolated environment | `pip install -r constraints/python-kite-ops.txt` + `--no-deps -e 'apps/kite-ops[test]'`; `pip check`; `pytest -q apps/kite-ops/tests` | **PASS — 21 tests, `pip check` clean** |
| FYERS isolated environment | `pip install -r constraints/python-fyers-recorder.txt` + `--no-deps -e 'apps/fyers-recorder[test]'`; `pip check`; `pytest -q apps/fyers-recorder/tests` | **PASS — 7 tests, `pip check` clean** |
| Provider boundary canary | `pip install --dry-run -e apps/fyers-recorder -e apps/kite-ops` in a clean venv | **PASS (expected failure)** — exit 1, `ResolutionImpossible`, 33 `requests` mentions |
| Wheel build and install | `scripts/check_wheels.py --dist dist` | **PASS — 3 wheels, 0 cache entries, 3 clean-venv installs, 3 CLI smoke runs** |
| C++ oracle, AppleClang 21 | CMake configure + build + `ctest` | **PASS — 1/1** |
| C++ oracle, GCC 16.2.0 | CMake configure + build + `ctest` | **PASS — 1/1** |
| Extras C++ probes, AppleClang 21 | CMake configure + build + `ctest` | **PASS — 1/1** |
| Extras C++ probes, GCC 16.2.0 | CMake configure + build + `ctest` | **PASS — 1/1** |
| C/C++ format | `clang-format 21.1.8 --style=file --dry-run --Werror` | **PASS — all 10 C/C++ files** |
| Action pinning | `python3 scripts/supply_chain.py pins` | **PASS — 43 remote references across three workflows** |
| Python constraint coverage | `python3 scripts/supply_chain.py constraints` | **PASS — 3 files, 79 exact pins, full declared coverage** |
| SBOM | `python3 scripts/supply_chain.py sbom` | **PASS — 342 components; byte-identical across runs, sha256 `e0a199e6…`** |
| Advisories, research | `python3 scripts/supply_chain.py audit research` | **PASS — 0 advisories** |
| Advisories, Kite | `python3 scripts/supply_chain.py audit kite-ops` | **PASS — 0 advisories** |
| Advisories, FYERS | `python3 scripts/supply_chain.py audit fyers-recorder` | **PASS — 51 inherited advisories waived, 0 undocumented** |
| Workflow syntax and inventory | YAML parse | **PASS — `ci.yml` 9 jobs, `nightly.yml` 5 jobs, `release.yml` 3 jobs** |
| Git policy | `python3 scripts/check_git_policy.py --strict` | **PASS** |
| Swift helper | `xcrun swiftc -typecheck` and `-O` | **PASS — Swift 6.4, Mach-O arm64** |
| Artifact hashes | `shasum -a 256 -c artifacts/SHA256SUMS` | **PASS — 8/8** |
| Deterministic demo | `quantlab demo` twice | **PASS — identical SHA-256 `2a214de59eee15cbe5d4b7e3c698463d43df48b36076d1bea7e3ba3b47e16084`** |
| Bundle identity | `scripts/verify_bundle.py --bundle … --zip … --self-test` | **PASS — 329 manifest entries, 331 archive members, modes, self-test** |
| Archive integrity | `unzip -t` | **PASS** |

**Composition:** 319 files inside `nirnay-quant/`, 9 in `artifacts/`, and `sbom.cdx.json` at the bundle root — 329 manifest entries and 331 archive members, the two extra being `BUILD-RESULTS.md` and the manifest itself. Every one of the 319 is a file the commit contains (320 minus `BUILD-RESULTS.md`, which ships at the bundle root instead). Seven archive members are `0o755`. Neither record file is inside the manifest, so this document can quote the digest above without invalidating it. The archive is reproducible: two runs on identical inputs produce the same SHA-256.

### Compiler contract

The shared exact-numeric/order-book sources use `__int128` for checked intermediate arithmetic. That is a GCC/Clang 64-bit extension, not ISO C++. The contract is explicit and enforced at configure time in both `verification/cpp-oracle/CMakeLists.txt` and `extras/cpp/CMakeLists.txt`: a 64-bit target is required, `__int128` support is probed with `check_cxx_source_compiles`, and configure fails with a named message otherwise. `extras/cpp` splits `eng_oracle` (shared sources, `-Wall -Wextra -Werror`, no `-Wpedantic`) from `eng_sbe` (ISO-clean, keeps `-Wpedantic -Werror`). All four configure/build/test combinations pass locally under AppleClang 21 and GCC 16.2.0; the `macos` CI job repeats the AppleClang pair on arm64.

### CI coverage

`ci.yml` runs nine job definitions producing ten job executions, `nightly.yml` five, `release.yml` three (a call to `ci.yml`, then the Linux bundle, then the macOS assets).

| Job | Workflow | Coverage |
|---|---|---|
| `rust` | ci, nightly | root workspace: fmt, check, xtask, deny, tests, clippy |
| `extras-rust` / `extras` | ci, nightly | separate `extras/` workspace |
| `cpp` | ci, nightly | pinned `clang-format 21.1.8`; root oracle and extras probes under GCC |
| `python-research` | ci, nightly | constraints, `pip-audit`, mandatory PyArrow, Parquet round-trip, tests, compileall, repository policy, schemas |
| `python-provider (kite)` / `(fyers)` | ci, nightly | matrix of isolated Kite and FYERS environments, audited, `pip check`, tests |
| `provider-boundary` | ci, nightly | canary that fails if the documented provider conflict silently resolves |
| `supply-chain` | ci, nightly | action pinning, constraint coverage, SBOM, digest |
| `packaging` | ci | clean wheel rebuild, cache rejection, clean-environment install, console-script smoke |
| `macos` | ci | Apple silicon Rust, AppleClang for both C++ targets, Swift typecheck and compile, macOS wheel build and install |
| `quality` | release | calls `ci.yml`; nothing publishes until it passes |
| `bundle` | release | rebuilds wheels and binaries from the tag, assembles and verifies the bundle, attests provenance, publishes assets |
| `macos-assets` | release | macOS arm64 binaries and Keychain helper, attested and published with platform-suffixed names |

The ten check names are recorded exactly as GitHub reports them, which is what a required-checks rule must contain. Hosted evidence for the tagged commit is recorded in `VERIFICATION.md` and linked from the release page; the local table above is the equivalent run of the same commands.

### Dependency isolation

`fyers-apiv3 3.2.1` pins `requests==2.31.0`, while `nirnay-kite-ops` requires `requests>=2.32,<3`. Neither bound was relaxed. The providers ship in separate environments and the combined boundary is tested as an expected failure. See `engineering/provider-environments.md`.

### Supply chain

All remote workflow actions are pinned to 40-character commit SHAs with `# vX` provenance comments — 43 references across `ci.yml`, `nightly.yml` and `release.yml`, including `actions/attest-build-provenance@4d101475…` (v4.2.2). Constraint sets: 37 pins for research, 10 for Kite, 32 for FYERS, compiled with `uv` against the Python 3.12 target CI uses. `pip-audit` gates every Python job with dated, owner-assigned waivers. `sbom.cdx.json` is a deterministic CycloneDX 1.5 document built from `Cargo.lock` plus the constraint sets. Full description: `engineering/supply-chain.md`.

**Accepted residual risk:** the FYERS environment reports 51 advisories against `aiohttp` (3.9.3), `requests` (2.31.0) and `setuptools` (68.0.0), all pinned by `fyers-apiv3 3.2.1`. They cannot be fixed inside this repository without breaking the provider contract. The waiver expires 2027-02-28; until FYERS publishes relaxed pins, the recorder is a research capture path with no order-routing authority.

### Packaging

Three wheels, built from a cleaned tree with `include-package-data = false`, carrying no `__pycache__`, `.pyc`, `build/` or `.egg-info` entries, each installed into a fresh virtual environment and executed there. The Kite wheel carries `kite_ops/native/keychain.swift`, which is invoked with `swift` at runtime rather than compiled into a binary at install time.

### Tooling and standards

`.rustfmt.toml`, `.clippy.toml`, `.pylintrc`, `.clang-format`, `.gitattributes` and `.editorconfig` are present; clang-format is pinned and CI-enforced. Pylint and EditorConfig remain strict configuration only, not CI gates: the pre-existing Python tree still reports roughly 910 pylint messages and 2,064 EditorConfig violations, and enforcing them would force a mass rewrite unrelated to this change set.

Git is governed by `engineering/git-policy.json` and `engineering/git-workflow.md`: linear `main`, one squashed commit per change, `<prefix>/<topic>` branches, conventional commit subjects, no tool attribution in commits, no tracked generated paths. `scripts/check_git_policy.py` validates what a working copy can observe. Server-side rules are defined in `engineering/repo-rulesets.json` and cannot be applied on the current plan, which is recorded rather than glossed over.

## Remaining work

- **Attestation and hosted release evidence** for the tagged commit, including the `gh attestation verify` output, recorded in `VERIFICATION.md`.
- **Server-side protection.** Rulesets and classic branch protection both require GitHub Pro on a private repository; the API refuses them today. Until then the pull-request rule is discipline, not enforcement.
- **Release authority.** Attestation binds bytes to a workflow run via OIDC; it does not establish who may tag. That needs the tag ruleset, which has the same plan dependency.
- **Wheel-hash pinning** in the constraint sets (needs multi-platform CI to be honest about it).
- **Parquet compatibility** beyond the synthetic round-trip: schema evolution, compression, row-group sizing, corrupted and interrupted writes, cross-version reads.
- **Licensed real-market data certification** — the first NSE cash session captured from Upstox and FYERS, with a session evidence report.

## Artifacts

`artifacts/` contains, all built in this environment: `nirnay_quantlab-0.1.0-py3-none-any.whl`, `nirnay_kite_ops-0.4.0-py3-none-any.whl`, `nirnay_fyers_recorder-0.4.0-py3-none-any.whl`, `nirnay-keychain-helper` (Swift, arm64), `nirnay_numeric_oracle_test`, `upstox-decode-export`, `upstox-recorder`, `upstox-smoke`, and `SHA256SUMS` covering the eight. All eight verify against the extracted bundle. The compiled binaries and Swift helper are macOS arm64 builds; `release.yml` rebuilds them per platform from the tagged commit, and those CI builds — not these — are the ones published.

Build caches (`target/`, `build/`, `dist/`, `*.egg-info`, `__pycache__`, `.pytest_cache`, `.ruff_cache`) are excluded from the bundle by `scripts/make_bundle.py` and from the repository by `.gitignore` and the git policy validator.

## Boundary

Live trading remains hard-disabled: `configs/live.toml` keeps live execution off, zero notional limits and an unconfigured broker. No Zerodha login, OAuth callback, Keychain secret read, FYERS connection, Upstox session or broker order was performed. No provider entitlement, market-data right, regulatory approval or independent risk deployment is claimed. This is a research and paper-trading release, not a production trading system.
