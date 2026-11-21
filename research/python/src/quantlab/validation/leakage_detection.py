"""Research-only holdout leakage detection over frozen experiment records."""
from __future__ import annotations

from typing import Any

from ..experiments.schema import (ExperimentError, diagnostic_holdout_flags, holdout_key,
                                  validate_period_geometry)


def detect_holdout_leakage(record: dict[str, Any]) -> dict[str, Any]:
    """Return a deterministic list of holdout-discipline violations."""
    leaks: list[str] = []
    if record.get("dataset_point_in_time") is not True:
        leaks.append("dataset_point_in_time_false")
    declaring_fields = ("dataset_version", "universe_id", "feature_version",
                        "validation_period", "holdout_period", "search_space")
    if any(field not in record for field in declaring_fields):
        leaks.append("missing_holdout_declaration")
    else:
        try:
            validate_period_geometry(record)
        except ExperimentError:
            leaks.append("invalid_period_geometry")
        expected = holdout_key(record["dataset_version"], record["universe_id"],
                               record["feature_version"], record["validation_period"],
                               record["holdout_period"], record.get("universe_version"),
                               record.get("feature_digest"))
        if record.get("holdout_key") != expected:
            leaks.append("holdout_key_mismatch")
    if any(flag is True for flag in diagnostic_holdout_flags(record.get("diagnostics", {}))):
        leaks.append("diagnostics_use_final_holdout")
    validation = record.get("validation", {})
    if isinstance(validation, dict) and validation.get("holdout_used_for_selection") is True:
        leaks.append("holdout_used_for_selection")
    results = record.get("results", {})
    splits = results.get("splits", {}) if isinstance(results, dict) else {}
    if isinstance(splits, dict):
        validation_indices = splits.get("validation_return_end_bar_indices_inclusive", [])
        holdout_indices = splits.get("untouched_holdout_return_end_bar_indices_inclusive", [])
        overlap = set(validation_indices) & set(holdout_indices)
        if validation_indices and holdout_indices and overlap:
            leaks.append("overlapping_report_split_indices")
    return {"status": "holdout_leakage_detected" if leaks else "no_holdout_leakage_detected",
            "experiment_id": record.get("experiment_id"), "leaks": sorted(set(leaks))}


def assert_no_holdout_leakage(record: dict[str, Any]) -> None:
    """Raise when a terminal record shows any holdout leakage."""
    result = detect_holdout_leakage(record)
    if result["leaks"]:
        raise ExperimentError("holdout leakage detected: " + ", ".join(result["leaks"]))
