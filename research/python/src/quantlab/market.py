"""Canonical multi-provider market observations.

Provider identifiers and timestamps retain their original semantics.  This
module does not imply that an SDK callback is an exchange tick.
"""
from __future__ import annotations

import hashlib
import json
import re
from dataclasses import asdict, dataclass
from decimal import Decimal, InvalidOperation
from typing import Any, Iterable


def _decimal(value: Any, field: str) -> Decimal | None:
    if value is None:
        return None
    if isinstance(value, bool):
        raise ValueError(f"{field} must be numeric, not boolean")
    try:
        result = Decimal(str(value))
    except (InvalidOperation, ValueError) as exc:
        raise ValueError(f"{field} is not a valid decimal") from exc
    if not result.is_finite():
        raise ValueError(f"{field} must be finite")
    return result


def _integer(value: Any, field: str) -> int:
    """Parse an exact integer from JSON integer/string/whole decimal inputs."""
    if isinstance(value, bool) or value is None:
        raise ValueError(f"{field} must be an integer")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        text = value.strip()
        if not re.fullmatch(r"[+-]?\d+", text):
            raise ValueError(f"{field} must be an integer")
        return int(text)
    number = _decimal(value, field)
    if number is None or number != number.to_integral_value():
        raise ValueError(f"{field} must be an exact integer")
    return int(number)


def _json_default(value: Any) -> str:
    if isinstance(value, Decimal):
        return format(value, "f")
    raise TypeError(type(value).__name__)


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False, default=_json_default).encode("utf-8")


@dataclass(frozen=True, slots=True)
class BookLevel:
    price: Decimal
    quantity: int
    order_count: int | None = None

    @classmethod
    def build(cls, price: Any, quantity: Any, order_count: Any = None) -> "BookLevel | None":
        p = _decimal(price, "level.price")
        if p is None:
            return None
        q = _integer(quantity, "level.quantity")
        if q < 0:
            raise ValueError("level.quantity must be non-negative")
        n = None if order_count is None else _integer(order_count, "level.order_count")
        if n is not None and n < 0:
            raise ValueError("level.order_count must be non-negative")
        if q == 0:
            return None
        if p <= 0:
            raise ValueError("level.price must be positive when quantity is non-zero")
        return cls(p, q, n)

    def as_dict(self) -> dict[str, Any]:
        return {"price": format(self.price, "f"), "quantity": self.quantity,
                "order_count": self.order_count}


