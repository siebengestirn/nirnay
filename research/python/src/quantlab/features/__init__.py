"""Feature-lineage package.

Re-exports the declarative lineage contract; no feature computation library is
claimed here. The definition digest helper is the tested public entry point for
declaring a feature-generation version.
"""
from __future__ import annotations

import hashlib

from .lineage import (FeatureLineage, assert_feature_lineage, feature_lineage_digest,
                      feature_lineage_from_dict, validate_feature_lineage)


def feature_definition_digest(source: bytes) -> str:
    """Digest the exact bytes of a feature definition for lineage declaration."""
    if not isinstance(source, bytes) or not source:
        raise ValueError("feature definition source must be non-empty bytes")
    return hashlib.sha256(source).hexdigest()


__all__ = [
    "FeatureLineage",
    "assert_feature_lineage",
    "feature_definition_digest",
    "feature_lineage_digest",
    "feature_lineage_from_dict",
    "validate_feature_lineage",
]
