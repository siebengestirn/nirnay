# Supply-chain controls

This repository previously left dependency integrity to `cargo deny` for Rust
only. Python resolution was floating, workflow actions were tag-pinned, and no
SBOM was produced. This document records what is now enforced, what it does not
cover, and which residual risk is knowingly accepted.

## 1. Workflow actions are pinned to commit SHAs

Every `uses:` reference in `.github/workflows/*.yml` names a full 40-character
commit SHA with a `# vX` provenance comment, for example:

```yaml
- uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4
```

A moving major tag (`@v4`) lets the action owner change the code that runs with
this repository's token. The SHA pin removes that. `taiki-e/install-action`
selects the tool through its `tool:` input rather than through the ref, because
pinning one of its `<tool>` tags by hash is explicitly discouraged upstream.

Gate: `python3 scripts/supply_chain.py pins` (CI job `supply-chain`, and nightly). It covers every workflow, including `release.yml`, and currently passes on 43 references.

Updating a pin is a reviewable change: resolve the tag with
`GET /repos/{owner}/{repo}/git/ref/tags/{tag}`, dereference annotated tags, and
record both the SHA and the major version in the comment.

## 2. Python dependency resolution is pinned

`constraints/*.txt` hold the full resolved graph per independently installable
surface, compiled with `uv` against the Python 3.12 target used by CI:

| Surface | Constraint file | Pins |
|---|---|---|
| `research/python` | `constraints/python-research.txt` | 37 |
| `apps/kite-ops` | `constraints/python-kite-ops.txt` | 10 |
| `apps/fyers-recorder` | `constraints/python-fyers-recorder.txt` | 32 |

CI installs from these files and then installs the local package with
`--no-deps`, so a resolver change cannot silently alter a release environment.
Regenerate with `make constraints`; see `constraints/README.md`.

Gate: `python3 scripts/supply_chain.py constraints` — fails when a declared
dependency is unpinned, when a pin violates its declared specifier, or when the
generation-command header is missing.

The provider surfaces stay separate because `fyers-apiv3 3.2.1` pins
`requests==2.31.0` against `nirnay-kite-ops`'s `requests>=2.32,<3`. See
`provider-environments.md`.

## 3. Python advisories are scanned, with dated waivers

`pip-audit` runs in every Python CI job:

```bash
python scripts/supply_chain.py audit research        # research environment
python scripts/supply_chain.py audit kite-ops
python scripts/supply_chain.py audit fyers-recorder
```

The command fails on any advisory not covered by `constraints/pip-audit-allowlist.toml`.
Each waiver requires an owner and an expiry date, and the gate warns when a waiver
no longer matches an observed package, so the list cannot rot silently.

Observed state at the 2026-11-13 release:

- `research`: 0 advisories.
- `kite-ops`: 0 advisories.
- `fyers-recorder`: 51 advisories against `aiohttp`, `requests` and `setuptools` —
  all inherited from `fyers-apiv3 3.2.1`, which pins `aiohttp==3.9.3`,
  `requests==2.31.0` and `setuptools==68.0.0`.

**Accepted residual risk:** those 51 advisories cannot be fixed inside this
repository. Upgrading the three packages breaks the provider SDK's own pins, and
the SDK is the only supported FYERS Data Socket client. The waiver expires
2027-02-28. Until FYERS publishes a release with relaxed pins — or the recorder is
replaced by a direct WebSocket capture path — the FYERS recorder is a
research-only capture tool. It must not be treated as a hardened production
service, and it holds no order-routing authority.

## 4. An SBOM is generated per release

`python3 scripts/supply_chain.py sbom <output>` writes a CycloneDX 1.5 JSON
document covering all `Cargo.lock` packages (with their cargo checksums and a
workspace-member marker) and every Python component in the constraint sets.

The output is deterministic: components are sorted by purl, no timestamp is
embedded, and the component list digest is recorded in
`metadata.properties[nirnay:component-digest]`, so two runs on the same inputs
produce a byte-identical file. `make sbom` writes `sbom.cdx.json` at the
repository root; the release bundle carries it beside `BUILD-RESULTS.md`.

## 5. What this still does not cover

- **No attestation has been produced yet.** The mechanism is wired into
  `release.yml` (section 6) but has never run, and custody of a release-signing
  identity is still undefined.
- **No wheel-hash pins.** The constraint files pin versions, not artifact hashes.
  Hash pinning would bind the lock to one platform's wheels, which this
  single-platform CI cannot honestly verify.
- **`cargo deny` covers Rust advisories, licences, bans and sources**; it is
  unchanged by this work and still runs in the `rust` CI job.
- **No hosted CI run is evidence.** The workflow changes in this delivery have
  been syntax-validated and the equivalent commands run locally; they have not
  executed on a pushed commit.

## 6. Release provenance and bundle identity

The r2 archive carried two identity defects. Both are now prevented by
construction rather than by care.

**One file list feeds both artefacts.** `scripts/make_bundle.py` stages the
tree, writes `SOURCE-MANIFEST.sha256` from the staged list, and then writes the
archive *from that same list*, with fixed member timestamps and each member's
own permission bits. A manifest path and an archive member name therefore
cannot diverge, and the same inputs produce the same archive bytes. r2 shipped `SBOM.cdx.json` while its
manifest listed `sbom.cdx.json`; on APFS `shasum -c` resolved the name happily,
while a case-sensitive runner would not have.

**Verification compares strings, not filesystem lookups.**

```bash
python3 scripts/verify_bundle.py --bundle /path/to/extracted --zip /path/to/bundle.zip
```

It checks each manifest path against the exact listed path, hashes every file
both in the tree and inside the archive, and requires the archive's member list
to equal the manifest's path list plus `BUILD-RESULTS.md` and the manifest
itself. It also refuses an ELF or Mach-O payload that is stored or extracted
without an execute bit — r3 wrote `0o644` for every member, so all five compiled
binaries extracted non-executable. It needs no case-sensitive filesystem, so the
same command is valid on macOS and on the Ubuntu runner. Run against the
superseded r2 archive it fails with four named errors, and `--self-test` clears
one execute bit in a copy of the archive and requires the run to fail: both are
evidence that the gates can fail.

**Source permissions survive the trip.** `make_bundle.py` derives the archived
mode from the staged file (`0o755` when any execute bit is set, `0o644`
otherwise) and writes `(stat.S_IFREG | mode) << 16`. Seven members are
`0o755` in r0. Nothing in the toolchain rewrites them afterwards, and Git
records the modes of the source tree as they are on disk, so an executable that
is committed keeps `100755`.

**A tag cannot publish on a red build.** `ci.yml` declares `workflow_call`, so
`release.yml` can call it as a reusable workflow. Its first job, `quality`, runs
the same ten checks that guard a pull request; `bundle` needs `quality`, and
`macos-assets` needs `bundle`, so the two jobs that call `gh release create` run
serially instead of racing. The chain is:

```
tag -> quality (ci.yml) -> bundle (linux) -> macos-assets (arm64)
```

**Release assets come from CI.** `.github/workflows/release.yml` triggers on
`v*` tags: it rebuilds the wheels and binaries from the tagged commit, runs the
packaging gate, assembles and re-verifies the bundle as a consumer would, and
attests the archive, wheels, binaries and SBOM with
`actions/attest-build-provenance@4d101475…` (v4.2.2) under `id-token: write` and
`attestations: write`. A second job does the same for the macOS arm64 binaries
and the Keychain helper, which are suffixed so they cannot overwrite the Linux
assets.

The attestation is what binds bytes to a repository, a commit and a workflow run.
A SHA-256 sidecar only proves the bytes survived the transfer.
