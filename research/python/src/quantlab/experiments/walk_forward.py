"""Adapter from chronological expanding splits to registered period geometry.

The real split utility lives in ``quantlab.walk_forward``. This module converts
its output into declarations accepted by ``validate_period_geometry`` and
refuses any split that overlaps the holdout or omits the declared embargo.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ..walk_forward import WalkForwardSplit, expanding_splits
from .schema import ExperimentError, build_search_space, content_hash, validate_period_geometry


@dataclass(frozen=True, slots=True)
class WalkForwardDeclaration:
    """One validated expanding split with its declared holdout boundary."""

    split_id: int
    train_period: dict[str, int]
    validation_period: dict[str, int]
    holdout_period: dict[str, int]
    embargo_observations: int

    def as_dict(self) -> dict[str, Any]:
        """Return a canonical JSON-safe view of this declaration."""
        return {
            "split_id": self.split_id,
            "train_period": dict(self.train_period),
            "validation_period": dict(self.validation_period),
            "holdout_period": dict(self.holdout_period),
            "embargo_observations": self.embargo_observations,
        }


def validate_split_geometry(split: WalkForwardSplit, holdout_period: dict[str, int],
                            embargo: int) -> None:
    """Refuse a split whose test window touches the holdout or lacks embargo."""
    if not isinstance(holdout_period, dict) or set(holdout_period) != {
        "start_index", "end_index"
    }:
        raise ExperimentError("holdout_period must declare start_index and end_index")
    holdout_start = holdout_period["start_index"]
    if split.test_start < split.train_end + embargo:
        raise ExperimentError("test window omits the declared train/test embargo")
    if split.test_end > holdout_start - embargo:
        raise ExperimentError("test window overlaps or is not embargoed from holdout")
    record = {
        "validation_period": {"start_index": split.test_start, "end_index": split.test_end},
        "holdout_period": dict(holdout_period),
        "search_space": {
            "embargo_observations": embargo,
            "train_period": {"start_index": split.train_start, "end_index": split.train_end},
        },
    }
    validate_period_geometry(record)


def build_walk_forward_declarations(
    observations: int,
    *,
    n_splits: int,
    min_train_fraction: float = 0.5,
    test_fraction: float = 0.1,
    embargo: int = 1,
    holdout_fraction: float = 0.2,
) -> dict[str, Any]:
    """Convert expanding splits into validated, holdout-safe declarations."""
    if type(observations) is not int or observations < 10:
        raise ExperimentError("observations must be an integer >= 10")
    if not 0.0 < holdout_fraction < 1.0:
        raise ExperimentError("holdout_fraction must be between zero and one")
    if type(embargo) is not int or embargo < 0:
        raise ExperimentError("embargo must be a non-negative integer")
    holdout_start = int(observations * (1.0 - holdout_fraction))
    development = holdout_start - embargo
    if development < 10:
        raise ExperimentError("holdout geometry leaves no usable development sample")
    holdout_period = {"start_index": holdout_start, "end_index": observations}
    splits = expanding_splits(
        development,
        n_splits=n_splits,
        min_train_fraction=min_train_fraction,
        test_fraction=test_fraction,
        embargo=embargo,
    )
    declarations: list[WalkForwardDeclaration] = []
    for split in splits:
        validate_split_geometry(split, holdout_period, embargo)
        declarations.append(WalkForwardDeclaration(
            split_id=split.split_id,
            train_period={"start_index": split.train_start, "end_index": split.train_end},
            validation_period={"start_index": split.test_start, "end_index": split.test_end},
            holdout_period=dict(holdout_period),
            embargo_observations=embargo,
        ))
    split_ids_digest = content_hash([
        {"split_id": item.split_id, "train_period": item.train_period,
         "validation_period": item.validation_period}
        for item in declarations
    ])
    return {
        "schema_version": 1,
        "method": "expanding_walk_forward",
        "holdout_period": holdout_period,
        "embargo_observations": embargo,
        "declarations": [item.as_dict() for item in declarations],
        "split_ids_sha256": split_ids_digest,
        "uses_final_holdout": False,
        "warning": "Research geometry only; no out-of-sample performance or alpha claim.",
    }


def search_space_for_declaration(
    declaration: WalkForwardDeclaration,
    family: str,
    configs: list[dict[str, Any]],
) -> dict[str, Any]:
    """Build a registered search space that carries one split's geometry."""
    return build_search_space(
        family,
        configs,
        train_period=dict(declaration.train_period),
        validation_period=dict(declaration.validation_period),
        holdout_period=dict(declaration.holdout_period),
        embargo_observations=declaration.embargo_observations,
        split_ids_sha256=content_hash([declaration.as_dict()]),
    )
