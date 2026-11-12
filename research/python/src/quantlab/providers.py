"""Canonical Upstox V3 and FYERS Data Socket normalisers.

The adapters consume decoded vendor payloads. They preserve raw hashes,
provider identifiers and timestamp semantics. Because neither documented
payload supplies an exchange sequence number with these callbacks, local
sequence numbers remain explicitly local. A trade observation is emitted
only when the vendor's last-trade tuple advances; aggressor side is never
inferred from a quote.
"""
from __future__ import annotations

from decimal import Decimal
from typing import Any

from .market import BookLevel, MarketEvent, _decimal, _integer, raw_hash


def _epoch_ns(value: Any, *, unit: str, field: str) -> int | None:
    if value is None or value == "":
        return None
    try:
        number = _integer(value, field)
    except ValueError as exc:
        raise ValueError(f"{field} must be an integer epoch value") from exc
    if number <= 0:
        raise ValueError(f"{field} must be a positive epoch value")
    factor = {"s": 1_000_000_000, "ms": 1_000_000, "us": 1_000, "ns": 1}[unit]
    return number * factor


def _book_levels(rows: Any, *, price_key_bid: str, qty_key_bid: str,
                 price_key_ask: str, qty_key_ask: str) -> tuple[tuple[BookLevel, ...], tuple[BookLevel, ...]]:
    bids, asks = [], []
    for row in rows or []:
        if not isinstance(row, dict):
            raise ValueError("market depth levels must be objects")
        b = BookLevel.build(row.get(price_key_bid), row.get(qty_key_bid, 0))
        a = BookLevel.build(row.get(price_key_ask), row.get(qty_key_ask, 0))
        if b: bids.append(b)
        if a: asks.append(a)
    bids.sort(key=lambda x: x.price, reverse=True)
    asks.sort(key=lambda x: x.price)
    return tuple(bids), tuple(asks)


def _trade_advanced(previous: tuple[int | None, Decimal | None, int | None, int | None] | None,
                    current: tuple[int | None, Decimal | None, int | None, int | None]) -> bool:
    """Conservative edge detector for last-trade snapshots, not a tick decoder.

    A new last-trade timestamp is evidence of a new last-trade observation. At
    equal timestamp precision, a larger cumulative session volume is required.
    A first snapshot is only a state seed and is never replayed as a trade.
    """
    if previous is None:
        return False
    old_time, _, _, old_volume = previous
    new_time, new_price, new_qty, new_volume = current
    if new_time is None or new_price is None or new_qty is None or new_qty <= 0:
        return False
    if old_time is None or new_time > old_time:
        return True
    if new_time == old_time and old_volume is not None and new_volume is not None:
        return new_volume > old_volume
    return False


