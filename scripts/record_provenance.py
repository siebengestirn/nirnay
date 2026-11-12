#!/usr/bin/env python3
"""Record build provenance as an in-toto statement, without GitHub attestations.

GitHub's `actions/attest-build-provenance` refuses user-owned private
repositories — the API answers "Feature not available for user-owned private
repositories" — so this repository cannot use it today. Publishing without any
provenance at all would be worse than publishing with an unsigned one, so the
workflow records what it can prove about itself:

    builder      the repository, workflow path and run id that produced the bytes
    materials    the commit SHA and ref the run was started from
    subjects     every published artifact, with its SHA-256

The result is a SLSA-shaped in-toto statement (`predicateType:
https://slsa.dev/provenance/v1`). It is **unsigned**: it is evidence of what the
workflow observed, not proof that the workflow was honest. Anyone can write a
file like this. Treat it as a manifest of claims attached to a run id that the
platform, not the file, authenticates — and replace it with a signed attestation
the day the repository moves to a plan that supports one.

    python scripts/record_provenance.py --output provenance.json \
        --subject dist/*.whl --subject artifacts/SHA256SUMS

Digests are computed from the files on disk after the build, so a subject path
that does not exist is an error rather than an omission.
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import sys
from pathlib import Path

STATEMENT_TYPE = "https://in-toto.io/Statement/v1"
PREDICATE_TYPE = "https://slsa.dev/provenance/v1"


def fail(messages: list[str]) -> int:
    for message in messages:
        print(f"FAIL: {message}", file=sys.stderr)
    return 1


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def expand(patterns: list[str]) -> tuple[list[Path], list[str]]:
    paths: list[Path] = []
    missing: list[str] = []
    for pattern in patterns:
        matched = sorted(Path(name) for name in glob.glob(pattern))
        if not matched:
            missing.append(pattern)
        paths.extend(matched)
    return paths, missing


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Record unsigned in-toto build provenance")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--subject", action="append", default=[], required=True, metavar="GLOB")
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY", "unknown"))
    parser.add_argument("--commit", default=os.environ.get("GITHUB_SHA", "unknown"))
    parser.add_argument("--ref", default=os.environ.get("GITHUB_REF", "unknown"))
    parser.add_argument("--run-id", default=os.environ.get("GITHUB_RUN_ID", "unknown"))
    parser.add_argument("--run-attempt", default=os.environ.get("GITHUB_RUN_ATTEMPT", "1"))
    parser.add_argument("--workflow", default=os.environ.get("GITHUB_WORKFLOW_REF", "unknown"))
    parser.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "unknown"))
    parser.add_argument("--server", default=os.environ.get("GITHUB_SERVER_URL", "https://github.com"))
    args = parser.parse_args(argv)

    paths, missing = expand(args.subject)
    if missing:
        return fail([f"subject pattern matched nothing: `{pattern}`" for pattern in missing])
    if not paths:
        return fail(["no subjects to record"])

    subjects = []
    seen: set[str] = set()
    for path in paths:
        name = path.as_posix()
        if name in seen:
            continue
        seen.add(name)
        subjects.append({"name": name, "digest": {"sha256": sha256(path)}})

    statement = {
        "_type": STATEMENT_TYPE,
        "subject": subjects,
        "predicateType": PREDICATE_TYPE,
        "predicate": {
            "buildDefinition": {
                "buildType": "https://github.com/actions/runner",
                "externalParameters": {
                    "workflow": args.workflow,
                    "event": args.event,
                    "ref": args.ref,
                },
                "internalParameters": {
                    "repository": args.repository,
                    "run_id": args.run_id,
                    "run_attempt": args.run_attempt,
                },
                "resolvedDependencies": [
                    {
                        "uri": f"git+{args.server}/{args.repository}@{args.ref}",
                        "digest": {"gitCommit": args.commit},
                    }
                ],
            },
            "runDetails": {
                "builder": {
                    "id": f"{args.server}/{args.repository}/actions/runs/{args.run_id}"
                },
                "metadata": {
                    "invocationId": f"{args.repository}/actions/runs/{args.run_id}/attempts/{args.run_attempt}"
                },
                "byproducts": [
                    {
                        "name": "github-actions-log",
                        "uri": f"{args.server}/{args.repository}/actions/runs/{args.run_id}",
                    }
                ],
            },
            "note": (
                "Unsigned provenance recorded by scripts/record_provenance.py: it states what "
                "this workflow observed, not that the workflow was honest. GitHub artifact "
                "attestations are unavailable for user-owned private repositories."
            ),
        },
    }

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(statement, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    print(f"provenance  {args.output} ({len(subjects)} subjects, unsigned)")
    for subject in subjects:
        print(f"  {subject['digest']['sha256']}  {subject['name']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
