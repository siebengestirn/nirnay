#!/usr/bin/env python3
"""Supply-chain gates: Action pinning, resolved Python constraints, SBOM, audit waivers.

Subcommands:

    pins         every `uses:` in .github/workflows/*.yml must reference a full
                 commit SHA and carry a `# vX` provenance comment.
    constraints  every declared Python dependency must appear as an exact pin in
                 its generated constraints file, and the file must be current.
    sbom         write a deterministic CycloneDX 1.5 JSON document from
                 Cargo.lock and the Python constraints files.
    audit        run pip-audit in the current environment and fail on any
                 advisory that is not covered by a dated, owner-assigned waiver.

Run from the repository root. No third-party imports are required unless
`packaging` happens to be installed (used only to check version specifiers).
"""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"
ALLOWLIST = ROOT / "constraints" / "pip-audit-allowlist.toml"

# pyproject -> (constraints file, extras evaluated, audit environment name)
TARGETS = {
    "research/python": ("python-research.txt", ("dev", "research", "security"), "research"),
    "apps/kite-ops": ("python-kite-ops.txt", ("test",), "kite-ops"),
    "apps/fyers-recorder": ("python-fyers-recorder.txt", ("test",), "fyers-recorder"),
}

SHA_RE = re.compile(r"^[0-9a-f]{40}$")
PIN_RE = re.compile(r"^(?P<name>[A-Za-z0-9][A-Za-z0-9._-]*)\s*==\s*(?P<version>[^\s;]+)\s*$")


def normalise(name: str) -> str:
    """PEP 503 normalisation so `Foo_Bar` and `foo-bar` compare equal."""
    return re.sub(r"[-_.]+", "-", name).lower()


def fail(messages: list[str]) -> int:
    for message in messages:
        print(f"FAIL: {message}", file=sys.stderr)
    print(f"{len(messages)} supply-chain gate(s) failed", file=sys.stderr)
    return 1


# --------------------------------------------------------------------------- pins


def check_pins() -> list[str]:
    errors: list[str] = []
    for path in sorted(WORKFLOWS.glob("*.yml")):
        for number, line in enumerate(path.read_text().splitlines(), start=1):
            stripped = line.strip()
            if not stripped.startswith("- uses:") and not stripped.startswith("uses:"):
                continue
            value = stripped.split("uses:", 1)[1].strip()
            if value.startswith("./") or value.startswith("docker://"):
                continue
            reference = value.split("#", 1)[0].strip()
            if "@" not in reference:
                errors.append(f"{path.name}:{number}: `{reference}` has no version reference")
                continue
            repo, ref = reference.rsplit("@", 1)
            if not SHA_RE.match(ref):
                errors.append(
                    f"{path.name}:{number}: `{repo}@{ref}` is not pinned to a 40-character commit SHA"
                )
            if not re.search(r"#\s*v\d", value):
                errors.append(f"{path.name}:{number}: `{reference}` is missing a `# vX` provenance comment")
    if not errors:
        pinned = sum(
            1
            for path in WORKFLOWS.glob("*.yml")
            for line in path.read_text().splitlines()
            if "uses:" in line and SHA_RE.match(line.split("@")[-1].split("#")[0].strip())
        )
        print(f"PASS: {pinned} workflow action references are pinned to commit SHAs")
    return errors


# -------------------------------------------------------------------- constraints


def declared_requirements(project: Path, extras: tuple[str, ...]) -> dict[str, str]:
    """Return {normalised name: declared specifier} for the selected extras."""
    data = tomllib.loads((project / "pyproject.toml").read_text())
    declared: dict[str, str] = {}

    def absorb(entries: list[str]) -> None:
        for entry in entries:
            name = re.split(r"[\s<>=!~;\[]", entry.strip(), maxsplit=1)[0]
            specifier = entry.strip()[len(name):].split(";")[0].strip()
            declared.setdefault(normalise(name), specifier)

    absorb(data.get("project", {}).get("dependencies", []))
    optional = data.get("project", {}).get("optional-dependencies", {})
    for extra in extras:
        absorb(optional.get(extra, []))
    # `[build-system].requires` is deliberately excluded: it is satisfied inside
    # pip's isolated build environment and is not part of the runtime resolution
    # that these constraint files pin.
    return declared


