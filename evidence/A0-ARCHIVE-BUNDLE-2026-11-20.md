# A0 — archive Git bundle correction — 2026-11-20

Dated evidence for the archive recovery defect and its fix. Immutable record.

## Defect

The full archive `nirnay-full-2026-11-20.zip` declared source snapshot
`3759f9ebf4edba977a54fbfd7c1b71aaa8fa045d` but its bundled Git history had:

| Ref | Old value |
|---|---|
| `refs/heads/main` | `64754990bbc5e905c8c80ca522756954145867f0` |
| `refs/heads/cleanup/archive-and-perf` | `3a2805f1b519fd3dd64f0620d0cf8a9c769efd20` |

Cloning the bundle and querying `3759f9e` returned "could not get object info":
the source snapshot was absent from the bundled history. The archive was not a
complete Git recovery bundle for its own source snapshot.

## Correction

- Regenerated `.local/perf/nirnay-all-refs.bundle` from the repository at
  `3759f9e`. `refs/heads/main`, `refs/remotes/origin/HEAD`,
  `refs/remotes/origin/main` and `HEAD` all resolve to `3759f9e`;
  `refs/nirnay-reconciliation/pre-main` is retained.
- Added `scripts/verify_archive_bundle.py`, which checks that the declared
  `checked_out_commit` matches `main_commit`, that the bundled bundle digest
  matches the manifest, that `refs/heads/main` resolves to the declared commit,
  and that the commit exists in and is an ancestor of the bundled HEAD. It also
  requires the archived source tree to match that commit's tree, allowing only
  files the manifest declares as untracked inclusions.
- Five self-tests prove the checker catches a stale bundle, a missing
  `refs/heads/main`, a tampered source-tree file, and accepts a matching or
  abbreviated commit.
- Added `make archive-verify` as the tracked, manually invocable gate. The
  operator-local archive builder (`.local/perf/build_review_bundle.py`, outside
  Git) calls the same verifier and refuses to publish a failing archive; no
  owner-excluded release script is modified.

## Result

The verifier reported both original defects on the stale archive before
correction, and reports thirteen findings on the hybrid archive produced before
this PR (one tracked file modified after the declared commit, one post-commit
file and eleven undeclared untracked inclusions), confirming the test is
sensitive to both failure modes. The archive must be rebuilt from a clean
checkout of the final merge commit after this change lands; until then
`make archive-verify` is expected to fail on the on-disk artifact.

No credential, licensed capture or broker action is involved. This record
covers archive integrity only and grants no trading or platform-readiness claim.
