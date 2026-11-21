"""Synthetic-only, bitemporal universe selection.

Selection is a pure function of caller-supplied membership rows and an explicit
knowledge time. A row first known after ``as_of_ns`` can never enter the
selected universe. No survivorship completeness or data-right claim is made.
"""
from __future__ import annotations

from dataclasses import dataclass
from datetime import date
from typing import Any, Sequence

from .._canonical import content_hash


@dataclass(frozen=True, slots=True)
class UniverseMembershipRow:
    """One synthetic membership fact with valid time and knowledge time."""

    membership_id: str
    universe_id: str
    instrument_id: str
    valid_from: date
    valid_to: date | None
    known_at_ns: int
    status: str = "active"
    supersedes_membership_id: str | None = None

    def __post_init__(self) -> None:
        for field in ("membership_id", "universe_id", "instrument_id"):
            value = getattr(self, field)
            if not isinstance(value, str) or not value.strip():
                raise ValueError(f"{field} must be a non-empty string")
        if type(self.valid_from) is not date:
            raise ValueError("valid_from must be a date")
        if self.valid_to is not None and type(self.valid_to) is not date:
            raise ValueError("valid_to must be a date or None")
        if self.valid_to is not None and self.valid_to < self.valid_from:
            raise ValueError("valid_to must not precede valid_from")
        if type(self.known_at_ns) is not int or self.known_at_ns <= 0:
            raise ValueError("known_at_ns must be a positive integer")
        if self.status not in {"active", "cancelled"}:
            raise ValueError("membership status must be active or cancelled")
        if self.status == "cancelled" and not self.supersedes_membership_id:
            raise ValueError("a cancelled membership must supersede a prior record")


@dataclass(frozen=True, slots=True)
class UniverseSpec:
    """A named universe observed at one valid date."""

    universe_id: str
    valid_on: date

    def __post_init__(self) -> None:
        if not isinstance(self.universe_id, str) or not self.universe_id.strip():
            raise ValueError("universe_id must be a non-empty string")
        if type(self.valid_on) is not date:
            raise ValueError("valid_on must be a date")

    def select(self, memberships: Sequence[UniverseMembershipRow],
               as_of_ns: int) -> tuple[str, ...]:
        """Return sorted instrument ids known and active at the given time."""
        if type(as_of_ns) is not int or as_of_ns <= 0:
            raise ValueError("as_of_ns must be a positive integer")
        if isinstance(memberships, (str, bytes)):
            raise ValueError("memberships must be a sequence of rows")
        visible = [
            row for row in memberships
            if isinstance(row, UniverseMembershipRow)
            and row.universe_id == self.universe_id
            and row.known_at_ns <= as_of_ns
        ]
        superseded = {
            row.supersedes_membership_id for row in visible
            if row.supersedes_membership_id
        }
        heads: dict[str, UniverseMembershipRow] = {}
        for row in visible:
            if row.membership_id in superseded or row.status != "active":
                continue
            if not (row.valid_from <= self.valid_on
                    and (row.valid_to is None or self.valid_on <= row.valid_to)):
                continue
            if row.instrument_id in heads:
                raise ValueError("ambiguous point-in-time universe membership")
            heads[row.instrument_id] = row
        return tuple(sorted(heads))

    def digest(self, memberships: Sequence[UniverseMembershipRow],
               as_of_ns: int) -> str:
        """Digest the exact selection for this specification and knowledge time."""
        body: dict[str, Any] = {
            "universe_id": self.universe_id,
            "valid_on": self.valid_on.isoformat(),
            "as_of_ns": as_of_ns,
            "instrument_ids": list(self.select(memberships, as_of_ns)),
        }
        return content_hash(body)
