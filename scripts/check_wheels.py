#!/usr/bin/env python3
"""Packaging gate: reject dirty wheels, then prove each one installs and runs.

The r2 release shipped wheels containing stale `__pycache__/*.pyc` files copied
out of a reused `build/lib` tree. The previous check only asserted that
`kite_ops/native/keychain.swift` was present, so it passed on a dirty wheel.
This gate inspects the full archive member list and then installs every wheel
into a fresh virtual environment and executes its console script.

    python scripts/check_wheels.py --dist dist
    python scripts/check_wheels.py --dist dist --no-install

Run from the repository root. Standard library only.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
import venv
import zipfile
from pathlib import Path

EXPECTED = {
    "nirnay_quantlab": {
        "required": {"quantlab/cli.py", "quantlab/risk_daemon.py"},
        "script": "quantlab",
    },
    "nirnay_kite_ops": {
        # The Swift helper is shipped as source and invoked with `swift`, so its
        # presence inside the wheel is a packaging contract, not a build detail.
        "required": {"kite_ops/cli.py", "kite_ops/native/keychain.swift"},
        "script": "nirnay-kite",
    },
    "nirnay_fyers_recorder": {
        "required": {"fyers_recorder/cli.py"},
        "script": "nirnay-fyers-recorder",
    },
}

# Anything matching these must never appear in a built wheel.
FORBIDDEN_SUBSTRINGS = ("__pycache__/", ".egg-info/", "/build/", "build/lib/")
FORBIDDEN_SUFFIXES = (".pyc", ".pyo", ".so.tmp", ".DS_Store")


def fail(messages: list[str]) -> int:
    for message in messages:
        print(f"FAIL: {message}", file=sys.stderr)
    print(f"{len(messages)} packaging gate(s) failed", file=sys.stderr)
    return 1


def inspect(wheel: Path) -> list[str]:
    errors: list[str] = []
    try:
        with zipfile.ZipFile(wheel) as archive:
            names = archive.namelist()
    except zipfile.BadZipFile as exc:
        return [f"{wheel.name}: not a valid zip archive ({exc})"]

    for name in names:
        if any(fragment in name for fragment in FORBIDDEN_SUBSTRINGS):
            errors.append(f"{wheel.name}: contains a build cache entry `{name}`")
        if name.endswith(FORBIDDEN_SUFFIXES):
            errors.append(f"{wheel.name}: contains a non-source entry `{name}`")

    dist_info = [name for name in names if name.endswith(".dist-info/METADATA")]
    if not dist_info:
        errors.append(f"{wheel.name}: no .dist-info/METADATA entry")

    return errors


def install_smoke(wheel: Path, script: str) -> list[str]:
    errors: list[str] = []
    with tempfile.TemporaryDirectory(prefix="nirnay-wheel-") as tmp:
        env = Path(tmp) / "venv"
        venv.EnvBuilder(with_pip=True, clear=True).create(env)
        python = env / "bin" / "python"

        install = subprocess.run(
            [str(python), "-m", "pip", "install", "--disable-pip-version-check", "--quiet", str(wheel)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        if install.returncode != 0:
            tail = "\n".join(install.stdout.strip().splitlines()[-15:])
            return [f"{wheel.name}: clean-environment install failed\n{tail}"]

        check = subprocess.run(
            [str(python), "-m", "pip", "check"],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        if check.returncode != 0:
            errors.append(f"{wheel.name}: `pip check` reported\n{check.stdout.strip()}")

        console = env / "bin" / script
        if not console.is_file():
            errors.append(f"{wheel.name}: console script `{script}` was not installed")
            return errors

        run = subprocess.run(
            [str(console), "--help"],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=120,
        )
        if run.returncode != 0:
            errors.append(f"{wheel.name}: `{script} --help` exited {run.returncode}\n{run.stdout.strip()}")
        elif "usage:" not in run.stdout.lower():
            errors.append(f"{wheel.name}: `{script} --help` produced no usage text")

    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Reject dirty wheels and smoke-test clean installs")
    parser.add_argument("--dist", type=Path, default=Path("dist"))
    parser.add_argument("--no-install", action="store_true", help="static inspection only")
    args = parser.parse_args(argv)

    wheels = sorted(args.dist.glob("*.whl"))
    if not wheels:
        return fail([f"no wheels found in {args.dist}"])

    errors: list[str] = []
    seen: dict[str, Path] = {}
    for wheel in wheels:
        matched = next((prefix for prefix in EXPECTED if wheel.name.startswith(prefix + "-")), None)
        if matched is None:
            errors.append(f"{wheel.name}: unexpected wheel in the distribution")
            continue
        seen[matched] = wheel
        errors.extend(inspect(wheel))

    missing = sorted(set(EXPECTED) - set(seen))
    if missing:
        errors.append(f"missing wheels: {', '.join(missing)}")
    extra = len(wheels) - len(seen)
    if extra > 0:
        errors.append(f"{extra} unexpected wheel(s) present")

    for prefix, wheel in sorted(seen.items()):
        with zipfile.ZipFile(wheel) as archive:
            names = set(archive.namelist())
        for required in sorted(EXPECTED[prefix]["required"]):
            if required not in names:
                errors.append(f"{wheel.name}: required entry `{required}` is missing")

    if not errors and not args.no_install:
        for prefix, wheel in sorted(seen.items()):
            errors.extend(install_smoke(wheel, EXPECTED[prefix]["script"]))

    if errors:
        return fail(errors)

    suffix = "" if args.no_install else "; each installed and ran in a clean environment"
    print(f"PASS: {len(seen)} wheels are clean of cache and build artefacts{suffix}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