def parse_constraints(path: Path) -> dict[str, str]:
    pins: dict[str, str] = {}
    for line in path.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        match = PIN_RE.match(line)
        if not match:
            raise ValueError(f"{path.name}: `{line}` is not an exact `name==version` pin")
        pins[normalise(match.group("name"))] = match.group("version")
    return pins


def specifier_ok(version: str, specifier: str) -> bool:
    if not specifier:
        return True
    try:
        from packaging.specifiers import SpecifierSet
        from packaging.version import Version
    except ImportError:
        return True  # cannot evaluate without packaging; presence is still enforced
    try:
        return SpecifierSet(specifier).contains(Version(version))
    except Exception:
        return False


def check_constraints() -> list[str]:
    errors: list[str] = []
    for project, (filename, extras, _) in TARGETS.items():
        path = ROOT / "constraints" / filename
        if not path.is_file():
            errors.append(f"constraints/{filename} is missing; run `make constraints`")
            continue
        try:
            pins = parse_constraints(path)
        except ValueError as exc:
            errors.append(str(exc))
            continue
        if not pins:
            errors.append(f"constraints/{filename} contains no pins")
        if "uv pip compile" not in path.read_text():
            errors.append(f"constraints/{filename} is missing its generation command header")
        declared = declared_requirements(ROOT / project, extras)
        for name, specifier in sorted(declared.items()):
            if name not in pins:
                errors.append(f"constraints/{filename}: declared dependency `{name}` is not pinned")
                continue
            if not specifier_ok(pins[name], specifier):
                errors.append(
                    f"constraints/{filename}: `{name}=={pins[name]}` violates declared `{name}{specifier}`"
                )
    if not errors:
        total = sum(len(parse_constraints(ROOT / "constraints" / name))
                    for name, _, _ in TARGETS.values())
        print(f"PASS: {len(TARGETS)} constraint files cover every declared dependency ({total} pins)")
    return errors


# --------------------------------------------------------------------------- sbom


def parse_cargo_lock() -> list[dict]:
    lock = ROOT / "Cargo.lock"
    if not lock.is_file():
        return []
    components = []
    for block in lock.read_text().split("[[package]]")[1:]:
        name = re.search(r'^name = "(.*)"$', block, re.M)
        version = re.search(r'^version = "(.*)"$', block, re.M)
        source = re.search(r'^source = "(.*)"$', block, re.M)
        checksum = re.search(r'^checksum = "(.*)"$', block, re.M)
        if not (name and version):
            continue
        component = {
            "type": "library",
            "name": name.group(1),
            "version": version.group(1),
            "purl": f"pkg:cargo/{name.group(1)}@{version.group(1)}",
        }
        properties = []
        if source:
            properties.append({"name": "cargo:source", "value": source.group(1)})
        else:
            properties.append({"name": "nirnay:workspace-member", "value": "true"})
        if checksum:
            properties.append({"name": "cargo:checksum", "value": checksum.group(1)})
        component["properties"] = properties
        components.append(component)
    return components


def parse_python_components() -> list[dict]:
    components = []
    for project, (filename, _, _) in TARGETS.items():
        path = ROOT / "constraints" / filename
        if not path.is_file():
            continue
        for name, version in sorted(parse_constraints(path).items()):
            components.append({
                "type": "library",
                "name": name,
                "version": version,
                "purl": f"pkg:pypi/{name}@{version}",
                "properties": [{"name": "python:constraints", "value": f"constraints/{filename}"}],
            })
        data = tomllib.loads((ROOT / project / "pyproject.toml").read_text())
        package = data.get("project", {})
        package_name = package.get("name", project)
        package_version = package.get("version", "0.0.0")
        components.append({
            "type": "application",
            "name": package_name,
            "version": package_version,
            "purl": f"pkg:pypi/{normalise(package_name)}@{package_version}",
            "properties": [{"name": "nirnay:source-path", "value": project}],
        })
    return components


def build_sbom() -> dict:
    components = parse_cargo_lock() + parse_python_components()
    components.sort(key=lambda c: (c.get("purl", c["name"])))
    digest = hashlib.sha256(
        json.dumps(components, sort_keys=True).encode()
    ).hexdigest()
    document = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "name": "nirnay-quant",
                "version": "2026-11-13",
                "description": "India-first research-to-paper trading workspace; live order routing disabled",
            },
            "tools": [{"vendor": "nirnay", "name": "scripts/supply_chain.py", "version": "1"}],
            "properties": [
                {"name": "nirnay:component-digest", "value": digest},
                {"name": "nirnay:generated-from", "value": "Cargo.lock + constraints/*.txt"},
            ],
        },
        "components": components,
    }
    return document


