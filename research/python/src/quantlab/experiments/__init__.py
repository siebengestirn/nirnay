"""Experiment registry, registered search and split-geometry package.

The original validation and hashing API is preserved. The registry contract,
the registered grid-search adapter and the walk-forward declaration adapter
share the same holdout and complete-accounting rules; none claims alpha.
"""
from __future__ import annotations

from .parameter_search import CLAIM_BOUNDARY, generate_configs, grid_search
from .registry import (DuplicateExperimentError, ExperimentError, ExperimentRegistry,
                       HoldoutClaim, HoldoutReuseError, Receipt, StoredRecord, append_result,
                       assert_holdout_key, build_search_space, claim_kind_for, content_hash,
                       holdout_key, holdout_key_for, make_attempt, register, validate_claim,
                       validate_experiment, validate_registered_experiment, verify_integrity)
from .schema import (canonical_json, freeze_experiment, freeze_registered_experiment,
                     load_experiment, recompute_report_hash, validate_complete_trial_accounting,
                     validate_period_geometry)
from .walk_forward import (WalkForwardDeclaration, build_walk_forward_declarations,
                           search_space_for_declaration, validate_split_geometry)

__all__ = [
    "CLAIM_BOUNDARY",
    "DuplicateExperimentError",
    "ExperimentError",
    "ExperimentRegistry",
    "HoldoutClaim",
    "HoldoutReuseError",
    "Receipt",
    "StoredRecord",
    "WalkForwardDeclaration",
    "append_result",
    "assert_holdout_key",
    "build_search_space",
    "build_walk_forward_declarations",
    "canonical_json",
    "claim_kind_for",
    "content_hash",
    "freeze_experiment",
    "freeze_registered_experiment",
    "generate_configs",
    "grid_search",
    "holdout_key",
    "holdout_key_for",
    "load_experiment",
    "make_attempt",
    "recompute_report_hash",
    "search_space_for_declaration",
    "register",
    "validate_claim",
    "validate_complete_trial_accounting",
    "validate_experiment",
    "validate_period_geometry",
    "validate_registered_experiment",
    "validate_split_geometry",
    "verify_integrity",
]
