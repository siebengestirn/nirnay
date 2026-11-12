"""Small, explicit return metrics for research reports.

These routines use floating point because they are statistical summaries, not
money accounting. Order/fee/cash accounting stays in the Rust domain model.
"""
from __future__ import annotations

import math
from collections.abc import Sequence


def summarize_returns(returns: Sequence[float], periods_per_year: int) -> dict[str, float | int]:
    if periods_per_year <= 0:
        raise ValueError("periods_per_year must be positive")
    if not returns:
        raise ValueError("returns must not be empty")
    values = [float(value) for value in returns]
    if any(not math.isfinite(value) for value in values):
        raise ValueError("returns must all be finite")
    if any(value < -1.0 for value in values):
        raise ValueError("a simple return cannot be below -100%")

    wealth = 1.0
    peak = 1.0
    max_drawdown = 0.0
    for value in values:
        wealth *= 1.0 + value
        peak = max(peak, wealth)
        if peak > 0.0:
            max_drawdown = max(max_drawdown, 1.0 - wealth / peak)

    mean = sum(values) / len(values)
    if len(values) > 1:
        variance = sum((value - mean) ** 2 for value in values) / (len(values) - 1)
        std = math.sqrt(variance)
    else:
        std = 0.0
    sharpe = math.sqrt(periods_per_year) * mean / std if std > 0.0 else 0.0
    years = len(values) / periods_per_year
    cagr = wealth ** (1.0 / years) - 1.0 if wealth > 0.0 and years > 0.0 else -1.0
    return {
        "observations": len(values),
        "cumulative_return": wealth - 1.0,
        "cagr": cagr,
        "max_drawdown": max_drawdown,
        "annualized_sharpe": sharpe,
    }
