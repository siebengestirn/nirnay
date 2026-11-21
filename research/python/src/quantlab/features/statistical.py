"""Pure, causal statistical features over synthetic numeric series.

Rolling autocorrelation, skewness and kurtosis are computed from trailing
windows only. Inputs are research statistics supplied by the caller; they are
checked for finiteness and never read from a provider or live service.
"""
from __future__ import annotations

from typing import Sequence

from ._common import (FeatureBatch, FeatureRow, OhlcvBar, closes_from_bars,
                      make_batch, mean, require_finite, require_window, sample_std)

FEATURE_VERSION = "statistical.v1"
_FAMILY = "statistical"


def autocorrelation(values: Sequence[float], lag: int,
                    window: int) -> list[float | None]:
    """Trailing Pearson autocorrelation at a fixed positive lag."""
    series = require_finite(values)
    require_window(lag, "lag", minimum=1)
    require_window(window, "window", minimum=3)
    if window - lag < 3:
        raise ValueError("window must leave at least three lagged pairs")
    result: list[float | None] = [None] * len(series)
    for index in range(window - 1, len(series)):
        prior = series[index - window + 1:index - lag + 1]
        later = series[index - window + 1 + lag:index + 1]
        prior_mean, later_mean = mean(prior), mean(later)
        covariance = sum(
            (left - prior_mean) * (right - later_mean)
            for left, right in zip(prior, later)
        ) / len(prior)
        sigma = sample_std(prior) * sample_std(later)
        result[index] = covariance / sigma if sigma > 0 else 0.0
    return result


def rolling_skew(values: Sequence[float], window: int) -> list[float | None]:
    """Trailing standardized third central moment (population basis)."""
    series = require_finite(values)
    require_window(window, "window", minimum=3)
    result: list[float | None] = [None] * len(series)
    for index in range(window - 1, len(series)):
        trailing = series[index - window + 1:index + 1]
        mu = mean(trailing)
        second = sum((value - mu) ** 2 for value in trailing) / window
        third = sum((value - mu) ** 3 for value in trailing) / window
        result[index] = third / (second ** 1.5) if second > 0 else 0.0
    return result


def rolling_kurtosis(values: Sequence[float], window: int, *,
                     excess: bool = True) -> list[float | None]:
    """Trailing fourth central moment, excess by default."""
    series = require_finite(values)
    require_window(window, "window", minimum=4)
    result: list[float | None] = [None] * len(series)
    for index in range(window - 1, len(series)):
        trailing = series[index - window + 1:index + 1]
        mu = mean(trailing)
        second = sum((value - mu) ** 2 for value in trailing) / window
        fourth = sum((value - mu) ** 4 for value in trailing) / window
        value = fourth / (second * second) if second > 0 else 0.0
        result[index] = value - 3.0 if excess else value
    return result


def compute_statistical(
    bars: Sequence[OhlcvBar],
    *,
    window: int = 20,
    lag: int = 1,
    knowledge_utc_ns: int | None = None,
) -> FeatureBatch:
    """Compute one aligned batch of causal statistical features."""
    if window - lag < 4:
        raise ValueError("window must leave at least four lagged pairs for kurtosis")
    closes = closes_from_bars(bars)
    autocorrelation_values = autocorrelation(closes, lag, window)
    skew_values = rolling_skew(closes, window)
    kurtosis_values = rolling_kurtosis(closes, window)
    rows: list[FeatureRow] = []
    for index in range(window - 1, len(bars)):
        rows.append(FeatureRow(
            timestamp_ns=bars[index].timestamp_ns,
            values={
                "autocorrelation": autocorrelation_values[index],
                "skew": skew_values[index],
                "excess_kurtosis": kurtosis_values[index],
            },
        ))
    return make_batch(
        FEATURE_VERSION,
        rows,
        family=_FAMILY,
        knowledge_utc_ns=knowledge_utc_ns,
        declared_config={"window": window, "lag": lag},
    )
