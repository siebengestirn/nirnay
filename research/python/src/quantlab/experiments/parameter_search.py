"""Registered, deterministic grid search over a declared configuration space.

No evaluation occurs before a registered, still-open experiment is verified.
Every attempted configuration is persisted as a make_attempt record, including
losers, and the terminal append is checked by the existing registry contracts
for complete trial accounting and holdout discipline.
"""
from __future__ import annotations

import itertools
from collections.abc import Callable, Mapping, Sequence
from typing import Any

from .schema import ExperimentError, content_hash, make_attempt
from .store import ExperimentRegistry

CLAIM_BOUNDARY = "research_screening_only_not_alpha_certification"


def generate_configs(spec: Mapping[str, Sequence[Any]]) -> list[dict[str, Any]]:
    """Enumerate the Cartesian product of a declared parameter grid."""
    if not isinstance(spec, Mapping) or not spec:
        raise ExperimentError("parameter specification must be a non-empty object")
    keys = sorted(spec)
    for key in keys:
        axis = spec[key]
        if not isinstance(key, str) or not key:
            raise ExperimentError("parameter names must be non-empty strings")
        if isinstance(axis, (str, bytes)) or not isinstance(axis, Sequence) or not len(axis):
            raise ExperimentError("each search axis must be a non-empty sequence")
    return [dict(zip(keys, values)) for values in itertools.product(*(spec[key] for key in keys))]


def _declared_configs(registered: dict[str, Any]) -> list[dict[str, Any]]:
    configs = registered["search_space"].get("configs")
    if not isinstance(configs, list) or not configs:
        raise ExperimentError("registered search space has no declared configurations")
    for config in configs:
        if not isinstance(config, dict):
            raise ExperimentError("declared configurations must be objects")
    return configs


def _validate_outcome(outcome: Any) -> None:
    if not isinstance(outcome, dict):
        raise ExperimentError("an evaluator must return an outcome object")
    try:
        content_hash(outcome)
    except ExperimentError as exc:
        raise ExperimentError(f"evaluator returned a non-canonical outcome: {exc}") from exc


def build_grid_report(configs: Sequence[dict[str, Any]], attempts: Sequence[dict[str, Any]],
                      *, search_space_sha256: str) -> dict[str, Any]:
    """Build a claim-limited report whose digest is recomputable."""
    report: dict[str, Any] = {
        "schema_version": 1,
        "method": "deterministic_grid_search",
        "status": "rejected_or_unproven",
        "warning": "Research screening only; not alpha certification or permission to trade.",
        "claim_boundary": CLAIM_BOUNDARY,
        "search_space_sha256": search_space_sha256,
        "expected_trials": len(configs),
        "candidate_family_size": len(attempts),
        "candidate_scores": [
            {"config": attempt["config"], "validation_net_mean":
             attempt["outcome"].get("validation_net_mean", 0.0), "selected": False}
            for attempt in attempts
        ],
        "uses_final_holdout": False,
    }
    report["report_sha256"] = content_hash(report)
    return report


def grid_search(
    spec: Mapping[str, Sequence[Any]],
    evaluate: Callable[[dict[str, Any]], dict[str, Any]],
    registry: ExperimentRegistry,
    experiment_id: str,
) -> tuple[dict[str, Any], Any]:
    """Evaluate every generated config under a registered experiment.

    Registration and membership checks happen before the evaluator is called.
    The returned receipt comes from an append_result call, so the registry
    enforces holdout disclosure, accounting and immutability.
    """
    from .search_adapter import assert_registered_for_search

    row = assert_registered_for_search(registry, experiment_id)
    registered = row.body
    declared = _declared_configs(registered)
    declared_digests = {content_hash(config) for config in declared}
    configs = generate_configs(spec)
    expected = registered["search_space"]["expected_trials"]
    if len(configs) != expected:
        raise ExperimentError("generated configuration count differs from expected_trials")
    unknown = [config for config in configs if content_hash(config) not in declared_digests]
    if unknown:
        raise ExperimentError("grid contains a configuration outside the registered search space")
    attempts: list[dict[str, Any]] = []
    for config in configs:
        outcome = evaluate(config)
        _validate_outcome(outcome)
        attempts.append(make_attempt(config, outcome))
    if len(attempts) != expected:
        raise ExperimentError("attempted configuration count differs from expected_trials")
    report = build_grid_report(
        configs, attempts,
        search_space_sha256=registered["search_space"]["search_space_sha256"],
    )
    terminal = {
        **registered,
        "status": "rejected",
        "attempted_configs": attempts,
        "result": {"status": report["status"], "report_sha256": report["report_sha256"],
                   "passes_minimum_net_expectancy": False},
        "validation": {
            "dataset_point_in_time": registered["dataset_point_in_time"],
            "holdout_used_for_selection": False,
            "diagnostics_use_final_holdout": False,
            "claim_boundary": CLAIM_BOUNDARY,
        },
        "results": report,
        "diagnostics": {
            "grid_coverage": {"attempted": len(attempts), "expected": expected,
                              "uses_final_holdout": False},
            "deflated_sharpe_validation_only": {"trials": len(attempts),
                                                 "method": "not_applied_research_only"},
            "uses_final_holdout": False,
        },
    }
    receipt = registry.append_result(terminal)
    return report, receipt
