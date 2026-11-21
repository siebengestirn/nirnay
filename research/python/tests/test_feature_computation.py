"""Synthetic, deterministic and look-ahead tests for pure feature modules."""
from __future__ import annotations

from decimal import Decimal

import pytest

from quantlab.features import (OhlcvBar, assert_feature_lineage, batch_as_dict,
                               compute_statistical, compute_technical, compute_volatility,
                               feature_lineage_digest)
from quantlab.features.statistical import (autocorrelation, rolling_kurtosis,
                                           rolling_skew)
from quantlab.features.technical import ema, roc, rolling_zscore, sma
from quantlab.features.volatility import (log_returns, parkinson_range, realized_vol,
                                          vol_ratio)

STEP_NS = 60_000_000_000


def _bars(count: int = 40) -> list[OhlcvBar]:
    rows: list[OhlcvBar] = []
    previous = Decimal("100")
    for index in range(count):
        close = previous + Decimal("0.5") + Decimal(index % 5) / Decimal(10)
        high = close + Decimal("1.25")
        low = min(previous, close) - Decimal("0.75")
        rows.append(OhlcvBar(
            timestamp_ns=1_700_000_000_000_000_000 + index * STEP_NS,
            open=previous,
            high=high,
            low=low,
            close=close,
            volume=100 + index,
        ))
        previous = close
    return rows


def _extreme_bar(bars: list[OhlcvBar]) -> OhlcvBar:
    last = bars[-1]
    price = last.close * Decimal(20)
    return OhlcvBar(last.timestamp_ns + STEP_NS, last.close, price,
                    last.close, price, 999_999)


def _rows_at_or_before(batch, timestamp_ns: int) -> list[dict]:
    return [row for row in batch_as_dict(batch)["rows"]
            if row["timestamp_ns"] <= timestamp_ns]


@pytest.mark.parametrize("compute", [compute_technical, compute_volatility,
                                     compute_statistical])
def test_batch_is_deterministic_and_lineage_binds(compute):
    bars = _bars()
    first = compute(bars)
    second = compute(list(bars))
    assert batch_as_dict(first) == batch_as_dict(second)
    assert first.rows_sha256 == second.rows_sha256
    assert first.lineage.knowledge_utc_ns <= first.lineage.as_of_utc_ns
    assert_feature_lineage(first.lineage.as_dict(), feature_lineage_digest(first.lineage))
    with pytest.raises(ValueError, match="digest mismatch"):
        assert_feature_lineage(first.lineage.as_dict(), "f" * 64)


@pytest.mark.parametrize("compute", [compute_technical, compute_volatility,
                                     compute_statistical])
def test_append_future_bar_does_not_change_earlier_rows(compute):
    bars = _bars()
    cut = 25
    cutoff = bars[cut].timestamp_ns
    original = compute(bars[:cut + 1])
    extended = compute(bars + [_extreme_bar(bars)])
    assert _rows_at_or_before(original, cutoff) == _rows_at_or_before(extended, cutoff)


@pytest.mark.parametrize("compute", [compute_technical, compute_volatility,
                                     compute_statistical])
def test_future_knowledge_declaration_is_refused(compute):
    bars = _bars()
    with pytest.raises(ValueError, match="knowledge time"):
        compute(bars, knowledge_utc_ns=bars[-1].timestamp_ns + 1)


def test_technical_primitives_match_direct_calculation():
    values = [float(index + 1) for index in range(10)]
    assert sma(values, 3)[2] == pytest.approx(2.0)
    assert sma(values, 3)[0] is None
    assert ema(values, 3)[0] == 1.0
    assert roc(values, 2)[2] == pytest.approx(2.0)
    assert rolling_zscore([1.0, 1.0, 1.0, 5.0], 3)[3] > 0


def test_volatility_primitives_are_finite_and_causal():
    bars = _bars()
    closes = [float(bar.close) for bar in bars]
    returns = log_returns(closes)
    assert len(returns) == len(closes)
    assert realized_vol(returns, 5)[5] is not None
    assert realized_vol(returns, 5)[4] is None
    highs = [float(bar.high) for bar in bars]
    lows = [float(bar.low) for bar in bars]
    assert parkinson_range(highs, lows, 5)[4] >= 0
    assert vol_ratio(returns, 5, 10)[10] is not None
    assert vol_ratio(returns, 5, 10)[9] is None


def test_statistical_primitives_reject_non_finite_and_short_windows():
    values = [float(index % 7) for index in range(30)]
    assert autocorrelation(values, 1, 10)[9] is not None
    assert rolling_skew(values, 8)[7] is not None
    assert rolling_kurtosis(values, 8)[7] is not None
    with pytest.raises(ValueError, match="finite"):
        sma([1.0, float("nan"), 3.0], 2)
    with pytest.raises(ValueError, match="finite"):
        realized_vol([0.0, float("inf")], 2)
    with pytest.raises(ValueError, match="finite"):
        rolling_skew([1.0, 2.0, float("nan")], 3)


def test_ohlcv_bar_validation_is_strict():
    with pytest.raises(ValueError, match="high"):
        OhlcvBar(1, Decimal("10"), Decimal("9"), Decimal("9"), Decimal("10"), 1)
    with pytest.raises(ValueError, match="low"):
        OhlcvBar(1, Decimal("10"), Decimal("10"), Decimal("11"), Decimal("10"), 1)
    with pytest.raises(ValueError, match="timestamp"):
        OhlcvBar(0, Decimal("10"), Decimal("10"), Decimal("9"), Decimal("10"), 1)
