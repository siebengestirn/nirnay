"""Deterministic event-driven cash-equity simulator.

The engine is intentionally a *single-instrument, single-active-order* model.
It consumes one selected provider feed, a bounded displayed L2 book and
explicit last-trade observations. It never claims FIFO certainty from L2.
Liquidity is consumed across repeated snapshots so unchanged displayed size
cannot be reused to fill the same simulated order over and over.
"""
from __future__ import annotations

import hashlib
from dataclasses import dataclass
from decimal import Decimal
from typing import Protocol, Sequence

from .market import MarketEvent

D = Decimal
BPS = D("10000")
_ACTIVE = {"pending_new", "working", "partially_filled", "pending_cancel"}


@dataclass(frozen=True, slots=True)
class StrategyContext:
    position_quantity: int
    has_open_order: bool
    cash: Decimal


@dataclass(frozen=True, slots=True)
class OrderIntent:
    client_order_id: str
    instrument_id: str
    side: str
    quantity: int
    limit_price: Decimal
    time_in_force_ns: int = 60_000_000_000

    def validate(self) -> None:
        if not self.client_order_id or not self.instrument_id:
            raise ValueError("intent identity is required")
        if self.side not in {"buy", "sell"}:
            raise ValueError("side must be buy or sell")
        if type(self.quantity) is not int or self.quantity <= 0:
            raise ValueError("quantity must be a positive integer")
        if not isinstance(self.limit_price, Decimal) or not self.limit_price.is_finite() or self.limit_price <= 0:
            raise ValueError("limit_price must be a positive finite Decimal")
        if type(self.time_in_force_ns) is not int or self.time_in_force_ns <= 0:
            raise ValueError("time_in_force must be a positive integer number of nanoseconds")


class Strategy(Protocol):
    strategy_id: str
    def on_event(self, event: MarketEvent, context: StrategyContext) -> OrderIntent | None: ...


@dataclass(frozen=True, slots=True)
class BacktestConfig:
    starting_cash: Decimal = D("100000")
    order_latency_ns: int = 20_000_000
    cancel_latency_ns: int = 10_000_000
    fee_bps: Decimal = D("2.0")
    maker_fee_bps: Decimal | None = None
    taker_fee_bps: Decimal | None = None
    impact_coefficient_bps: Decimal = D("1.0")
    max_participation: Decimal = D("0.10")
    max_book_age_ns: int = 1_000_000_000
    queue_model: str = "conservative_l2"
    queue_ahead_fraction: Decimal = D("1.0")
    seed: int = 7
    source_provider: str | None = None

    def validate(self) -> None:
        decimal_values = [self.starting_cash, self.fee_bps, self.impact_coefficient_bps,
                          self.max_participation, self.queue_ahead_fraction]
        decimal_values.extend(x for x in (self.maker_fee_bps, self.taker_fee_bps) if x is not None)
        if any(not isinstance(x, Decimal) or not x.is_finite() for x in decimal_values):
            raise ValueError("monetary and rate configuration values must be finite Decimals")
        if self.starting_cash <= 0 or self.order_latency_ns < 0 or self.cancel_latency_ns < 0:
            raise ValueError("invalid cash or latency configuration")
        if type(self.max_book_age_ns) is not int or self.max_book_age_ns <= 0:
            raise ValueError("max_book_age_ns must be a positive integer")
        if type(self.seed) is not int:
            raise ValueError("seed must be an integer")
        if any(x < 0 for x in (self.fee_bps, self.impact_coefficient_bps,
                               self.maker_fee_bps if self.maker_fee_bps is not None else D(0),
                               self.taker_fee_bps if self.taker_fee_bps is not None else D(0))):
            raise ValueError("cost assumptions must be non-negative")
        if not (D("0") < self.max_participation <= D("1")):
            raise ValueError("max_participation must be in (0, 1]")
        if self.queue_model not in {"conservative_l2", "seeded_l2_sensitivity"}:
            raise ValueError("unknown queue model")
        if not (D("0") <= self.queue_ahead_fraction <= D("1")):
            raise ValueError("queue_ahead_fraction must be in [0, 1]")
        if self.source_provider is not None and self.source_provider not in {"upstox", "fyers", "synthetic"}:
            raise ValueError("source_provider must be upstox, fyers, synthetic or None")

    def fee_rate(self, liquidity: str) -> Decimal:
        configured = self.maker_fee_bps if liquidity == "maker" else self.taker_fee_bps
        return self.fee_bps if configured is None else configured


