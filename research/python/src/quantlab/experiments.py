"""Pre-registered experiment records and immutable identifiers."""
from __future__ import annotations

import hashlib
import json
import math
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any


class ExperimentError(ValueError):
    """Raised when an experiment is incomplete or internally inconsistent."""


_REQUIRED_FIELDS = {
    "experiment_id",
    "hypothesis",
    "economic_rationale",
    "dataset_ref",
    "dataset_manifest_sha256",
    "dataset_point_in_time",
    "code_revision",
    "config",
    "status",
    "registered_at_utc",
    "falsification_rule",
}
_ALLOWED_FIELDS = _REQUIRED_FIELDS | {"result", "validation"}
_TERMINAL_STATUSES = {"completed", "candidate", "rejected"}
_VALID_STATUSES = {"registered", "running", *_TERMINAL_STATUSES}


def _validate_json_value(value: Any, path: str = "$") -> None:
    """Reject Python values that cannot be represented as strict JSON."""
    if value is None or isinstance(value, (str, bool, int)):
        return
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ExperimentError(f"{path} must not contain NaN or infinity")
        return
    if isinstance(value, list):
        for index, item in enumerate(value):
            _validate_json_value(item, f"{path}[{index}]")
        return
    if isinstance(value, dict):
        for key, item in value.items():
            if not isinstance(key, str):
                raise ExperimentError(f"{path} contains a non-string object key")
            _validate_json_value(item, f"{path}.{key}")
        return
    raise ExperimentError(f"{path} contains a non-JSON value: {type(value).__name__}")


def canonical_json(value: Any) -> bytes:
    """Encode strict JSON with a stable key order and no non-finite numbers."""
    _validate_json_value(value)
    try:
        encoded = json.dumps(
            value,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
            allow_nan=False,
        )
    except (TypeError, ValueError) as exc:
        raise ExperimentError(f"value cannot be canonically encoded: {exc}") from exc
    return encoded.encode("utf-8")


def content_hash(value: Any) -> str:
    return hashlib.sha256(canonical_json(value)).hexdigest()


def validate_experiment(record: dict[str, Any]) -> None:
    if not isinstance(record, dict):
        raise ExperimentError("experiment record must be an object")

    missing = sorted(_REQUIRED_FIELDS - record.keys())
    if missing:
        raise ExperimentError(f"missing required fields: {', '.join(missing)}")
    unknown = sorted(record.keys() - _ALLOWED_FIELDS) if all(
        isinstance(key, str) for key in record
    ) else []
    if unknown:
        raise ExperimentError(f"unknown fields: {', '.join(unknown)}")
    if any(not isinstance(key, str) for key in record):
        raise ExperimentError("experiment record keys must be strings")

    for field in (
        "experiment_id",
        "hypothesis",
        "economic_rationale",
        "dataset_ref",
        "code_revision",
        "falsification_rule",
    ):
        if not isinstance(record[field], str) or not record[field].strip():
            raise ExperimentError(f"{field} must be a non-empty string")

    if not isinstance(record["dataset_point_in_time"], bool):
        raise ExperimentError("dataset_point_in_time must be a boolean")

    digest = record["dataset_manifest_sha256"]
    if not isinstance(digest, str) or len(digest) != 64 or any(
        char not in "0123456789abcdef" for char in digest
    ):
        raise ExperimentError("dataset_manifest_sha256 must be lowercase SHA-256")

    status = record["status"]
    if not isinstance(status, str) or status not in _VALID_STATUSES:
        raise ExperimentError("unknown experiment status")

    if not isinstance(record["config"], dict):
        raise ExperimentError("config must be an object")
    for optional_object in ("result", "validation"):
        if optional_object in record and not isinstance(record[optional_object], dict):
            raise ExperimentError(f"{optional_object} must be an object")

    timestamp = record["registered_at_utc"]
    if not isinstance(timestamp, str):
        raise ExperimentError("registered_at_utc must be an ISO-8601 UTC string")
    try:
        registered = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    except ValueError as exc:
        raise ExperimentError("registered_at_utc must be an ISO-8601 timestamp") from exc
    if registered.tzinfo is None or registered.utcoffset() is None:
        raise ExperimentError("registered_at_utc must include a timezone")
    if registered.utcoffset() != timedelta(0):
        raise ExperimentError("registered_at_utc must use UTC (Z or +00:00)")

    if status in _TERMINAL_STATUSES:
        if not isinstance(record.get("result"), dict):
            raise ExperimentError("terminal experiment status requires result object")
        if not isinstance(record.get("validation"), dict):
            raise ExperimentError("terminal experiment status requires validation object")

    if record["dataset_point_in_time"] is False and status == "candidate":
        raise ExperimentError("a dataset with known point-in-time leakage cannot be a candidate")

    # This also rejects NaN/Infinity, non-string nested keys, and non-JSON types.
    canonical_json(record)


def freeze_experiment(record: dict[str, Any]) -> tuple[dict[str, Any], str]:
    """Validate and return a detached canonical record plus its content hash."""
    validate_experiment(record)
    frozen = json.loads(canonical_json(record))
    return frozen, content_hash(frozen)


def load_experiment(path: str | Path) -> dict[str, Any]:
    try:
        record = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ExperimentError(f"could not load experiment record: {exc}") from exc
    if not isinstance(record, dict):
        raise ExperimentError("experiment record must be a JSON object")
    validate_experiment(record)
    return record
