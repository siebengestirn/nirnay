"""Immutable simulation state and its deterministic content hash."""
from __future__ import annotations

from dataclasses import dataclass, replace
from decimal import Decimal
from types import MappingProxyType
from typing import Mapping

from .events import ExecutionReport, SimBookLevel, UnmarkedResidual, digest, to_jsonable
from .execution.base import BaselineParams

_ACTIVE_STATUSES = frozenset(
    {"pending_new", "working", "partially_filled", "pending_cancel"}
)


def frozen_map(items: Mapping | None = None) -> Mapping:
    """Return a sorted, read-only copy of a mapping."""
    if not items:
        return MappingProxyType({})
    return MappingProxyType(dict(sorted(items.items(), key=lambda pair: str(pair[0]))))


def _levels_map(levels: tuple[SimBookLevel, ...]) -> dict[Decimal, int]:
    return {level.price: level.quantity for level in levels}


def _increments(previous: dict[Decimal, int], current: dict[Decimal, int]) -> tuple:
    rows = [
        (price, quantity - previous.get(price, 0))
        for price, quantity in current.items()
        if quantity > previous.get(price, 0)
    ]
    return tuple(sorted((price, amount) for price, amount in rows if amount > 0))


@dataclass(frozen=True, slots=True)
class QueueState:
    """Displayed queue ahead of a resting order, with a high-water ceiling."""

    ahead: int = 0
    high_water: int = 0

    def __post_init__(self) -> None:
        for name in ("ahead", "high_water"):
            value = getattr(self, name)
            if type(value) is not int or value < 0:
                raise ValueError(f"{name} must be a non-negative integer")


@dataclass(frozen=True, slots=True)
class BookState:
    """Full displayed book plus the net new liquidity of the latest update."""

    bids: tuple[SimBookLevel, ...] = ()
    asks: tuple[SimBookLevel, ...] = ()
    new_bid_liquidity: tuple[tuple[Decimal, int], ...] = ()
    new_ask_liquidity: tuple[tuple[Decimal, int], ...] = ()
    last_update_ns: int = 0
    depth_granularity: str = "mbp"

    def apply_snapshot(self, bids: tuple[SimBookLevel, ...], asks: tuple[SimBookLevel, ...],
                       update_ns: int, depth_granularity: str) -> BookState:
        """Replace displayed levels and derive positive-only new increments."""
        bid_increments = _increments(_levels_map(self.bids), _levels_map(bids))
        ask_increments = _increments(_levels_map(self.asks), _levels_map(asks))
        return BookState(
            bids=bids,
            asks=asks,
            new_bid_liquidity=bid_increments,
            new_ask_liquidity=ask_increments,
            last_update_ns=update_ns,
            depth_granularity=depth_granularity,
        )

    def apply_delta(self, side: str, updates: tuple[SimBookLevel, ...], update_ns: int,
                    depth_granularity: str) -> BookState:
        """Apply one-sided level updates with positive-only new increments."""
        current = _levels_map(self.bids if side == "bid" else self.asks)
        increments = _increments(current, _levels_map(updates))
        for level in updates:
            if level.quantity == 0:
                current.pop(level.price, None)
            else:
                current[level.price] = level.quantity
        rebuilt = tuple(
            sorted(
                (SimBookLevel(price, quantity) for price, quantity in current.items()),
                key=lambda level: level.price,
                reverse=side == "bid",
            )
        )
        if side == "bid":
            return BookState(
                bids=rebuilt,
                asks=self.asks,
                new_bid_liquidity=increments,
                new_ask_liquidity=(),
                last_update_ns=update_ns,
                depth_granularity=depth_granularity,
            )
        return BookState(
            bids=self.bids,
            asks=rebuilt,
            new_bid_liquidity=(),
            new_ask_liquidity=increments,
            last_update_ns=update_ns,
            depth_granularity=depth_granularity,
        )

    def displayed(self, side: str) -> dict[Decimal, int]:
        """Return the displayed levels for one side."""
        return _levels_map(self.bids if side == "bid" else self.asks)


