"""Multi-instrument event-driven cash-equity portfolio backtester.

This portfolio engine consumes exactly one provider's event stream to avoid
counting duplicate cross-provider liquidity. It supports multiple NSE cash
instruments, independent per-symbol order lifecycles, partial fills, latency,
conservative L2 queue approximation, fees, spread and impact charges, cash and
gross-exposure budgets. It is explicitly long-only: short inventory, borrow,
margin and settlement rules require a separately specified product model.
"""
from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
from decimal import Decimal
from types import MappingProxyType
from typing import Mapping, Protocol, Sequence

from .backtest import OrderIntent, SimFill, SimOrder
from .market import MarketEvent

D = Decimal
BPS = D("10000")


@dataclass(frozen=True, slots=True)
class PortfolioContext:
    cash: Decimal
    equity: Decimal
    positions: Mapping[str, int]
    open_orders: frozenset[str]


class PortfolioStrategy(Protocol):
    strategy_id: str
    def on_event(self, event: MarketEvent, context: PortfolioContext) -> OrderIntent | None: ...


@dataclass(frozen=True, slots=True)
class PortfolioBacktestConfig:
    starting_cash: Decimal = D("1000000")
    order_latency_ns: int = 20_000_000
    cancel_latency_ns: int = 10_000_000
    fee_bps: Decimal = D("2.0")
    maker_fee_bps: Decimal | None = None
    taker_fee_bps: Decimal | None = None
    impact_coefficient_bps: Decimal = D("1.0")
    max_participation: Decimal = D("0.10")
    max_book_age_ns: int = 1_000_000_000
    max_order_notional: Decimal = D("100000")
    max_gross_exposure: Decimal = D("1000000")
    source_provider: str | None = None

    def validate(self) -> None:
        values = [self.starting_cash, self.fee_bps, self.impact_coefficient_bps,
                  self.max_participation, self.max_order_notional, self.max_gross_exposure]
        values += [x for x in (self.maker_fee_bps, self.taker_fee_bps) if x is not None]
        if any(not isinstance(x, Decimal) or not x.is_finite() for x in values):
            raise ValueError("portfolio config monetary/rate fields must be finite Decimal values")
        if self.starting_cash <= 0 or self.max_order_notional <= 0 or self.max_gross_exposure <= 0:
            raise ValueError("portfolio cash and exposure limits must be positive")
        if self.order_latency_ns < 0 or self.cancel_latency_ns < 0 or type(self.max_book_age_ns) is not int or self.max_book_age_ns <= 0:
            raise ValueError("latency and book-age settings are invalid")
        if not (D(0) < self.max_participation <= D(1)):
            raise ValueError("max_participation must be in (0,1]")
        if any(x < 0 for x in (self.fee_bps, self.impact_coefficient_bps,
                               self.maker_fee_bps or D(0), self.taker_fee_bps or D(0))):
            raise ValueError("fees and impact must be non-negative")
        if self.source_provider is not None and self.source_provider not in {"upstox", "fyers", "synthetic"}:
            raise ValueError("source_provider must be upstox, fyers, synthetic or None")

    def fee(self, liquidity: str) -> Decimal:
        configured = self.maker_fee_bps if liquidity == "maker" else self.taker_fee_bps
        return self.fee_bps if configured is None else configured


@dataclass(frozen=True, slots=True)
class PortfolioBacktestResult:
    strategy_id: str
    source_provider: str
    events: int
    orders: int
    fills: tuple[SimFill, ...]
    rejections: tuple[str, ...]
    ending_cash: Decimal
    positions: Mapping[str, int]
    marks: Mapping[str, Decimal]
    gross_exposure: Decimal
    ending_equity: Decimal
    gross_pnl: Decimal
    fees: Decimal
    impact_cost: Decimal
    spread_cost: Decimal
    net_pnl: Decimal
    unfilled_quantity: int
    model: str

    def summary(self) -> dict[str, object]:
        return {
            "strategy_id": self.strategy_id, "source_provider": self.source_provider,
            "event_count": self.events, "order_count": self.orders, "fill_count": len(self.fills),
            "filled_quantity": sum(x.quantity for x in self.fills),
            "unfilled_quantity": self.unfilled_quantity,
            "ending_cash": str(self.ending_cash),
            "positions": dict(sorted(self.positions.items())),
            "marks": {k: str(v) for k, v in sorted(self.marks.items())},
            "gross_exposure": str(self.gross_exposure), "ending_equity": str(self.ending_equity),
            "gross_pnl": str(self.gross_pnl), "fees": str(self.fees),
            "impact_cost": str(self.impact_cost), "spread_cost": str(self.spread_cost),
            "net_pnl": str(self.net_pnl), "model": self.model,
            "scope": "multi-instrument NSE cash; long-only; no margin or borrow",
            "queue_claim": "market-by-price approximation using visible aggregate depth; not exchange FIFO or L3; hidden liquidity and cancel priority are unobservable",
        }


