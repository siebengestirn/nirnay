"""Synthetic universe selection with knowledge-time and digest falsifiers."""
from __future__ import annotations

from datetime import date

import pytest

from quantlab.data import UniverseMembershipRow, UniverseSpec

VALID_ON = date(2026, 1, 15)
AS_OF = 1_700_000_000_000_000_000


def _row(membership_id: str, instrument_id: str, *, known_at_ns: int = AS_OF,
         status: str = "active", supersedes: str | None = None,
         valid_from: date = date(2025, 1, 1),
         valid_to: date | None = None) -> UniverseMembershipRow:
    return UniverseMembershipRow(
        membership_id=membership_id, universe_id="fixture-universe",
        instrument_id=instrument_id, valid_from=valid_from, valid_to=valid_to,
        known_at_ns=known_at_ns, status=status,
        supersedes_membership_id=supersedes,
    )


def test_select_is_synthetic_only_and_deterministic():
    rows = [_row("m1", "in.a"), _row("m2", "in.b")]
    spec = UniverseSpec("fixture-universe", VALID_ON)
    assert spec.select(rows, AS_OF) == ("in.a", "in.b")
    assert spec.select(list(reversed(rows)), AS_OF) == ("in.a", "in.b")
    assert spec.digest(rows, AS_OF) == spec.digest(rows, AS_OF)


def test_membership_known_after_as_of_cannot_enter():
    rows = [_row("m1", "in.a"), _row("m2", "in.future", known_at_ns=AS_OF + 1)]
    spec = UniverseSpec("fixture-universe", VALID_ON)
    assert spec.select(rows, AS_OF) == ("in.a",)
    assert spec.select(rows, AS_OF + 1) == ("in.a", "in.future")


def test_supersession_status_and_validity_are_applied():
    rows = [
        _row("m1", "in.a"),
        _row("m2", "in.a", known_at_ns=AS_OF + 1, supersedes="m1"),
        _row("m3", "in.cancelled", status="cancelled", supersedes="m0"),
        _row("m4", "in.expired", valid_to=date(2025, 6, 1)),
        _row("m5", "in.future-valid", valid_from=date(2027, 1, 1)),
    ]
    spec = UniverseSpec("fixture-universe", VALID_ON)
    assert spec.select(rows, AS_OF) == ("in.a",)
    with pytest.raises(ValueError, match="ambiguous"):
        spec.select([_row("x1", "in.dup"), _row("x2", "in.dup")], AS_OF)


def test_digest_changes_with_selection_and_inputs_are_validated():
    spec = UniverseSpec("fixture-universe", VALID_ON)
    original = spec.digest([_row("m1", "in.a")], AS_OF)
    changed = spec.digest([_row("m1", "in.a"), _row("m2", "in.b")], AS_OF)
    assert original != changed
    with pytest.raises(ValueError, match="as_of_ns"):
        spec.select([], 0)
    with pytest.raises(ValueError, match="known_at_ns"):
        _row("m9", "in.x", known_at_ns=0)
    with pytest.raises(ValueError, match="valid_to"):
        _row("m9", "in.x", valid_to=date(2024, 1, 1))
