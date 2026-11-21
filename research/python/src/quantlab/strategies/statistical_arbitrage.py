"""Synthetic two-leg spread intents from a causal spread z-score.

Prices are exact ``Decimal`` values supplied by the caller. A signal emits
research intent records only; it never accesses cash, positions, an OMS or a
broker, and it makes no profitability or cointegration-certification claim.
"""
from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from typing import Sequence

from ..features.technical import rolling_zscore


@dataclass(frozen=True, slots=True)
class PairIntent:
    """One research intent to enter or exit a two-leg synthetic spread."""

    index: int
    timestamp_ns: int
    spread_z: float
    target_position: int
    leg_a_side: str
    leg_b_side: str
    quantity: int

    def as_dict(self) -> dict[str, int | float | str]:
        """Return a JSON-safe view of this research intent."""
        return {"index": self.index, "timestamp_ns": self.timestamp_ns,
                "spread_z": self.spread_z, "target_position": self.target_position,
                "leg_a_side": self.leg_a_side, "leg_b_side": self.leg_b_side,
                "quantity": self.quantity}


def spread_series(price_a: Sequence[Decimal],
                  price_b: Sequence[Decimal]) -> list[Decimal]:
    """Return the exact price spread for two aligned synthetic price series."""
    if not price_a or len(price_a) != len(price_b):
        raise ValueError("aligned non-empty price series are required")
    result: list[Decimal] = []
    for left, right in zip(price_a, price_b):
        for value in (left, right):
            if not isinstance(value, Decimal) or not value.is_finite() or value <= 0:
                raise ValueError("pair prices must be positive finite Decimals")
        result.append(left - right)
    return result


def pair_signals(spread: Sequence[Decimal], *, window: int,
                 entry_z: float, exit_z: float = 0.0) -> list[int]:
    """Return causal target positions in the spread from trailing z-scores."""
    if not isinstance(entry_z, (int, float)) or isinstance(entry_z, bool):
        raise ValueError("entry_z must be a real number")
    if entry_z <= 0:
        raise ValueError("entry_z must be positive")
    if not isinstance(exit_z, (int, float)) or isinstance(exit_z, bool):
        raise ValueError("exit_z must be a real number")
    if exit_z < 0 or exit_z >= entry_z:
        raise ValueError("exit_z must be non-negative and below entry_z")
    floats = [float(value) for value in spread]
    zscores = rolling_zscore(floats, window)
    position = 0
    targets: list[int] = []
    for zscore in zscores:
        if zscore is None:
            targets.append(position)
            continue
        if position == 0 and zscore >= entry_z:
            position = 1
        elif position == 0 and zscore <= -entry_z:
            position = -1
        elif position != 0 and abs(zscore) <= exit_z:
            position = 0
        targets.append(position)
    return targets


def pair_intents(timestamps_ns: Sequence[int], price_a: Sequence[Decimal],
                 price_b: Sequence[Decimal], *, window: int, entry_z: float,
                 exit_z: float = 0.0, quantity: int = 1) -> list[PairIntent]:
    """Emit one two-leg intent per synthetic spread position change."""
    if type(quantity) is not int or quantity <= 0:
        raise ValueError("quantity must be a positive integer")
    if len(timestamps_ns) != len(price_a):
        raise ValueError("timestamps and prices must be aligned")
    spread = spread_series(price_a, price_b)
    targets = pair_signals(spread, window=window, entry_z=entry_z, exit_z=exit_z)
    floats = [float(value) for value in spread]
    zscores = rolling_zscore(floats, window)
    intents: list[PairIntent] = []
    previous = 0
    for index, target in enumerate(targets):
        if target == previous:
            continue
        zscore = zscores[index]
        if zscore is None:
            continue
        leg_a_side = "buy" if target > 0 else "sell" if target < 0 else "flat"
        leg_b_side = "sell" if target > 0 else "buy" if target < 0 else "flat"
        intents.append(PairIntent(
            index=index,
            timestamp_ns=timestamps_ns[index],
            spread_z=zscore,
            target_position=target,
            leg_a_side=leg_a_side,
            leg_b_side=leg_b_side,
            quantity=quantity,
        ))
        previous = target
    return intents
