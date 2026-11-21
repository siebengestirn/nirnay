"""Pure event reducer and deterministic simulation engine.

Only ``ExecutionReport`` events may change cash and positions.  Book and trade
events emit execution reports; the engine feeds those reports back through the
same reducer, so P&L is always recomputed from emitted fills and marks.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
from decimal import ROUND_HALF_UP, Decimal, localcontext
from typing import Any, Protocol, Sequence

from .events import (
    EVENT_TYPES,
    AuctionPhase,
    BookDelta,
    BookSnapshot,
    ConfigChange,
    EventEnvelope,
    ExecutionReport,
    InstrumentChange,
    MarketStatus,
    OrderAdd,
    OrderCancel,
    OrderIntent,
    OrderModify,
    SimulationEvent,
    TradePrint,
    UnmarkedResidual,
    digest,
    event_ordering_key,
    from_jsonable,
    to_jsonable,
)
from .execution.base import BaselineParams, ConservativeMbpModel, ExecutionModel, default_cost_model
from .execution.costs import BPS
from .state import (
    BookState,
    ChargeRecord,
    ExposureState,
    MarkState,
    OrderState,
    QueueState,
    RiskBudgets,
    SimState,
    book_key,
    empty_state,
    queue_key,
    state_hash,
)

_DAY_NS = 86_400_000_000_000
_TRADABLE_STATUSES = frozenset({"open", "resume"})


@dataclass(frozen=True, slots=True)
class ReplaySeeds:
    """Every seed consulted by a run; no PRNG state is hidden elsewhere."""

    config_seed: int
    strategy_seed: int
    latency_seed: int
    impact_seed: int

    def __post_init__(self) -> None:
        for name in ("config_seed", "strategy_seed", "latency_seed", "impact_seed"):
            value = getattr(self, name)
            if type(value) is not int:
                raise ValueError(f"{name} must be an integer")

    def digest(self) -> str:
        """Return the deterministic digest of all seeds."""
        return digest(to_jsonable(self))

    def to_dict(self) -> dict[str, int]:
        """Return the canonical serializable form."""
        return dict(to_jsonable(self))

    @classmethod
    def from_dict(cls, data: dict[str, int]) -> ReplaySeeds:
        """Reconstruct seeds from their canonical form."""
        return from_jsonable(cls, data)


@dataclass(frozen=True, slots=True)
class EngineConfig:
    """Run-level configuration; monetary values remain exact Decimals."""

    starting_cash: Decimal
    strategy_id: str = "sim.replay"
    risk_budgets: RiskBudgets = RiskBudgets()
    order_latency_ns: int = 20_000_000
    cancel_latency_ns: int = 10_000_000
    max_participation: Decimal = Decimal("0.10")
    max_book_age_ns: int = 1_000_000_000

    def __post_init__(self) -> None:
        if not isinstance(self.starting_cash, Decimal) or self.starting_cash <= 0:
            raise ValueError("starting_cash must be a positive finite Decimal")
        if not isinstance(self.strategy_id, str) or not self.strategy_id:
            raise ValueError("strategy_id is required")
        if not isinstance(self.risk_budgets, RiskBudgets):
            raise ValueError("risk_budgets must be a RiskBudgets")
        for name in ("order_latency_ns", "cancel_latency_ns", "max_book_age_ns"):
            value = getattr(self, name)
            if type(value) is not int or value <= 0:
                raise ValueError(f"{name} must be a positive integer")
        if not isinstance(self.max_participation, Decimal) or not (
            Decimal("0") < self.max_participation <= Decimal("1")
        ):
            raise ValueError("max_participation must be in (0, 1]")

    def config_hash(self) -> str:
        """Return the deterministic digest of this configuration."""
        return digest(to_jsonable(self))

    def to_dict(self) -> dict[str, Any]:
        """Return the canonical serializable form."""
        return to_jsonable(self)

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> EngineConfig:
        """Reconstruct configuration from its canonical form."""
        return from_jsonable(cls, data)


@dataclass(frozen=True, slots=True)
class StrategyContext:
    """Read-only view given to a strategy; never cash or position mutable state."""

    instrument_id: str
    logical_time_ns: int
    cash: Decimal
    position: int
    has_open_order: bool
    mark: Decimal | None


class Strategy(Protocol):
    """A strategy emits order intents only, never execution or accounting."""

    strategy_id: str

    def on_event(self, event: SimulationEvent, context: StrategyContext) -> OrderIntent | None:
        ...


@dataclass(frozen=True, slots=True)
class SimResult:
    """All recomputed outputs of one deterministic run."""

    strategy_id: str
    event_count: int
    emitted: tuple[SimulationEvent, ...]
    consumed_events: tuple[SimulationEvent, ...]
    fills: tuple[ExecutionReport, ...]
    ending_cash: Decimal
    positions: dict[str, int]
    marks: dict[str, Decimal]
    net_pnl: Decimal
    gross_pnl: Decimal
    fees: Decimal
    impact_cost: Decimal
    spread_cost: Decimal
    unmarked_residual: bool
    unmarked_details: tuple[UnmarkedResidual, ...]
    borrow_cost: Decimal
    funding_cost: Decimal
    charges: tuple[ChargeRecord, ...]
    final_state: SimState
    state_hash: str
    config_hash: str
    model_digests: tuple[str, ...]
    seeds_digest: str

    def summary(self) -> dict[str, object]:
        """Return a compact, claim-limited view of the run."""
        return {
            "strategy_id": self.strategy_id,
            "event_count": self.event_count,
            "fill_count": len(self.fills),
            "filled_quantity": sum(item.quantity for item in self.fills),
            "ending_cash": format(self.ending_cash, "f"),
            "positions": {key: self.positions[key] for key in sorted(self.positions)},
            "net_pnl": format(self.net_pnl, "f"),
            "gross_pnl": format(self.gross_pnl, "f"),
            "fees": format(self.fees, "f"),
            "impact_cost": format(self.impact_cost, "f"),
            "spread_cost": format(self.spread_cost, "f"),
            "borrow_cost": format(self.borrow_cost, "f"),
            "funding_cost": format(self.funding_cost, "f"),
            "charge_basis": "research-only; borrow/margin/funding are assumed, not calibrated",
            "unmarked_residual": self.unmarked_residual,
            "state_hash": self.state_hash,
            "config_hash": self.config_hash,
            "model_digests": list(self.model_digests),
            "seeds_digest": self.seeds_digest,
            "queue_position_claim": "unavailable; MBP approximation, not FIFO certainty",
            "research_scope": "research simulator; no exchange certification or live use",
        }


def _report(
    base: EventEnvelope,
    client_order_id: str,
    report_type: str,
    *,
    suffix: str = "",
    quantity: int = 0,
    price: Decimal | None = None,
    fees: Decimal = Decimal("0"),
    impact_cost: Decimal = Decimal("0"),
    spread_cost: Decimal = Decimal("0"),
    liquidity: str | None = None,
    reason: str | None = None,
    logical_time_ns: int | None = None,
    source_sequence: int | None = None,
    sampled_ns: int | None = None,
    effective_ns: int | None = None,
) -> ExecutionReport:
    """Derive an execution report from a causing event with full lineage."""
    return ExecutionReport(
        event_id=f"er:{report_type}:{client_order_id}:{base.event_id}:{suffix}",
        source_frame_sha256=base.source_frame_sha256,
        instrument_id=base.instrument_id,
        venue=base.venue,
        logical_time_ns=base.logical_time_ns if logical_time_ns is None else logical_time_ns,
        available_time_ns=base.available_time_ns,
        origin_class="simulated",
        derived_from=(base.event_id,),
        lineage=base.lineage,
        source_sequence=base.source_sequence + 1 if source_sequence is None else source_sequence,
        client_order_id=client_order_id,
        report_type=report_type,
        quantity=quantity,
        price=price,
        fees=fees,
        impact_cost=impact_cost,
        spread_cost=spread_cost,
        liquidity=liquidity,
        reason=reason,
        sampled_ns=sampled_ns,
        effective_ns=effective_ns,
    )


def _timed_reports(state: SimState, event: EventEnvelope) -> tuple[SimState, tuple]:
    """Emit new/expire/cancel_ack reports whose effective time has arrived."""
    orders = dict(state.orders)
    out: list[SimulationEvent] = []
    for order_id in sorted(orders):
        order = orders[order_id]
        if order.venue != event.venue or order.instrument_id != event.instrument_id:
            continue
        if order.status == "pending_new" and not order.announced:
            if event.logical_time_ns >= order.activates_ns:
                out.append(_report(event, order_id, "new"))
                orders[order_id] = replace_order(order, announced=True, status="working")
        current = orders[order_id]
        cancel_due = current.cancel_effective_ns is not None
        cancel_due = cancel_due and event.logical_time_ns >= current.cancel_effective_ns
        if cancel_due:
            if current.status in {"working", "partially_filled", "pending_cancel"}:
                out.append(_report(event, order_id, "cancel_ack"))
                orders[order_id] = replace_order(current, status="canceled")
            continue
        if current.cancel_effective_ns is None and event.logical_time_ns >= current.expires_ns:
            if current.active:
                out.append(_report(event, order_id, "expire"))
                orders[order_id] = replace_order(current, status="expired")
    return state.updated(orders=orders), tuple(out)


def replace_order(order: OrderState, **changes: object) -> OrderState:
    """Return an order with selected immutable fields replaced."""
    return replace(order, **changes)


def _side_for_order(side: str) -> str:
    return "bid" if side == "buy" else "ask"


def _print_hits_order(event: TradePrint, order: OrderState) -> bool:
    if order.side == "buy":
        return event.aggressor_side == "sell" and event.price <= order.limit_price
    return event.aggressor_side == "buy" and event.price >= order.limit_price


def _tradable(status: str) -> bool:
    """Return whether a market status permits fills and new orders."""
    return status in _TRADABLE_STATUSES


def _tick_at(state: SimState, instrument: str, ns: int) -> Decimal | None:
    """Return the latest instrument tick size effective at ``ns``."""
    chosen: tuple[int, Decimal] | None = None
    for effective_ns, tick in state.tick_rows.get(instrument, ()):
        if effective_ns <= ns and (chosen is None or effective_ns > chosen[0]):
            chosen = (effective_ns, tick)
    return None if chosen is None else chosen[1]


def _apply_tick(price: Decimal, tick: Decimal | None) -> Decimal:
    """Round a fill price to the effective tick; no rounding when undeclared."""
    if tick is None:
        return price
    with localcontext() as context:
        context.prec = 50
        steps = (price / tick).quantize(Decimal("1"), rounding=ROUND_HALF_UP)
    return tick * steps


def _margin_allows(state: SimState, price: Decimal, quantity: int) -> bool:
    """Apply the declared margin floor; research equity proxy, no ledger claim."""
    margin = state.params.cost_model.margin
    if margin is None:
        return True
    notional = price * quantity
    equity = state.cash + sum(
        (Decimal(qty) * mark.price for instrument, qty in state.positions.items()
         if qty > 0 and (mark := state.marks.get(instrument)) is not None),
        Decimal("0"),
    )
    if equity < notional * margin.initial_bps / BPS:
        return False
    gross = sum(abs(qty) * price for qty in state.positions.values()) + notional
    return equity >= gross * margin.maintenance_bps / BPS


def _risk_allows(state: SimState, order: OrderState, quantity: int,
                 price: Decimal, fees: Decimal) -> bool:
    notional = price * quantity
    if order.side == "buy":
        if state.cash < notional + fees:
            return False
        if notional > state.risk_budgets.max_notional:
            return False
    else:
        borrow = state.params.cost_model.borrow_terms(order.instrument_id)
        allowed = state.positions.get(order.instrument_id, 0)
        if borrow is not None and borrow.available:
            allowed += borrow.max_quantity
        if quantity > allowed:
            return False
    if not _margin_allows(state, price, quantity):
        return False
    gross = sum(abs(qty) * price for qty in state.positions.values())
    return gross + notional <= state.risk_budgets.max_gross_exposure


def _update_exposure(state: SimState, event: ExecutionReport, new_position: int) -> dict:
    """Rebasis the net exposure after a fill; accrual already reached fill time."""
    exposures = dict(state.exposures)
    instrument = event.instrument_id
    if new_position == 0:
        exposures.pop(instrument, None)
        return exposures
    side = "long" if new_position > 0 else "short"
    prior = exposures.get(instrument)
    if prior is not None and prior.side == side:
        if abs(new_position) >= prior.quantity:
            notional = prior.notional + event.price * event.quantity
        else:
            notional = event.price * abs(new_position)
        opened_ns = prior.opened_ns
    else:
        notional = event.price * abs(new_position)
        opened_ns = event.logical_time_ns
    exposures[instrument] = ExposureState(
        instrument=instrument,
        side=side,
        quantity=abs(new_position),
        notional=notional,
        opened_ns=opened_ns,
        accrued_to_ns=event.logical_time_ns,
    )
    return exposures


def _accrue(state: SimState, event: EventEnvelope) -> tuple[SimState, tuple]:
    """Accrue borrow and funding over logical time, emitting derived charge reports."""
    now = event.logical_time_ns
    costs = state.params.cost_model
    funding = costs.funding_rate(now)
    exposures = dict(state.exposures)
    charges = state.charges
    borrow_cost = state.borrow_cost
    funding_cost = state.funding_cost
    out: list[SimulationEvent] = []
    for instrument in sorted(exposures):
        exposure = exposures[instrument]
        held_ns = now - exposure.accrued_to_ns
        if held_ns <= 0:
            continue
        if exposure.side == "short":
            terms = costs.borrow_terms(instrument)
            if terms is not None and terms.available:
                amount = terms.cost_bps_per_day * exposure.notional * Decimal(held_ns) / (
                    BPS * Decimal(_DAY_NS)
                )
                charges += (ChargeRecord(
                    instrument, "borrow", amount, held_ns, now,
                    terms.cost_bps_per_day, terms.assumption,
                ),)
                borrow_cost += amount
                out.append(_report(
                    event, f"charge:{instrument}:borrow", "charge", suffix=str(now),
                    fees=amount,
                    reason=f"borrow cost; assumption={terms.assumption or 'artifact-pinned'}",
                ))
        if funding is not None:
            amount = funding.charge(exposure.notional, held_ns)
            charges += (ChargeRecord(
                instrument, "funding", amount, held_ns, now,
                funding.rate_bps_per_day, funding.assumption,
            ),)
            funding_cost += amount
            out.append(_report(
                event, f"charge:{instrument}:funding", "charge", suffix=str(now),
                fees=amount,
                reason=f"funding cost; assumption={funding.assumption or 'artifact-pinned'}",
            ))
        exposures[instrument] = replace(exposure, accrued_to_ns=now)
    return state.updated(
        exposures=exposures,
        charges=charges,
        borrow_cost=borrow_cost,
        funding_cost=funding_cost,
    ), tuple(out)


def _participation_cap(state: SimState, quantity: int, displayed_qty: int) -> int:
    cap = int(Decimal(displayed_qty) * state.params.max_participation)
    return min(quantity, max(cap, 0))


def _on_book(state: SimState, event: BookSnapshot | BookDelta) -> tuple[SimState, tuple]:
    state, timed = _timed_reports(state, event)
    out = list(timed)
    key = book_key(event.venue, event.instrument_id)
    previous = state.books.get(key, BookState())
    if isinstance(event, BookSnapshot):
        book = previous.apply_snapshot(
            event.bids, event.asks, event.logical_time_ns, event.depth_granularity
        )
    else:
        book = previous.apply_delta(
            event.side, event.updates, event.logical_time_ns, event.depth_granularity
        )
    books = dict(state.books)
    books[key] = book
    marks = dict(state.marks)
    if book.bids and book.asks and book.bids[0].price < book.asks[0].price:
        midpoint = (book.bids[0].price + book.asks[0].price) / Decimal("2")
        marks[event.instrument_id] = MarkState(midpoint, event.logical_time_ns, event.event_id)

    orders = dict(state.orders)
    queues = dict(state.queues)
    for order_id in sorted(orders):
        order = orders[order_id]
        if order.venue != event.venue or order.instrument_id != event.instrument_id:
            continue
        if not order.active:
            continue
        queue = queues.get(queue_key(event.venue, order_id), QueueState())
        observed = book.displayed(_side_for_order(order.side)).get(order.limit_price, 0)
        if observed > queue.high_water:
            queues[queue_key(event.venue, order_id)] = QueueState(
                ahead=queue.ahead + observed - queue.high_water,
                high_water=observed,
            )

    if not _tradable(state.market_status):
        unmarked = tuple(
            item for item in state.unmarked_residual if item.instrument_id not in marks
        )
        return state.updated(
            books=books, marks=marks, orders=orders, queues=queues,
            unmarked_residual=unmarked,
        ), tuple(out)

    for order_id in sorted(orders):
        order = orders[order_id]
        if order.venue != event.venue or order.instrument_id != event.instrument_id:
            continue
        if not order.active or not order.announced:
            continue
        fresh = 0 <= event.logical_time_ns - book.last_update_ns <= state.params.max_book_age_ns
        if not fresh:
            continue
        opposite = "ask" if order.side == "buy" else "bid"
        levels = book.asks if opposite == "ask" else book.bids
        increments = dict(book.new_ask_liquidity if opposite == "ask" else book.new_bid_liquidity)
        displayed = book.displayed(opposite)
        remaining = order.remaining
        for index, level in enumerate(levels):
            if remaining <= 0:
                break
            if order.side == "buy" and level.price > order.limit_price:
                continue
            if order.side == "sell" and level.price < order.limit_price:
                continue
            tranche = increments.get(level.price, 0)
            if tranche <= 0:
                continue
            quantity = _participation_cap(state, min(remaining, tranche), displayed[level.price])
            if quantity <= 0:
                continue
            impact = state.params.cost_model.impact.impact_bps(
                Decimal(quantity) / Decimal(max(displayed[level.price], 1))
            )
            slippage = state.params.cost_model.slippage_schedule.rate(
                event.logical_time_ns, event.venue, event.instrument_id
            )
            price = Decimal(level.price)
            if order.side == "buy":
                price = price + price * (impact + slippage) / BPS
            else:
                price = price - price * (impact + slippage) / BPS
            price = _apply_tick(price, _tick_at(state, event.instrument_id, event.logical_time_ns))
            if order.side == "buy" and price > order.limit_price:
                continue
            if order.side == "sell" and price < order.limit_price:
                continue
            fees = price * quantity * state.params.cost_model.fee_schedule.rate(
                event.logical_time_ns, event.venue, event.instrument_id, order.side, "taker"
            ) / BPS
            if not _risk_allows(state, order, quantity, price, fees):
                continue
            out.append(
                _report(
                    event,
                    order_id,
                    "fill",
                    suffix=f"taker:{index}",
                    quantity=quantity,
                    price=price,
                    fees=fees,
                    impact_cost=abs(price - level.price) * quantity,
                    spread_cost=Decimal("0"),
                    liquidity="taker",
                )
            )
            increments[level.price] = tranche - quantity
            remaining -= quantity
    unmarked = tuple(
        item for item in state.unmarked_residual if item.instrument_id not in marks
    )
    return state.updated(
        books=books,
        marks=marks,
        orders=orders,
        queues=queues,
        unmarked_residual=unmarked,
    ), tuple(out)


def _on_trade(state: SimState, event: TradePrint) -> tuple[SimState, tuple]:
    state, timed = _timed_reports(state, event)
    out = list(timed)
    if not _tradable(state.market_status) or not event.queue_advancing:
        return state, tuple(out)
    orders = dict(state.orders)
    queues = dict(state.queues)
    for order_id in sorted(orders):
        order = orders[order_id]
        if order.venue != event.venue or order.instrument_id != event.instrument_id:
            continue
        if not order.active or not order.announced or not _print_hits_order(event, order):
            continue
        key = queue_key(event.venue, order_id)
        queue = queues.get(key, QueueState())
        consumed = min(queue.ahead, event.quantity)
        queues[key] = QueueState(ahead=queue.ahead - consumed, high_water=queue.high_water)
        executable = max(0, event.quantity - consumed)
        if executable <= 0:
            continue
        quantity = _participation_cap(state, min(order.remaining, executable), event.quantity)
        if quantity <= 0:
            continue
        price = _apply_tick(
            order.limit_price,
            _tick_at(state, event.instrument_id, event.logical_time_ns),
        )
        fees = price * quantity * state.params.cost_model.fee_schedule.rate(
            event.logical_time_ns, event.venue, event.instrument_id, order.side, "maker"
        ) / BPS
        if not _risk_allows(state, order, quantity, price, fees):
            continue
        out.append(
            _report(
                event,
                order_id,
                "fill",
                suffix="maker",
                quantity=quantity,
                price=price,
                fees=fees,
                liquidity="maker",
            )
        )
    return state.updated(orders=orders, queues=queues), tuple(out)


def _on_execution_report(state: SimState, event: ExecutionReport) -> tuple[SimState, tuple]:
    if event.report_type == "fill" and any(f.event_id == event.event_id for f in state.fills):
        return state, ()
    if event.report_type in {"charge", "latency_sample"}:
        cash = state.cash - event.fees if event.report_type == "charge" else state.cash
        if cash < 0:
            raise AssertionError("cash-equity accounting invariant violated")
        return state.updated(cash=cash), ()
    orders = dict(state.orders)
    order = orders.get(event.client_order_id)
    if order is None:
        if event.report_type == "reject":
            return state, ()
        raise ValueError(f"execution report for unknown order: {event.client_order_id}")
    positions = dict(state.positions)
    exposures = state.exposures
    cash = state.cash
    fills = state.fills
    unmarked = state.unmarked_residual
    if event.report_type == "ack":
        orders[event.client_order_id] = replace_order(order, acknowledged=True)
    elif event.report_type == "new":
        status = "working" if order.status == "pending_new" else order.status
        orders[event.client_order_id] = replace_order(order, announced=True, status=status)
    elif event.report_type == "reject":
        orders[event.client_order_id] = replace_order(
            order, status="rejected", last_reason=event.reason
        )
    elif event.report_type == "cancel_ack":
        orders[event.client_order_id] = replace_order(
            order, status="canceled", cancel_effective_ns=event.logical_time_ns
        )
    elif event.report_type == "expire":
        orders[event.client_order_id] = replace_order(order, status="expired")
    elif event.report_type == "fill":
        if order.side == "buy":
            cash -= event.price * event.quantity + event.fees
            positions[event.instrument_id] = positions.get(event.instrument_id, 0) + event.quantity
        else:
            cash += event.price * event.quantity - event.fees
            positions[event.instrument_id] = positions.get(event.instrument_id, 0) - event.quantity
        if cash < 0:
            raise AssertionError("cash-equity accounting invariant violated")
        exposures = _update_exposure(state, event, positions[event.instrument_id])
        remaining = order.remaining - event.quantity
        if remaining < 0:
            raise AssertionError("fill quantity exceeds order remaining")
        status = "filled" if remaining == 0 else "partially_filled"
        orders[event.client_order_id] = replace_order(order, remaining=remaining, status=status)
        fills = fills + (event,)
        marks = state.marks
        unmarked = tuple(
            item
            for item in unmarked
            if not (
                item.instrument_id == event.instrument_id
                and event.instrument_id in marks
            )
        )
        held = positions.get(event.instrument_id, 0)
        if held != 0 and event.instrument_id not in marks and not any(
            item.instrument_id == event.instrument_id for item in unmarked
        ):
            unmarked = unmarked + (
                UnmarkedResidual(event.instrument_id, abs(held), event.logical_time_ns),
            )
    return state.updated(
        cash=cash,
        positions=positions,
        exposures=exposures,
        orders=orders,
        fills=fills,
        unmarked_residual=unmarked,
    ), ()


def _on_order_add(state: SimState, event: OrderAdd) -> tuple[SimState, tuple]:
    if event.client_order_id in state.orders:
        raise ValueError(f"duplicate client_order_id: {event.client_order_id}")
    if not _tradable(state.market_status):
        reject = _report(
            event,
            event.client_order_id,
            "reject",
            reason=f"order rejected: market status {state.market_status}",
        )
        return state, (reject,)
    if event.side == "sell":
        borrow = state.params.cost_model.borrow_terms(event.instrument_id)
        available = state.positions.get(event.instrument_id, 0)
        if borrow is not None and borrow.available:
            available += borrow.max_quantity
        if event.quantity > available:
            reject = _report(
                event,
                event.client_order_id,
                "reject",
                reason="short rejected: no borrow availability",
            )
            return state, (reject,)
    activation_latency = state.params.latency_sample_ns(
        state.latency_seed, event.client_order_id, "order"
    )
    activates = event.logical_time_ns + activation_latency
    order = OrderState(
        client_order_id=event.client_order_id,
        venue=event.venue,
        instrument_id=event.instrument_id,
        side=event.side,
        quantity=event.quantity,
        remaining=event.quantity,
        limit_price=event.limit_price,
        created_ns=event.logical_time_ns,
        activates_ns=activates,
        expires_ns=activates + event.time_in_force_ns,
        venue_order_id=event.venue_order_id,
    )
    orders = dict(state.orders)
    orders[event.client_order_id] = order
    queues = dict(state.queues)
    queues[queue_key(event.venue, event.client_order_id)] = QueueState()
    ack = _report(event, event.client_order_id, "ack")
    out: list[SimulationEvent] = [ack]
    declared = state.params.latency
    if declared is not None and declared.shape == "uniform":
        out.append(_report(
            event,
            event.client_order_id,
            "latency_sample",
            suffix="order",
            reason=f"seeded latency draw; assumption={declared.assumption or 'artifact-pinned'}",
            sampled_ns=activation_latency,
            effective_ns=activates,
        ))
    return state.updated(orders=orders, queues=queues), tuple(out)


def _on_order_modify(state: SimState, event: OrderModify) -> tuple[SimState, tuple]:
    order = state.orders.get(event.client_order_id)
    if order is None:
        raise ValueError(f"modify for unknown order: {event.client_order_id}")
    quantity = order.quantity if event.new_quantity is None else event.new_quantity
    price = order.limit_price if event.new_limit_price is None else event.new_limit_price
    remaining = min(order.remaining, quantity)
    orders = dict(state.orders)
    orders[event.client_order_id] = replace_order(
        order, quantity=quantity, limit_price=price, remaining=remaining
    )
    return state.updated(orders=orders), ()


def _on_order_cancel(state: SimState, event: OrderCancel) -> tuple[SimState, tuple]:
    order = state.orders.get(event.client_order_id)
    if order is None:
        raise ValueError(f"cancel for unknown order: {event.client_order_id}")
    cancel_latency = state.params.latency_sample_ns(
        state.latency_seed, event.client_order_id, "cancel"
    )
    cancel_effective_ns = event.logical_time_ns + cancel_latency
    orders = dict(state.orders)
    orders[event.client_order_id] = replace_order(
        order,
        status="pending_cancel",
        cancel_effective_ns=cancel_effective_ns,
    )
    out: list[SimulationEvent] = []
    declared = state.params.latency
    if declared is not None and declared.shape == "uniform":
        out.append(_report(
            event,
            event.client_order_id,
            "latency_sample",
            suffix="cancel",
            reason=f"seeded cancel draw; assumption={declared.assumption or 'artifact-pinned'}",
            sampled_ns=cancel_latency,
            effective_ns=cancel_effective_ns,
        ))
    return state.updated(orders=orders), tuple(out)


def _on_config_change(state: SimState, event: ConfigChange) -> tuple[SimState, tuple]:
    configs = dict(state.effective_configs)
    rows = list(configs.get(event.config_key, ()))
    rows.append((event.effective_ns, event.config_digest))
    configs[event.config_key] = tuple(sorted(set(rows)))
    return state.updated(effective_configs=configs), ()


def _on_instrument_change(state: SimState, event: InstrumentChange) -> tuple[SimState, tuple]:
    effective_ns = event.logical_time_ns if event.effective_ns is None else event.effective_ns
    key = f"instrument:{event.instrument_id}:{event.field}"
    configs = dict(state.effective_configs)
    rows = list(configs.get(key, ()))
    rows.append((effective_ns, digest(to_jsonable([event.old_value, event.new_value]))))
    configs[key] = tuple(sorted(set(rows)))
    tick_rows = state.tick_rows
    if event.field == "tick_size":
        try:
            tick = Decimal(event.new_value)
        except (ArithmeticError, ValueError) as exc:
            raise ValueError("tick_size new_value must be a decimal") from exc
        if not tick.is_finite() or tick <= 0:
            raise ValueError("tick_size new_value must be positive and finite")
        instrument_rows = list(state.tick_rows.get(event.instrument_id, ()))
        instrument_rows.append((effective_ns, tick))
        tick_rows = {**state.tick_rows, event.instrument_id: tuple(sorted(set(instrument_rows)))}
    return state.updated(effective_configs=configs, tick_rows=tick_rows), ()


def _on_auction(state: SimState, event: AuctionPhase) -> tuple[SimState, tuple]:
    """Emit one synthetic uncross at the declared reference price.

    The rule is an assumption: no venue call, priority or auction truth is
    claimed.  A repeated uncross at the same venue/instrument/time is inert.
    """
    state, timed = _timed_reports(state, event)
    out: list[SimulationEvent] = list(timed)
    key = f"auction:{event.venue}:{event.instrument_id}"
    configs = dict(state.effective_configs)
    rows = list(configs.get(key, ()))
    marker = (event.logical_time_ns, digest("uncross"))
    already_uncrossed = event.phase == "uncross" and marker in rows
    rows.append((event.logical_time_ns, digest(event.phase)))
    configs[key] = tuple(sorted(set(rows)))
    if (
        event.phase == "uncross"
        and event.reference_price is not None
        and not already_uncrossed
        and _tradable(state.market_status)
    ):
        price = _apply_tick(
            event.reference_price,
            _tick_at(state, event.instrument_id, event.logical_time_ns),
        )
        orders = dict(state.orders)
        for order_id in sorted(orders):
            order = orders[order_id]
            if order.venue != event.venue or order.instrument_id != event.instrument_id:
                continue
            if not order.active or not order.announced:
                continue
            if order.side == "buy" and order.limit_price < price:
                continue
            if order.side == "sell" and order.limit_price > price:
                continue
            fees = price * order.remaining * state.params.cost_model.fee_schedule.rate(
                event.logical_time_ns, event.venue, event.instrument_id, order.side, "taker"
            ) / BPS
            if not _risk_allows(state, order, order.remaining, price, fees):
                continue
            out.append(_report(
                event,
                order_id,
                "fill",
                suffix="auction",
                quantity=order.remaining,
                price=price,
                fees=fees,
                liquidity="taker",
                reason="synthetic uncross at declared reference price; assumption",
            ))
    return state.updated(effective_configs=configs), tuple(out)


def _dispatch(state: SimState, event: SimulationEvent) -> tuple[SimState, tuple]:
    """Advance state for one typed input event without time-based accrual."""
    if isinstance(event, (BookSnapshot, BookDelta)):
        return _on_book(state, event)
    if isinstance(event, TradePrint):
        return _on_trade(state, event)
    if isinstance(event, ExecutionReport):
        return _on_execution_report(state, event)
    if isinstance(event, OrderAdd):
        return _on_order_add(state, event)
    if isinstance(event, OrderModify):
        return _on_order_modify(state, event)
    if isinstance(event, OrderCancel):
        return _on_order_cancel(state, event)
    if isinstance(event, ConfigChange):
        return _on_config_change(state, event)
    if isinstance(event, InstrumentChange):
        return _on_instrument_change(state, event)
    if isinstance(event, MarketStatus):
        return state.updated(market_status=event.status), ()
    if isinstance(event, AuctionPhase):
        return _on_auction(state, event)
    raise ValueError(f"unsupported simulation event: {type(event).__name__}")


def reduce(state: SimState, event: SimulationEvent) -> tuple[SimState, tuple[SimulationEvent, ...]]:
    """Pure reducer: accrue charges, then advance state and emit typed events."""
    state, charges = _accrue(state, event)
    state, out = _dispatch(state, event)
    return state, charges + out


class SimulationEngine:
    """Deterministic driver that reduces events and re-reduces emitted reports."""

    def __init__(self, config: EngineConfig):
        if not isinstance(config, EngineConfig):
            raise ValueError("config must be an EngineConfig")
        self.config = config

    def run(
        self,
        events: Sequence[SimulationEvent],
        strategy: Strategy | None = None,
        models: ExecutionModel | None = None,
        seeds: ReplaySeeds | None = None,
    ) -> SimResult:
        """Run a deterministic replay from synthetic or recorded events."""
        if not isinstance(self.config, EngineConfig):
            raise ValueError("config must be an EngineConfig")
        if seeds is None:
            seeds = ReplaySeeds(0, 0, 0, 0)
        if models is None:
            params = BaselineParams(
                order_latency_ns=self.config.order_latency_ns,
                cancel_latency_ns=self.config.cancel_latency_ns,
                max_participation=self.config.max_participation,
                max_book_age_ns=self.config.max_book_age_ns,
                cost_model=default_cost_model(),
            )
            models = ConservativeMbpModel(params)
        pending: list[SimulationEvent] = []
        seen_ids: set[str] = set()
        for event in events:
            if not isinstance(event, EVENT_TYPES):
                raise ValueError("run requires typed simulation events")
            if event.event_id in seen_ids:
                raise ValueError(f"duplicate event_id in replay: {event.event_id}")
            seen_ids.add(event.event_id)
            pending.append(event)
        pending.sort(key=event_ordering_key)
        state = empty_state(
            self.config.starting_cash,
            models.params,
            self.config.risk_budgets,
            latency_seed=seeds.latency_seed,
        )
        emitted: list[SimulationEvent] = []
        consumed: list[SimulationEvent] = []
        insertion = 0
        while pending:
            index = min(range(len(pending)), key=lambda pos: event_ordering_key(pending[pos]))
            event = pending.pop(index)
            if event.event_id in {item.event_id for item in consumed}:
                raise ValueError(f"duplicate event_id in replay: {event.event_id}")
            consumed.append(event)
            state, out = reduce(state, event)
            for output in out:
                emitted.append(output)
                state, nested = reduce(state, output)
                if nested:
                    raise AssertionError("emitted reports must not emit further events")
            if strategy is not None and not isinstance(
                event, (ExecutionReport, OrderAdd, OrderModify, OrderCancel)
            ):
                intent = strategy.on_event(event, self._context(state, event))
                if intent is not None:
                    intent.validate()
                    insertion += 1
                    add = self._intent_to_order_add(intent, event, insertion)
                    if add.event_id in seen_ids:
                        raise ValueError(f"duplicate generated order event: {add.event_id}")
                    seen_ids.add(add.event_id)
                    pending.append(add)
        fills = tuple(
            item
            for item in emitted
            if isinstance(item, ExecutionReport) and item.report_type == "fill"
        )
        fees = sum((item.fees for item in fills), Decimal("0"))
        impact_cost = sum((item.impact_cost for item in fills), Decimal("0"))
        spread_cost = sum((item.spread_cost for item in fills), Decimal("0"))
        marks = {key: value.price for key, value in state.marks.items()}
        marked_value = sum(
            (
                Decimal(quantity) * marks[key]
                for key, quantity in state.positions.items()
                if key in marks
            ),
            Decimal("0"),
        )
        net_pnl = state.cash + marked_value - state.starting_cash
        strategy_id = strategy.strategy_id if strategy is not None else self.config.strategy_id
        return SimResult(
            strategy_id=strategy_id,
            event_count=len(consumed),
            emitted=tuple(emitted),
            consumed_events=tuple(consumed),
            fills=fills,
            ending_cash=state.cash,
            positions=dict(sorted(state.positions.items())),
            marks=dict(sorted(marks.items())),
            net_pnl=net_pnl,
            gross_pnl=net_pnl + fees + impact_cost + spread_cost,
            fees=fees,
            impact_cost=impact_cost,
            spread_cost=spread_cost,
            unmarked_residual=bool(state.unmarked_residual),
            unmarked_details=state.unmarked_residual,
            borrow_cost=state.borrow_cost,
            funding_cost=state.funding_cost,
            charges=state.charges,
            final_state=state,
            state_hash=state_hash(state),
            config_hash=self.config.config_hash(),
            model_digests=(models.model_hash(), models.params.cost_model.cost_hash()),
            seeds_digest=seeds.digest(),
        )

    def _context(self, state: SimState, event: SimulationEvent) -> StrategyContext:
        instrument = event.instrument_id
        mark = state.marks.get(instrument)
        return StrategyContext(
            instrument_id=instrument,
            logical_time_ns=event.logical_time_ns,
            cash=state.cash,
            position=state.positions.get(instrument, 0),
            has_open_order=any(
                order.instrument_id == instrument and order.active
                for order in state.orders.values()
            ),
            mark=None if mark is None else mark.price,
        )

    @staticmethod
    def _intent_to_order_add(intent: OrderIntent, cause: EventEnvelope,
                             insertion: int) -> OrderAdd:
        return OrderAdd(
            event_id=f"sim:add:{cause.event_id}:{intent.client_order_id}",
            source_frame_sha256=cause.source_frame_sha256,
            instrument_id=intent.instrument_id,
            venue=cause.venue,
            logical_time_ns=cause.logical_time_ns,
            available_time_ns=cause.available_time_ns,
            origin_class="simulated",
            derived_from=(cause.event_id,),
            lineage=cause.lineage,
            source_sequence=cause.source_sequence + 1_000_000 + insertion,
            client_order_id=intent.client_order_id,
            side=intent.side,
            quantity=intent.quantity,
            limit_price=intent.limit_price,
            time_in_force_ns=intent.time_in_force_ns,
        )