@dataclass(frozen=True, slots=True)
class MarkState:
    """A mark derived from an explicit two-sided book or trade observation."""

    price: Decimal
    mark_time_ns: int
    source_event_id: str

    def __post_init__(self) -> None:
        if not isinstance(self.price, Decimal) or not self.price.is_finite() or self.price <= 0:
            raise ValueError("mark price must be a positive finite Decimal")
        if type(self.mark_time_ns) is not int or self.mark_time_ns <= 0:
            raise ValueError("mark_time_ns must be a positive integer")
        if not isinstance(self.source_event_id, str) or not self.source_event_id:
            raise ValueError("source_event_id is required")


@dataclass(frozen=True, slots=True)
class OrderState:
    """Lifecycle of one simulated order, advanced only by typed events."""

    client_order_id: str
    venue: str
    instrument_id: str
    side: str
    quantity: int
    remaining: int
    limit_price: Decimal
    created_ns: int
    activates_ns: int
    expires_ns: int
    venue_order_id: str | None = None
    status: str = "pending_new"
    acknowledged: bool = False
    announced: bool = False
    cancel_effective_ns: int | None = None
    last_reason: str | None = None

    @property
    def active(self) -> bool:
        """Return whether the order can still trade."""
        return self.remaining > 0 and self.status in _ACTIVE_STATUSES


@dataclass(frozen=True, slots=True)
class RiskBudgets:
    """Notional, gross and participation ceilings used by the reducer."""

    max_notional: Decimal = Decimal("1000000")
    max_gross_exposure: Decimal = Decimal("1000000")
    max_participation: Decimal = Decimal("0.10")

    def __post_init__(self) -> None:
        for name in ("max_notional", "max_gross_exposure", "max_participation"):
            value = getattr(self, name)
            if not isinstance(value, Decimal) or not value.is_finite() or value <= 0:
                raise ValueError(f"{name} must be a positive finite Decimal")
        if self.max_participation > Decimal("1"):
            raise ValueError("max_participation cannot exceed 1")


@dataclass(frozen=True, slots=True)
class SimState:
    """Complete immutable simulation state; no wall clock and no floats."""

    cash: Decimal
    starting_cash: Decimal
    params: BaselineParams
    positions: Mapping[str, int] = MappingProxyType({})
    books: Mapping[tuple[str, str], BookState] = MappingProxyType({})
    queues: Mapping[tuple[str, str], QueueState] = MappingProxyType({})
    orders: Mapping[str, OrderState] = MappingProxyType({})
    marks: Mapping[str, MarkState] = MappingProxyType({})
    fills: tuple[ExecutionReport, ...] = ()
    risk_budgets: RiskBudgets = RiskBudgets()
    market_status: str = "open"
    effective_configs: Mapping[str, tuple[tuple[int, str], ...]] = MappingProxyType({})
    unmarked_residual: tuple[UnmarkedResidual, ...] = ()
    consumed_event_ids: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        if not isinstance(self.cash, Decimal) or not self.cash.is_finite():
            raise ValueError("cash must be a finite Decimal")
        if not isinstance(self.starting_cash, Decimal) or self.starting_cash <= 0:
            raise ValueError("starting_cash must be a positive finite Decimal")
        if not isinstance(self.params, BaselineParams):
            raise ValueError("params must be a BaselineParams")

    def updated(self, **changes: object) -> SimState:
        """Return a new state with selected fields replaced and maps frozen."""
        for name in ("positions", "books", "queues", "orders", "marks", "effective_configs"):
            if name in changes:
                changes[name] = frozen_map(changes[name])
        return replace(self, **changes)


def empty_state(starting_cash: Decimal, params: BaselineParams,
                risk_budgets: RiskBudgets | None = None) -> SimState:
    """Build the initial state for a replay run."""
    return SimState(
        cash=starting_cash,
        starting_cash=starting_cash,
        params=params,
        risk_budgets=risk_budgets or RiskBudgets(),
    )


def queue_key(venue: str, client_order_id: str) -> tuple[str, str]:
    """Return the venue-scoped queue key for an order."""
    return venue, client_order_id


def book_key(venue: str, instrument_id: str) -> tuple[str, str]:
    """Return the venue-scoped book key for an instrument."""
    return venue, instrument_id


def state_hash(state: SimState) -> str:
    """Return the deterministic SHA-256 of the canonical state representation."""
    return digest(to_jsonable(state))
