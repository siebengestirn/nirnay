"""Conservative MBP baseline execution adapter.

This reproduces the existing research semantics: displayed increments only,
high-water queue tracking, explicit trade prints with a known aggressor for
passive fills, declared fixed or seeded latency and square-root impact from an
assumption.  It declares ``queue_truth=unavailable`` and must never emit a FIFO
claim.  Seeded latency is a research assumption, not a calibrated venue law.
"""
from __future__ import annotations

import hashlib
from dataclasses import dataclass
from decimal import Decimal
from typing import Protocol

from ..events import from_jsonable, is_sha256, to_jsonable, digest
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
_LATENCY_SHAPES = frozenset({"fixed", "uniform"})
_LATENCY_KINDS = frozenset({"order", "cancel"})
_DRAW_SPACE = 1 << 64


class QueueTruthUnavailable(ValueError):
    """Raised when a caller asks an MBP-only model for FIFO certainty."""


def _require_provenance(artifact_sha256: str | None, assumption: str | None,
                        name: str) -> None:
    """Require exactly one declaration; the artifact is not calibrated here."""
    if (artifact_sha256 is None) == (assumption is None):
        raise ValueError(f"{name} requires exactly one of artifact_sha256 or assumption")
    if artifact_sha256 is not None and not is_sha256(artifact_sha256):
        raise ValueError(f"{name} artifact_sha256 must be a lowercase SHA-256 digest")
    if assumption is not None and (not isinstance(assumption, str) or not assumption.strip()):
        raise ValueError(f"{name} assumption must be a non-empty string")


@dataclass(frozen=True, slots=True)
class LatencyDistribution:
    """Declared seeded latency shape; never a calibrated venue distribution."""

    shape: str
    min_ns: int
    max_ns: int
    artifact_sha256: str | None = None
    assumption: str | None = None

    def __post_init__(self) -> None:
        if self.shape not in _LATENCY_SHAPES:
            raise ValueError(f"latency shape must be one of {sorted(_LATENCY_SHAPES)}")
        for name in ("min_ns", "max_ns"):
            value = getattr(self, name)
            if type(value) is not int or value < 0:
                raise ValueError(f"{name} must be a non-negative integer")
        if self.max_ns < self.min_ns:
            raise ValueError("max_ns cannot precede min_ns")
        if self.shape == "fixed" and self.min_ns != self.max_ns:
            raise ValueError("a fixed latency distribution requires min_ns == max_ns")
        _require_provenance(self.artifact_sha256, self.assumption, "LatencyDistribution")

    def digest(self) -> str:
        """Return the canonical digest of the declared distribution."""
        return digest(to_jsonable(self))


@dataclass(frozen=True, slots=True)
class BaselineParams:
    """All tunable parameters of the conservative baseline adapter."""

    order_latency_ns: int
    cancel_latency_ns: int
    max_participation: Decimal
    max_book_age_ns: int
    cost_model: CostModel
    latency: LatencyDistribution | None = None

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
        if self.latency is not None and not isinstance(self.latency, LatencyDistribution):
            raise ValueError("latency must be a LatencyDistribution or None")

    def latency_sample_ns(self, seed: int, context: str, kind: str) -> int:
        """Return a seeded latency draw in ns; no wall clock or Python hash used."""
        if kind not in _LATENCY_KINDS:
            raise ValueError(f"latency kind must be one of {sorted(_LATENCY_KINDS)}")
        if type(seed) is not int or not isinstance(context, str) or not context:
            raise ValueError("a latency sample needs an integer seed and a context")
        if self.latency is None:
            return self.order_latency_ns if kind == "order" else self.cancel_latency_ns
        if self.latency.shape == "fixed":
            return self.latency.min_ns
        material = f"{seed}:{kind}:{context}".encode("utf-8")
        raw = int.from_bytes(hashlib.sha256(material).digest()[:8], "big")
        span = self.latency.max_ns - self.latency.min_ns
        return self.latency.min_ns + (span * raw) // _DRAW_SPACE

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

    def latency_sample(self, seed: int, context: str, kind: str) -> int: ...

    def activation_time_ns(self, event_time_ns: int, *, seed: int = 0,
                           context: str = "") -> int: ...

    def cancel_time_ns(self, event_time_ns: int, *, seed: int = 0,
                       context: str = "") -> int: ...

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

    def latency_sample(self, seed: int, context: str, kind: str) -> int:
        """Return the declared seeded latency draw for one order lifecycle step."""
        return self.params.latency_sample_ns(seed, context, kind)

    def activation_time_ns(self, event_time_ns: int, *, seed: int = 0,
                           context: str = "") -> int:
        """Return the activation time using the declared latency distribution."""
        if type(event_time_ns) is not int or event_time_ns <= 0:
            raise ValueError("event_time_ns must be a positive integer")
        return event_time_ns + self.latency_sample(seed, context or "activation", "order")

    def cancel_time_ns(self, event_time_ns: int, *, seed: int = 0,
                       context: str = "") -> int:
        """Return the cancel-effective time using the declared distribution."""
        if type(event_time_ns) is not int or event_time_ns <= 0:
            raise ValueError("event_time_ns must be a positive integer")
        return event_time_ns + self.latency_sample(seed, context or "cancel", "cancel")

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