class MultiInstrumentPortfolioBacktester:
    def __init__(self, config: PortfolioBacktestConfig):
        config.validate()
        self.config = config

    def run(self, events: Sequence[MarketEvent], strategy: PortfolioStrategy) -> PortfolioBacktestResult:
        if not events:
            raise ValueError("portfolio replay requires events")
        if any(e.instrument_id is None for e in events):
            raise ValueError("portfolio replay contains unresolved instrument mappings")
        providers = {e.provider for e in events}
        if self.config.source_provider is None and len(providers) != 1:
            raise ValueError("mixed-provider replay requires source_provider; cross-feed liquidity must not be summed")
        provider = self.config.source_provider or next(iter(providers))
        selected = [e for e in events if e.provider == provider]
        if not selected:
            raise ValueError(f"no events for selected provider {provider}")
        ordered = sorted(selected, key=lambda e: (e.received_utc_ns,
                         e.local_sequence if e.local_sequence is not None else -1,
                         e.instrument_id or "", e.event_id))
        if len({e.event_id for e in ordered}) != len(ordered):
            raise ValueError("duplicate event_id in portfolio replay")

        cash = self.config.starting_cash
        positions: dict[str, int] = defaultdict(int)
        marks: dict[str, Decimal] = {}
        orders: dict[str, SimOrder] = {}
        active_by_instrument: dict[str, str] = {}
        fills: list[SimFill] = []
        rejects: list[str] = []
        fees = impact_total = spread_total = D(0)
        books: dict[str, tuple[tuple[object, ...], tuple[object, ...]]] = {}
        book_time: dict[str, int] = {}
        displayed: dict[str, dict[str, dict[Decimal, int]]] = defaultdict(lambda: {"bid": {}, "ask": {}})
        queue_high: dict[str, int] = {}
        queue_ahead: dict[str, int] = {}
        last_seq: dict[str, int] = {}

        def active_order(instrument: str) -> SimOrder | None:
            order_id = active_by_instrument.get(instrument)
            return None if order_id is None else orders[order_id]

        def equity() -> Decimal:
            return cash + sum((D(qty) * marks[symbol] for symbol, qty in positions.items()
                               if qty and symbol in marks), D(0))

        def gross_exposure() -> Decimal:
            return sum((D(abs(qty)) * marks[symbol] for symbol, qty in positions.items()
                        if qty and symbol in marks), D(0))

        def reserved_buy_notional() -> Decimal:
            # Filled quantity is already reflected in cash/positions. Reserve
            # only the still-open remainder to prevent cross-symbol overcommit.
            total = D(0)
            for order in orders.values():
                if order.status in {"pending_new", "working", "partially_filled", "pending_cancel"} and order.intent.side == "buy":
                    total += D(order.remaining) * order.intent.limit_price
            return total

        def fill(order: SimOrder, event: MarketEvent, qty: int, price: Decimal,
                 liquidity: str, displayed_qty: int) -> None:
            nonlocal cash, fees, impact_total, spread_total
            instrument = order.intent.instrument_id
            qty = min(qty, order.remaining)
            if order.intent.side == "sell":
                qty = min(qty, positions[instrument])
            if qty <= 0 or displayed_qty <= 0:
                return
            fee = D(qty) * price * self.config.fee(liquidity) / BPS
            impact_bps = self.config.impact_coefficient_bps * D(qty) / D(max(1, displayed_qty))
            impact = D(qty) * price * impact_bps / BPS
            mid = event.midpoint or price
            spread = D(qty) * max(D(0), price - mid if order.intent.side == "buy" else mid - price)
            if order.intent.side == "buy":
                cash -= D(qty) * price + fee + impact
                positions[instrument] += qty
            else:
                cash += D(qty) * price - fee - impact
                positions[instrument] -= qty
            order.remaining -= qty
            if order.remaining == 0:
                order.status = "filled"
                active_by_instrument.pop(instrument, None)
            else:
                order.status = "partially_filled"
            fees += fee; impact_total += impact; spread_total += spread
            fills.append(SimFill(order.intent.client_order_id, event.event_id,
                                 event.received_utc_ns, order.intent.side,
                                 qty, price, fee, impact, spread, liquidity))

        for event in ordered:
            instrument = event.instrument_id
            assert instrument is not None
            if event.local_sequence is not None:
                prior = last_seq.get(instrument)
                if prior is not None and event.local_sequence <= prior:
                    raise ValueError(f"provider sequence regression for {instrument}")
                last_seq[instrument] = event.local_sequence
            if event.midpoint is not None:
                marks[instrument] = event.midpoint
            is_book = event.event_kind in {"quote", "depth"} and "crossed_book" not in event.quality_flags
            changed = False
            new = {"bid": {}, "ask": {}}
            if is_book:
                old_snapshot = books.get(instrument)
                snapshot = (tuple((x.price, x.quantity) for x in event.bids),
                            tuple((x.price, x.quantity) for x in event.asks))
                changed = snapshot != old_snapshot
                books[instrument] = snapshot
                for side, levels in (("bid", event.bids), ("ask", event.asks)):
                    previous = displayed[instrument][side]
                    current = {x.price: x.quantity for x in levels}
                    new[side] = {p: (q if p not in previous else max(0, q - previous[p]))
                                 for p, q in current.items()}
                    displayed[instrument][side] = current
                book_time[instrument] = event.received_utc_ns
            order = active_order(instrument)
            if order is not None:
                if event.received_utc_ns >= order.expires_ns and order.cancel_effective_ns is None:
                    order.status = "pending_cancel"
                    order.cancel_effective_ns = order.expires_ns + self.config.cancel_latency_ns
                if order.cancel_effective_ns is not None and event.received_utc_ns >= order.cancel_effective_ns:
                    order.status = "expired"
                    active_by_instrument.pop(instrument, None)
                    order = None
            if order is not None:
                activated = False
                if order.status == "pending_new" and is_book and event.received_utc_ns >= order.activates_ns:
                    order.status = "working"; activated = True
                    queue_side = "bid" if order.intent.side == "buy" else "ask"
                    shown = next((x.quantity for x in (event.bids if queue_side == "bid" else event.asks)
                                  if x.price == order.intent.limit_price), 0)
                    queue_ahead[order.intent.client_order_id] = shown
                    queue_high[order.intent.client_order_id] = shown
                fresh = (instrument in book_time and
                         0 <= event.received_utc_ns - book_time[instrument] <= self.config.max_book_age_ns)
                if is_book and order.status in {"working", "partially_filled", "pending_cancel"} and not activated:
                    # L2 cannot tell whether new displayed orders joined ahead
                    # or behind us. For a conservative market-by-price model,
                    # every positive increase at our price is assumed ahead.
                    queue_side = "bid" if order.intent.side == "buy" else "ask"
                    visible_at_price = next((x.quantity for x in (event.bids if queue_side == "bid" else event.asks)
                                             if x.price == order.intent.limit_price), 0)
                    high = queue_high.get(order.intent.client_order_id, 0)
                    increase = max(0, visible_at_price - high)
                    if increase:
                        queue_ahead[order.intent.client_order_id] = queue_ahead.get(order.intent.client_order_id, 0) + increase
                        queue_high[order.intent.client_order_id] = visible_at_price
                if is_book and fresh and (activated or changed) and order.remaining:
                    contra = event.asks if order.intent.side == "buy" else event.bids
                    side_name = "ask" if order.intent.side == "buy" else "bid"
                    for level in contra:
                        crosses = level.price <= order.intent.limit_price if order.intent.side == "buy" else level.price >= order.intent.limit_price
                        if not crosses:
                            continue
                        tranche = level.quantity if activated else new[side_name].get(level.price, 0)
                        cap = int(D(tranche) * self.config.max_participation)
                        if cap:
                            fill(order, event, min(order.remaining, cap), level.price, "taker", level.quantity)
                        if order.remaining == 0:
                            break
                if (order.remaining and event.event_kind == "trade" and fresh and
                        event.last_price is not None and event.last_quantity and event.aggressor_side):
                    through = event.last_price <= order.intent.limit_price if order.intent.side == "buy" else event.last_price >= order.intent.limit_price
                    aggressor_hits = event.aggressor_side == ("sell" if order.intent.side == "buy" else "buy")
                    if through and aggressor_hits:
                        ahead = queue_ahead.get(order.intent.client_order_id, 0)
                        consumed = min(ahead, event.last_quantity)
                        queue_ahead[order.intent.client_order_id] = ahead - consumed
                        executable = max(0, event.last_quantity - consumed)
                        cap = int(D(event.last_quantity) * self.config.max_participation)
                        if executable and cap:
                            fill(order, event, min(executable, cap), order.intent.limit_price,
                                 "maker", event.last_quantity)

            if "crossed_book" in event.quality_flags:
                continue
            open_ids = frozenset(active_by_instrument.values())
            context = PortfolioContext(cash, equity(), MappingProxyType(dict(positions)), open_ids)
            intent = strategy.on_event(event, context)
            if intent is None:
                continue
            try:
                intent.validate()
                if intent.instrument_id != instrument:
                    raise ValueError("strategy intent must target the instrument of its triggering event")
                if intent.client_order_id in orders:
                    raise ValueError("duplicate client_order_id")
                if instrument in active_by_instrument:
                    raise ValueError("one active order per instrument is permitted")
                notional = D(intent.quantity) * intent.limit_price
                if notional > self.config.max_order_notional:
                    raise ValueError("order notional exceeds portfolio config cap")
                if intent.side == "sell" and intent.quantity > positions[instrument]:
                    raise ValueError("cash-equity shorting is not supported by this engine")
                if intent.side == "buy":
                    reserve = notional * (D(1) + max(self.config.fee("maker"), self.config.fee("taker")) / BPS
                                          + self.config.impact_coefficient_bps / BPS)
                    if reserve + reserved_buy_notional() > cash:
                        raise ValueError("buy exceeds available cash after reserving all open buy orders and costs")
                    estimated_gross = gross_exposure() + reserved_buy_notional() + notional
                    if estimated_gross > self.config.max_gross_exposure:
                        raise ValueError("projected gross exposure exceeds portfolio cap")
                sim = SimOrder(intent, event.received_utc_ns,
                               event.received_utc_ns + self.config.order_latency_ns,
                               event.received_utc_ns + self.config.order_latency_ns + intent.time_in_force_ns,
                               intent.quantity)
                orders[intent.client_order_id] = sim
                active_by_instrument[instrument] = intent.client_order_id
            except ValueError as exc:
                rejects.append(f"{intent.client_order_id}: {exc}")

        marked_value = sum((D(qty) * marks[symbol] for symbol, qty in positions.items()
                            if qty and symbol in marks), D(0))
        ending_equity = cash + marked_value
        net_pnl = ending_equity - self.config.starting_cash
        unfilled = sum(x.remaining for x in orders.values() if x.remaining > 0)
        gross_pnl = net_pnl + fees + impact_total + spread_total
        return PortfolioBacktestResult(
            strategy.strategy_id, provider, len(ordered), len(orders), tuple(fills), tuple(rejects),
            cash, MappingProxyType({k: v for k, v in positions.items() if v}),
            MappingProxyType(dict(marks)), gross_exposure(), ending_equity, gross_pnl,
            fees, impact_total, spread_total, net_pnl, unfilled,
            f"multi_instrument_l2_queue;latency_ns={self.config.order_latency_ns};"
            f"max_participation={self.config.max_participation};max_book_age_ns={self.config.max_book_age_ns};"
            f"impact_coefficient_bps={self.config.impact_coefficient_bps}",
        )