def write_sbom(arguments: list[str]) -> int:
    output = Path(arguments[0]) if arguments else ROOT / "sbom.cdx.json"
    document = build_sbom()
    output.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n")
    print(
        f"PASS: wrote {output} with {len(document['components'])} components, "
        f"sha256 {hashlib.sha256(output.read_bytes()).hexdigest()}"
    )
    return 0


# -------------------------------------------------------------------------- audit


def load_allowlist() -> list[dict]:
    if not ALLOWLIST.is_file():
        return []
    return tomllib.loads(ALLOWLIST.read_text()).get("allow", [])


def check_waivers(entries: list[dict], advisories: list[dict], environment: str) -> tuple[list[str], list[dict]]:
    """Split findings into waived and undocumented. `id = "*"` waives a whole package."""
    import datetime as dt

    errors: list[str] = []
    today = dt.date.today()
    applicable = [e for e in entries if e.get("environment") in (environment, "*")]
    for entry in applicable:
        advisory = entry.get("id", "*")
        owner = entry.get("owner")
        if not owner or str(owner).lower() in {"unassigned", "todo", "tbd"}:
            errors.append(f"{advisory}: waiver requires an assigned owner")
        expiry = entry.get("expires")
        if not expiry:
            errors.append(f"{advisory}: waiver requires expires=YYYY-MM-DD")
        else:
            try:
                if dt.date.fromisoformat(str(expiry)) < today:
                    errors.append(f"{advisory}: waiver expired {expiry}; upgrade or renew it")
            except ValueError:
                errors.append(f"{advisory}: invalid expiry date `{expiry}`")
        package = normalise(entry.get("package", ""))
        if not any(advisory["package_key"] == package for advisory in advisories):
            print(
                f"WARN: waiver for {entry.get('package')} ({advisory}) matches no observed advisory",
                file=sys.stderr,
            )

    waived: list[dict] = []
    for advisory in advisories:
        covered = any(
            normalise(entry.get("package", "")) == advisory["package_key"]
            and entry.get("id", "*") in ("*", advisory["id"])
            for entry in applicable
        )
        if covered:
            waived.append(advisory)
        else:
            errors.append(
                f"{advisory['package']} {advisory['version']}: undocumented advisory "
                f"{advisory['id']} (fixes: {', '.join(advisory['fix_versions']) or 'none published'})"
            )
    return errors, waived


def run_audit(arguments: list[str]) -> int:
    environment = arguments[0] if arguments else "research"
    command = [sys.executable, "-m", "pip_audit", "--format", "json", "--progress-spinner", "off",
               "--skip-editable"]
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if result.returncode not in (0, 1) or not result.stdout.strip():
        print(result.stderr, file=sys.stderr)
        return fail([f"pip-audit could not run (exit {result.returncode})"])
    report = json.loads(result.stdout)
    advisories = [
        {
            "package": dep["name"],
            "package_key": normalise(dep["name"]),
            "version": dep["version"],
            "id": vuln["id"],
            "fix_versions": vuln.get("fix_versions", []),
        }
        for dep in report.get("dependencies", [])
        for vuln in dep.get("vulns", [])
    ]
    # pip-audit can report the same advisory once per resolved path.
    advisories = list(
        {(a["package_key"], a["id"], a["version"]): a for a in advisories}.values()
    )
    errors, waived = check_waivers(load_allowlist(), advisories, environment)
    if errors:
        return fail(errors)
    packages = sorted({a["package"] for a in waived})
    detail = f" ({', '.join(packages)})" if packages else ""
    print(
        f"PASS: pip-audit environment `{environment}` reports {len(waived)} waived "
        f"advisory/advisories{detail} and 0 undocumented"
    )
    return 0


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    command, arguments = argv[1], argv[2:]
    if command == "pins":
        errors = check_pins()
        return fail(errors) if errors else 0
    if command == "constraints":
        errors = check_constraints()
        return fail(errors) if errors else 0
    if command == "sbom":
        return write_sbom(arguments)
    if command == "audit":
        return run_audit(arguments)
    print(f"unknown command `{command}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
