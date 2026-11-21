"""Per-configuration contribution decomposition with strict reconciliation.

Components must sum to the supplied synthetic total within an explicit
tolerance. Non-finite inputs and non-reconciling decompositions raise before a
report digest is produced.
"""
from __future__ import annotations

import math
from collections.abc import Mapping
from typing import Any

from ..experiments.schema import content_hash

DEFAULT_TOLERANCE = 1e-9


def assert_reconciliation(total: float, components: Mapping[str, float], *,
                          tolerance: float = DEFAULT_TOLERANCE) -> float:
    """Return the residual and refuse a non-reconciling decomposition."""
    if isinstance(total, bool) or not isinstance(total, (int, float)):
        raise ValueError("total must be a real number")
    total_value = float(total)
    if not math.isfinite(total_value):
        raise ValueError("total must be finite")
    if isinstance(tolerance, bool) or not isinstance(tolerance, (int, float)):
        raise ValueError("tolerance must be a real number")
    if not math.isfinite(float(tolerance)) or tolerance <= 0:
        raise ValueError("tolerance must be positive and finite")
    if not isinstance(components, Mapping) or not components:
        raise ValueError("components must be a non-empty mapping")
    values: dict[str, float] = {}
    for name, value in components.items():
        if not isinstance(name, str) or not name:
            raise ValueError("component names must be non-empty strings")
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError("component contributions must be real numbers")
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("component contributions must be finite")
        values[name] = number
    residual = total_value - sum(values.values())
    if not math.isclose(residual, 0.0, rel_tol=0.0, abs_tol=float(tolerance)):
        raise ValueError("attribution components do not reconcile to the total")
    return residual


def attribute(total: float, components: Mapping[str, float], *,
              tolerance: float = DEFAULT_TOLERANCE) -> dict[str, Any]:
    """Build a deterministic attribution report reconciled to a synthetic total."""
    if not isinstance(components, Mapping) or not components:
        raise ValueError("components must be a non-empty mapping")
    ordered = {name: float(components[name]) for name in sorted(components)}
    residual = assert_reconciliation(total, ordered, tolerance=tolerance)
    report: dict[str, Any] = {
        "schema_version": 1,
        "total": float(total),
        "components": ordered,
        "residual": residual,
        "tolerance": float(tolerance),
        "reconciled": True,
        "claim_boundary": "research_screening_only_not_alpha_certification",
    }
    report["report_sha256"] = content_hash(report)
    return report
