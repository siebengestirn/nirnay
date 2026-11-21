"""Adapter that lets a parameter search run only after registration.

The registered record is the authority: its cost model, declared family and
holdout geometry are checked against the generated report before any terminal
record is appended. No report is emitted for an unregistered experiment.
"""
from __future__ import annotations

import hashlib
from dataclasses import fields
from typing import Any, Sequence

from ..alpha_search import AlphaConfig, AlphaCosts, Bar, search_alpha
from .holdout import assert_holdout_key
from .schema import (ExperimentError, Receipt, content_hash, make_attempt,
                     validate_registered_experiment)
from .store import ExperimentRegistry, StoredRecord


def assert_registered_for_search(registry: ExperimentRegistry, experiment_id: str) -> StoredRecord:
    """Refuse searches without a prior, still-open registered record."""
    if not isinstance(experiment_id, str) or not experiment_id.strip():
        raise ExperimentError("experiment_id is required for alpha-search")
    row = registry.record(experiment_id)
    if row is None:
        raise ExperimentError(f"experiment {experiment_id} is not registered")
    if row.status != "registered":
        raise ExperimentError(f"experiment {experiment_id} is already terminal")
    validate_registered_experiment(row.body)
    assert_holdout_key(row.body)
    return row


def _declared_costs(body: dict[str, Any], override: AlphaCosts | None) -> AlphaCosts:
    if override is not None:
        return override
    assumptions = body["cost_model"]["assumptions"]
    allowed = {field.name for field in fields(AlphaCosts)}
    unknown = set(assumptions) - allowed
    if unknown:
        raise ExperimentError(f"unsupported cost assumptions: {', '.join(sorted(unknown))}")
    return AlphaCosts(**assumptions)


def _declared_family(body: dict[str, Any]) -> tuple[AlphaConfig, ...] | None:
    configs = body["search_space"].get("candidate_configs")
    if configs is None:
        return None
    if not isinstance(configs, list) or not configs:
        raise ExperimentError("candidate_configs must be a non-empty list")
    family = tuple(AlphaConfig(**config) for config in configs)
    if len(family) != len(body["search_space"]["configs"]):
        raise ExperimentError("candidate_configs differ from the declared configuration list")
    return family


def _bars_digest(bars: Sequence[Bar]) -> str:
    encoded = "\n".join(f"{bar.timestamp_ns},{float(bar.close):.17g}" for bar in bars).encode()
    return hashlib.sha256(encoded).hexdigest()


def build_terminal_record(registered: dict[str, Any], report: dict[str, Any]) -> dict[str, Any]:
    """Freeze a search report into a new terminal record (never an in-place edit)."""
    attempts = []
    selected = report["selected"]
    for score in report["candidate_scores"]:
        config = dict(score["config"])
        outcome = {"validation_net_mean": score["validation_net_mean"],
                   "validation_holm_p": score["validation_holm_p"],
                   "holdout_net_mean": score["holdout_net_mean"],
                   "selected": config == selected}
        attempts.append(make_attempt(config, outcome))
    if len(attempts) != registered["search_space"]["expected_trials"]:
        raise ExperimentError(
            "attempted configuration count differs from the registered search space"
        )
    if report.get("candidate_family_size") != len(attempts):
        raise ExperimentError("report trial count differs from persisted attempts")
    status = "completed" if report["passes_minimum_net_expectancy"] else "rejected"
    validation = {
        "dataset_point_in_time": registered["dataset_point_in_time"],
        "holdout_used_for_selection": False,
        "diagnostics_use_final_holdout": False,
        "claim_boundary": "research_screening_only_not_alpha_certification",
    }
    result = {"status": report["status"], "report_sha256": report["report_sha256"],
              "passes_minimum_net_expectancy": report["passes_minimum_net_expectancy"]}
    return {**registered, "status": status, "attempted_configs": attempts,
            "result": result, "validation": validation, "results": report,
            "diagnostics": report["multiple_testing_diagnostics"]}


def run_registered_search(registry: ExperimentRegistry, experiment_id: str,
                          bars: Sequence[Bar], *, costs: AlphaCosts | None = None,
                          **search_kwargs: Any) -> tuple[dict[str, Any], Receipt]:
    """Run a bounded search only for a registered experiment and append the result."""
    row = assert_registered_for_search(registry, experiment_id)
    body = row.body
    expected_input = body["search_space"].get("dataset_sha256")
    if expected_input is not None and expected_input != _bars_digest(bars):
        raise ExperimentError(
            "alpha-search input digest differs from the registered dataset digest"
        )
    report = search_alpha(bars, costs=_declared_costs(body, costs),
                          candidates=_declared_family(body), **search_kwargs)
    splits = report["splits"]
    declared_validation = body["validation_period"]
    declared_holdout = body["holdout_period"]
    if set(declared_validation) != {"start_index", "end_index"} or set(
        declared_holdout
    ) != {"start_index", "end_index"}:
        raise ExperimentError("registered periods must use return-bar indices")
    validation_start = declared_validation["start_index"]
    holdout_end = declared_holdout["end_index"]
    reported_validation = splits["validation_return_end_bar_indices_inclusive"]
    reported_holdout = splits["untouched_holdout_return_end_bar_indices_inclusive"]
    if reported_validation[0] != validation_start or reported_holdout[1] != holdout_end - 1:
        raise ExperimentError("generated report split geometry differs from the registration")
    terminal = build_terminal_record(body, report)
    receipt = registry.append_result(terminal)
    return report, receipt


def report_content_digest(report: dict[str, Any]) -> str:
    """Return the canonical digest of a report for external comparison."""
    body = {key: value for key, value in report.items() if key != "report_sha256"}
    return content_hash(body)
