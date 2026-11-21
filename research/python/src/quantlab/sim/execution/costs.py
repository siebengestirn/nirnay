"""Effective-dated, typed cost assumptions with no fabricated calibration."""
from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal, localcontext

from ..events import digest, is_sha256, to_jsonable

BPS = Decimal("10000")
_SHAPES = frozenset({"sqrt", "linear", "power"})


def _decimal(value: object, name: str, *, minimum: Decimal | None = None) -> Decimal:
    if not isinstance(value, Decimal) or not value.is_finite():
        raise ValueError(f"{name} must be a finite Decimal")
    if minimum is not None and value < minimum:
        raise ValueError(f"{name} must be >= {minimum}")
    return value


def _integer(value: object, name: str, minimum: int = 0) -> int:
    if type(value) is not int or value < minimum:
        raise ValueError(f"{name} must be an integer >= {minimum}")
    return value


def _artifact(artifact_sha256: str | None, assumption: str | None, name: str) -> None:
    if (artifact_sha256 is None) == (assumption is None):
        raise ValueError(f"{name} requires exactly one of artifact_sha256 or assumption")
    if artifact_sha256 is not None and not is_sha256(artifact_sha256):
        raise ValueError(f"{name} artifact_sha256 must be a lowercase SHA-256 digest")
    if assumption is not None and (not isinstance(assumption, str) or not assumption.strip()):
        raise ValueError(f"{name} assumption must be a non-empty string")


@dataclass(frozen=True, slots=True)
class FeeEntry:
    """One effective-dated fee row keyed by venue, instrument, side and role."""

    effective_from_ns: int
    venue: str
    instrument: str
    side: str
    liquidity: str
    rate_bps: Decimal
    artifact_sha256: str | None = None
    assumption: str | None = None

    def __post_init__(self) -> None:
        _integer(self.effective_from_ns, "effective_from_ns", 1)
        for name in ("venue", "instrument"):
            if not isinstance(getattr(self, name), str) or not getattr(self, name):
                raise ValueError(f"{name} must be a non-empty string or '*'")
        if self.side not in {"buy", "sell", "*"}:
            raise ValueError("side must be buy, sell or '*'")
        if self.liquidity not in {"maker", "taker", "*"}:
            raise ValueError("liquidity must be maker, taker or '*'")
        _decimal(self.rate_bps, "rate_bps", minimum=Decimal("0"))
        _artifact(self.artifact_sha256, self.assumption, "FeeEntry")

    def matches(self, effective_ns: int, venue: str, instrument: str, side: str,
                liquidity: str) -> bool:
        """Return whether this row applies at ``effective_ns`` to the given order."""
        return (
            self.effective_from_ns <= effective_ns
            and self.venue in {venue, "*"}
            and self.instrument in {instrument, "*"}
            and self.side in {side, "*"}
            and self.liquidity in {liquidity, "*"}
        )


@dataclass(frozen=True, slots=True)
class FeeSchedule:
    """Immutable set of fee rows; the latest matching effective row wins."""

    entries: tuple[FeeEntry, ...]

    def __post_init__(self) -> None:
        if not isinstance(self.entries, tuple) or not self.entries:
            raise ValueError("FeeSchedule requires a non-empty tuple of FeeEntry")
        if any(not isinstance(entry, FeeEntry) for entry in self.entries):
            raise ValueError("FeeSchedule entries must be FeeEntry instances")

    def rate(self, effective_ns: int, venue: str, instrument: str, side: str,
             liquidity: str) -> Decimal:
        """Return the effective fee rate in basis points."""
        matches = [
            entry
            for entry in self.entries
            if entry.matches(effective_ns, venue, instrument, side, liquidity)
        ]
        if not matches:
            raise ValueError(f"no fee row effective at {effective_ns} for {venue}/{instrument}")
        chosen = max(
            matches,
            key=lambda entry: (
                entry.effective_from_ns,
                entry.venue,
                entry.instrument,
                entry.side,
                entry.liquidity,
            ),
        )
        return chosen.rate_bps


@dataclass(frozen=True, slots=True)
class SlippageEntry:
    """Effective-dated venue/instrument slippage in basis points."""

    effective_from_ns: int
    venue: str
    instrument: str
    bps: Decimal
    artifact_sha256: str | None = None
    assumption: str | None = None

    def __post_init__(self) -> None:
        _integer(self.effective_from_ns, "effective_from_ns", 1)
        for name in ("venue", "instrument"):
            if not isinstance(getattr(self, name), str) or not getattr(self, name):
                raise ValueError(f"{name} must be a non-empty string or '*'")
        _decimal(self.bps, "bps", minimum=Decimal("0"))
        _artifact(self.artifact_sha256, self.assumption, "SlippageEntry")


@dataclass(frozen=True, slots=True)
class SlippageSchedule:
    """Immutable slippage rows; the latest matching effective row wins."""

    entries: tuple[SlippageEntry, ...]

    def __post_init__(self) -> None:
        if not isinstance(self.entries, tuple) or not self.entries:
            raise ValueError("SlippageSchedule requires a non-empty tuple of SlippageEntry")
        if any(not isinstance(entry, SlippageEntry) for entry in self.entries):
            raise ValueError("SlippageSchedule entries must be SlippageEntry instances")

    def rate(self, effective_ns: int, venue: str, instrument: str) -> Decimal:
        """Return the effective slippage rate in basis points."""
        matches = [
            entry
            for entry in self.entries
            if entry.effective_from_ns <= effective_ns
            and entry.venue in {venue, "*"}
            and entry.instrument in {instrument, "*"}
        ]
        if not matches:
            raise ValueError(
                f"no slippage row effective at {effective_ns} for {venue}/{instrument}"
            )
        chosen = max(
            matches,
            key=lambda entry: (entry.effective_from_ns, entry.venue, entry.instrument),
        )
        return chosen.bps


