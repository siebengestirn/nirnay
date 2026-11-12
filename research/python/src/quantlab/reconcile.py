"""Cross-provider reconciliation with explicit comparability and quality states."""
from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from collections import defaultdict
from .market import MarketEvent


@dataclass(frozen=True, slots=True)
class ReconciliationResult:
    instrument_id: str
    status: str
    providers: tuple[str, ...]
    receive_skew_ns: int | None
    source_skew_ns: int | None
    price_difference_bps: Decimal | None
    price_basis: str | None
    detail: str


def reconcile_pair(a: MarketEvent, b: MarketEvent, *, now_ns: int,
                   max_age_ns: int = 5_000_000_000,
                   max_receive_skew_ns: int = 2_000_000_000,
                   max_source_skew_ns: int = 2_000_000_000,
                   max_price_difference_bps: Decimal = Decimal("5")) -> ReconciliationResult:
    if not a.instrument_id or a.instrument_id != b.instrument_id:
        raise ValueError("reconciliation requires the same resolved canonical instrument")
    if a.provider == b.provider:
        raise ValueError("cross-feed reconciliation requires two distinct providers")
    if a.provider not in {"upstox", "fyers"} or b.provider not in {"upstox", "fyers"}:
        raise ValueError("only supported broker market-data providers can be cross-reconciled")
    if a.event_kind not in {"quote", "depth"} or b.event_kind not in {"quote", "depth"}:
        raise ValueError("reconciliation requires quote/depth state observations, not inferred trade events")
    if now_ns <= 0 or min(max_age_ns, max_receive_skew_ns, max_source_skew_ns) < 0:
        raise ValueError("reconciliation clock and tolerances must be valid")
    if not isinstance(max_price_difference_bps, Decimal) or not max_price_difference_bps.is_finite() or max_price_difference_bps < 0:
        raise ValueError("price tolerance must be a finite non-negative Decimal")
    if a.venue is None or b.venue is None or a.venue != b.venue:
        return ReconciliationResult(a.instrument_id, "venue_mismatch", tuple(sorted((a.provider, b.provider))),
                                   None, None, None, None,
                                   "canonical instrument mapping does not resolve both observations to the same venue")
    if a.last_trade_time_semantics is None or b.last_trade_time_semantics is None:
        return ReconciliationResult(a.instrument_id, "source_time_incomparable", tuple(sorted((a.provider, b.provider))),
                                   None, None, None, None,
                                   "both feeds must expose explicit last-trade timestamp semantics")
    providers = tuple(sorted((a.provider, b.provider)))
    age_a, age_b = now_ns - a.received_utc_ns, now_ns - b.received_utc_ns
    if min(age_a, age_b) < 0:
        return ReconciliationResult(a.instrument_id, "future_receive_timestamp", providers, None, None, None, None,
                                    "runtime clock precedes a receive timestamp")
    if max(age_a, age_b) > max_age_ns:
        return ReconciliationResult(a.instrument_id, "stale", providers, None, None, None, None,
                                    f"provider observation age exceeds {max_age_ns} ns")
    receive_skew = abs(a.received_utc_ns - b.received_utc_ns)
    if receive_skew > max_receive_skew_ns:
        return ReconciliationResult(a.instrument_id, "receive_time_incomparable", providers, receive_skew, None, None, None,
                                    "observations are too far apart in local receive time")
    # Upstox currentTs is an envelope receive timestamp, whereas FYERS
    # exch_feed_time is an exchange feed time. Compare last-trade timestamps
    # only when both adapters expose that same semantic field.
    if a.last_trade_time_ns is None or b.last_trade_time_ns is None:
        return ReconciliationResult(a.instrument_id, "source_time_incomparable", providers, receive_skew, None, None, None,
                                    "one or both providers lack comparable last-trade timestamps")
    source_skew = abs(a.last_trade_time_ns - b.last_trade_time_ns)
    if source_skew > max_source_skew_ns:
        return ReconciliationResult(a.instrument_id, "source_time_incomparable", providers, receive_skew, source_skew, None, None,
                                    "source timestamps exceed configured clock/skew tolerance")
    if "crossed_book" in a.quality_flags or "crossed_book" in b.quality_flags:
        return ReconciliationResult(a.instrument_id, "invalid_book", providers, receive_skew, source_skew,
                                    None, None, "at least one provider reports a crossed book")
    # Compare like with like. If both feeds have a two-sided book, use midpoint;
    # otherwise compare the last-traded price only when both expose it.
    if a.best_bid is not None and a.best_ask is not None and b.best_bid is not None and b.best_ask is not None:
        pa = (a.best_bid.price + a.best_ask.price) / Decimal(2)
        pb = (b.best_bid.price + b.best_ask.price) / Decimal(2)
        price_basis = "two_sided_book_midpoint"
    elif a.last_price is not None and b.last_price is not None:
        pa, pb = a.last_price, b.last_price
        price_basis = "last_traded_price"
    else:
        return ReconciliationResult(a.instrument_id, "price_basis_incomparable", providers, receive_skew, source_skew, None, None,
                                    "both providers must expose comparable two-sided midpoints or last-traded prices")
    if pa <= 0 or pb <= 0:
        return ReconciliationResult(a.instrument_id, "price_unavailable", providers, receive_skew, source_skew, None, price_basis,
                                    "one or both selected price values are non-positive")
    diff_bps = abs(pa - pb) / ((pa + pb) / Decimal(2)) * Decimal(10_000)
    if diff_bps > max_price_difference_bps:
        return ReconciliationResult(a.instrument_id, "price_divergence", providers, receive_skew, source_skew,
                                    diff_bps, price_basis, "cross-provider price difference exceeds tolerance")
    return ReconciliationResult(a.instrument_id, "consistent_within_tolerance", providers, receive_skew,
                                source_skew, diff_bps, price_basis,
                                "observations are comparable and within configured tolerance")


def reconcile_latest(events: list[MarketEvent], **kwargs: int) -> list[ReconciliationResult]:
    """Reconcile latest comparable quote/depth *state*, never inferred LTP events.

    Provider callbacks can emit several canonical observations at one receive
    timestamp (for example a depth snapshot and an LTP observation). Selection
    is deterministic and prefers a two-sided book, then a valid last-trade
    timestamp, then provider-local sequence and event id.
    """
    grouped: dict[str, dict[str, MarketEvent]] = defaultdict(dict)
    def rank(event: MarketEvent) -> tuple[int, int, int, int, str]:
        two_sided = int(event.best_bid is not None and event.best_ask is not None)
        timed = int(event.last_trade_time_ns is not None and event.last_trade_time_semantics is not None)
        seq = event.local_sequence if event.local_sequence is not None else -1
        return (event.received_utc_ns, two_sided, timed, seq, event.event_id)
    for event in events:
        if event.instrument_id and event.event_kind in {"quote", "depth"}:
            prior = grouped[event.instrument_id].get(event.provider)
            if prior is None or rank(event) > rank(prior):
                grouped[event.instrument_id][event.provider] = event
    results = []
    instruments = sorted({e.instrument_id for e in events if e.instrument_id})
    for instrument in instruments:
        providers = grouped.get(instrument, {})
        if "upstox" in providers and "fyers" in providers:
            results.append(reconcile_pair(providers["upstox"], providers["fyers"], **kwargs))
        else:
            present = tuple(sorted(providers))
            results.append(ReconciliationResult(instrument, "provider_missing", present,
                                                None, None, None, None,
                                                "both Upstox and FYERS quote/depth state observations are required"))
    return results
