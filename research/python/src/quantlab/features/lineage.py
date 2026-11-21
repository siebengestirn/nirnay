"""Declarative feature lineage with as-of/knowledge-time validation.

A feature lineage is data, not a service. It records when a feature definition
became known and the observation time it was computed as of. Both bounds are
checked against the point-in-time event that consumes the row, and a stored
digest detects edits to the declaration itself.
"""
from __future__ import annotations

import hashlib
from dataclasses import asdict, dataclass
from typing import Any

from ..market import canonical_bytes


def _is_sha256(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(
        char in "0123456789abcdef" for char in value
    )


@dataclass(frozen=True, slots=True)
class FeatureLineage:
    feature_version: str
    definition_sha256: str
    as_of_utc_ns: int
    knowledge_utc_ns: int
    declared_config: dict[str, Any]

    def __post_init__(self) -> None:
        if not isinstance(self.feature_version, str) or not self.feature_version.strip():
            raise ValueError("feature_version must be a non-empty string")
        if not _is_sha256(self.definition_sha256):
            raise ValueError("definition_sha256 must be a lowercase SHA-256 digest")
        for field in ("as_of_utc_ns", "knowledge_utc_ns"):
            value = getattr(self, field)
            if type(value) is not int or value <= 0:
                raise ValueError(f"{field} must be a positive integer UTC timestamp")
        if not isinstance(self.declared_config, dict):
            raise ValueError("declared_config must be an object")
        validate_feature_lineage(self)

    def as_dict(self) -> dict[str, Any]:
        return asdict(self)

    def digest(self) -> str:
        return feature_lineage_digest(self)


def validate_feature_lineage(lineage: FeatureLineage, *, event_utc_ns: int | None = None,
                             created_at_utc_ns: int | None = None) -> None:
    """Check that features were known before they were computed and observed."""
    if not isinstance(lineage, FeatureLineage):
        raise ValueError("feature lineage must be a FeatureLineage value")
    if lineage.knowledge_utc_ns > lineage.as_of_utc_ns:
        raise ValueError("feature knowledge time must not be after its as-of time")
    if event_utc_ns is not None:
        if type(event_utc_ns) is not int or event_utc_ns <= 0:
            raise ValueError("event_utc_ns must be a positive integer")
        if lineage.as_of_utc_ns > event_utc_ns:
            raise ValueError("feature as-of time must not be after the consuming event")
    if created_at_utc_ns is not None:
        if type(created_at_utc_ns) is not int or created_at_utc_ns <= 0:
            raise ValueError("created_at_utc_ns must be a positive integer")
        if lineage.as_of_utc_ns > created_at_utc_ns:
            raise ValueError("feature as-of time must not be after dataset creation")


def feature_lineage_digest(lineage: FeatureLineage) -> str:
    """Return a canonical digest of the full declarative lineage block."""
    return hashlib.sha256(canonical_bytes(lineage.as_dict())).hexdigest()


def assert_feature_lineage(lineage: Any, expected_digest: Any, *,
                           event_utc_ns: int | None = None,
                           created_at_utc_ns: int | None = None) -> None:
    """Verify the stored digest and the bitemporal bounds of a decoded block."""
    restored = feature_lineage_from_dict(lineage)
    if restored is None or not _is_sha256(expected_digest):
        raise ValueError("dataset feature lineage declaration is missing or malformed")
    validate_feature_lineage(restored, event_utc_ns=event_utc_ns,
                             created_at_utc_ns=created_at_utc_ns)
    if feature_lineage_digest(restored) != expected_digest:
        raise ValueError("dataset feature lineage digest mismatch")


def feature_lineage_from_dict(value: Any) -> FeatureLineage | None:
    """Decode and validate a feature lineage stored in a dataset manifest or row."""
    if value is None:
        return None
    if not isinstance(value, dict) or set(value) != {
        "feature_version", "definition_sha256", "as_of_utc_ns",
        "knowledge_utc_ns", "declared_config",
    }:
        raise ValueError("feature lineage declaration has missing or unknown fields")
    return FeatureLineage(**value)