class UpstoxV3Normalizer:
    """Normalise decoded Market Data Feed V3 payloads plus capture metadata."""

    def __init__(self, instrument_map: dict[str, str] | None = None, venue_map: dict[str, str] | None = None):
        self.instrument_map = instrument_map or {}
        self.venue_map = venue_map or {}
        self._last_trade: dict[str, tuple[int | None, Decimal | None, int | None, int | None]] = {}

    def normalize(self, message: dict[str, Any], *, received_utc_ns: int,
                  received_monotonic_ns: int | None = None,
                  local_sequence: int | None = None,
                  source_frame_sha256: str | None = None) -> list[MarketEvent]:
        """Return quote/depth state and (when new) a distinct trade observation."""
        if not isinstance(message, dict) or not isinstance(message.get("feeds"), dict):
            raise ValueError("Upstox V3 message must contain a feeds object")
        if type(received_utc_ns) is not int or received_utc_ns <= 0:
            raise ValueError("received_utc_ns must be a positive integer")
        out: list[MarketEvent] = []
        for provider_key, wrapped in sorted(message["feeds"].items()):
            if not isinstance(provider_key, str) or not isinstance(wrapped, dict):
                raise ValueError("Upstox feed entries must map instrument keys to objects")
            feed = wrapped.get("fullFeed") or wrapped.get("ff") or {}
            if not isinstance(feed, dict):
                raise ValueError("Upstox fullFeed must be an object")
            market = feed.get("marketFF") or feed.get("indexFF") or {}
            if not isinstance(market, dict):
                raise ValueError("Upstox market feed must be an object")
            if not market and isinstance(wrapped.get("marketFF"), dict):
                market = wrapped["marketFF"]
            ltpc = wrapped.get("ltpc") or market.get("ltpc") or {}
            if not isinstance(ltpc, dict):
                raise ValueError("Upstox LTPC must be an object")
            market_level = market.get("marketLevel") or {}
            depth = market_level.get("bidAskQuote", []) if isinstance(market_level, dict) else []
            if not depth and isinstance(wrapped.get("firstLevelWithGreeks"), dict):
                first = wrapped["firstLevelWithGreeks"].get("firstDepth", {})
                depth = [{"bidP": first.get("bidP"), "bidQ": first.get("bidQ"),
                          "askP": first.get("askP"), "askQ": first.get("askQ")}]
            bids, asks = _book_levels(depth, price_key_bid="bidP", qty_key_bid="bidQ",
                                      price_key_ask="askP", qty_key_ask="askQ")
            ltt_ns = _epoch_ns(ltpc.get("ltt"), unit="ms", field="ltpc.ltt")
            # Upstox currentTs is documented as a feed tick/receive timestamp,
            # not a per-instrument exchange matching-engine sequence/time.
            envelope_ns = _epoch_ns(message.get("currentTs"), unit="ms", field="currentTs")
            last_price = _decimal(ltpc.get("ltp"), "ltpc.ltp")
            last_quantity = _integer(ltpc["ltq"], "ltpc.ltq") if ltpc.get("ltq") not in (None, "") else None
            volume = _integer(market["vtt"], "marketFF.vtt") if market.get("vtt") not in (None, "") else None
            current_trade = (ltt_ns, last_price, last_quantity, volume)
            prior_trade = self._last_trade.get(provider_key)
            flags = []
            if prior_trade is not None and prior_trade[0] is not None and ltt_ns is not None and ltt_ns < prior_trade[0]:
                flags.append("trade_timestamp_regression")
            if prior_trade is not None and ltt_ns == prior_trade[0] and prior_trade[3] is not None and volume is not None and volume < prior_trade[3]:
                flags.append("cumulative_volume_regression")
            if ltt_ns is None and last_price is not None:
                flags.append("last_trade_time_missing")
            if not bids or not asks:
                flags.append("incomplete_depth")
            if bids and asks and bids[0].price > asks[0].price:
                flags.append("crossed_book")
            if envelope_ns is None:
                flags.append("provider_envelope_timestamp_missing")
            event_kind = "index" if provider_key.startswith("NSE_INDEX|") else ("depth" if depth else "quote")
            digest = raw_hash({"key": provider_key, "wrapped": wrapped})
            venue = self.venue_map.get(provider_key) or provider_key.split("|", 1)[0].split("_", 1)[0]
            quote = MarketEvent(
                schema_version=1,
                event_id=f"upstox:{local_sequence if local_sequence is not None else received_utc_ns}:{provider_key}:{digest[:16]}:book",
                provider="upstox", provider_instrument_id=provider_key,
                instrument_id=self.instrument_map.get(provider_key), venue=venue,
                event_kind=event_kind, received_utc_ns=received_utc_ns,
                received_monotonic_ns=received_monotonic_ns, local_sequence=local_sequence,
                source_timestamp_ns=envelope_ns,
                source_timestamp_semantics="upstox_currentTs_feed_envelope_epoch_ms" if envelope_ns else None,
                last_trade_time_ns=ltt_ns,
                last_trade_time_semantics="upstox_ltpc_ltt_epoch_ms" if ltt_ns else None,
                last_price=last_price, last_quantity=last_quantity, cumulative_volume=volume,
                aggressor_side=None, bids=bids, asks=asks, raw_sha256=digest,
                quality_flags=tuple(flags), source_frame_sha256=source_frame_sha256,
            )
            out.append(quote)
            if _trade_advanced(prior_trade, current_trade):
                out.append(MarketEvent(
                    schema_version=1,
                    event_id=f"upstox:{local_sequence if local_sequence is not None else received_utc_ns}:{provider_key}:{digest[:16]}:trade",
                    provider="upstox", provider_instrument_id=provider_key,
                    instrument_id=self.instrument_map.get(provider_key), venue=venue,
                    event_kind="last_trade_observation", received_utc_ns=received_utc_ns,
                    received_monotonic_ns=received_monotonic_ns, local_sequence=local_sequence,
                    source_timestamp_ns=envelope_ns,
                    source_timestamp_semantics="upstox_currentTs_feed_envelope_epoch_ms" if envelope_ns else None,
                    last_trade_time_ns=ltt_ns, last_trade_time_semantics="upstox_ltpc_ltt_epoch_ms",
                    last_price=last_price, last_quantity=last_quantity, cumulative_volume=volume,
                    aggressor_side=None, bids=bids, asks=asks, raw_sha256=digest,
                    quality_flags=tuple(flags + ["trade_observation_inferred_from_ltt_and_volume_change", "not_a_confirmed_trade_print", "aggressor_side_unknown"]),
                    source_frame_sha256=source_frame_sha256,
                ))
            if ltt_ns is not None and (prior_trade is None or prior_trade[0] is None or
                    ltt_ns > prior_trade[0] or (ltt_ns == prior_trade[0] and
                    (prior_trade[3] is None or volume is None or volume >= prior_trade[3]))):
                self._last_trade[provider_key] = current_trade
        return out


