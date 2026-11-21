"""Conservative MBP baseline execution adapter.

This reproduces the existing research semantics: displayed increments only,
high-water queue tracking, explicit trade prints with a known aggressor for
passive fills, fixed latency and square-root impact from a declared assumption.
It declares ``queue_truth=unavailable`` and must never emit a FIFO claim.
"""
from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from typing import Protocol

from ..events import from_jsonable, to_jsonable, digest
from .costs import (
    BPS,
    CostModel,
    FeeEntry,
    FeeSchedule,
    ImpactConfig,
    SlippageEntry,
    SlippageSchedule,
)

_PRICING_SIDES = frozenset({"buy", "sell"})


class QueueTruthUnavailable(ValueError):
    """Raised when a caller asks an MBP-only model for FIFO certainty."""


@dataclass(frozen=True, slots=True)
class BaselineParams:
    """All tunable parameters of the conservative baseline adapter."""

    order_latency_ns: int
    cancel_latency_ns: int
    max_participation: Decimal
    max_book_age_ns: int
    cost_model: CostModel

    def __post_init__(self) -> None:
        for name in ("order_latency_ns", "cancel_latency_ns", "max_book_age_ns"):
            value = getattr(self, name)
            if type(value) is not int or value <= 0:
                raise ValueError(f"{name} must be a positive integer")
        if not isinstance(self.max_participation, Decimal):
            raise ValueError("max_participation must be a Decimal")
        if not (Decimal("0") < self.max_participation <= Decimal("1")):
            raise ValueError("max_participation must be in (0, 1]")
        if not isinstance(self.cost_model, CostModel):
            raise ValueError("cost_model must be a CostModel")

    def to_dict(self) -> dict[str, object]:
        """Return the canonical serializable form."""
        return to_jsonable(self)

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> BaselineParams:
        """Reconstruct params from their canonical form."""
        return from_jsonable(cls, data)


class ExecutionModel(Protocol):
    """Injected venue/product behaviour; the core never hard-codes a venue."""

    priority_model_id: str
    queue_truth: str

    def fees(self, effective_ns: int, venue: str, instrument: str, side: str,
             liquidity: str) -> Decimal: ...

    def slippage(self, effective_ns: int, venue: str, instrument: str) -> Decimal: ...

    def impact_bps(self, participation: Decimal) -> Decimal: ...

    def activation_time_ns(self, event_time_ns: int) -> int: ...

    def cancel_time_ns(self, event_time_ns: int) -> int: ...

    def fifo_claim(self, venue: str, instrument: str) -> str: ...

    def model_hash(self) -> str: ...


@dataclass(frozen=True, slots=True)
class ConservativeMbpModel:
    """Default falsifiable baseline: MBP only, no FIFO claim, fixed latency."""

    params: BaselineParams
    priority_model_id: str = "conservative_mbp_v1"
    queue_truth: str = "unavailable"

    def __post_init__(self) -> None:
        if not isinstance(self.params, BaselineParams):
            raise ValueError("params must be a BaselineParams")

    def fees(self, effective_ns: int, venue: str, instrument: str, side: str,
             liquidity: str) -> Decimal:
        """Return the effective-dated fee rate in basis points."""
        if side not in _PRICING_SIDES:
            raise ValueError("side must be buy or sell")
        return self.params.cost_model.fee_schedule.rate(
            effective_ns, venue, instrument, side, liquidity
        )

    def slippage(self, effective_ns: int, venue: str, instrument: str) -> Decimal:
        """Return the effective-dated venue/instrument slippage in basis points."""
        return self.params.cost_model.slippage_schedule.rate(
            effective_ns, venue, instrument
        )

    def impact_bps(self, participation: Decimal) -> Decimal:
        """Return the declared impact displacement in basis points."""
        return self.params.cost_model.impact.impact_bps(participation)

    def activation_time_ns(self, event_time_ns: int) -> int:
        """Return the fixed-latency activation time."""
        if type(event_time_ns) is not int or event_time_ns <= 0:
            raise ValueError("event_time_ns must be a positive integer")
        return event_time_ns + self.params.order_latency_ns

    def cancel_time_ns(self, event_time_ns: int) -> int:
        """Return the fixed-latency cancel-effective time."""
        if type(event_time_ns) is not int or event_time_ns <= 0:
            raise ValueError("event_time_ns must be a positive integer")
        return event_time_ns + self.params.cancel_latency_ns

    def execution_price(self, base_price: Decimal, side: str, impact_bps: Decimal,
                        slippage_bps: Decimal) -> Decimal:
        """Apply signed impact and slippage to a displayed base price."""
        if side not in _PRICING_SIDES:
            raise ValueError("side must be buy or sell")
        sign = Decimal("1") if side == "buy" else Decimal("-1")
        return base_price + sign * base_price * (impact_bps + slippage_bps) / BPS

    def fifo_claim(self, venue: str, instrument: str) -> str:
        """Refuse to certify FIFO from displayed market-by-price data."""
        raise QueueTruthUnavailable(
            f"queue_truth=unavailable for {venue}/{instrument}; MBP cannot certify FIFO"
        )

    def model_hash(self) -> str:
        """Return a deterministic digest of model identity and parameters."""
        return digest(
            to_jsonable(
                {
                    "priority_model_id": self.priority_model_id,
                    "queue_truth": self.queue_truth,
                    "params": self.params.to_dict(),
                }
            )
        )


def default_impact(*, assumption: str = "fixed sqrt coefficient; not calibrated") -> ImpactConfig:
    """Build the backward-compatible square-root impact assumption."""
    return ImpactConfig(
        shape="sqrt",
        coefficient_bps=Decimal("1.0"),
        assumption=assumption,
    )


def default_cost_model() -> CostModel:
    """Build the backward-compatible flat fee and slippage assumption set."""
    fee = FeeEntry(
        effective_from_ns=1,
        venue="*",
        instrument="*",
        side="*",
        liquidity="*",
        rate_bps=Decimal("2.0"),
        assumption="legacy flat fee; not calibrated",
    )
    slippage = SlippageEntry(
        effective_from_ns=1,
        venue="*",
        instrument="*",
        bps=Decimal("0"),
        assumption="legacy zero slippage; not calibrated",
    )
    return CostModel(
        fee_schedule=FeeSchedule((fee,)),
        slippage_schedule=SlippageSchedule((slippage,)),
        impact=default_impact(),
    )
