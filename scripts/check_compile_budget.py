#!/usr/bin/env python3
"""Measure and bound workspace compile cost.

This gate runs the two dominant Rust build phases with a private target
directory so the measurement is reproducible and cannot be masked by a warm
cache. It records wall time and target size, then enforces explicit budgets.
Results are written to ``.local/perf/last-run.json`` (operator-local only).

The measurement does not build the archived workspace: predecessor material is
kept under ``.local/extras`` and is deliberately outside every build graph.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PERF = ROOT / ".local" / "perf"
TARGET = PERF / "budget-target"
COMPILE_ENV = dict(os.environ, CARGO_TARGET_DIR=str(TARGET))

# Cold-build budgets on the reference host (Apple silicon, 16-inch class).
# They are generous enough for CI variance but fail a genuine regression.
BUDGETS = {"check_seconds": 120.0, "test_build_seconds": 180.0, "target_gib": 6.0}


def run(label: str, command: list[str]) -> float:
    started = time.monotonic()
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, env=COMPILE_ENV)
    elapsed = time.monotonic() - started
    if result.returncode != 0:
        sys.stderr.write(f"[FAIL] {label} exited {result.returncode}\n")
        sys.stderr.write(result.stdout[-4000:])
        sys.stderr.write(result.stderr[-4000:])
        raise SystemExit(1)
    print(f"[passed] {label}: {elapsed:.1f}s")
    return elapsed


def dir_gib(path: Path) -> float:
    if not path.exists():
        return 0.0
    total = sum(file.stat().st_size for file in path.rglob("*") if file.is_file())
    return total / (1024**3)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--keep-target", action="store_true")
    parser.add_argument("--json", type=Path)
    arguments = parser.parse_args()

    PERF.mkdir(parents=True, exist_ok=True)
    if not arguments.keep_target and TARGET.exists():
        shutil.rmtree(TARGET)

    check_seconds = run("cargo check --workspace --all-targets",
                        ["cargo", "check", "--workspace", "--all-targets", "--locked"])
    test_seconds = run("cargo test --workspace --all-targets --no-run",
                       ["cargo", "test", "--workspace", "--all-targets", "--locked", "--no-run"])
    size = dir_gib(TARGET)

    report = {
        "check_seconds": round(check_seconds, 2),
        "test_build_seconds": round(test_seconds, 2),
        "target_gib": round(size, 2),
        "budgets": BUDGETS,
        "excludes_archived_workspace": True,
    }
    (PERF / "last-run.json").write_text(json.dumps(report, indent=2) + "\n")
    if arguments.json:
        arguments.json.parent.mkdir(parents=True, exist_ok=True)
        arguments.json.write_text(json.dumps(report, indent=2) + "\n")

    failures = [f"{name}={report[name]} exceeds {limit}" for name, limit in BUDGETS.items()
                if report[name] > limit]
    if failures:
        for failure in failures:
            print(f"[FAIL] compile budget: {failure}")
        return 1
    print("[PASS] compile budget within bounds")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
