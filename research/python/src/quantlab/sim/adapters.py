"""Adapter from canonical ``MarketEvent`` observations to simulation events.

The adapter never upgrades an SDK callback to an exchange event.  Last-trade
observations and prints without a known aggressor are admitted as reconstructed
or as non-queue-advancing prints and can never consume a resting queue.
"""
from __future__ import annotations

from typing import Any, Sequence

from ..market import MarketEvent, raw_hash
from .events import (
    BookSnapshot,
    SimBookLevel,
    SimulationEvent,
    TradePrint,
)


def _lineage(event: MarketEvent, source_frame_sha256: str | None) -> str:
    candidate = source_frame_sha256 or event.source_frame_sha256 or raw_hash(event.as_dict())
    return candidate


def _envelope(event: MarketEvent, suffix: str, *, origin_class: str,
              source_frame_sha256: str, derived_from: tuple[str, ...],
              source_sequence: int) -> dict[str, Any]:
    if event.instrument_id is None or event.venue is None:
        raise ValueError("adapter requires a resolved instrument and venue")
    if origin_class == "exchange_event":
        raise ValueError("market adapter cannot certify origin_class=exchange_event")
    received = event.received_utc_ns
    return {
        "event_id": f"{event.event_id}:sim:{suffix}",
        "source_frame_sha256": source_frame_sha256,
        "instrument_id": event.instrument_id,
        "venue": event.venue,
        "logical_time_ns": received,
        "available_time_ns": received,
        "origin_class": origin_class,
        "derived_from": derived_from,
        "lineage": (event.event_id,),
        "source_sequence": source_sequence,
    }


def _levels(rows: tuple) -> tuple[SimBookLevel, ...]:
    return tuple(SimBookLevel(level.price, level.quantity, level.order_count) for level in rows)


def quarantine_reason(event: MarketEvent) -> str | None:
    """Return why a canonical event cannot advance a simulated queue."""
    if event.event_kind == "last_trade_observation":
        return "last_trade_observation is reconstructed, not a confirmed trade print"
    if event.event_kind == "trade" and event.aggressor_side is None:
        return "aggressor_side unknown; print cannot prove the resting side was hit"
    if event.event_kind in {"status", "index"}:
        return "status/index events carry no matching semantics and are quarantined"
    return None


def market_event_to_sim_events(
    event: MarketEvent,
    *,
    origin_class: str = "sdk_callback",
    source_frame_sha256: str | None = None,
) -> tuple[SimulationEvent, ...]:
    """Map one canonical market event into zero or more simulation events."""
    lineage = _lineage(event, source_frame_sha256)
    common = {
        "origin_class": origin_class,
        "source_frame_sha256": lineage,
        "derived_from": (event.event_id,),
        "source_sequence": event.local_sequence or 0,
    }
    if event.event_kind in {"quote", "depth"}:
        envelope = _envelope(event, "book", **common)
        return (
            BookSnapshot(
                **envelope,
                bids=_levels(event.bids),
                asks=_levels(event.asks),
                depth_granularity="mbp",
            ),
        )
    if event.event_kind == "trade":
        if event.last_price is None or event.last_quantity is None:
            return ()
        envelope = _envelope(event, "trade", **common)
        return (
            TradePrint(
                **envelope,
                price=event.last_price,
                quantity=event.last_quantity,
                aggressor_side=event.aggressor_side,
                exchange_sequence=None,
            ),
        )
    if event.event_kind == "last_trade_observation":
        if event.last_price is None or event.last_quantity is None:
            return ()
        reconstructed = dict(common)
        reconstructed["origin_class"] = "reconstructed"
        reconstructed["derived_from"] = (event.event_id, "not_a_confirmed_trade_print")
        envelope = _envelope(event, "ltp", **reconstructed)
        return (
            TradePrint(
                **envelope,
                price=event.last_price,
                quantity=max(event.last_quantity, 0) or 1,
                aggressor_side=None,
                exchange_sequence=None,
            ),
        )
    return ()


def market_events_to_sim_events(
    events: Sequence[MarketEvent],
    *,
    origin_class: str = "sdk_callback",
) -> tuple[SimulationEvent, ...]:
    """Map an ordered sequence of canonical events into simulation events."""
    result: list[SimulationEvent] = []
    for event in events:
        result.extend(
            market_event_to_sim_events(event, origin_class=origin_class)
        )
    return tuple(result)
