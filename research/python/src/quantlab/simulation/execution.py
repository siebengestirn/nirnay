"""Namespace shim for typed execution and cost models.

All names are direct re-exports of the canonical ``quantlab.sim.execution``
package; this module adds no execution behavior.
"""
from __future__ import annotations

from ..sim.execution import (
    BaselineParams,
    ConservativeMbpModel,
    ExecutionModel,
    QueueTruthUnavailable,
    default_cost_model,
    default_impact,
)
from ..sim.execution.costs import (
    BPS,
    BorrowTerms,
    CostModel,
    FeeEntry,
    FeeSchedule,
    FundingTerms,
    ImpactConfig,
    MarginTerms,
    SlippageEntry,
    SlippageSchedule,
)

__all__ = [
    "BPS",
    "BaselineParams",
    "BorrowTerms",
    "ConservativeMbpModel",
    "CostModel",
    "ExecutionModel",
    "FeeEntry",
    "FeeSchedule",
    "FundingTerms",
    "ImpactConfig",
    "MarginTerms",
    "QueueTruthUnavailable",
    "SlippageEntry",
    "SlippageSchedule",
    "default_cost_model",
    "default_impact",
]
