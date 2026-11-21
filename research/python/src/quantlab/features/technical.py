"""Pure, causal technical features over synthetic OHLCV bars.

Every indicator at bar index ``t`` uses only bars with index ``<= t``. Rolling
windows are trailing; appending a later bar cannot change an earlier output.
These are research statistics, not trading signals or alpha claims.
"""
from __future__ import annotations

from typing import Sequence

from ._common import (FeatureBatch, FeatureRow, OhlcvBar, closes_from_bars,
                      make_batch, require_finite, require_window, sample_std)

FEATURE_VERSION = "technical.v1"
_FAMILY = "technical"


def sma(values: Sequence[float], window: int) -> list[float | None]:
    """Trailing simple moving average, aligned to the input series."""
    series = require_finite(values)
    require_window(window, "window")
    result: list[float | None] = [None] * len(series)
    running = 0.0
    for index, value in enumerate(series):
        running += value
        if index >= window:
            running -= series[index - window]
        if index >= window - 1:
            result[index] = running / window
    return result


def ema(values: Sequence[float], span: int) -> list[float]:
    """Recursive exponential moving average seeded by the first observation."""
    series = require_finite(values)
    require_window(span, "span")
    alpha = 2.0 / (span + 1.0)
    result = [series[0]]
    for value in series[1:]:
        result.append(alpha * value + (1.0 - alpha) * result[-1])
    return result


def roc(values: Sequence[float], period: int) -> list[float | None]:
    """Trailing rate of change as a simple fractional return."""
    series = require_finite(values)
    require_window(period, "period")
    result: list[float | None] = [None] * len(series)
    for index in range(period, len(series)):
        result[index] = series[index] / series[index - period] - 1.0
    return result


def rolling_zscore(values: Sequence[float], window: int) -> list[float | None]:
    """Trailing z-score using the unbiased sample standard deviation."""
    series = require_finite(values)
    require_window(window, "window")
    result: list[float | None] = [None] * len(series)
    for index in range(window - 1, len(series)):
        trailing = series[index - window + 1:index + 1]
        sigma = sample_std(trailing)
        result[index] = (series[index] - sum(trailing) / window) / sigma if sigma > 0 else 0.0
    return result


def compute_technical(
    bars: Sequence[OhlcvBar],
    *,
    window: int = 5,
    fast: int = 3,
    slow: int = 10,
    knowledge_utc_ns: int | None = None,
) -> FeatureBatch:
    """Compute one aligned batch of causal technical features."""
    require_window(slow, "slow")
    if fast >= slow:
        raise ValueError("fast span must be smaller than slow span")
    closes = closes_from_bars(bars)
    averages = sma(closes, window)
    fast_ema = ema(closes, fast)
    slow_ema = ema(closes, slow)
    momentum = roc(closes, window)
    zscores = rolling_zscore(closes, window)
    first = max(window, slow)
    rows: list[FeatureRow] = []
    for index in range(first, len(bars)):
        rows.append(FeatureRow(
            timestamp_ns=bars[index].timestamp_ns,
            values={
                "sma": averages[index],
                "ema_fast": fast_ema[index],
                "ema_slow": slow_ema[index],
                "ema_spread": fast_ema[index] - slow_ema[index],
                "roc": momentum[index],
                "zscore": zscores[index],
            },
        ))
    return make_batch(
        FEATURE_VERSION,
        rows,
        family=_FAMILY,
        knowledge_utc_ns=knowledge_utc_ns,
        declared_config={"window": window, "fast": fast, "slow": slow},
    )
