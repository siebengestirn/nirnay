"""Deterministic robustness diagnostics over development-only observations.

These helpers never read holdout indices or outcomes: every returned diagnostic
declares ``uses_final_holdout: False`` so the registered-experiment contract
continues to reject holdout leakage at terminal append.
"""
from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from typing import Any

from .._canonical import content_hash


def segment_stability(segment_net_means: Sequence[float]) -> dict[str, Any]:
    """Flag sign reversals and dispersion across contiguous net-mean segments."""
    if isinstance(segment_net_means, (str, bytes)) or not segment_net_means:
        raise ValueError("at least one segment net mean is required")
    values: list[float] = []
    for value in segment_net_means:
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError("segment net means must be real numbers")
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("segment net means must be finite")
        values.append(number)
    signs = {1 for value in values if value > 0} | {-1 for value in values if value < 0}
    consistent = len(signs) == 1 and 0 not in values
    mu = sum(values) / len(values)
    dispersion = math.sqrt(
        sum((value - mu) ** 2 for value in values) / len(values)
    ) / abs(mu) if mu != 0 else 0.0
    if not math.isfinite(dispersion):
        raise ValueError("segment dispersion must be finite")
    magnitude_ratio = (
        max(abs(value) for value in values) / min(abs(value) for value in values)
        if all(value != 0 for value in values) else 0.0
    )
    unstable = not consistent
    return {
        "status": "unstable" if unstable else "stable",
        "sign_consistent": consistent,
        "segments": len(values),
        "minimum_segment_mean": min(values),
        "maximum_segment_mean": max(values),
        "dispersion": dispersion,
        "magnitude_ratio": magnitude_ratio,
        "uses_final_holdout": False,
    }


def parameter_sensitivity(attempts: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    """Measure rank stability of recorded configurations across two halves."""
    if isinstance(attempts, (str, bytes)) or len(attempts) < 4:
        raise ValueError("at least four attempted configurations are required")
    scored: list[tuple[str, float]] = []
    for attempt in attempts:
        if not isinstance(attempt, Mapping) or "config_sha256" not in attempt:
            raise ValueError("attempts must carry a config_sha256")
        outcome = attempt.get("outcome")
        if not isinstance(outcome, Mapping) or "validation_net_mean" not in outcome:
            raise ValueError("attempt outcomes must carry validation_net_mean")
        score = outcome["validation_net_mean"]
        if isinstance(score, bool) or not isinstance(score, (int, float)):
            raise ValueError("validation_net_mean must be a real number")
        if not math.isfinite(float(score)):
            raise ValueError("validation_net_mean must be finite")
        digest = attempt["config_sha256"]
        if not isinstance(digest, str) or not digest:
            raise ValueError("config_sha256 must be a non-empty string")
        scored.append((digest, float(score)))
    half = len(scored) // 2
    first = _ranks(scored[:half])
    second = _ranks(scored[half:])
    common = sorted(set(first) & set(second))
    if len(common) < 2:
        return {"status": "not_computable", "rank_correlation": 0.0,
                "configurations": len(scored), "uses_final_holdout": False}
    x_values = [first[key] for key in common]
    y_values = [second[key] for key in common]
    rho = _pearson(x_values, y_values)
    unstable = rho < 0.5
    return {
        "status": "unstable" if unstable else "stable",
        "rank_correlation": rho,
        "configurations": len(scored),
        "compared_configurations": len(common),
        "uses_final_holdout": False,
    }


def _ranks(items: Sequence[tuple[str, float]]) -> dict[str, float]:
    order = sorted(items, key=lambda item: (-item[1], item[0]))
    return {name: float(index + 1) for index, (name, _) in enumerate(order)}


def _pearson(left: Sequence[float], right: Sequence[float]) -> float:
    left_mean = sum(left) / len(left)
    right_mean = sum(right) / len(right)
    covariance = sum(
        (a - left_mean) * (b - right_mean) for a, b in zip(left, right)
    )
    left_norm = math.sqrt(sum((a - left_mean) ** 2 for a in left))
    right_norm = math.sqrt(sum((b - right_mean) ** 2 for b in right))
    if left_norm <= 0 or right_norm <= 0:
        return 0.0
    value = covariance / (left_norm * right_norm)
    return max(-1.0, min(1.0, value))


def robustness_digest(diagnostics: Mapping[str, Any]) -> str:
    """Digest a robustness diagnostic block for a frozen research report."""
    if not isinstance(diagnostics, Mapping) or not diagnostics:
        raise ValueError("robustness diagnostics must be a non-empty object")
    if any(value is True for value in _holdout_flags(diagnostics)):
        raise ValueError("robustness diagnostics may not use the final holdout")
    return content_hash(dict(diagnostics))


def _holdout_flags(value: Any) -> list[bool]:
    flags: list[bool] = []
    if isinstance(value, Mapping):
        for key, item in value.items():
            if key == "uses_final_holdout" and isinstance(item, bool):
                flags.append(item)
            if isinstance(item, Mapping):
                flags.extend(_holdout_flags(item))
    return flags