@dataclass(slots=True)
class SimOrder:
    intent: OrderIntent
    submitted_ns: int
    activates_ns: int
    expires_ns: int
    remaining: int
    queue_ahead: int | None = None
    queue_observed_high_water: int = 0
    cancel_effective_ns: int | None = None
    status: str = "pending_new"


@dataclass(frozen=True, slots=True)
class SimFill:
    client_order_id: str
    event_id: str
    timestamp_ns: int
    side: str
    quantity: int
    price: Decimal
    fees: Decimal
    impact_cost: Decimal
    spread_cost: Decimal
    liquidity: str


@dataclass(frozen=True, slots=True)
class BacktestResult:
    strategy_id: str
    event_count: int
    order_count: int
    fills: tuple[SimFill, ...]
    rejected_intents: tuple[str, ...]
    ending_cash: Decimal
    ending_position: int
    mark_price: Decimal | None
    net_pnl: Decimal
    gross_pnl: Decimal
    fees: Decimal
    impact_cost: Decimal
    spread_cost: Decimal
    unfilled_quantity: int
    execution_model: str
    seed: int
    source_provider: str
    instrument_id: str

    def summary(self) -> dict[str, object]:
        return {
            "strategy_id": self.strategy_id, "event_count": self.event_count,
            "order_count": self.order_count, "fill_count": len(self.fills),
            "filled_quantity": sum(x.quantity for x in self.fills),
            "unfilled_quantity": self.unfilled_quantity,
            "ending_cash": str(self.ending_cash), "ending_position": self.ending_position,
            "mark_price": None if self.mark_price is None else str(self.mark_price),
            "gross_pnl": str(self.gross_pnl), "fees": str(self.fees),
            "impact_cost": str(self.impact_cost), "spread_cost": str(self.spread_cost),
            "net_pnl": str(self.net_pnl),
            "execution_model": self.execution_model, "seed": self.seed,
            "source_provider": self.source_provider, "instrument_id": self.instrument_id,
            "portfolio_scope": "single cash-equity instrument; long-only; one active order",
            "queue_position_claim": "L2 displayed-queue approximation; not FIFO certainty",
        }


