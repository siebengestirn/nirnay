"""Reporting package shim with byte-stable serialization and comparison.

Research performance helpers are re-exported from the existing flat module;
the new comparison helper operates only on frozen terminal experiment records.
"""
from __future__ import annotations

from ..market import canonical_bytes
from ..metrics import summarize_returns
from .attribution import assert_reconciliation, attribute
from .comparison import ComparisonError, compare_experiments, freeze_for_comparison
from .performance import drawdown_duration, performance_report
from .risk import exposure_concentration, max_drawdown, risk_report, worst_period


def research_report_bytes(value: object) -> bytes:
    """Serialize a research report deterministically for CLI output."""
    return canonical_bytes(value) + b"\n"


__all__ = [
    "ComparisonError",
    "assert_reconciliation",
    "attribute",
    "compare_experiments",
    "drawdown_duration",
    "exposure_concentration",
    "freeze_for_comparison",
    "max_drawdown",
    "performance_report",
    "research_report_bytes",
    "risk_report",
    "summarize_returns",
    "worst_period",
]
