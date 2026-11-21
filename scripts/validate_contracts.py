#!/usr/bin/env python3
"""Validate active JSON/TOML syntax and JSON Schema Draft 2020-12 documents."""
from __future__ import annotations

import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXCLUDED = {
    "extras",
    ".local",
    "legacy",
    "target",
    ".git",
    ".pytest_cache",
    "__pycache__",
    ".venv",
    "venv",
}


def active(path: Path) -> bool:
    return not any(part in EXCLUDED for part in path.relative_to(ROOT).parts)


def main() -> int:
    json_count = 0
    toml_count = 0
    schema_count = 0
    errors: list[str] = []
    for path in sorted(ROOT.rglob("*.json")):
        if not active(path):
            continue
        try:
            json.loads(path.read_text(encoding="utf-8"))
            json_count += 1
        except Exception as exc:
            errors.append(f"JSON {path.relative_to(ROOT)}: {type(exc).__name__}: {exc}")
    for path in sorted(ROOT.rglob("*.toml")):
        if not active(path):
            continue
        try:
            tomllib.loads(path.read_text(encoding="utf-8"))
            toml_count += 1
        except Exception as exc:
            errors.append(f"TOML {path.relative_to(ROOT)}: {type(exc).__name__}: {exc}")
    try:
        from jsonschema import Draft202012Validator
    except ImportError:
        errors.append("jsonschema is required for schema self-validation: pip install jsonschema")
    else:
        for path in sorted((ROOT / "schemas").rglob("*.json")):
            if not active(path):
                continue
            try:
                schema = json.loads(path.read_text(encoding="utf-8"))
                if schema.get("$schema") == "https://json-schema.org/draft/2020-12/schema":
                    Draft202012Validator.check_schema(schema)
                    schema_count += 1
            except Exception as exc:
                errors.append(f"SCHEMA {path.relative_to(ROOT)}: {type(exc).__name__}: {exc}")
        # Contract vectors: instance files under tests/golden are validated
        # against the schema named by their `schema` field. A vector may wrap
        # its payload in `instance`, or be the instance itself.
        vectors = (
            *ROOT.glob("crates/*/tests/golden/*.json"),
            *ROOT.glob("apps/*/tests/golden/*.json"),
            *ROOT.glob("research/*/tests/golden/*.json"),
        )
        for path in sorted(vectors):
            if not active(path):
                continue
            try:
                vector = json.loads(path.read_text(encoding="utf-8"))
                schema_name = vector.get("schema")
                if not schema_name:
                    # Plain Rust contract fixtures are not schema vectors.
                    continue
                schema_path = ROOT / schema_name
                schema = json.loads(schema_path.read_text(encoding="utf-8"))
                instance = vector.get("instance", vector)
                Draft202012Validator(schema).validate(instance)
            except Exception as exc:
                errors.append(f"VECTOR {path.relative_to(ROOT)}: {type(exc).__name__}: {exc}")
    if errors:
        print("\n".join(f"FAIL: {x}" for x in errors), file=sys.stderr)
        return 1
    print(f"PASS: {json_count} active JSON files, {toml_count} active TOML files, "
          f"{schema_count} Draft 2020-12 schemas validated")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
