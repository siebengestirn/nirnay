"""Simulation package shim.

Re-exports the existing event-driven and portfolio simulators under the target
package name. ``cost_assumptions_digest`` is the new tested helper used to bind
cost assumptions into an experiment record; no new matching engine is added.
"""
from __future__ import annotations

import hashlib
from dataclasses import asdict, is_dataclass
from typing import Any

from ..alpha_search import AlphaCosts
from ..backtest import BacktestConfig, EventDrivenBacktester
from ..market import canonical_bytes
from ..portfolio_backtest import MultiInstrumentPortfolioBacktester, PortfolioBacktestConfig


def cost_assumptions_digest(costs: Any) -> str:
    """Digest the exact cost assumptions supplied to a simulation."""
    if is_dataclass(costs) and not isinstance(costs, type):
        body: Any = asdict(costs)
    elif isinstance(costs, dict):
        body = costs
    else:
        raise ValueError("cost assumptions must be a dataclass or object")
    return hashlib.sha256(canonical_bytes(body)).hexdigest()


__all__ = [
    "AlphaCosts",
    "BacktestConfig",
    "EventDrivenBacktester",
    "MultiInstrumentPortfolioBacktester",
    "PortfolioBacktestConfig",
    "cost_assumptions_digest",
]
