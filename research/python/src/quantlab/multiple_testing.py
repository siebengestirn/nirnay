"""Multiple-comparison correction helpers for hypothesis search."""
from __future__ import annotations

import math
from collections.abc import Sequence


def holm_adjust(p_values: Sequence[float]) -> list[float]:
    """Return Holm-Bonferroni adjusted p-values in original input order.

    The adjustment controls family-wise error under arbitrary dependence.
    It does not repair a misspecified test, data snooping outside the declared
    family, or repeated reuse of a nominal holdout set.
    """
    values = [float(value) for value in p_values]
    if any(not math.isfinite(value) or value < 0.0 or value > 1.0 for value in values):
        raise ValueError("p-values must be finite and between zero and one")
    count = len(values)
    order = sorted(range(count), key=lambda index: (values[index], index))
    adjusted = [0.0] * count
    running_max = 0.0
    for rank, original_index in enumerate(order):
        candidate = min(1.0, (count - rank) * values[original_index])
        running_max = max(running_max, candidate)
        adjusted[original_index] = running_max
    return adjusted
