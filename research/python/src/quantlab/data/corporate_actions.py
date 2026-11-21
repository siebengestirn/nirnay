"""Exact adjustment factors for supported synthetic corporate actions.

Splits and bonuses are priced by an exact new-share/old-share ratio. Rights,
mergers, demergers and other complex kinds are refused with a typed error
instead of silently approximating them. No survivorship or data-right claim is
made, and no raw price is edited.
"""
from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal, localcontext
from datetime import date
from typing import Sequence

SUPPORTED_KINDS = frozenset({"split", "bonus"})


class UnsupportedCorporateAction(ValueError):
    """Raised when an action kind has no approved exact transform."""


@dataclass(frozen=True, slots=True)
class SyntheticCorporateAction:
    """One synthetic, self-contained corporate-action declaration."""

    action_id: str
    instrument_id: str
    kind: str
    effective_date: date
    known_at_ns: int
    share_ratio_numerator: int = 1
    share_ratio_denominator: int = 1
    status: str = "effective"

    def __post_init__(self) -> None:
        if not isinstance(self.action_id, str) or not self.action_id.strip():
            raise ValueError("action_id must be a non-empty string")
        if not isinstance(self.instrument_id, str) or not self.instrument_id.strip():
            raise ValueError("instrument_id must be a non-empty string")
        if not isinstance(self.kind, str) or not self.kind.strip():
            raise ValueError("kind must be a non-empty string")
        if type(self.effective_date) is not date:
            raise ValueError("effective_date must be a date")
        if type(self.known_at_ns) is not int or self.known_at_ns <= 0:
            raise ValueError("known_at_ns must be a positive integer")
        if type(self.share_ratio_numerator) is not int or type(
            self.share_ratio_denominator
        ) is not int:
            raise ValueError("share ratio must use exact integers")
        if self.share_ratio_numerator <= 0 or self.share_ratio_denominator <= 0:
            raise ValueError("share ratio must use positive integers")
        if self.status not in {"announced", "effective", "cancelled"}:
            raise ValueError("action status must be announced, effective or cancelled")


def adjustment_factor(action: SyntheticCorporateAction) -> Decimal:
    """Return the exact Decimal price factor for one split or bonus action."""
    if not isinstance(action, SyntheticCorporateAction):
        raise ValueError("action must be a SyntheticCorporateAction")
    if action.status == "cancelled":
        return Decimal(1)
    if action.kind not in SUPPORTED_KINDS:
        raise UnsupportedCorporateAction(
            f"action {action.action_id} ({action.kind}) has no approved exact transform"
        )
    with localcontext() as context:
        context.prec = 40
        return Decimal(action.share_ratio_denominator) / Decimal(
            action.share_ratio_numerator
        )


def adjustment_factors(
    actions: Sequence[SyntheticCorporateAction],
    *,
    instrument_id: str,
    observation_date: date,
    known_at_ns: int,
) -> tuple[Decimal, tuple[str, ...]]:
    """Compose known, effective factors for prices before each action date."""
    if type(known_at_ns) is not int or known_at_ns <= 0:
        raise ValueError("known_at_ns must be a positive integer")
    if type(observation_date) is not date:
        raise ValueError("observation_date must be a date")
    if not isinstance(instrument_id, str) or not instrument_id.strip():
        raise ValueError("instrument_id must be a non-empty string")
    factor = Decimal(1)
    lineage: list[str] = []
    for action in actions:
        if not isinstance(action, SyntheticCorporateAction):
            raise ValueError("actions must contain synthetic corporate actions")
        if action.instrument_id != instrument_id:
            continue
        if action.known_at_ns > known_at_ns or action.status == "cancelled":
            continue
        if action.effective_date <= observation_date:
            continue
        factor *= adjustment_factor(action)
        lineage.append(action.action_id)
    return factor, tuple(lineage)
