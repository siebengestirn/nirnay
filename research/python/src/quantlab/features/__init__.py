"""Feature lineage and pure, causal feature computation.

The declarative lineage contract is re-exported alongside the tested entry
points for declaring a feature-generation version. The technical, volatility
and statistical modules compute deterministic research features from synthetic
bars; they claim no alpha, data rights or live readiness.
"""
from __future__ import annotations

import hashlib

from ._common import (FeatureBatch, FeatureRow, OhlcvBar, batch_as_dict,
                      definition_digest)
from .lineage import (FeatureLineage, assert_feature_lineage, feature_lineage_digest,
                      feature_lineage_from_dict, validate_feature_lineage)
from .statistical import compute_statistical
from .technical import compute_technical
from .volatility import compute_volatility


def feature_definition_digest(source: bytes) -> str:
    """Digest the exact bytes of a feature definition for lineage declaration."""
    if not isinstance(source, bytes) or not source:
        raise ValueError("feature definition source must be non-empty bytes")
    return hashlib.sha256(source).hexdigest()


__all__ = [
    "FeatureBatch",
    "FeatureLineage",
    "FeatureRow",
    "OhlcvBar",
    "assert_feature_lineage",
    "batch_as_dict",
    "compute_statistical",
    "compute_technical",
    "compute_volatility",
    "definition_digest",
    "feature_definition_digest",
    "feature_lineage_digest",
    "feature_lineage_from_dict",
    "validate_feature_lineage",
]
