"""Public mean-reversion signal, promoted verbatim from the alpha screen.

The value at index ``t`` uses only the trailing close window ending at ``t-1``
and the close at ``t``. It is a research position indicator only.
"""
from __future__ import annotations

import math
from typing import Sequence


def mean_reversion_signal(closes: Sequence[float], *, lookback: int,
                          threshold_bps: float = 0.0) -> list[int]:
    """Return one long/flat indicator per close using a trailing z-score."""
    if type(lookback) is not int or lookback < 2:
        raise ValueError("lookback must be an integer >= 2")
    if isinstance(threshold_bps, bool) or not isinstance(threshold_bps, (int, float)):
        raise ValueError("threshold_bps must be a real number")
    if not math.isfinite(float(threshold_bps)):
        raise ValueError("threshold_bps must be finite")
    size = len(closes)
    output = [0] * size
    for index in range(lookback, size):
        window = closes[index - lookback:index]
        window_mean = sum(window) / len(window)
        variance = sum((value - window_mean) ** 2 for value in window) / max(
            1, len(window) - 1
        )
        sigma = math.sqrt(variance)
        zscore = (closes[index] - window_mean) / sigma if sigma > 0 else 0.0
        output[index] = 1 if zscore < -abs(threshold_bps) / 100.0 else 0
    return output
