#!/usr/bin/env python3
"""Verify a release bundle case-sensitively, on any filesystem.

The r2 bundle shipped `SBOM.cdx.json` at the archive root while
`SOURCE-MANIFEST.sha256` listed `sbom.cdx.json`. macOS agreed with itself
(APFS is case-insensitive); the Ubuntu runner would not have. `shasum -c`
cannot catch this because the shell resolves the name through the filesystem.

This verifier compares manifest paths and archive member names as exact
strings, and asserts that the archive's member list equals the manifest's path
list plus the two record files. It needs no case-sensitive filesystem.

    python scripts/verify_bundle.py --bundle /path/to/extracted
    python scripts/verify_bundle.py --bundle /path/to/extracted --zip /path/to/bundle.zip

Record files that are deliberately outside the manifest:
    BUILD-RESULTS.md      describes the bundle, so it cannot be inside its own hash set
    SOURCE-MANIFEST.sha256
"""
from __future__ import annotations

import argparse
import hashlib
import sys
import zipfile
from pathlib import Path, PurePosixPath

MANIFEST_NAME = "SOURCE-MANIFEST.sha256"
RECORD_FILES = {"BUILD-RESULTS.md", MANIFEST_NAME}
IGNORED = {".DS_Store"}


def fail(messages: list[str]) -> int:
    for message in messages:
        print(f"FAIL: {message}", file=sys.stderr)
    print(f"{len(messages)} bundle gate(s) failed", file=sys.stderr)
    return 1


def read_manifest(path: Path) -> tuple[dict[str, str], list[str]]:
    errors: list[str] = []
    entries: dict[str, str] = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if not line.strip():
            continue
        digest, _, name = line.partition("  ")
        if not digest or not name:
            errors.append(f"{MANIFEST_NAME}:{number}: not `hash  path` format")
            continue
        if len(digest) != 64 or any(character not in "0123456789abcdef" for character in digest):
            errors.append(f"{MANIFEST_NAME}:{number}: `{digest}` is not a lowercase SHA-256")
        if name.startswith("/") or "\\" in name or ".." in PurePosixPath(name).parts:
            errors.append(f"{MANIFEST_NAME}:{number}: `{name}` is not a clean relative POSIX path")
        if name in entries:
            errors.append(f"{MANIFEST_NAME}:{number}: `{name}` is listed twice")
        entries[name] = digest
    return entries, errors


def list_files(root: Path) -> dict[str, Path]:
    found: dict[str, Path] = {}
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        found[relative] = path
    return found


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_directory(bundle: Path, entries: dict[str, str]) -> list[str]:
    errors: list[str] = []
    present = list_files(bundle)

    # Exact string membership, never a filesystem lookup: this is the check that
    # a case-insensitive volume cannot perform for us.
    for name, expected in entries.items():
        if name not in present:
            near = sorted(candidate for candidate in present if candidate.lower() == name.lower())
            hint = f" (found `{near[0]}` instead)" if near else ""
            errors.append(f"listed file is missing: `{name}`{hint}")
            continue
        actual = sha256(present[name])
        if actual != expected:
            errors.append(f"hash mismatch: `{name}` is {actual}, manifest says {expected}")

    unlisted = sorted(set(present) - set(entries) - RECORD_FILES - IGNORED)
    for name in unlisted:
        errors.append(f"file is present but not listed: `{name}`")
    return errors


def check_zip(archive_path: Path, entries: dict[str, str]) -> list[str]:
    errors: list[str] = []
    with zipfile.ZipFile(archive_path) as archive:
        members = [name for name in archive.namelist() if not name.endswith("/")]
        duplicates = {name for name in members if members.count(name) > 1}
        for name in sorted(duplicates):
            errors.append(f"archive contains `{name}` more than once")

        expected = set(entries) | RECORD_FILES
        for name in sorted(expected - set(members)):
            near = sorted(member for member in members if member.lower() == name.lower())
            hint = f" (archive stores `{near[0]}`)" if near else ""
            errors.append(f"manifest path missing from archive: `{name}`{hint}")
        for name in sorted(set(members) - expected):
            near = sorted(candidate for candidate in expected if candidate.lower() == name.lower())
            hint = f" (manifest lists `{near[0]}`)" if near else ""
            errors.append(f"archive member not accounted for: `{name}`{hint}")

        for name, digest in entries.items():
            if name not in members:
                continue
            actual = hashlib.sha256(archive.read(name)).hexdigest()
            if actual != digest:
                errors.append(f"archive member hash mismatch: `{name}`")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Case-sensitive release bundle verification")
    parser.add_argument("--bundle", type=Path, required=True, help="extracted bundle root")
    parser.add_argument("--zip", type=Path, help="distribution archive to cross-check")
    args = parser.parse_args(argv)

    manifest = args.bundle / MANIFEST_NAME
    if not manifest.is_file():
        return fail([f"{manifest} is missing"])

    entries, errors = read_manifest(manifest)
    if not entries:
        return fail(errors + ["manifest is empty"])

    errors.extend(check_directory(args.bundle, entries))
    if args.zip:
        if not args.zip.is_file():
            errors.append(f"{args.zip} is missing")
        else:
            errors.extend(check_zip(args.zip, entries))

    if errors:
        return fail(errors)

    scope = f"{len(entries)} manifest entries"
    if args.zip:
        scope += ", archive member list and per-member hashes"
    print(f"PASS: {scope} verified case-sensitively against {args.bundle}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
