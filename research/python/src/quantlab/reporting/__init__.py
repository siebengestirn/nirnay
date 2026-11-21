"""Reporting package shim with byte-stable serialization and comparison.

Research performance helpers are re-exported from the existing flat module;
the new comparison helper operates only on frozen terminal experiment records.
"""
from __future__ import annotations

from ..market import canonical_bytes
from ..metrics import summarize_returns
from .comparison import ComparisonError, compare_experiments, freeze_for_comparison


def research_report_bytes(value: object) -> bytes:
    """Serialize a research report deterministically for CLI output."""
    return canonical_bytes(value) + b"\n"


__all__ = [
    "ComparisonError",
    "compare_experiments",
    "freeze_for_comparison",
    "research_report_bytes",
    "summarize_returns",
]
