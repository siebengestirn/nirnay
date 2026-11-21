"""Exact split/bonus factors and typed refusal for unsupported actions."""
from __future__ import annotations

from datetime import date
from decimal import Decimal

import pytest

from quantlab.data import (SyntheticCorporateAction, UnsupportedCorporateAction,
                           adjustment_factor, adjustment_factors)
from quantlab.data.corporate_actions import SUPPORTED_KINDS

EFFECTIVE = date(2026, 2, 1)
KNOWN = 1_700_000_000_000_000_000


def _action(action_id: str, kind: str, *, numerator: int = 1, denominator: int = 1,
            effective_date: date = EFFECTIVE, known_at_ns: int = KNOWN,
            status: str = "effective") -> SyntheticCorporateAction:
    return SyntheticCorporateAction(
        action_id=action_id, instrument_id="in.a", kind=kind,
        effective_date=effective_date, known_at_ns=known_at_ns,
        share_ratio_numerator=numerator, share_ratio_denominator=denominator,
        status=status,
    )


def test_split_and_bonus_factors_are_exact_decimals():
    assert adjustment_factor(_action("s1", "split", numerator=2)) == Decimal("0.5")
    assert adjustment_factor(_action("s2", "split", numerator=4,
                                     denominator=1)) == Decimal("0.25")
    assert adjustment_factor(_action("b1", "bonus", numerator=1,
                                     denominator=1)) == Decimal("1")
    assert SUPPORTED_KINDS == frozenset({"split", "bonus"})


@pytest.mark.parametrize("kind", ["rights", "merger", "demerger", "demonstration"])
def test_unsupported_action_kinds_never_yield_a_factor(kind):
    with pytest.raises(UnsupportedCorporateAction, match="no approved exact transform"):
        adjustment_factor(_action("u1", kind))


def test_composed_factors_respect_knowledge_time_and_effective_date():
    actions = [
        _action("s1", "split", numerator=2),
        _action("b1", "bonus", numerator=3),
        _action("s-future", "split", numerator=10, known_at_ns=KNOWN + 1),
    ]
    factor, lineage = adjustment_factors(
        actions, instrument_id="in.a", observation_date=date(2026, 1, 1),
        known_at_ns=KNOWN,
    )
    assert factor == Decimal("1") / Decimal("6")
    assert lineage == ("s1", "b1")
    same_day, same_lineage = adjustment_factors(
        actions, instrument_id="in.a", observation_date=EFFECTIVE, known_at_ns=KNOWN
    )
    assert same_day == Decimal("1")
    assert same_lineage == ()


def test_cancelled_and_other_instrument_actions_are_ignored():
    cancelled = _action("c1", "split", numerator=2, status="cancelled")
    future = SyntheticCorporateAction(
        action_id="future", instrument_id="in.a", kind="rights",
        effective_date=EFFECTIVE, known_at_ns=KNOWN + 1,
    )
    factor, lineage = adjustment_factors(
        [cancelled, future], instrument_id="in.a",
        observation_date=date(2026, 1, 1), known_at_ns=KNOWN,
    )
    assert factor == Decimal("1")
    assert lineage == ()
    with pytest.raises(UnsupportedCorporateAction):
        adjustment_factors(
            [future], instrument_id="in.a",
            observation_date=date(2026, 1, 1), known_at_ns=KNOWN + 1,
        )


def test_action_declaration_is_strict():
    with pytest.raises(ValueError, match="positive integers"):
        _action("bad", "split", numerator=0)
    with pytest.raises(ValueError, match="known_at_ns"):
        _action("bad", "split", known_at_ns=0)
    with pytest.raises(ValueError, match="action must be"):
        adjustment_factor("not-an-action")
