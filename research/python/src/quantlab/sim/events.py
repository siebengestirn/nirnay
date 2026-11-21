"""Frozen, typed simulation events with fail-closed lineage validation.

This module extends (never mutates) the canonical ``MarketEvent`` vocabulary.
Every event carries a content-addressed lineage reference, event/available time,
a constrained ``origin_class`` and an explicit ``derived_from`` list.  A source
that cannot certify an exchange tick may not assert ``exchange_event``.
"""
from __future__ import annotations

import collections.abc
import hashlib
import json
import re
from dataclasses import dataclass, fields, is_dataclass
from decimal import Decimal, InvalidOperation
from typing import Any, Mapping, get_args, get_origin, get_type_hints

_HEX = re.compile(r"[0-9a-f]{64}\Z")
_ORIGIN_CLASSES = frozenset(
    {
        "exchange_event",
        "sdk_callback",
        "raw_wire_frame",
        "local_receive",
        "reconstructed",
        "simulated",
        "synthetic",
    }
)
_DEPTH_GRANULARITIES = frozenset({"mbp", "mbo"})
_SIDES = frozenset({"buy", "sell"})
_ORDER_STATUSES = frozenset(
    {
        "ack",
        "new",
        "reject",
        "fill",
        "cancel_ack",
        "expire",
    }
)
_REPORT_TYPES = _ORDER_STATUSES
_AUCTION_PHASES = frozenset({"pre_open", "uncross", "close"})
_MARKET_STATUSES = frozenset({"halt", "circuit", "resume", "open"})


def is_sha256(value: object) -> bool:
    """Return whether ``value`` is a lowercase 64-character SHA-256 digest."""
    return isinstance(value, str) and bool(_HEX.fullmatch(value))


