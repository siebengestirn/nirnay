"""Attribution reconciliation and integrity tests."""
from __future__ import annotations

import pytest

from quantlab.experiments.reproducibility import assert_report_hash
from quantlab.reporting import assert_reconciliation, attribute

COMPONENTS = {"momentum": 0.015, "mean_reversion": -0.004, "costs": -0.001}


def test_components_reconcile_to_synthetic_total():
    total = sum(COMPONENTS.values())
    first = attribute(total, COMPONENTS)
    second = attribute(total, dict(COMPONENTS))
    assert first == second
    assert first["reconciled"] is True
    assert first["residual"] == pytest.approx(0.0)
    assert sum(first["components"].values()) == pytest.approx(total)
    assert_report_hash(first)


def test_non_reconciling_attribution_raises():
    with pytest.raises(ValueError, match="do not reconcile"):
        attribute(1.0, COMPONENTS)
    with pytest.raises(ValueError, match="do not reconcile"):
        assert_reconciliation(0.01, {"a": 0.009})


def test_non_finite_inputs_raise_before_digest():
    with pytest.raises(ValueError, match="finite"):
        attribute(float("nan"), COMPONENTS)
    with pytest.raises(ValueError, match="finite"):
        attribute(0.01, {"a": float("inf")})
    with pytest.raises(ValueError, match="tolerance"):
        attribute(0.01, {"a": 0.01}, tolerance=0.0)


def test_tampered_attribution_report_fails_digest():
    report = attribute(sum(COMPONENTS.values()), COMPONENTS)
    report["components"]["momentum"] = 0.0
    with pytest.raises(ValueError, match="mismatch"):
        assert_report_hash(report)
