"""Multiple-testing diagnostics for systematic strategy search.

Implements combinatorially symmetric cross-validation (CSCV) as an estimate of
the Probability of Backtest Overfitting (PBO), plus a documented approximate
Deflated Sharpe diagnostic. These are research diagnostics, not proof of alpha.
The former flat ``quantlab.validation`` module is re-exported here unchanged.
"""
from __future__ import annotations

import itertools
import math
from statistics import NormalDist
from typing import Sequence

from .leakage_detection import assert_no_holdout_leakage, detect_holdout_leakage

_EULER_GAMMA = 0.5772156649015329
_NORMAL = NormalDist()


def _mean(values: Sequence[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def _sample_std(values: Sequence[float]) -> float:
    if len(values) < 2:
        return 0.0
    mu = _mean(values)
    return math.sqrt(sum((x - mu) ** 2 for x in values) / (len(values) - 1))


def _sharpe(values: Sequence[float]) -> float:
    sd = _sample_std(values)
    return 0.0 if sd <= 1e-15 else _mean(values) / sd


def cscv_pbo(candidate_returns: Sequence[Sequence[float]], *, blocks: int = 8,
             max_combinations: int = 1000) -> dict[str, object]:
    """Estimate PBO using equal contiguous blocks and symmetric IS/OOS splits.

    `candidate_returns[candidate][observation]` must contain aligned net returns
    from the development sample only (never the final holdout). Each split uses
    half the blocks in-sample and the complement out-of-sample. A split counts
    toward PBO when the in-sample winner ranks below the OOS median.
    """
    matrix = [list(row) for row in candidate_returns]
    if len(matrix) < 2:
        raise ValueError("CSCV requires at least two candidate return series")
    if type(blocks) is not int or blocks < 6 or blocks % 2:
        raise ValueError("CSCV blocks must be an even integer >= 6")
    if type(max_combinations) is not int or max_combinations < 1:
        raise ValueError("max_combinations must be positive")
    length = len(matrix[0])
    if length < blocks * 2 or any(len(row) != length for row in matrix):
        raise ValueError("candidate return series must have equal length >= 2 * blocks")
    if any(not math.isfinite(x) for row in matrix for x in row):
        raise ValueError("CSCV returns must all be finite")
    block_size = length // blocks
    usable = block_size * blocks
    # Discard only the final remainder to preserve equal contiguous blocks.
    index_blocks = [list(range(i * block_size, (i + 1) * block_size)) for i in range(blocks)]
    splits = list(itertools.combinations(range(blocks), blocks // 2))
    if len(splits) > max_combinations:
        raise ValueError(
            f"CSCV has {len(splits)} splits, above max_combinations={max_combinations}"
        )
    lambdas: list[float] = []
    below_median = 0
    for in_blocks in splits:
        in_set = set(in_blocks)
        in_indices = [idx for block in in_blocks for idx in index_blocks[block]]
        out_indices = [
            idx for block in range(blocks) if block not in in_set for idx in index_blocks[block]
        ]
        in_scores = [_sharpe([row[i] for i in in_indices]) for row in matrix]
        winner = max(range(len(matrix)), key=lambda i: (in_scores[i], -i))
        out_scores = [_sharpe([row[i] for i in out_indices]) for row in matrix]
        # Mid-rank ties; normalize strictly inside (0,1) for finite logit.
        value = out_scores[winner]
        less = sum(x < value for x in out_scores)
        equal = sum(x == value for x in out_scores)
        rank = less + (equal + 1) / 2.0  # worst=1, best=N, average tie rank
        omega = min(1 - 1e-12, max(1e-12, rank / (len(matrix) + 1.0)))
        lam = math.log(omega / (1.0 - omega))
        lambdas.append(lam)
        if lam < 0:
            below_median += 1
    return {
        "method": "CSCV_PBO",
        "pbo_estimate": below_median / len(lambdas),
        "splits": len(lambdas),
        "blocks": blocks,
        "block_size": block_size,
        "observations_used": usable,
        "observations_omitted_from_tail": length - usable,
        "candidate_count": len(matrix),
        "interpretation": "fraction of symmetric splits whose in-sample winner ranks "
                          "below the OOS median",
        "lambdas": [round(x, 12) for x in lambdas],
        "uses_final_holdout": False,
    }


def deflated_sharpe_ratio(returns: Sequence[float], *, trials: int) -> dict[str, float | int | str]:
    """Approximate DSR probability against the expected maximum Sharpe under null.

    Uses sample skewness/kurtosis and the standard expected-maximum-normal
    approximation. Returns are per-period and must be net of configured costs.
    This approximation is unsuitable as a sole acceptance gate, especially
    with serial dependence or a highly adaptive/non-independent trial set.
    """
    values = list(returns)
    if type(trials) is not int or trials < 1:
        raise ValueError("trials must be a positive integer")
    if len(values) < 3 or any(not math.isfinite(x) for x in values):
        raise ValueError("DSR requires at least three finite net returns")
    mean = _mean(values)
    sd = _sample_std(values)
    if sd <= 1e-15:
        return {"probability": 0.5, "observed_sharpe_per_period": 0.0,
                "expected_max_sharpe_per_period": 0.0, "trials": trials,
                "method": "approximate_DSR"}
    sr = mean / sd
    centered = [x - mean for x in values]
    m2 = sum(x * x for x in centered) / len(values)
    m3 = sum(x ** 3 for x in centered) / len(values)
    m4 = sum(x ** 4 for x in centered) / len(values)
    skew = m3 / (m2 ** 1.5) if m2 > 0 else 0.0
    kurtosis = m4 / (m2 * m2) if m2 > 0 else 3.0  # non-excess kurtosis
    n = len(values)
    variance_sr = (1.0 - skew * sr + ((kurtosis - 1.0) / 4.0) * sr * sr) / max(1, n - 1)
    sigma_sr = math.sqrt(max(variance_sr, 1e-16))
    if trials == 1:
        expected_max = 0.0
    else:
        p1 = min(1 - 1e-12, max(1e-12, 1 - 1 / trials))
        p2 = min(1 - 1e-12, max(1e-12, 1 - 1 / (trials * math.e)))
        expected_max = sigma_sr * ((1 - _EULER_GAMMA) * _NORMAL.inv_cdf(p1)
                                   + _EULER_GAMMA * _NORMAL.inv_cdf(p2))
    z = (sr - expected_max) / sigma_sr
    probability = min(1.0, max(0.0, _NORMAL.cdf(z)))
    return {
        "probability": probability,
        "observed_sharpe_per_period": sr,
        "expected_max_sharpe_per_period": expected_max,
        "trials": trials,
        "method": "approximate_DSR_expected_max_normal",
    }


__all__ = [
    "assert_no_holdout_leakage",
    "cscv_pbo",
    "deflated_sharpe_ratio",
    "detect_holdout_leakage",
]
