# Git and history policy

`engineering/git-policy.json` is the machine-readable policy for this repository;
this document is the procedure that goes with it. `scripts/check_git_policy.py`
enforces what can be checked from a working copy.

The policy exists because the first push of this project created a history that
never happened: two commits describing a local development sequence as if it had
been published in stages, one of them credited to a tool. Main must describe what
was built, once, and nothing else.

## History model

`main` is linear. Work happens on a topic branch, is reviewed as a pull request,
and lands as **one squashed commit**. A topic branch carries exactly one coherent
change; if a second concern appears, it gets its own branch.

```
main ──●──●──●        one squashed commit per change, no merge commits
        ╲ ╱
         ●           topic branch, deleted after the merge is verified
```

Forbidden on published commits: rebase, `reset --hard`, amend. A commit that has
been pushed is finished. Correct it with a new commit, or — if the published
content itself is wrong rather than incomplete — with a revert.

`git-policy.json` lists `e19a85c` and `20b4a01` as grandfathered merge commits
from the project's earlier history. No merge commit is created after them.

## Branch naming

```
<prefix>/<topic>
```

Prefixes: `feat`, `fix`, `docs`, `test`. The topic matches
`^[a-z0-9][a-z0-9._-]*$` — lowercase, no spaces, no slashes beyond the one after
the prefix.

| Example | Accepted |
|---|---|
| `fix/exec-modes` | yes |
| `feat/upstox-session-capture` | yes |
| `fix/r0-release-gate` | yes |
| `harden/r0` | no — `harden` is not a prefix |
| `fix/Exec_Modes` | no — uppercase |

## Commit subjects

```
^(feat|fix|docs|test|refactor|perf|build|ci|chore|revert)(\(scope\))?: .{1,64}$
```

Maximum 72 characters including the type and scope. `fix`, `update`, `cleanup`,
`final` and `final fix` are rejected as whole subjects: they say nothing about
what changed.

The body explains why. It does not carry attribution to tools or agents. Tools
are not authors or co-authors; a `Co-Authored-By:` trailer names a human who
actually wrote the code. A subject line that reports a release identity states
which release it is — `feat: nirnay r0 ...` for the initial import, not "r3 then
r0".

## Working tree

Start from a clean tree:

```bash
git switch main
git pull --ff-only
git status --porcelain    # must print nothing
```

Finish with a clean tree, no stash, and the branch deleted once its merge is
verified on `main` (green checks on the merge commit, not on the pull request
head). Deletion is a deliberate act after verification, never automatic.

## Inventory

After a merge is verified the working copy should be exactly:

| Item | Target |
|---|---|
| local branches | `main` only |
| remotes | `origin` only |
| remote-tracking branches | `origin/main` |
| worktrees | 1 |
| stashes | 0 |
| tracked generated files | 0 |

Pruning refreshes remote-tracking refs and nothing else: `git fetch --prune
origin`. It must not delete local branches, remote branches, or rewrite history.

## Generated paths

Never tracked: `.DS_Store`, `.vscode`, `__pycache__`, `.pytest_cache`,
`.mypy_cache`, `.ruff_cache`, `.cache`, `.venv`, `venv`, `target`, `build`,
`build-asan`, `build-*`, `dist`, `inventory.json`, `conformance_report.json`,
`repomix.config.json`, `*.pyc`, `*.pyo`.

`.gitignore` covers them, but `.gitignore` only advises: `git add -f` overrides
it and `git commit -a` on an old index does not. The validator checks the index
directly, because a tracked build directory is how these files reach a release.

One exception is recorded in the policy, with its reason: the archived
predecessor gate report at `extras/evidence/conformance_report.json` is kept on
purpose as evidence that the original conformance gate did not pass. It is not
a build output of this repository — the generator writes to
`target/evidence/` — and it is not regenerated. Every other path matching the
forbidden components is a failure, including a copy of that report elsewhere.

## Enforcement

```bash
python3 scripts/check_git_policy.py            # check the current branch
python3 scripts/check_git_policy.py --strict   # also require a clean tree
make git-policy
```

It validates the branch name, the subject of every commit since `main` (or the
tip commit when `main` is the tip), the index for generated paths, and the
inventory above. Failures are named; the exit status is non-zero.

Two things it cannot check, and this is deliberate honesty rather than a gap in
the script:

- **Server-side protection.** GitHub refuses both rulesets and classic branch
  protection for user-owned private repositories on the free plan: the API
  answers `403 Upgrade to GitHub Pro or make this repository public`. The
  repository stays private by decision — source confidentiality is worth more
  than platform enforcement of a rule one maintainer already follows — so the
  pull-request requirement holds by discipline, not by enforcement. The ruleset
  definitions are kept in `engineering/repo-rulesets.json`, ready to apply when
  the plan changes.
- **Who may create a release tag.** Tag rulesets are subject to the same
  restriction. The release authority is a convention until it can be a rule:
  only the repository owner tags `v*`, and only from a merge commit on `main`
  whose checks are green.

What the validator does enforce is the part a platform rule cannot help with
anyway: the shape of `main`. A rewritten history, an unsquashed merge series, a
commit subject that reports a release as one thing locally and another
publicly, or a tracked build directory all fail locally, before a push.

## Releases

A version tag is created from a merge commit on `main`, never from a topic
branch and never from a working copy with uncommitted changes. The tag names the
release; `.github/workflows/release.yml` builds from that commit, runs the same
gates as CI through a reusable workflow, and publishes with build provenance
attestation. Local binaries are never uploaded in place of CI builds.

Local development identities (r0, r1, r2, r3 …) are working labels, not
publication history. Only what is on `main` is real; a local label that was
never pushed does not become a commit series later.
