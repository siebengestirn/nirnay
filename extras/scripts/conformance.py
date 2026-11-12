#!/usr/bin/env python3
"""Machine-readable conformance report: Rust vs C++ on shared fixtures.

Runs the Rust and C++ conformance drivers on every tests/fixtures/*.txt
file, diffs their JSON output byte-for-byte, checks golden SBE bytes, and
writes conformance_report.json.

This script is a mandatory gate: both drivers must exist and run. A missing
driver, a missing fixture set, or any mismatch exits non-zero. There is no
skipped-comparison success path.
"""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = sorted((ROOT / "tests" / "fixtures").glob("*.txt"))
GOLDEN = ROOT / "tests" / "golden" / "sample_replay.sbe"
REPORT_PATH = ROOT / "target" / "evidence" / "conformance_report.json"


def run_driver(cmd: list[str], fixture: Path) -> str:
    proc = subprocess.run(
        cmd,
        stdin=fixture.open("rb"),
        capture_output=True,
        timeout=120,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"{cmd} failed on {fixture.name}: {proc.stderr.decode()[:500]}"
        )
    return proc.stdout.decode()


def write_report(report: dict) -> None:
    REPORT_PATH.parent.mkdir(parents=True, exist_ok=True)
    REPORT_PATH.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"conformance report written to {REPORT_PATH}")


def main(argv: list[str]) -> int:
    # --require-sbe: required CI mode; the two-driver book-only invocation
    # is rejected there and the SBE comparison is mandatory.
    require_sbe = "--require-sbe" in argv
    args = [a for a in argv if a != "--require-sbe"]
    if len(args) not in (3, 5):
        print(
            "usage: conformance.py [--require-sbe] <cpp_driver> <rust_driver> [cpp_sbe_dump rust_sbe_dump]",
            file=sys.stderr,
        )
        return 2
    if require_sbe and len(args) != 5:
        print(
            "conformance gate FAIL: --require-sbe requires both SBE drivers",
            file=sys.stderr,
        )
        return 2
    cpp_driver, rust_driver = args[1], args[2]
    cpp_sbe = args[3] if len(args) == 5 else None
    rust_sbe = args[4] if len(args) == 5 else None
    report: dict = {
        "fixtures": [],
        "golden": {"present": GOLDEN.exists()},
        "conformance": {},
        "gate_passed": False,
        "fixtures_checked": 0,
    }
    if GOLDEN.exists():
        digest = hashlib.sha256(GOLDEN.read_bytes()).hexdigest()
        report["golden"]["sha256"] = digest
        report["golden"]["bytes"] = GOLDEN.stat().st_size
    # Fail-closed: both drivers must be present and executable.
    for label, driver in (("cpp", cpp_driver), ("rust", rust_driver)):
        if not driver or not Path(driver).is_file():
            print(
                f"conformance gate FAIL: {label} driver missing: {driver!r}",
                file=sys.stderr,
            )
            write_report(report)
            return 2
    if not FIXTURES:
        print(
            "conformance gate FAIL: no fixtures in tests/fixtures/",
            file=sys.stderr,
        )
        write_report(report)
        return 2
    ok = True
    for fixture in FIXTURES:
        try:
            cpp_out = run_driver([cpp_driver], fixture)
            rust_out = run_driver([rust_driver], fixture)
        except RuntimeError as error:
            print(f"conformance gate FAIL: {error}", file=sys.stderr)
            ok = False
            report["conformance"][fixture.name] = {"error": str(error)}
            continue
        passed = cpp_out == rust_out
        ok = ok and passed
        report["conformance"][fixture.name] = {
            "rust_cpp_identical": passed,
            "lines": len(cpp_out.splitlines()),
        }
        report["fixtures_checked"] += 1
        if not passed:
            print(
                f"conformance gate FAIL: {fixture.name} outputs differ",
                file=sys.stderr,
            )
    report["gate_passed"] = ok and report["fixtures_checked"] > 0
    # SBE round-trip gate: both decoders must produce identical canonical
    # dumps of the golden capture. This is full-stream equivalence for the
    # golden corpus, not a single fixture diff.
    if cpp_sbe or rust_sbe:
        if (
            not cpp_sbe
            or not rust_sbe
            or not Path(cpp_sbe).is_file()
            or not Path(rust_sbe).is_file()
        ):
            print(
                "conformance gate FAIL: SBE dump driver missing",
                file=sys.stderr,
            )
            report["gate_passed"] = False
            write_report(report)
            return 2
        try:
            cpp_out = subprocess.run(
                [cpp_sbe, str(GOLDEN), "4096"],
                capture_output=True,
                text=True,
                timeout=120,
                check=False,
            )
            rust_out = subprocess.run(
                [rust_sbe, str(GOLDEN), "4096"],
                capture_output=True,
                text=True,
                timeout=120,
                check=False,
            )
            if cpp_out.returncode != 0 or rust_out.returncode != 0:
                raise RuntimeError(
                    f"SBE dump failed: cpp={cpp_out.returncode} rust={rust_out.returncode}"
                )
            sbe_identical = cpp_out.stdout == rust_out.stdout
            report["sbe_roundtrip"] = {
                "rust_cpp_identical": sbe_identical,
                "lines": len(cpp_out.stdout.splitlines()),
                "golden": GOLDEN.relative_to(ROOT).as_posix(),
            }
            report["gate_passed"] = report["gate_passed"] and sbe_identical
            if not sbe_identical:
                print(
                    "conformance gate FAIL: SBE canonical dumps differ",
                    file=sys.stderr,
                )
        except RuntimeError as error:
            print(f"conformance gate FAIL: {error}", file=sys.stderr)
            report["sbe_roundtrip"] = {"error": str(error)}
            report["gate_passed"] = False
    write_report(report)
    print(
        f"conformance report written to {REPORT_PATH}: {'PASS' if report['gate_passed'] else 'FAIL'}"
    )
    return 0 if report["gate_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
