"""Pure return-based research risk measures.

Floats are used because these are statistical summaries, not money accounting.
Every public entry point rejects non-finite inputs before computing a result.
"""
from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from typing import Any

from ..experiments.schema import content_hash


def _finite_returns(returns: Sequence[float]) -> list[float]:
    if isinstance(returns, (str, bytes)) or not returns:
        raise ValueError("returns must be a non-empty sequence")
    values: list[float] = []
    for value in returns:
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError("returns must be real numbers")
        number = float(value)
        if not math.isfinite(number) or number < -1.0:
            raise ValueError("returns must be finite and not below -100%")
        values.append(number)
    return values


def worst_period(returns: Sequence[float]) -> float:
    """Return the most negative simple return in the sample."""
    return min(_finite_returns(returns))


def max_drawdown(returns: Sequence[float]) -> float:
    """Return the deepest peak-to-trough wealth decline in the sample."""
    wealth = 1.0
    peak = 1.0
    deepest = 0.0
    for value in _finite_returns(returns):
        wealth *= 1.0 + value
        peak = max(peak, wealth)
        deepest = max(deepest, 1.0 - wealth / peak)
    return deepest


def exposure_concentration(exposures: Mapping[str, float]) -> float:
    """Return the normalized Herfindahl index of absolute exposures."""
    if not isinstance(exposures, Mapping) or not exposures:
        raise ValueError("exposures must be a non-empty mapping")
    magnitudes: list[float] = []
    for name, value in exposures.items():
        if not isinstance(name, str) or not name:
            raise ValueError("exposure names must be non-empty strings")
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError("exposures must be real numbers")
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("exposures must be finite")
        magnitudes.append(abs(number))
    total = sum(magnitudes)
    if total <= 0:
        raise ValueError("exposures must have a positive gross magnitude")
    return sum((value / total) ** 2 for value in magnitudes)


def risk_report(returns: Sequence[float],
                exposures: Mapping[str, float]) -> dict[str, Any]:
    """Build a deterministic, claim-limited risk report with a digest."""
    values = _finite_returns(returns)
    report: dict[str, Any] = {
        "schema_version": 1,
        "observations": len(values),
        "worst_period_return": worst_period(values),
        "max_drawdown": max_drawdown(values),
        "exposure_concentration": exposure_concentration(exposures),
        "claim_boundary": "research_screening_only_not_alpha_certification",
        "queue_truth_claim": "unavailable",
    }
    report["report_sha256"] = content_hash(report)
    return report
