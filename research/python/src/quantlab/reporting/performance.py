"""Return-based performance reporting with a recomputable report digest.

All figures are research statistics in floating point; money and cash
accounting remain in the exact Rust domain model. Non-finite inputs are refused
before any digest is taken.
"""
from __future__ import annotations

import math
from collections.abc import Sequence
from typing import Any

from ..experiments.schema import content_hash
from ..metrics import summarize_returns


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


def drawdown_duration(returns: Sequence[float]) -> dict[str, float | int]:
    """Return longest and current periods spent below the running wealth peak."""
    values = _finite_returns(returns)
    wealth = 1.0
    peak = 1.0
    current = 0
    longest = 0
    max_drawdown = 0.0
    for value in values:
        wealth *= 1.0 + value
        peak = max(peak, wealth)
        if wealth < peak:
            current += 1
            longest = max(longest, current)
            max_drawdown = max(max_drawdown, 1.0 - wealth / peak)
        else:
            current = 0
    return {"observations": len(values), "longest_drawdown_periods": longest,
            "current_drawdown_periods": current, "max_drawdown": max_drawdown}


def performance_report(returns: Sequence[float], periods_per_year: int) -> dict[str, Any]:
    """Build a deterministic performance report with a verifiable digest."""
    if type(periods_per_year) is not int or periods_per_year <= 0:
        raise ValueError("periods_per_year must be a positive integer")
    values = _finite_returns(returns)
    summary = summarize_returns(values, periods_per_year)
    report: dict[str, Any] = {
        "schema_version": 1,
        "periods_per_year": periods_per_year,
        "summary": summary,
        "drawdown": drawdown_duration(values),
        "claim_boundary": "research_screening_only_not_alpha_certification",
    }
    report["report_sha256"] = content_hash(report)
    return report