@dataclass(frozen=True, slots=True)
class ImpactConfig:
    """Impact shape and coefficient with an explicit calibration or assumption."""

    shape: str
    coefficient_bps: Decimal
    exponent: Decimal = Decimal("0.5")
    artifact_sha256: str | None = None
    assumption: str | None = None

    def __post_init__(self) -> None:
        if self.shape not in _SHAPES:
            raise ValueError(f"impact shape must be one of {sorted(_SHAPES)}")
        _decimal(self.coefficient_bps, "coefficient_bps", minimum=Decimal("0"))
        _decimal(self.exponent, "exponent", minimum=Decimal("0"))
        if self.shape != "power" and self.exponent != Decimal("0.5"):
            raise ValueError("exponent is only used by the power shape")
        _artifact(self.artifact_sha256, self.assumption, "ImpactConfig")

    def impact_bps(self, participation: Decimal) -> Decimal:
        """Return the displacement in basis points for a participation fraction."""
        fraction = _decimal(participation, "participation", minimum=Decimal("0"))
        with localcontext() as context:
            context.prec = 28
            if fraction == 0:
                factor = Decimal("0")
            elif self.shape == "sqrt":
                factor = fraction.sqrt()
            elif self.shape == "linear":
                factor = fraction
            else:
                factor = (fraction.ln() * self.exponent).exp()
            return self.coefficient_bps * factor


@dataclass(frozen=True, slots=True)
class BorrowTerms:
    """Borrow availability and cost for one instrument."""

    instrument: str
    available: bool
    max_quantity: int
    cost_bps_per_day: Decimal
    artifact_sha256: str | None = None
    assumption: str | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.instrument, str) or not self.instrument:
            raise ValueError("instrument must be a non-empty string")
        if type(self.available) is not bool:
            raise ValueError("available must be a boolean")
        _integer(self.max_quantity, "max_quantity")
        _decimal(self.cost_bps_per_day, "cost_bps_per_day", minimum=Decimal("0"))
        _artifact(self.artifact_sha256, self.assumption, "BorrowTerms")


@dataclass(frozen=True, slots=True)
class MarginTerms:
    """Initial and maintenance margin expressed in basis points."""

    initial_bps: Decimal
    maintenance_bps: Decimal
    artifact_sha256: str | None = None
    assumption: str | None = None

    def __post_init__(self) -> None:
        _decimal(self.initial_bps, "initial_bps", minimum=Decimal("0"))
        _decimal(self.maintenance_bps, "maintenance_bps", minimum=Decimal("0"))
        if self.maintenance_bps > self.initial_bps:
            raise ValueError("maintenance_bps cannot exceed initial_bps")
        _artifact(self.artifact_sha256, self.assumption, "MarginTerms")


@dataclass(frozen=True, slots=True)
class FundingTerms:
    """Effective-dated time-dependent funding rate."""

    effective_from_ns: int
    rate_bps_per_day: Decimal
    day_ns: int = 86_400_000_000_000
    artifact_sha256: str | None = None
    assumption: str | None = None

    def __post_init__(self) -> None:
        _integer(self.effective_from_ns, "effective_from_ns", 1)
        _decimal(self.rate_bps_per_day, "rate_bps_per_day", minimum=Decimal("0"))
        _integer(self.day_ns, "day_ns", 1)
        _artifact(self.artifact_sha256, self.assumption, "FundingTerms")

    def charge(self, notional: Decimal, held_ns: int) -> Decimal:
        """Return the funding charge for a notional held over ``held_ns``."""
        amount = _decimal(notional, "notional")
        duration = _integer(held_ns, "held_ns")
        return amount * self.rate_bps_per_day * Decimal(duration) / (BPS * Decimal(self.day_ns))


@dataclass(frozen=True, slots=True)
class CostModel:
    """Complete, serializable cost assumption bundle selected by config hash."""

    fee_schedule: FeeSchedule
    slippage_schedule: SlippageSchedule
    impact: ImpactConfig
    borrow: tuple[BorrowTerms, ...] = ()
    margin: MarginTerms | None = None
    funding: tuple[FundingTerms, ...] = ()

    def __post_init__(self) -> None:
        if not isinstance(self.fee_schedule, FeeSchedule):
            raise ValueError("fee_schedule must be a FeeSchedule")
        if not isinstance(self.slippage_schedule, SlippageSchedule):
            raise ValueError("slippage_schedule must be a SlippageSchedule")
        if not isinstance(self.impact, ImpactConfig):
            raise ValueError("impact must be an ImpactConfig")
        if any(not isinstance(item, BorrowTerms) for item in self.borrow):
            raise ValueError("borrow must contain BorrowTerms")
        if any(not isinstance(item, FundingTerms) for item in self.funding):
            raise ValueError("funding must contain FundingTerms")

    def cost_hash(self) -> str:
        """Return the deterministic digest of every cost parameter."""
        return digest(to_jsonable(self))

    def borrow_terms(self, instrument: str) -> BorrowTerms | None:
        """Return borrow terms for an instrument or None when unavailable."""
        for terms in self.borrow:
            if terms.instrument == instrument:
                return terms
        return None

    def funding_rate(self, effective_ns: int) -> FundingTerms | None:
        """Return the latest funding terms effective at ``effective_ns``."""
        matches = [item for item in self.funding if item.effective_from_ns <= effective_ns]
        if not matches:
            return None
        return max(matches, key=lambda item: item.effective_from_ns)
