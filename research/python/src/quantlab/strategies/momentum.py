"""Public momentum signal, promoted verbatim from the bounded alpha screen.

The function is pure and causal: the value at index ``t`` depends only on
closes with index ``<= t``. It emits a research position indicator, not an
order, a profitability claim or an alpha certification.
"""
from __future__ import annotations

import math
from typing import Sequence


def momentum_signal(closes: Sequence[float], *, lookback: int,
                    threshold_bps: float = 0.0) -> list[int]:
    """Return one long/flat indicator per close using a trailing return."""
    if type(lookback) is not int or lookback < 2:
        raise ValueError("lookback must be an integer >= 2")
    if isinstance(threshold_bps, bool) or not isinstance(threshold_bps, (int, float)):
        raise ValueError("threshold_bps must be a real number")
    if not math.isfinite(float(threshold_bps)):
        raise ValueError("threshold_bps must be finite")
    size = len(closes)
    output = [0] * size
    for index in range(lookback, size):
        change_bps = (closes[index] / closes[index - lookback] - 1.0) * 10_000
        output[index] = 1 if change_bps > threshold_bps else 0
    return output
