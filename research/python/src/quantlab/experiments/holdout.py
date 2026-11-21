"""Holdout identity, geometry checks and claim-kind selection.

Pure helpers only: the atomic UNIQUE insert lives in the store. This module
never performs I/O so the same rules can be applied by the CLI, the registry
and the comparison command.
"""
from __future__ import annotations

from typing import Any

from .schema import ExperimentError, HoldoutClaim, HoldoutReuseError, holdout_key


def holdout_key_for(record: dict[str, Any]) -> str:
    """Recompute a record's holdout identity from its declaring fields."""
    try:
        return holdout_key(record["dataset_version"], record["universe_id"],
                           record["feature_version"], record["validation_period"],
                           record["holdout_period"], record.get("universe_version"),
                           record.get("feature_digest"))
    except KeyError as exc:
        raise ExperimentError(f"holdout declaration is missing {exc}") from exc


def assert_holdout_key(record: dict[str, Any]) -> None:
    """Refuse a record whose stored holdout_key was altered."""
    if holdout_key_for(record) != record.get("holdout_key"):
        raise ExperimentError("holdout_key does not match its declaring inputs")


def claim_kind_for(record: dict[str, Any]) -> str:
    """Map the declared holdout_use to the atomic claim kind."""
    use = record.get("holdout_use", "selection")
    if use not in {"selection", "verification_only"}:
        raise ExperimentError("holdout_use must be selection or verification_only")
    return use


def validate_claim(record: dict[str, Any], *, existing_selection: HoldoutClaim | None,
                   replicates: str | None = None) -> str:
    """Apply holdout discipline before a UNIQUE insert is attempted.

    A selection claim is consumed once. A verification-only claim is always
    allowed but must name the experiment it replicates when a selection claim
    already exists.
    """
    kind = claim_kind_for(record)
    declared_replicates = replicates if replicates is not None else record.get("replicates")
    if kind == "selection":
        if existing_selection is not None:
            raise HoldoutReuseError(
                f"holdout already claimed for selection by {existing_selection.experiment_id}"
            )
        if declared_replicates is not None:
            raise ExperimentError("a selection claim must not name replicates")
        return kind
    if not isinstance(declared_replicates, str) or not declared_replicates.strip():
        raise ExperimentError("verification_only claims must name replicates")
    if existing_selection is not None and declared_replicates != existing_selection.experiment_id:
        raise ExperimentError("verification_only replicates must name the selection claim owner")
    return kind
