"""Chronological expanding-window splits; never shuffle time-series data."""
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class WalkForwardSplit:
    split_id: int
    train_start: int
    train_end: int
    test_start: int
    test_end: int


def expanding_splits(
    observations: int,
    *,
    n_splits: int,
    min_train_fraction: float = 0.5,
    test_fraction: float = 0.1,
    embargo: int = 0,
) -> list[WalkForwardSplit]:
    """Build expanding train windows followed by disjoint future test windows.

    Indices are half-open `[start, end)`. `embargo` observations are omitted
    between train and test to reduce leakage from overlapping labels/holding
    periods. This utility does not choose features or tune on the final holdout.
    """
    if observations <= 0:
        raise ValueError("observations must be positive")
    if n_splits <= 0:
        raise ValueError("n_splits must be positive")
    if not 0.0 < min_train_fraction < 1.0:
        raise ValueError("min_train_fraction must be between zero and one")
    if not 0.0 < test_fraction < 1.0:
        raise ValueError("test_fraction must be between zero and one")
    if embargo < 0:
        raise ValueError("embargo must not be negative")

    first_train_end = int(observations * min_train_fraction)
    test_size = int(observations * test_fraction)
    if test_size < 1:
        raise ValueError("test_fraction yields an empty test window")
    splits: list[WalkForwardSplit] = []
    train_end = first_train_end
    for split_id in range(n_splits):
        test_start = train_end + embargo
        test_end = test_start + test_size
        if test_end > observations:
            raise ValueError("requested splits do not fit in the observation range")
        splits.append(
            WalkForwardSplit(
                split_id=split_id,
                train_start=0,
                train_end=train_end,
                test_start=test_start,
                test_end=test_end,
            )
        )
        train_end = test_end
    return splits
