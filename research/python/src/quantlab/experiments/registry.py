"""Public registry facade.

The facade keeps the small call surface used by the CLI and comparison tool
while the implementation stays split into schema, holdout, store and verify
modules with independently tested behaviour.
"""
from __future__ import annotations

from typing import Any

from .holdout import assert_holdout_key, claim_kind_for, holdout_key_for, validate_claim
from .schema import (ExperimentError, HoldoutClaim, HoldoutReuseError, Receipt,
                     build_search_space, content_hash, holdout_key, make_attempt,
                     validate_experiment, validate_registered_experiment)
from .store import DuplicateExperimentError, ExperimentRegistry, StoredRecord


def register(registry: ExperimentRegistry, record: dict[str, Any]) -> Receipt:
    """Register a frozen experiment and claim its holdout."""
    return registry.register(record)


def append_result(registry: ExperimentRegistry, record: dict[str, Any]) -> Receipt:
    """Append a terminal result to an already registered experiment."""
    return registry.append_result(record)


def verify_integrity(registry: ExperimentRegistry, **kwargs: Any) -> dict[str, Any]:
    """Verify the registry's internal integrity without mutating it."""
    return registry.verify_integrity(**kwargs)


__all__ = [
    "DuplicateExperimentError",
    "ExperimentError",
    "ExperimentRegistry",
    "HoldoutClaim",
    "HoldoutReuseError",
    "Receipt",
    "StoredRecord",
    "append_result",
    "assert_holdout_key",
    "build_search_space",
    "claim_kind_for",
    "content_hash",
    "holdout_key",
    "holdout_key_for",
    "make_attempt",
    "register",
    "validate_claim",
    "validate_experiment",
    "validate_registered_experiment",
    "verify_integrity",
]
