"""Shared synthetic bar and feature-row primitives for pure computation.

Every helper here is deterministic and operates only on values supplied by the
caller. Prices are exact ``Decimal`` values; derived statistics are floats and
are checked for finiteness. Nothing in this module reads provider data, a
service or a live clock.
"""
from __future__ import annotations

import hashlib
import math
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Sequence

from .._canonical import content_hash
from .lineage import FeatureLineage

DEFINITION_DOMAIN = b"NIRNAY-FEATURE-DEFINITION-V1\x00"


def definition_digest(name: str) -> str:
    """Return a stable definition digest for a named pure feature family."""
    if not isinstance(name, str) or not name.strip():
        raise ValueError("feature family name is required")
    return hashlib.sha256(DEFINITION_DOMAIN + name.encode("utf-8")).hexdigest()


def require_finite(values: Sequence[float]) -> list[float]:
    """Return float values, refusing NaN, infinity and non-numeric inputs."""
    if isinstance(values, (str, bytes)):
        raise ValueError("numeric sequence must not be a string")
    result: list[float] = []
    for value in values:
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError("numeric sequence must contain real numbers")
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("numeric sequence must contain only finite values")
        result.append(number)
    return result


def require_window(value: int, name: str, *, minimum: int = 2) -> None:
    """Validate an integer rolling-window length."""
    if type(value) is not int or value < minimum:
        raise ValueError(f"{name} must be an integer >= {minimum}")


def mean(values: Sequence[float]) -> float:
    """Return the arithmetic mean of an already-validated finite sequence."""
    return sum(values) / len(values) if values else 0.0


def sample_std(values: Sequence[float]) -> float:
    """Return the unbiased sample standard deviation of finite values."""
    if len(values) < 2:
        return 0.0
    mu = mean(values)
    return math.sqrt(sum((value - mu) ** 2 for value in values) / (len(values) - 1))


@dataclass(frozen=True, slots=True)
class OhlcvBar:
    """One synthetic OHLCV observation with an explicit UTC nanosecond clock."""

    timestamp_ns: int
    open: Decimal
    high: Decimal
    low: Decimal
    close: Decimal
    volume: int

    def __post_init__(self) -> None:
        if type(self.timestamp_ns) is not int or self.timestamp_ns <= 0:
            raise ValueError("bar timestamp_ns must be a positive integer")
        for name in ("open", "high", "low", "close"):
            value = getattr(self, name)
            if not isinstance(value, Decimal) or not value.is_finite() or value <= 0:
                raise ValueError(f"bar {name} must be a positive finite Decimal")
        if self.high < max(self.open, self.close, self.low):
            raise ValueError("bar high must cover open, close and low")
        if self.low > min(self.open, self.close, self.high):
            raise ValueError("bar low must be at or below open, close and high")
        if type(self.volume) is not int or self.volume < 0:
            raise ValueError("bar volume must be a non-negative integer")


def closes_from_bars(bars: Sequence[OhlcvBar]) -> list[float]:
    """Return finite float closes for a sequence of synthetic bars."""
    if not bars:
        raise ValueError("at least one bar is required")
    return require_finite([float(bar.close) for bar in bars])


@dataclass(frozen=True, slots=True)
class FeatureRow:
    """One causally computed feature row bound to a bar timestamp."""

    timestamp_ns: int
    values: dict[str, float | None]

    def __post_init__(self) -> None:
        if type(self.timestamp_ns) is not int or self.timestamp_ns <= 0:
            raise ValueError("row timestamp_ns must be a positive integer")
        if not isinstance(self.values, dict) or not self.values:
            raise ValueError("row values must be a non-empty object")
        for key, value in self.values.items():
            if not isinstance(key, str) or not key:
                raise ValueError("feature names must be non-empty strings")
            if value is not None and (
                isinstance(value, bool) or not isinstance(value, (int, float))
                or not math.isfinite(float(value))
            ):
                raise ValueError("feature values must be finite numbers or None")

    def as_dict(self) -> dict[str, Any]:
        """Return a canonical JSON-safe view of this row."""
        return {"timestamp_ns": self.timestamp_ns,
                "values": {key: self.values[key] for key in sorted(self.values)}}


@dataclass(frozen=True, slots=True)
class FeatureBatch:
    """A deterministic batch of feature rows plus its declarative lineage."""

    feature_version: str
    rows: tuple[FeatureRow, ...]
    lineage: FeatureLineage
    rows_sha256: str


def make_batch(
    feature_version: str,
    rows: Sequence[FeatureRow],
    *,
    family: str,
    knowledge_utc_ns: int | None = None,
    declared_config: dict[str, Any] | None = None,
) -> FeatureBatch:
    """Bind computed rows to a bitemporal lineage and a content digest."""
    if not isinstance(feature_version, str) or not feature_version.strip():
        raise ValueError("feature_version must be a non-empty string")
    if not rows:
        raise ValueError("a feature batch must contain at least one row")
    as_of_utc_ns = max(row.timestamp_ns for row in rows)
    known = rows[0].timestamp_ns if knowledge_utc_ns is None else knowledge_utc_ns
    lineage = FeatureLineage(
        feature_version=feature_version,
        definition_sha256=definition_digest(family),
        as_of_utc_ns=as_of_utc_ns,
        knowledge_utc_ns=known,
        declared_config={} if declared_config is None else dict(declared_config),
    )
    digest = content_hash([row.as_dict() for row in rows])
    return FeatureBatch(feature_version, tuple(rows), lineage, digest)


def batch_as_dict(batch: FeatureBatch) -> dict[str, Any]:
    """Return a canonical JSON-safe view of a feature batch."""
    return {"feature_version": batch.feature_version,
            "rows": [row.as_dict() for row in batch.rows],
            "lineage": batch.lineage.as_dict(),
            "rows_sha256": batch.rows_sha256}