@dataclass(frozen=True, slots=True)
class MarketEvent:
    schema_version: int
    event_id: str
    provider: str
    provider_instrument_id: str
    instrument_id: str | None
    venue: str | None
    event_kind: str
    received_utc_ns: int
    received_monotonic_ns: int | None
    local_sequence: int | None
    source_timestamp_ns: int | None
    source_timestamp_semantics: str | None
    last_trade_time_ns: int | None
    last_trade_time_semantics: str | None
    last_price: Decimal | None
    last_quantity: int | None
    cumulative_volume: int | None
    aggressor_side: str | None
    bids: tuple[BookLevel, ...]
    asks: tuple[BookLevel, ...]
    raw_sha256: str
    quality_flags: tuple[str, ...] = ()
    # SHA-256 of the durable capture-journal record when one is available.
    # raw_sha256 above remains the hash of the provider payload represented here.
    source_frame_sha256: str | None = None

    def __post_init__(self) -> None:
        if not self.event_id:
            raise ValueError("event_id is required")
        if self.schema_version != 1:
            raise ValueError("unsupported canonical market event schema")
        if self.provider not in {"upstox", "fyers", "synthetic"}:
            raise ValueError("provider must be upstox, fyers or synthetic")
        if not self.provider_instrument_id:
            raise ValueError("provider_instrument_id is required")
        if self.event_kind not in {"quote", "trade", "last_trade_observation", "depth", "index", "status"}:
            raise ValueError("unsupported event_kind")
        for name in ("received_utc_ns", "received_monotonic_ns", "local_sequence",
                     "source_timestamp_ns", "last_trade_time_ns", "last_quantity", "cumulative_volume"):
            value = getattr(self, name)
            if value is not None and (type(value) is not int or value < 0):
                raise ValueError(f"{name} must be a non-negative integer")
        if self.received_utc_ns <= 0:
            raise ValueError("received_utc_ns must be positive")
        if self.last_quantity is not None and self.last_quantity < 0:
            raise ValueError("last_quantity must be non-negative")
        if self.cumulative_volume is not None and self.cumulative_volume < 0:
            raise ValueError("cumulative_volume must be non-negative")
        if self.aggressor_side not in {None, "buy", "sell"}:
            raise ValueError("aggressor_side must be buy, sell or unknown")
        if self.last_price is not None and (not self.last_price.is_finite() or self.last_price <= 0):
            raise ValueError("last_price must be positive and finite")
        if self.source_timestamp_ns is not None and self.source_timestamp_ns <= 0:
            raise ValueError("source_timestamp_ns must be positive when supplied")
        if self.last_trade_time_ns is not None and self.last_trade_time_ns <= 0:
            raise ValueError("last_trade_time_ns must be positive when supplied")
        if self.event_kind == "trade" and (self.last_price is None or self.last_quantity is None or self.last_quantity <= 0):
            raise ValueError("trade events require a positive last price and quantity")
        if self.event_kind == "last_trade_observation" and (self.last_price is None or self.last_trade_time_ns is None):
            raise ValueError("last-trade observations require a price and explicit last-trade timestamp")
        if len(self.raw_sha256) != 64 or any(c not in "0123456789abcdef" for c in self.raw_sha256):
            raise ValueError("raw_sha256 must be lowercase SHA-256 hex")
        if self.source_frame_sha256 is not None and (len(self.source_frame_sha256) != 64 or
                any(c not in "0123456789abcdef" for c in self.source_frame_sha256)):
            raise ValueError("source_frame_sha256 must be lowercase SHA-256 hex when supplied")
        if self.bids and any(a.price < b.price for a, b in zip(self.bids, self.bids[1:])):
            raise ValueError("bid levels must be ordered best-to-worst")
        if self.asks and any(a.price > b.price for a, b in zip(self.asks, self.asks[1:])):
            raise ValueError("ask levels must be ordered best-to-worst")

    @property
    def best_bid(self) -> BookLevel | None:
        return self.bids[0] if self.bids else None

    @property
    def best_ask(self) -> BookLevel | None:
        return self.asks[0] if self.asks else None

    @property
    def midpoint(self) -> Decimal | None:
        if self.best_bid and self.best_ask:
            return (self.best_bid.price + self.best_ask.price) / Decimal(2)
        return self.last_price

    def as_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version, "event_id": self.event_id,
            "provider": self.provider, "provider_instrument_id": self.provider_instrument_id,
            "instrument_id": self.instrument_id, "venue": self.venue,
            "event_kind": self.event_kind, "received_utc_ns": self.received_utc_ns,
            "received_monotonic_ns": self.received_monotonic_ns,
            "local_sequence": self.local_sequence,
            "source_timestamp_ns": self.source_timestamp_ns,
            "source_timestamp_semantics": self.source_timestamp_semantics,
            "last_trade_time_ns": self.last_trade_time_ns,
            "last_trade_time_semantics": self.last_trade_time_semantics,
            "last_price": None if self.last_price is None else format(self.last_price, "f"),
            "last_quantity": self.last_quantity, "cumulative_volume": self.cumulative_volume,
            "aggressor_side": self.aggressor_side,
            "bids": [x.as_dict() for x in self.bids], "asks": [x.as_dict() for x in self.asks],
            "raw_sha256": self.raw_sha256, "quality_flags": list(self.quality_flags),
            "source_frame_sha256": self.source_frame_sha256,
        }


def raw_hash(payload: Any) -> str:
    return hashlib.sha256(canonical_bytes(payload)).hexdigest()


def as_levels(rows: Iterable[tuple[Any, Any, Any]]) -> tuple[BookLevel, ...]:
    levels = [BookLevel.build(p, q, n) for p, q, n in rows]
    return tuple(x for x in levels if x is not None)


def market_event_from_dict(value: dict[str, Any]) -> MarketEvent:
    """Strictly reconstruct a canonical event from its JSON representation."""
    if not isinstance(value, dict):
        raise ValueError("market event must be an object")
    expected = set(MarketEvent.__dataclass_fields__)
    required = expected - {"source_frame_sha256"}  # optional v1 extension for capture-journal linkage
    if not required.issubset(value) or set(value) - expected:
        raise ValueError(f"market event fields differ: missing={sorted(required-set(value))}, unknown={sorted(set(value)-expected)}")
    from decimal import Decimal
    try:
        return MarketEvent(
            schema_version=value["schema_version"], event_id=value["event_id"],
            provider=value["provider"], provider_instrument_id=value["provider_instrument_id"],
            instrument_id=value["instrument_id"], venue=value["venue"], event_kind=value["event_kind"],
            received_utc_ns=value["received_utc_ns"], received_monotonic_ns=value["received_monotonic_ns"],
            local_sequence=value["local_sequence"], source_timestamp_ns=value["source_timestamp_ns"],
            source_timestamp_semantics=value["source_timestamp_semantics"],
            last_trade_time_ns=value["last_trade_time_ns"],
            last_trade_time_semantics=value["last_trade_time_semantics"],
            last_price=None if value["last_price"] is None else Decimal(value["last_price"]),
            last_quantity=value["last_quantity"], cumulative_volume=value["cumulative_volume"],
            aggressor_side=value["aggressor_side"],
            bids=tuple(BookLevel(Decimal(x["price"]), x["quantity"], x["order_count"]) for x in value["bids"]),
            asks=tuple(BookLevel(Decimal(x["price"]), x["quantity"], x["order_count"]) for x in value["asks"]),
            raw_sha256=value["raw_sha256"], quality_flags=tuple(value["quality_flags"]),
            source_frame_sha256=value.get("source_frame_sha256"),
        )
    except (KeyError, TypeError, ValueError) as exc:
        raise ValueError(f"invalid canonical market event: {exc}") from exc