def _require_text(value: object, name: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{name} must be a non-empty string")
    return value


def _require_int(value: object, name: str, minimum: int = 0) -> int:
    if type(value) is not int or value < minimum:
        raise ValueError(f"{name} must be an integer >= {minimum}")
    return value


def _require_decimal(value: object, name: str) -> Decimal:
    if not isinstance(value, Decimal) or not value.is_finite():
        raise ValueError(f"{name} must be a finite Decimal")
    return value


def _require_sha256(value: object, name: str) -> str:
    if not is_sha256(value):
        raise ValueError(f"{name} must be a lowercase SHA-256 digest")
    return str(value)


def _require_tuple(value: object, name: str) -> tuple[str, ...]:
    if not isinstance(value, tuple) or any(not isinstance(x, str) for x in value):
        raise ValueError(f"{name} must be a tuple of strings")
    return value


def _require_choice(value: object, name: str, choices: frozenset[str]) -> str:
    text = _require_text(value, name)
    if text not in choices:
        raise ValueError(f"{name} must be one of {sorted(choices)}")
    return text


def canonical_bytes(value: Any) -> bytes:
    """Serialize a JSON-compatible value deterministically."""
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")


def digest(value: Any) -> str:
    """Return the SHA-256 digest of a canonical JSON representation."""
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def to_jsonable(value: Any) -> Any:
    """Recursively convert dataclasses, Decimals and tuples to JSON values."""
    if isinstance(value, Decimal):
        return format(value, "f")
    if isinstance(value, tuple):
        return [to_jsonable(item) for item in value]
    if isinstance(value, Mapping):
        return {str(k): to_jsonable(value[k]) for k in sorted(value, key=str)}
    if is_dataclass(value) and not isinstance(value, type):
        return {item.name: to_jsonable(getattr(value, item.name)) for item in fields(value)}
    return value


def _coerce(value: Any, annotation: Any) -> Any:
    if value is None:
        return None
    origin = get_origin(annotation)
    if annotation is Decimal:
        try:
            parsed = Decimal(str(value))
        except (InvalidOperation, ValueError) as exc:
            raise ValueError(f"not a Decimal: {value!r}") from exc
        if not parsed.is_finite():
            raise ValueError("Decimal must be finite")
        return parsed
    if annotation is int:
        if type(value) is bool:
            raise ValueError("boolean supplied where an integer is required")
        return int(value)
    if annotation is str:
        return str(value)
    if annotation is bool:
        return bool(value)
    if origin is tuple:
        args = get_args(annotation)
        if len(args) == 2 and args[1] is Ellipsis:
            return tuple(_coerce(item, args[0]) for item in value)
        return tuple(_coerce(item, arg) for item, arg in zip(value, args))
    if origin in (dict, collections.abc.Mapping):
        args = get_args(annotation)
        key_type = args[0] if args else str
        value_type = args[1] if len(args) > 1 else Any
        return {_coerce(k, key_type): _coerce(v, value_type) for k, v in sorted(
            value.items(), key=lambda pair: str(pair[0])
        )}
    if is_dataclass(annotation) and isinstance(annotation, type):
        return from_jsonable(annotation, value)
    for arg in get_args(annotation):
        try:
            return _coerce(value, arg)
        except (TypeError, ValueError):
            continue
    return value


def from_jsonable(cls: type, data: Mapping[str, Any]) -> Any:
    """Reconstruct a frozen dataclass from its canonical JSON representation."""
    hints = get_type_hints(cls)
    missing = [name for name in (f.name for f in fields(cls)) if name not in data]
    if missing:
        raise ValueError(f"{cls.__name__} fields missing: {sorted(missing)}")
    kwargs = {item.name: _coerce(data[item.name], hints[item.name]) for item in fields(cls)}
    return cls(**kwargs)


@dataclass(frozen=True, slots=True, kw_only=True)
class EventEnvelope:
    """Common, immutable identity and time surface shared by every event."""

    event_id: str
    source_frame_sha256: str
    instrument_id: str
    venue: str
    logical_time_ns: int
    available_time_ns: int
    origin_class: str
    derived_from: tuple[str, ...] = ()
    lineage: tuple[str, ...] = ()
    source_sequence: int = 0
    schema_version: int = 1
    certification_ref: str | None = None

    def __post_init__(self) -> None:
        _require_text(self.event_id, "event_id")
        _require_sha256(self.source_frame_sha256, "source_frame_sha256")
        _require_text(self.instrument_id, "instrument_id")
        _require_text(self.venue, "venue")
        _require_int(self.logical_time_ns, "logical_time_ns", 1)
        _require_int(self.available_time_ns, "available_time_ns", 0)
        if self.available_time_ns < self.logical_time_ns:
            raise ValueError("available_time_ns cannot precede logical_time_ns")
        _require_choice(self.origin_class, "origin_class", _ORIGIN_CLASSES)
        _require_tuple(self.derived_from, "derived_from")
        _require_tuple(self.lineage, "lineage")
        _require_int(self.source_sequence, "source_sequence")
        if type(self.schema_version) is not int or self.schema_version != 1:
            raise ValueError("unsupported simulation event schema version")
        # exchange_event requires a sha256 certification_ref.  This check is
        # format-only; it does not authenticate the referenced certification.
        if self.origin_class == "exchange_event":
            if not is_sha256(self.certification_ref):
                raise ValueError("exchange_event requires a sha256 certification_ref")
        elif self.certification_ref is not None:
            raise ValueError("certification_ref is only valid for exchange_event")

    def ordering_key(self) -> tuple[int, str, str, int, str]:
        """Return the deterministic replay ordering key."""
        return (
            self.logical_time_ns,
            self.venue,
            self.instrument_id,
            self.source_sequence,
            self.event_id,
        )


@dataclass(frozen=True, slots=True)
class SimBookLevel:
    """One displayed book level; quantity zero denotes a deletion in a delta."""

    price: Decimal
    quantity: int
    order_count: int | None = None

    @classmethod
    def build(cls, price: Any, quantity: Any, order_count: Any = None) -> SimBookLevel:
        try:
            parsed_price = Decimal(str(price))
        except (InvalidOperation, ValueError) as exc:
            raise ValueError("level price is not a decimal") from exc
        if not parsed_price.is_finite() or parsed_price <= 0:
            raise ValueError("level price must be positive and finite")
        if type(quantity) is not int or quantity < 0:
            raise ValueError("level quantity must be a non-negative integer")
        if order_count is not None and (type(order_count) is not int or order_count < 0):
            raise ValueError("order_count must be a non-negative integer")
        return cls(parsed_price, quantity, order_count)

    def __post_init__(self) -> None:
        _require_decimal(self.price, "level.price")
        if self.price <= 0:
            raise ValueError("level.price must be positive")
        _require_int(self.quantity, "level.quantity")
        if self.order_count is not None:
            _require_int(self.order_count, "level.order_count")


def _validate_levels(levels: tuple[SimBookLevel, ...], name: str, *, allow_zero: bool) -> None:
    if not isinstance(levels, tuple):
        raise ValueError(f"{name} must be a tuple of SimBookLevel")
    for level in levels:
        if not isinstance(level, SimBookLevel):
            raise ValueError(f"{name} entries must be SimBookLevel")
        if not allow_zero and level.quantity == 0:
            raise ValueError(f"{name} quantities must be positive")
    for left, right in zip(levels, levels[1:]):
        if right.price <= left.price:
            raise ValueError(f"{name} prices must be strictly ascending from the best level")


@dataclass(frozen=True, slots=True, kw_only=True)
class TradePrint(EventEnvelope):
    """An explicit trade print; reconstructed prints never advance a queue."""

    price: Decimal
    quantity: int
    aggressor_side: str | None = None
    exchange_sequence: int | None = None

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_decimal(self.price, "price")
        if self.price <= 0:
            raise ValueError("price must be positive")
        _require_int(self.quantity, "quantity", 1)
        if self.aggressor_side is not None:
            _require_choice(self.aggressor_side, "aggressor_side", _SIDES)
        if self.exchange_sequence is not None:
            _require_int(self.exchange_sequence, "exchange_sequence", 1)

    @property
    def queue_advancing(self) -> bool:
        """Return whether this print may conservatively consume a resting queue."""
        return self.origin_class != "reconstructed" and self.aggressor_side is not None


@dataclass(frozen=True, slots=True, kw_only=True)
class BookSnapshot(EventEnvelope):
    """A full displayed book snapshot at a declared granularity."""

    bids: tuple[SimBookLevel, ...]
    asks: tuple[SimBookLevel, ...]
    depth_granularity: str = "mbp"

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _validate_levels(self.bids, "bids", allow_zero=False)
        _validate_levels(self.asks, "asks", allow_zero=False)
        _require_choice(self.depth_granularity, "depth_granularity", _DEPTH_GRANULARITIES)
        if self.bids and self.asks and self.bids[0].price >= self.asks[0].price:
            raise ValueError("crossed or locked book is not a valid snapshot")
        if self.depth_granularity == "mbo" and any(
            level.order_count is None for level in (*self.bids, *self.asks)
        ):
            raise ValueError("mbo snapshots require per-order counts")


@dataclass(frozen=True, slots=True, kw_only=True)
class BookDelta(EventEnvelope):
    """Level updates for one side; zero quantity removes that price level."""

    side: str
    updates: tuple[SimBookLevel, ...]
    depth_granularity: str = "mbp"

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_choice(self.side, "side", frozenset({"bid", "ask"}))
        _validate_levels(self.updates, "updates", allow_zero=True)
        _require_choice(self.depth_granularity, "depth_granularity", _DEPTH_GRANULARITIES)


@dataclass(frozen=True, slots=True, kw_only=True)
class AuctionPhase(EventEnvelope):
    """An explicit auction phase transition with logical effective time."""

    phase: str
    reference_price: Decimal | None = None

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_choice(self.phase, "phase", _AUCTION_PHASES)
        if self.reference_price is not None:
            _require_decimal(self.reference_price, "reference_price")


@dataclass(frozen=True, slots=True, kw_only=True)
class MarketStatus(EventEnvelope):
    """A halt, circuit or resume with an explicit effective interval."""

    status: str
    effective_until_ns: int | None = None
    reason: str = ""

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_choice(self.status, "status", _MARKET_STATUSES)
        if self.effective_until_ns is not None:
            _require_int(self.effective_until_ns, "effective_until_ns", 1)
            if self.effective_until_ns < self.logical_time_ns:
                raise ValueError("effective_until_ns precedes logical_time_ns")


@dataclass(frozen=True, slots=True, kw_only=True)
class InstrumentChange(EventEnvelope):
    """An effective-dated instrument terms change (tick, lot, symbol, expiry)."""

    field: str
    new_value: str
    old_value: str | None = None

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_choice(
            self.field,
            "field",
            frozenset({"tick_size", "lot_size", "symbol", "expiry"}),
        )
        _require_text(self.new_value, "new_value")


@dataclass(frozen=True, slots=True, kw_only=True)
class OrderIntent:
    """A strategy intent only: no cash, positions or venue access."""

    client_order_id: str
    instrument_id: str
    side: str
    quantity: int
    limit_price: Decimal
    time_in_force_ns: int = 60_000_000_000

    def validate(self) -> None:
        """Fail closed on malformed intents."""
        _require_text(self.client_order_id, "client_order_id")
        _require_text(self.instrument_id, "instrument_id")
        _require_choice(self.side, "side", _SIDES)
        _require_int(self.quantity, "quantity", 1)
        _require_decimal(self.limit_price, "limit_price")
        if self.limit_price <= 0:
            raise ValueError("limit_price must be positive")
        _require_int(self.time_in_force_ns, "time_in_force_ns", 1)


@dataclass(frozen=True, slots=True, kw_only=True)
class OrderAdd(EventEnvelope):
    """A simulated order submission, fully linked to its causation event."""

    client_order_id: str
    side: str
    quantity: int
    limit_price: Decimal
    venue_order_id: str | None = None
    time_in_force_ns: int = 60_000_000_000

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        OrderIntent(
            client_order_id=self.client_order_id,
            instrument_id=self.instrument_id,
            side=self.side,
            quantity=self.quantity,
            limit_price=self.limit_price,
            time_in_force_ns=self.time_in_force_ns,
        ).validate()
        if self.venue_order_id is not None:
            _require_text(self.venue_order_id, "venue_order_id")


@dataclass(frozen=True, slots=True, kw_only=True)
class OrderModify(EventEnvelope):
    """An amendment to an existing simulated order."""

    client_order_id: str
    venue_order_id: str | None = None
    new_quantity: int | None = None
    new_limit_price: Decimal | None = None

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_text(self.client_order_id, "client_order_id")
        if self.new_quantity is not None:
            _require_int(self.new_quantity, "new_quantity", 1)
        if self.new_limit_price is not None:
            _require_decimal(self.new_limit_price, "new_limit_price")
            if self.new_limit_price <= 0:
                raise ValueError("new_limit_price must be positive")
        if self.new_quantity is None and self.new_limit_price is None:
            raise ValueError("OrderModify must change quantity or limit price")


@dataclass(frozen=True, slots=True, kw_only=True)
class OrderCancel(EventEnvelope):
    """A cancel request; exposure persists until a cancel_ack is processed."""

    client_order_id: str
    venue_order_id: str | None = None

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_text(self.client_order_id, "client_order_id")


@dataclass(frozen=True, slots=True, kw_only=True)
class ExecutionReport(EventEnvelope):
    """First-class execution state transition emitted and consumed by the core."""

    client_order_id: str
    report_type: str
    venue_order_id: str | None = None
    quantity: int = 0
    price: Decimal | None = None
    fees: Decimal = Decimal("0")
    impact_cost: Decimal = Decimal("0")
    spread_cost: Decimal = Decimal("0")
    liquidity: str | None = None
    reason: str | None = None

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_text(self.client_order_id, "client_order_id")
        _require_choice(self.report_type, "report_type", _REPORT_TYPES)
        _require_int(self.quantity, "quantity")
        for name in ("fees", "impact_cost", "spread_cost"):
            _require_decimal(getattr(self, name), name)
        if self.price is not None:
            _require_decimal(self.price, "price")
            if self.price <= 0:
                raise ValueError("price must be positive")
        if self.liquidity is not None:
            _require_choice(self.liquidity, "liquidity", frozenset({"maker", "taker"}))
        if self.report_type == "fill":
            if self.quantity <= 0 or self.price is None or self.liquidity is None:
                raise ValueError("a fill report requires quantity, price and liquidity")
        if self.report_type == "reject" and not self.reason:
            raise ValueError("a reject report requires a reason")


@dataclass(frozen=True, slots=True, kw_only=True)
class ConfigChange(EventEnvelope):
    """An effective-dated configuration or schedule reference."""

    config_key: str
    config_digest: str
    effective_ns: int

    def __post_init__(self) -> None:
        EventEnvelope.__post_init__(self)
        _require_text(self.config_key, "config_key")
        _require_sha256(self.config_digest, "config_digest")
        _require_int(self.effective_ns, "effective_ns", 1)


@dataclass(frozen=True, slots=True)
class UnmarkedResidual:
    """A nonzero position with no mark at ``as_of_ns`` (never valued at zero)."""

    instrument_id: str
    quantity: int
    as_of_ns: int

    def __post_init__(self) -> None:
        _require_text(self.instrument_id, "instrument_id")
        _require_int(self.quantity, "quantity", 1)
        _require_int(self.as_of_ns, "as_of_ns", 1)


EVENT_TYPES: tuple[type[EventEnvelope], ...] = (
    TradePrint,
    BookSnapshot,
    BookDelta,
    AuctionPhase,
    MarketStatus,
    InstrumentChange,
    OrderAdd,
    OrderModify,
    OrderCancel,
    ExecutionReport,
    ConfigChange,
)
_EVENT_BY_NAME = {cls.__name__: cls for cls in EVENT_TYPES}

SimulationEvent = (
    TradePrint
    | BookSnapshot
    | BookDelta
    | AuctionPhase
    | MarketStatus
    | InstrumentChange
    | OrderAdd
    | OrderModify
    | OrderCancel
    | ExecutionReport
    | ConfigChange
)


def event_ordering_key(event: SimulationEvent) -> tuple[int, str, str, int, str]:
    """Return ``(logical_time_ns, venue, instrument_id, source_sequence, event_id)``."""
    return event.ordering_key()


def event_to_dict(event: SimulationEvent) -> dict[str, Any]:
    """Serialize any simulation event to a canonical JSON-compatible object."""
    result = to_jsonable(event)
    if not isinstance(result, dict):
        raise TypeError("events must serialize to objects")
    result["event_type"] = type(event).__name__
    return result


def event_from_dict(data: Mapping[str, Any]) -> SimulationEvent:
    """Strictly reconstruct an event; unknown variants fail closed."""
    if not isinstance(data, Mapping) or "event_type" not in data:
        raise ValueError("simulation event requires event_type")
    name = data["event_type"]
    if not isinstance(name, str) or name not in _EVENT_BY_NAME:
        raise ValueError(f"unknown simulation event type: {name}")
    cls = _EVENT_BY_NAME[name]
    payload = {key: value for key, value in data.items() if key != "event_type"}
    reconstructed = from_jsonable(cls, payload)
    if type(reconstructed).__name__ != name:
        raise ValueError("reconstructed event type does not match event_type")
    return reconstructed


def event_hash(event: SimulationEvent) -> str:
    """Return the content digest of a simulation event."""
    return digest(event_to_dict(event))