class EventDrivenBacktester:
    def __init__(self, config: BacktestConfig):
        config.validate()
        self.config = config

    def run(self, events: Sequence[MarketEvent], strategy: Strategy) -> BacktestResult:
        if not events:
            raise ValueError("backtest requires market events")
        seen: set[str] = set()
        mapped = [e for e in events if e.instrument_id is not None]
        if len(mapped) != len(events):
            raise ValueError("backtest input contains unresolved provider instrument IDs")
        instruments = {e.instrument_id for e in mapped}
        if len(instruments) != 1:
            raise ValueError("this simulator is single-instrument; portfolio replay requires a portfolio engine")
        providers = {e.provider for e in mapped}
        if self.config.source_provider is None and len(providers) != 1:
            raise ValueError("mixed-provider replay requires explicit source_provider; do not double-count cross-feed liquidity")
        selected_provider = self.config.source_provider or next(iter(providers))
        selected = [e for e in mapped if e.provider == selected_provider]
        if not selected:
            raise ValueError(f"no events for selected provider {selected_provider}")
        instrument_id = next(iter(instruments))
        if any(e.instrument_id != instrument_id for e in selected):
            raise ValueError("selected provider events do not share one canonical instrument")
        ordered = sorted(selected, key=lambda e: (e.received_utc_ns,
                          e.local_sequence if e.local_sequence is not None else -1,
                          e.event_id))

        cash = self.config.starting_cash
        position = 0
        orders: dict[str, SimOrder] = {}
        fills: list[SimFill] = []
        rejected: list[str] = []
        total_fees = D("0")
        total_impact = D("0")
        total_spread_cost = D("0")
        last_mark: Decimal | None = None
        last_book_midpoint: Decimal | None = None
        last_book_ns: int | None = None
        # side -> price -> last displayed quantity.  A new liquidity tranche is
        # the positive size increment only; identical snapshots are not volume.
        displayed_state: dict[str, dict[Decimal, int]] = {"bid": {}, "ask": {}}
        new_liquidity: dict[str, dict[Decimal, int]] = {"bid": {}, "ask": {}}
        last_book_snapshot: tuple[object, ...] | None = None

        def refresh_side(side: str, levels) -> dict[Decimal, int]:
            old = displayed_state[side]
            current = {level.price: level.quantity for level in levels}
            increments = {price: (qty if price not in old else max(0, qty - old[price]))
                          for price, qty in current.items()}
            displayed_state[side] = current
            new_liquidity[side] = increments
            return increments

        def apply_fill(order: SimOrder, event: MarketEvent, qty: int, base_price: Decimal,
                       liquidity_role: str, displayed_qty: int) -> int:
            nonlocal cash, position, total_fees, total_impact, total_spread_cost
            if qty <= 0 or displayed_qty <= 0:
                return 0
            qty = min(qty, order.remaining)
            if order.intent.side == "sell":
                qty = min(qty, max(position, 0))
            max_by_participation = int(D(displayed_qty) * self.config.max_participation)
            qty = min(qty, max_by_participation)
            if qty <= 0:
                return 0
            participation = D(qty) / D(max(displayed_qty, 1))
            impact_bps = self.config.impact_coefficient_bps * participation.sqrt() if liquidity_role == "taker" else D(0)
            sign = D("1") if order.intent.side == "buy" else D("-1")
            impact_per_share = base_price * impact_bps / BPS
            execution_price = base_price + sign * impact_per_share
            if ((order.intent.side == "buy" and execution_price > order.intent.limit_price) or
                    (order.intent.side == "sell" and execution_price < order.intent.limit_price)):
                return 0
            notional = execution_price * qty
            fee_rate = self.config.fee_rate(liquidity_role)
            fees = notional * fee_rate / BPS
            # Impact is already embedded in taker execution_price. It is
            # itemised for analysis but must not be debited a second time.
            # Report impact as a positive cost for both buy and sell orders.
            # The signed price displacement is already reflected in cash/P&L.
            impact_cost = abs(impact_per_share) * qty
            # Compare the touch/limit base price with the last valid two-sided
            # midpoint. Positive is a spread cost; negative is maker price
            # improvement. The midpoint is from a fresh book because book_fresh
            # is checked before this fill path is entered.
            side_sign = D("1") if order.intent.side == "buy" else D("-1")
            spread_cost = (D("0") if last_book_midpoint is None else
                           side_sign * (base_price - last_book_midpoint) * qty)
            if order.intent.side == "buy":
                cash -= notional + fees
                position += qty
            else:
                cash += notional - fees
                position -= qty
            if cash < 0 or position < 0:
                raise AssertionError("cash-equity accounting invariant violated")
            total_fees += fees
            total_impact += impact_cost
            total_spread_cost += spread_cost
            fills.append(SimFill(order.intent.client_order_id, event.event_id,
                                 event.received_utc_ns, order.intent.side, qty,
                                 execution_price, fees, impact_cost, spread_cost, liquidity_role))
            order.remaining -= qty
            order.status = "filled" if order.remaining == 0 else "partially_filled"
            return qty

        def active_orders() -> list[SimOrder]:
            return [o for o in orders.values() if o.remaining > 0 and o.status in _ACTIVE]

        for event in ordered:
            if event.event_id in seen:
                raise ValueError(f"duplicate event_id in replay: {event.event_id}")
            seen.add(event.event_id)
            if event.midpoint is not None:
                last_mark = event.midpoint

            is_book = event.event_kind in {"quote", "depth"} and "crossed_book" not in event.quality_flags
            book_changed = False
            if is_book:
                before = last_book_snapshot
                current_snapshot = (
                    tuple((x.price, x.quantity) for x in event.bids),
                    tuple((x.price, x.quantity) for x in event.asks),
                )
                book_changed = current_snapshot != before
                last_book_snapshot = current_snapshot
                refresh_side("bid", event.bids)
                refresh_side("ask", event.asks)
                last_book_ns = event.received_utc_ns
                if event.best_bid is not None and event.best_ask is not None:
                    last_book_midpoint = (event.best_bid.price + event.best_ask.price) / D("2")
                # Conservatively add new visible size at a resting order's
                # level only when it exceeds that level's previous high-water.
                for order in active_orders():
                    side = "bid" if order.intent.side == "buy" else "ask"
                    observed = next((x.quantity for x in (event.bids if side == "bid" else event.asks)
                                     if x.price == order.intent.limit_price), 0)
                    if observed > order.queue_observed_high_water:
                        if order.queue_ahead is not None:
                            order.queue_ahead += observed - order.queue_observed_high_water
                        order.queue_observed_high_water = observed

            # Only a confirmed `trade` event can consume modeled resting queue.
            # Vendor LTP snapshots are represented by last_trade_observation and
            # are deliberately excluded: they are not trade prints.
            for order in active_orders():
                if event.received_utc_ns >= order.expires_ns and order.cancel_effective_ns is None:
                    order.status = "pending_cancel"
                    order.cancel_effective_ns = order.expires_ns + self.config.cancel_latency_ns
                if order.cancel_effective_ns is not None and event.received_utc_ns >= order.cancel_effective_ns:
                    order.status = "expired"
                    continue
                activated_now = False
                if order.status == "pending_new":
                    if event.received_utc_ns < order.activates_ns or not is_book:
                        continue
                    order.status = "working"
                    activated_now = True
                    side_levels = event.bids if order.intent.side == "buy" else event.asks
                    displayed = next((x.quantity for x in side_levels if x.price == order.intent.limit_price), 0)
                    if self.config.queue_model == "conservative_l2":
                        order.queue_ahead = displayed
                    else:
                        raw = hashlib.sha256(f"{self.config.seed}:{order.intent.client_order_id}".encode()).digest()
                        pct = int.from_bytes(raw[:4], "big") / (2**32 - 1)
                        order.queue_ahead = int(displayed * float(self.config.queue_ahead_fraction) * pct)
                    order.queue_observed_high_water = displayed

                # Fresh book snapshots only: do not use stale depth or repeat
                # liquidity from a trade-only observation.
                book_fresh = (last_book_ns is not None and
                              0 <= event.received_utc_ns - last_book_ns <= self.config.max_book_age_ns)
                if is_book and book_fresh and order.remaining > 0 and (activated_now or book_changed):
                    opposite_side = "ask" if order.intent.side == "buy" else "bid"
                    levels = event.asks if opposite_side == "ask" else event.bids
                    eligible = [x for x in levels if (x.price <= order.intent.limit_price if order.intent.side == "buy"
                                                        else x.price >= order.intent.limit_price)]
                    for level in eligible:
                        if order.remaining <= 0:
                            break
                        # On activation, the current book is eligible once. On
                        # later updates, only net newly displayed size is fresh
                        # executable liquidity. An identical snapshot cannot
                        # be reused; a reduction does not imply our fill.
                        tranche = level.quantity if activated_now else new_liquidity[opposite_side].get(level.price, 0)
                        if tranche <= 0:
                            continue
                        filled = apply_fill(order, event, min(order.remaining, tranche),
                                            level.price, "taker", level.quantity)

                if event.event_kind == "trade" and order.remaining > 0 and book_fresh:
                    trade_price = event.last_price
                    trade_qty = event.last_quantity or 0
                    if trade_price is None or trade_qty <= 0:
                        continue
                    at_or_through = (trade_price <= order.intent.limit_price if order.intent.side == "buy"
                                     else trade_price >= order.intent.limit_price)
                    if not at_or_through:
                        continue
                    # A print with unknown aggressor cannot prove that the
                    # resting side was hit. Even a through-price observation may
                    # be an unattributed/derived print from another venue.
                    if event.aggressor_side is None:
                        continue
                    eligible_aggressor = (event.aggressor_side == "sell" if order.intent.side == "buy"
                                          else event.aggressor_side == "buy")
                    if not eligible_aggressor:
                        continue
                    prior_queue = order.queue_ahead or 0
                    queue_consumed = min(prior_queue, trade_qty)
                    order.queue_ahead = prior_queue - queue_consumed
                    executable = max(0, trade_qty - queue_consumed)
                    if executable:
                        # No optimistic price improvement is assumed for maker fills.
                        apply_fill(order, event, min(executable, order.remaining),
                                   order.intent.limit_price, "maker", max(trade_qty, 1))

            if "crossed_book" in event.quality_flags:
                continue
            context = StrategyContext(position, bool(active_orders()), cash)
            intent = strategy.on_event(event, context)
            if intent is not None:
                try:
                    intent.validate()
                    if intent.instrument_id != instrument_id or intent.instrument_id != event.instrument_id:
                        raise ValueError("strategy intent instrument differs from selected replay instrument")
                    if intent.client_order_id in orders:
                        raise ValueError("duplicate client_order_id")
                    active = active_orders()
                    if active:
                        raise ValueError("simulator admits one active order at a time")
                    if intent.side == "sell":
                        if intent.quantity > position:
                            raise ValueError("sell quantity exceeds long position; shorting is disabled")
                    else:
                        fee_impact_bps = max(self.config.fee_rate("maker"), self.config.fee_rate("taker")) + self.config.impact_coefficient_bps
                        reserve = D(intent.quantity) * intent.limit_price * (D("1") + fee_impact_bps / BPS)
                        if reserve > cash:
                            raise ValueError("buy order exceeds available cash after modeled fee/impact reserve")
                    order = SimOrder(intent, event.received_utc_ns,
                                     event.received_utc_ns + self.config.order_latency_ns,
                                     event.received_utc_ns + self.config.order_latency_ns + intent.time_in_force_ns,
                                     intent.quantity)
                    orders[intent.client_order_id] = order
                except ValueError as exc:
                    rejected.append(f"{intent.client_order_id}: {exc}")

        gross_pnl = cash + (D(position) * last_mark if last_mark is not None else D("0")) - self.config.starting_cash
        net_pnl = gross_pnl
        unfilled = sum(x.remaining for x in orders.values() if x.remaining > 0)
        fee_label = f"maker={self.config.fee_rate('maker')};taker={self.config.fee_rate('taker')}"
        return BacktestResult(
            strategy_id=strategy.strategy_id, event_count=len(ordered), order_count=len(orders),
            fills=tuple(fills), rejected_intents=tuple(rejected), ending_cash=cash,
            ending_position=position, mark_price=last_mark, net_pnl=net_pnl,
            gross_pnl=gross_pnl + total_fees + total_impact + total_spread_cost,
            fees=total_fees, impact_cost=total_impact, spread_cost=total_spread_cost,
            unfilled_quantity=unfilled,
            execution_model=f"{self.config.queue_model};latency_ns={self.config.order_latency_ns};"
                            f"max_participation={self.config.max_participation};fees_bps={fee_label};"
                            f"impact_coefficient_bps={self.config.impact_coefficient_bps};"
                            f"max_book_age_ns={self.config.max_book_age_ns}",
            seed=self.config.seed, source_provider=selected_provider, instrument_id=instrument_id)
