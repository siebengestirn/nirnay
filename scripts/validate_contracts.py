#!/usr/bin/env python3
"""Validate active JSON/TOML syntax, schemas and registry instances."""
from __future__ import annotations

import argparse
import copy
import json
import sys
import tomllib
from pathlib import Path

try:
    from jsonschema import Draft202012Validator, FormatChecker
except ImportError:  # pragma: no cover - dependency is present in the pinned env
    Draft202012Validator = None
    FormatChecker = None

ROOT = Path(__file__).resolve().parents[1]
REGISTRY_INSTANCE = ROOT / "configs/providers/source-products.v1.json"
REGISTRY_SCHEMA = ROOT / "schemas/providers/source-product-registry.v1.schema.json"
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


def validator_for(schema: dict) -> object:
    """Build a Draft 2020-12 validator with date/format checking enabled."""
    if Draft202012Validator is None or FormatChecker is None:
        raise RuntimeError("jsonschema is required for instance validation")
    return Draft202012Validator(schema, format_checker=FormatChecker())


def negative_vectors(instance: dict) -> dict[str, dict]:
    """Registry mutations that the schema must reject, one per failure class."""
    illegal_state = copy.deepcopy(instance)
    illegal_state["products"][0]["rights"]["capture"]["state"] = "MAYBE"
    missing_right = copy.deepcopy(instance)
    del missing_right["products"][0]["rights"]["store"]
    extra_property = copy.deepcopy(instance)
    extra_property["products"][0]["rights"]["unexpected"] = {"state": "UNKNOWN"}
    snapshot_incoherence = copy.deepcopy(instance)
    snapshot_incoherence["products"][0]["incremental_updates"] = "snapshot_only"
    snapshot_incoherence["products"][0]["snapshot"] = False
    bad_date = copy.deepcopy(instance)
    bad_date["products"][0]["rights"]["capture"]["review_date"] = "20-11-2026"
    return {
        "illegal_state": illegal_state,
        "missing_right": missing_right,
        "extra_property": extra_property,
        "snapshot_incoherence": snapshot_incoherence,
        "bad_date_format": bad_date,
    }


def self_test() -> int:
    """Prove the validator rejects each negative vector and accepts the real one."""
    try:
        schema = json.loads(REGISTRY_SCHEMA.read_text(encoding="utf-8"))
        instance = json.loads(REGISTRY_INSTANCE.read_text(encoding="utf-8"))
        validator = validator_for(schema)
    except Exception as exc:
        print(f"FAIL: registry self-test setup: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    errors: list[str] = []
    try:
        validator.validate(instance)
    except Exception as exc:
        errors.append(f"checked-in registry must validate: {type(exc).__name__}: {exc}")
    vectors = negative_vectors(instance)
    for name, vector in vectors.items():
        try:
            validator.validate(vector)
        except Exception:
            continue
        errors.append(f"negative vector {name} was accepted")
    if errors:
        print("\n".join(f"FAIL: {x}" for x in errors), file=sys.stderr)
        return 1
    print(f"PASS: registry schema self-test ({len(vectors)} negative vectors rejected)")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    # Negative vectors run in every invocation so CI and nightly, which call
    # this script directly, exercise the same gate as `make schema-check`.
    if self_test() != 0:
        return 1
    json_count = 0
    toml_count = 0
    schema_count = 0
    instance_count = 0
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
    if Draft202012Validator is None:
        errors.append("jsonschema is required for schema validation: pip install jsonschema")
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
        # Registry instance: the real config is checked against its own schema,
        # including date formats. Serde parsing in Rust does not check these.
        try:
            schema = json.loads(REGISTRY_SCHEMA.read_text(encoding="utf-8"))
            instance = json.loads(REGISTRY_INSTANCE.read_text(encoding="utf-8"))
            validator_for(schema).validate(instance)
            instance_count += 1
        except Exception as exc:
            errors.append(
                f"INSTANCE {REGISTRY_INSTANCE.relative_to(ROOT)}: "
                f"{type(exc).__name__}: {exc}"
            )
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
                validator_for(schema).validate(instance)
            except Exception as exc:
                errors.append(f"VECTOR {path.relative_to(ROOT)}: {type(exc).__name__}: {exc}")
    if errors:
        print("\n".join(f"FAIL: {x}" for x in errors), file=sys.stderr)
        return 1
    print(f"PASS: {json_count} active JSON files, {toml_count} active TOML files, "
          f"{schema_count} Draft 2020-12 schemas validated, "
          f"{instance_count} registry instance validated")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
