"""Pure return-based risk report tests and non-finite falsifiers."""
from __future__ import annotations

import pytest

from quantlab.experiments.reproducibility import assert_report_hash
from quantlab.reporting import exposure_concentration, max_drawdown, risk_report, worst_period

RETURNS = [0.02, -0.03, 0.01, -0.01, 0.04]
EXPOSURES = {"a": 0.5, "b": 0.3, "c": 0.2}


def test_risk_primitives_match_direct_calculation():
    assert worst_period(RETURNS) == -0.03
    wealth = 1.0
    peak = 1.0
    expected = 0.0
    for value in RETURNS:
        wealth *= 1.0 + value
        peak = max(peak, wealth)
        expected = max(expected, 1.0 - wealth / peak)
    assert max_drawdown(RETURNS) == pytest.approx(expected)
    assert exposure_concentration(EXPOSURES) == pytest.approx(
        0.5 ** 2 + 0.3 ** 2 + 0.2 ** 2
    )


def test_risk_report_is_deterministic_and_digest_verifies():
    first = risk_report(RETURNS, EXPOSURES)
    second = risk_report(list(RETURNS), dict(EXPOSURES))
    assert first == second
    assert_report_hash(first)
    assert first["queue_truth_claim"] == "unavailable"
    assert first["claim_boundary"] == "research_screening_only_not_alpha_certification"


def test_non_finite_returns_and_exposures_raise():
    with pytest.raises(ValueError, match="finite"):
        worst_period([0.01, float("inf")])
    with pytest.raises(ValueError, match="finite"):
        max_drawdown([float("nan")])
    with pytest.raises(ValueError, match="finite"):
        exposure_concentration({"a": float("nan")})
    with pytest.raises(ValueError, match="positive gross"):
        exposure_concentration({"a": 0.0, "b": 0.0})


def test_tampered_risk_report_fails_digest():
    report = risk_report(RETURNS, EXPOSURES)
    report["worst_period_return"] = 0.0
    with pytest.raises(ValueError, match="mismatch"):
        assert_report_hash(report)
