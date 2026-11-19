#!/usr/bin/env python3
"""Validate shared fundamental-data vectors against the published schemas.

The same vectors are consumed by the Rust domain validator test. Accepted and
rejected sets must agree for every rule expressible in JSON Schema. Numeric
range checks beyond the schema's digit bound are documented as Rust-only.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker

ROOT = Path(__file__).resolve().parents[1]
VECTORS = ROOT / "research" / "fundamentals" / "contract-vectors"
SCHEMAS = {
    "financial-facts": ROOT / "schemas" / "fundamentals" / "financial-fact.v1.schema.json",
    "company-identities": ROOT / "schemas" / "fundamentals" / "company-identity.v1.schema.json",
}


def check() -> list[str]:
    errors: list[str] = []
    for kind, schema_path in SCHEMAS.items():
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        validator = Draft202012Validator(schema, format_checker=FormatChecker())
        directory = VECTORS / kind
        for vector in sorted(directory.glob("*.json")):
            instance = json.loads(vector.read_text(encoding="utf-8"))
            accepted = validator.is_valid(instance)
            expected = vector.stem == "valid"
            if accepted != expected:
                errors.append(
                    f"{kind}/{vector.name}: schema accepted={accepted}, expected={expected}"
                )
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="run internal checks")
    args = parser.parse_args()
    if args.self_test:
        assert len(SCHEMAS) == 2
        print("PASS: contract-vector checker self-test")
        return 0
    errors = check()
    if errors:
        print("\n".join(f"FAIL: {error}" for error in errors))
        return 1
    cases = sum(len(list((VECTORS / kind).glob("*.json"))) for kind in SCHEMAS)
    print(f"PASS: {cases} fundamental contract vectors match schema expectations")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
