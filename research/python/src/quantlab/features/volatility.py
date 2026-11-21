"""Pure, causal volatility features over synthetic OHLCV bars.

Realized volatility uses trailing close-to-close log returns; the range
estimator uses trailing high/low log ranges. All outputs are research
statistics computed only from bars at or before the row timestamp.
"""
from __future__ import annotations

import math
from typing import Sequence

from ._common import (FeatureBatch, FeatureRow, OhlcvBar, require_finite,
                      require_window, sample_std, make_batch)

FEATURE_VERSION = "volatility.v1"
_FAMILY = "volatility"
_PARKINSON_SCALE = 4.0 * math.log(2.0)


def log_returns(closes: Sequence[float]) -> list[float]:
    """Close-to-close log returns; the first element has no predecessor."""
    series = require_finite(closes)
    if any(value <= 0 for value in series):
        raise ValueError("closes must be positive")
    result = [0.0]
    for index in range(1, len(series)):
        result.append(math.log(series[index] / series[index - 1]))
    return result


def realized_vol(returns: Sequence[float], window: int) -> list[float | None]:
    """Trailing sample standard deviation of log returns."""
    series = require_finite(returns)
    require_window(window, "window", minimum=2)
    result: list[float | None] = [None] * len(series)
    for index in range(window, len(series)):
        result[index] = sample_std(series[index - window:index])
    return result


def parkinson_range(highs: Sequence[float], lows: Sequence[float],
                    window: int) -> list[float | None]:
    """Trailing Parkinson-style range estimator from high/low log ranges."""
    high_values = require_finite(highs)
    low_values = require_finite(lows)
    if len(high_values) != len(low_values):
        raise ValueError("highs and lows must have equal length")
    if any(high <= 0 or low <= 0 for high, low in zip(high_values, low_values)):
        raise ValueError("highs and lows must be positive")
    require_window(window, "window", minimum=2)
    result: list[float | None] = [None] * len(high_values)
    for index in range(window - 1, len(high_values)):
        squares = [
            math.log(high_values[item] / low_values[item]) ** 2
            for item in range(index - window + 1, index + 1)
        ]
        result[index] = math.sqrt(sum(squares) / window / _PARKINSON_SCALE)
    return result


def vol_ratio(returns: Sequence[float], short_window: int,
              long_window: int) -> list[float | None]:
    """Trailing ratio of short-horizon to long-horizon realized volatility."""
    require_window(short_window, "short_window", minimum=2)
    require_window(long_window, "long_window", minimum=2)
    if short_window >= long_window:
        raise ValueError("short_window must be smaller than long_window")
    short = realized_vol(returns, short_window)
    long = realized_vol(returns, long_window)
    result: list[float | None] = []
    for short_value, long_value in zip(short, long):
        if short_value is None or long_value is None or long_value <= 0:
            result.append(None)
        else:
            result.append(short_value / long_value)
    return result


def compute_volatility(
    bars: Sequence[OhlcvBar],
    *,
    window: int = 5,
    short_window: int = 5,
    long_window: int = 20,
    knowledge_utc_ns: int | None = None,
) -> FeatureBatch:
    """Compute one aligned batch of causal volatility features."""
    if not bars:
        raise ValueError("at least one bar is required")
    closes = [float(bar.close) for bar in bars]
    highs = [float(bar.high) for bar in bars]
    lows = [float(bar.low) for bar in bars]
    returns = log_returns(closes)
    realized = realized_vol(returns, window)
    parkinson = parkinson_range(highs, lows, window)
    ratio = vol_ratio(returns, short_window, long_window)
    first = max(window, long_window)
    rows: list[FeatureRow] = []
    for index in range(first, len(bars)):
        rows.append(FeatureRow(
            timestamp_ns=bars[index].timestamp_ns,
            values={
                "realized_vol": realized[index],
                "parkinson_range": parkinson[index],
                "vol_ratio": ratio[index],
            },
        ))
    return make_batch(
        FEATURE_VERSION,
        rows,
        family=_FAMILY,
        knowledge_utc_ns=knowledge_utc_ns,
        declared_config={"window": window, "short_window": short_window,
                         "long_window": long_window},
    )
