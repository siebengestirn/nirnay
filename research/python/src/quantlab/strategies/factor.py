"""Cross-sectional factor weights from synthetic scores.

Scores are supplied by the caller; this module does not source or certify any
factor data. Weights are long/short, net-flat and normalized to an explicitly
declared gross budget so a portfolio cannot silently change its size.
"""
from __future__ import annotations

import math
from collections.abc import Mapping
from typing import Any


def factor_weights(scores: Mapping[str, float], *, budget: float = 1.0,
                   tolerance: float = 1e-9) -> dict[str, float]:
    """Rank scores into long/short legs and normalize to the gross budget.

    Ties break by instrument identifier for deterministic output. With an odd
    number of instruments the middle rank receives zero weight.
    """
    if not isinstance(scores, Mapping) or len(scores) < 2:
        raise ValueError("factor scores must map at least two instruments")
    if isinstance(budget, bool) or not isinstance(budget, (int, float)):
        raise ValueError("budget must be a real number")
    if not math.isfinite(float(budget)) or budget <= 0:
        raise ValueError("budget must be positive and finite")
    if isinstance(tolerance, bool) or not isinstance(tolerance, (int, float)):
        raise ValueError("tolerance must be a real number")
    if not math.isfinite(float(tolerance)) or tolerance <= 0:
        raise ValueError("tolerance must be positive and finite")
    ordered: list[tuple[str, float]] = []
    for name, value in scores.items():
        if not isinstance(name, str) or not name:
            raise ValueError("instrument identifiers must be non-empty strings")
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError("factor scores must be real numbers")
        if not math.isfinite(float(value)):
            raise ValueError("factor scores must be finite")
        ordered.append((name, float(value)))
    ordered.sort(key=lambda item: (-item[1], item[0]))
    half = len(ordered) // 2
    raw: dict[str, int] = {}
    for rank, (name, _) in enumerate(ordered):
        if rank < half:
            raw[name] = 1
        elif rank >= len(ordered) - half:
            raw[name] = -1
        else:
            raw[name] = 0
    gross = sum(abs(value) for value in raw.values())
    scale = float(budget) / gross
    weights = {name: raw[name] * scale for name in sorted(raw)}
    assert_factor_budget(weights, budget, tolerance=tolerance)
    return weights


def assert_factor_budget(weights: Mapping[str, Any], budget: float, *,
                         tolerance: float = 1e-9) -> None:
    """Refuse a factor portfolio whose gross exposure leaves its budget."""
    if not isinstance(weights, Mapping) or not weights:
        raise ValueError("factor weights must be a non-empty object")
    if isinstance(budget, bool) or not isinstance(budget, (int, float)):
        raise ValueError("budget must be a real number")
    if not math.isfinite(float(budget)) or budget <= 0:
        raise ValueError("budget must be positive and finite")
    values: list[float] = []
    for name, value in weights.items():
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError("factor weights must be real numbers")
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("factor weights must be finite")
        values.append(number)
    gross = sum(abs(number) for number in values)
    if not math.isclose(gross, float(budget), rel_tol=0.0, abs_tol=float(tolerance)):
        raise ValueError("factor weights do not reconcile to the declared gross budget")
    if not math.isclose(sum(values), 0.0, rel_tol=0.0, abs_tol=float(tolerance)):
        raise ValueError("factor weights are not net-flat")
