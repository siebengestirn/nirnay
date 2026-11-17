"""Deterministic reference strategies for simulator tests."""
from __future__ import annotations

from decimal import Decimal

from .backtest import OrderIntent, StrategyContext
from .market import MarketEvent


class DeterministicEmaCross:
    """Streaming long-only EMA crossover fixture.

    EMA state is recursive and never recomputed from a truncated rolling window.
    A signal uses only the event just received; the simulator applies configured
    order latency. This is a deterministic systems test, not validated alpha.
    """

    strategy_id = "fixture.ema-cross.v2"

    def __init__(self, *, instrument_id: str, fast: int = 3, slow: int = 7,
                 quantity: int = 1, min_observations: int | None = None,
                 aggressive_limit_bps: Decimal = Decimal("10")):
        if not instrument_id or fast < 2 or slow <= fast or quantity <= 0:
            raise ValueError("invalid EMA crossover configuration")
        if min_observations is not None and (type(min_observations) is not int or min_observations < slow):
            raise ValueError("min_observations must be an integer at least as large as slow")
        if not aggressive_limit_bps.is_finite() or aggressive_limit_bps < 0:
            raise ValueError("aggressive_limit_bps must be finite and non-negative")
        self.instrument_id = instrument_id
        self.fast = fast
        self.slow = slow
        self.quantity = quantity
        self.min_observations = min_observations if min_observations is not None else slow
        self.aggressive_limit_bps = aggressive_limit_bps
        self._fast_ema: Decimal | None = None
        self._slow_ema: Decimal | None = None
        self._previous_fast: Decimal | None = None
        self._previous_slow: Decimal | None = None
        self._observations = 0
        self._intent_sequence = 0

    @staticmethod
    def _update(previous: Decimal | None, value: Decimal, span: int) -> Decimal:
        if previous is None:
            return value
        alpha = Decimal(2) / Decimal(span + 1)
        return alpha * value + (Decimal(1) - alpha) * previous

    def on_event(self, event: MarketEvent, context: StrategyContext) -> OrderIntent | None:
        if event.instrument_id != self.instrument_id or event.event_kind not in {"quote", "depth"}:
            return None
        price = event.midpoint
        if price is None or price <= 0:
            return None

        self._previous_fast, self._previous_slow = self._fast_ema, self._slow_ema
        self._fast_ema = self._update(self._fast_ema, price, self.fast)
        self._slow_ema = self._update(self._slow_ema, price, self.slow)
        self._observations += 1

        if self._observations < self.min_observations or context.has_open_order:
            return None
        crossed_up = (self._previous_fast is not None and self._previous_slow is not None
                      and self._previous_fast <= self._previous_slow
                      and self._fast_ema > self._slow_ema)
        crossed_down = (self._previous_fast is not None and self._previous_slow is not None
                        and self._previous_fast >= self._previous_slow
                        and self._fast_ema < self._slow_ema)

        if crossed_up and context.position_quantity == 0 and event.best_ask:
            side, quantity = "buy", self.quantity
            limit = event.best_ask.price * (Decimal(1) + self.aggressive_limit_bps / Decimal(10_000))
        elif crossed_down and context.position_quantity > 0 and event.best_bid:
            side, quantity = "sell", context.position_quantity
            limit = event.best_bid.price * (Decimal(1) - self.aggressive_limit_bps / Decimal(10_000))
        else:
            return None

        self._intent_sequence += 1
        return OrderIntent(f"{self.strategy_id}:{self._intent_sequence:06d}",
                           self.instrument_id, side, quantity, limit)
