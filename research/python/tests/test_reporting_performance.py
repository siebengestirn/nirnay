"""Performance report determinism, drawdown duration and falsifiers."""
from __future__ import annotations

import pytest

from quantlab.experiments.reproducibility import assert_report_hash
from quantlab.reporting import drawdown_duration, performance_report, summarize_returns

RETURNS = [0.01, -0.02, 0.03, -0.01, 0.02, 0.0, -0.03, 0.04]


def test_performance_report_is_deterministic_and_digest_verifies():
    first = performance_report(RETURNS, 252)
    second = performance_report(list(RETURNS), 252)
    assert first == second
    assert_report_hash(first)
    assert first["summary"] == summarize_returns(RETURNS, 252)
    assert first["drawdown"]["observations"] == len(RETURNS)


def test_drawdown_duration_counts_consecutive_periods_below_peak():
    result = drawdown_duration([0.10, -0.01, -0.02, 0.05, -0.01])
    assert result["longest_drawdown_periods"] == 2
    assert result["max_drawdown"] > 0
    assert result["current_drawdown_periods"] == 1


def test_drawdown_duration_handles_all_gains():
    result = drawdown_duration([0.01, 0.02, 0.03])
    assert result["longest_drawdown_periods"] == 0
    assert result["max_drawdown"] == 0.0


def test_non_finite_and_invalid_inputs_raise():
    with pytest.raises(ValueError, match="finite"):
        performance_report([0.01, float("nan")], 252)
    with pytest.raises(ValueError, match="below"):
        drawdown_duration([-1.5])
    with pytest.raises(ValueError, match="positive"):
        performance_report(RETURNS, 0)


def test_tampered_report_digest_fails():
    report = performance_report(RETURNS, 252)
    report["summary"]["observations"] = 999
    with pytest.raises(ValueError, match="mismatch"):
        assert_report_hash(report)
