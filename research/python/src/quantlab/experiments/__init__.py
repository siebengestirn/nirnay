"""Experiment registry package.

The flat ``quantlab.experiments`` module was moved here verbatim as a
compatibility shim: the original validation and hashing API is unchanged and
the registry contract extends it without invalidating existing records.
"""
from __future__ import annotations

from .registry import (DuplicateExperimentError, ExperimentError, ExperimentRegistry,
                       HoldoutClaim, HoldoutReuseError, Receipt, StoredRecord, append_result,
                       assert_holdout_key, build_search_space, claim_kind_for, content_hash,
                       holdout_key, holdout_key_for, make_attempt, register, validate_claim,
                       validate_experiment, validate_registered_experiment, verify_integrity)
from .schema import (canonical_json, freeze_experiment, freeze_registered_experiment,
                     load_experiment, recompute_report_hash, validate_complete_trial_accounting)

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
    "canonical_json",
    "claim_kind_for",
    "content_hash",
    "freeze_experiment",
    "freeze_registered_experiment",
    "holdout_key",
    "holdout_key_for",
    "load_experiment",
    "make_attempt",
    "recompute_report_hash",
    "register",
    "validate_claim",
    "validate_complete_trial_accounting",
    "validate_experiment",
    "validate_registered_experiment",
    "verify_integrity",
]