class FyersV3Normalizer:
    """Normalise FYERS SDK-decoded SymbolUpdate/DepthUpdate callbacks."""

    def __init__(self, instrument_map: dict[str, str] | None = None):
        self.instrument_map = instrument_map or {}
        self._last_trade: dict[str, tuple[int | None, Decimal | None, int | None, int | None]] = {}

    def normalize_observations(self, message: dict[str, Any], *, received_utc_ns: int,
                               received_monotonic_ns: int | None = None,
                               local_sequence: int | None = None,
                               source_frame_sha256: str | None = None) -> list[MarketEvent]:
        if not isinstance(message, dict):
            raise ValueError("FYERS callback must be an object")
        if type(received_utc_ns) is not int or received_utc_ns <= 0:
            raise ValueError("received_utc_ns must be a positive integer")
        kind = message.get("type")
        if kind not in {"sf", "dp", "if"}:
            return []  # connection/subscription acknowledgements are not market observations
        symbol = message.get("symbol")
        if not isinstance(symbol, str) or not symbol:
            raise ValueError("FYERS market message lacks symbol")
        if kind == "dp":
            bids, asks = [], []
            for i in range(1, 6):
                b = BookLevel.build(message.get(f"bid_price{i}"), message.get(f"bid_size{i}", 0), message.get(f"bid_order{i}"))
                a = BookLevel.build(message.get(f"ask_price{i}"), message.get(f"ask_size{i}", 0), message.get(f"ask_order{i}"))
                if b: bids.append(b)
                if a: asks.append(a)
            event_kind = "depth"
        else:
            b = BookLevel.build(message.get("bid_price"), message.get("bid_size", 0))
            a = BookLevel.build(message.get("ask_price"), message.get("ask_size", 0))
            bids, asks = ([b] if b else []), ([a] if a else [])
            event_kind = "index" if kind == "if" else "quote"
        source_ns = _epoch_ns(message.get("exch_feed_time"), unit="s", field="exch_feed_time")
        trade_ns = _epoch_ns(message.get("last_traded_time"), unit="s", field="last_traded_time")
        last_price = _decimal(message.get("ltp"), "ltp")
        last_quantity = _integer(message["last_traded_qty"], "last_traded_qty") if message.get("last_traded_qty") is not None else None
        volume = _integer(message["vol_traded_today"], "vol_traded_today") if message.get("vol_traded_today") is not None else None
        current_trade = (trade_ns, last_price, last_quantity, volume)
        previous_trade = self._last_trade.get(symbol)
        flags = []
        if previous_trade is not None and previous_trade[0] is not None and trade_ns is not None and trade_ns < previous_trade[0]:
            flags.append("trade_timestamp_regression")
        if previous_trade is not None and trade_ns == previous_trade[0] and previous_trade[3] is not None and volume is not None and volume < previous_trade[3]:
            flags.append("cumulative_volume_regression")
        if source_ns is None:
            flags.append("exchange_feed_time_missing")
        if not bids or not asks:
            flags.append("incomplete_depth")
        if bids and asks and bids[0].price > asks[0].price:
            flags.append("crossed_book")
        digest = raw_hash(message)
        venue = symbol.split(":", 1)[0] if ":" in symbol else None
        quote = MarketEvent(
            schema_version=1,
            event_id=f"fyers:{local_sequence if local_sequence is not None else received_utc_ns}:{symbol}:{digest[:16]}:book",
            provider="fyers", provider_instrument_id=symbol,
            instrument_id=self.instrument_map.get(symbol), venue=venue,
            event_kind=event_kind, received_utc_ns=received_utc_ns,
            received_monotonic_ns=received_monotonic_ns, local_sequence=local_sequence,
            source_timestamp_ns=source_ns,
            source_timestamp_semantics="fyers_exch_feed_time_epoch_seconds" if source_ns else None,
            last_trade_time_ns=trade_ns,
            last_trade_time_semantics="fyers_last_traded_time_epoch_seconds" if trade_ns else None,
            last_price=last_price, last_quantity=last_quantity, cumulative_volume=volume,
            aggressor_side=None, bids=tuple(bids), asks=tuple(asks), raw_sha256=digest,
            quality_flags=tuple(flags), source_frame_sha256=source_frame_sha256,
        )
        output = [quote]
        if kind == "sf" and _trade_advanced(previous_trade, current_trade):
            output.append(MarketEvent(
                schema_version=1,
                event_id=f"fyers:{local_sequence if local_sequence is not None else received_utc_ns}:{symbol}:{digest[:16]}:trade",
                provider="fyers", provider_instrument_id=symbol,
                instrument_id=self.instrument_map.get(symbol), venue=venue,
                event_kind="last_trade_observation", received_utc_ns=received_utc_ns,
                received_monotonic_ns=received_monotonic_ns, local_sequence=local_sequence,
                source_timestamp_ns=source_ns,
                source_timestamp_semantics="fyers_exch_feed_time_epoch_seconds" if source_ns else None,
                last_trade_time_ns=trade_ns, last_trade_time_semantics="fyers_last_traded_time_epoch_seconds",
                last_price=last_price, last_quantity=last_quantity, cumulative_volume=volume,
                aggressor_side=None, bids=tuple(bids), asks=tuple(asks), raw_sha256=digest,
                quality_flags=tuple(flags + ["trade_observation_inferred_from_ltt_and_volume_change", "not_a_confirmed_trade_print", "aggressor_side_unknown"]),
                source_frame_sha256=source_frame_sha256,
            ))
        if trade_ns is not None and (previous_trade is None or previous_trade[0] is None or
                trade_ns > previous_trade[0] or (trade_ns == previous_trade[0] and
                (previous_trade[3] is None or volume is None or volume >= previous_trade[3]))):
            self._last_trade[symbol] = current_trade
        return output

    def normalize(self, message: dict[str, Any], *, received_utc_ns: int,
                  received_monotonic_ns: int | None = None,
                  local_sequence: int | None = None,
                  source_frame_sha256: str | None = None) -> MarketEvent | None:
        """Backward-compatible single-state accessor; use normalize_observations for trades."""
        observations = self.normalize_observations(message, received_utc_ns=received_utc_ns,
            received_monotonic_ns=received_monotonic_ns, local_sequence=local_sequence,
            source_frame_sha256=source_frame_sha256)
        return observations[0] if observations else None
