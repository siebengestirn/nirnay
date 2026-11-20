# Markdown prose sweep — 2026-11-19

**Tool:** `scripts/check_prose_style.py --all` (rules PROSE-001..004).
**Baseline:** merged `main` at `adcf9be706be72e5f85107793f0775b5462e8c8c`.
**Sequence:** the checker and gate wiring merged in PR #57; the sweep and
remediation follow in a separate change. The complete sweep ran before any
remediation, then ran again after it.

## Results summary

| Category | Count | Disposition |
|---|---|---|
| Resolved | 38 | Meaning-preserving rewrites in approved scopes |
| Blocked: owner authorization | 3 | `BUILD-RESULTS.md`, unchanged |
| Blocked: predecessor provenance | 1 | `.local/extras/README.md`, unchanged |
| Accepted risk | 0 | None |
| Unresolved other | 0 | None |

Initial full-tree findings: 42 (2 PROSE-001 false positives from the first
checker version were removed by the precise pattern merged in PR #57; the
remediation set is 42 PROSE-002 findings).

## Resolved findings

| File | Count |
|---|---|
| `README.md` | 1 |
| `engineering/anti-patterns.md` | 15 |
| `engineering/decisions/ADR-FND-001-corporate-financial-facts.md` | 1 |
| `engineering/decisions/ADR-M1-002-authorization-authority.md` | 2 |
| `engineering/decisions/ADR-M1-003-oms-authority.md` | 1 |
| `engineering/decisions/ADR-M1-004-dependency-and-port-rules.md` | 1 |
| `engineering/decisions/ADR-M1-006-ordering-and-partitions.md` | 2 |
| `engineering/decisions/ADR-M1-008-durability-acknowledgement.md` | 2 |
| `engineering/decisions/ADR-M1-009-replay-contract.md` | 5 |
| `engineering/decisions/ADR-M1-011-evidence-model.md` | 5 |
| `engineering/git-workflow.md` | 1 |
| `engineering/research/backtesting-and-alpha.md` | 1 |
| `engineering/research/provider-normalization.md` | 1 |

Every rewrite removed an intent adverb without changing the contract, rule ID,
falsifier or evidence claim in the sentence.

## Blocked — owner authorization required

`BUILD-RESULTS.md` is owner-maintained and excluded from agent edits. The three
findings stay visible and unchanged:

| Location | Rule | Impact | Remediation required |
|---|---|---|---|
| `BUILD-RESULTS.md:59` | PROSE-002 (`merely`) | bundle-exclusion wording | owner-authored rewrite of the sentence about names ending in `build` |
| `BUILD-RESULTS.md:90` | PROSE-002 (`actually`) | release-record wording | owner-authored rewrite of the published-artifact sentence |
| `BUILD-RESULTS.md:209` | PROSE-002 (`actually`) | branch-protection wording | owner-authored rewrite of the enforcement sentence |

Each is marked `BLOCKED — OWNER AUTHORIZATION REQUIRED`. None is counted as
resolved, and none was suppressed to obtain a clean audit.

## Blocked — predecessor provenance preserved

`.local/extras/README.md:91` (PROSE-002, `merely`) is historical predecessor reference
material. It stays unchanged to preserve provenance; the disposition is recorded
in `engineering/extras-audit.md`.

## Verification

The post-remediation sweep reports exactly four findings: the three owner-owned
lines and one predecessor-reference line. Full project gates run after this
record is committed. A blocked finding is not a resolved finding.
