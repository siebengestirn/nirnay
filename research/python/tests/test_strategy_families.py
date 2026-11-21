"""Public strategy-family signal tests against the existing screen and falsifiers."""
from __future__ import annotations

from decimal import Decimal

import pytest

from quantlab.alpha_search import AlphaConfig, _signals
from quantlab.strategies import (assert_factor_budget, factor_weights,
                                 mean_reversion_signal, momentum_signal, pair_intents,
                                 pair_signals, signals_for_family, spread_series)

CLOSES = [100.0 + (index % 9) * 1.75 + index * 0.2 for index in range(60)]


def _prices() -> tuple[list[Decimal], list[Decimal]]:
    left = [Decimal("100") + Decimal(index) / Decimal(4) for index in range(40)]
    right = [Decimal("100") + Decimal(index) / Decimal(10) for index in range(40)]
    return left, right


def test_promoted_signals_match_existing_screen_byte_for_byte():
    for family in ("momentum", "mean_reversion"):
        config = AlphaConfig(family, lookback=5, threshold_bps=10.0)
        expected = _signals(CLOSES, config)
        public = signals_for_family(CLOSES, family, lookback=5, threshold_bps=10.0)
        assert public == expected
    assert momentum_signal(CLOSES, lookback=5, threshold_bps=10.0) == _signals(
        CLOSES, AlphaConfig("momentum", lookback=5, threshold_bps=10.0)
    )
    assert mean_reversion_signal(CLOSES, lookback=5, threshold_bps=10.0) == _signals(
        CLOSES, AlphaConfig("mean_reversion", lookback=5, threshold_bps=10.0)
    )


def test_ema_cross_dispatch_matches_existing_screen_and_unknown_family_raises():
    config = AlphaConfig("ema_cross", lookback=8, fast=3, slow=8)
    assert signals_for_family(CLOSES, "ema_cross", fast=3, slow=8) == _signals(CLOSES, config)
    with pytest.raises(ValueError, match="unknown signal family"):
        signals_for_family(CLOSES, "machine_learning")


@pytest.mark.parametrize("family", ["momentum", "mean_reversion"])
def test_appending_future_bar_does_not_change_earlier_signal(family):
    original = signals_for_family(CLOSES[:30], family, lookback=5, threshold_bps=10.0)
    extended = signals_for_family(CLOSES + [10_000.0], family, lookback=5,
                                  threshold_bps=10.0)
    assert extended[:len(original)] == original


def test_factor_weights_are_deterministic_net_flat_and_budgeted():
    scores = {"a": 1.0, "b": 4.0, "c": 3.0, "d": 2.0, "e": 5.0}
    first = factor_weights(scores, budget=2.0)
    second = factor_weights(scores, budget=2.0)
    assert first == second
    assert list(first) == sorted(first)
    assert_factor_budget(first, 2.0)
    assert sum(abs(value) for value in first.values()) == pytest.approx(2.0)
    assert sum(first.values()) == pytest.approx(0.0)
    assert first["e"] == pytest.approx(0.5)
    assert first["a"] == pytest.approx(-0.5)
    assert first["c"] == 0.0


def test_factor_budget_falsifier_and_invalid_scores():
    with pytest.raises(ValueError, match="gross budget"):
        assert_factor_budget({"a": 0.25, "b": -0.25}, 1.0)
    with pytest.raises(ValueError, match="at least two"):
        factor_weights({"only": 1.0})
    with pytest.raises(ValueError, match="finite"):
        factor_weights({"a": float("nan"), "b": 1.0})


def test_pair_spread_and_zscore_signals_are_causal():
    left, right = _prices()
    spread = spread_series(left, right)
    assert spread[0] == Decimal("0")
    assert spread[-1] == Decimal("5.85")
    first = pair_signals(spread[:20], window=5, entry_z=1.0)
    second = pair_signals(spread, window=5, entry_z=1.0)
    assert second[:len(first)] == first
    with pytest.raises(ValueError, match="positive"):
        pair_signals(spread, window=5, entry_z=0.0)


def test_pair_intents_are_two_leg_and_research_only():
    left, right = _prices()
    timestamps = [1_700_000_000_000_000_000 + index for index in range(len(left))]
    intents = pair_intents(timestamps, left, right, window=5, entry_z=0.5)
    assert intents
    for intent in intents:
        row = intent.as_dict()
        assert set(row) == {"index", "timestamp_ns", "spread_z", "target_position",
                            "leg_a_side", "leg_b_side", "quantity"}
        assert (row["leg_a_side"], row["leg_b_side"]) in {
            ("buy", "sell"), ("sell", "buy")
        }
        assert row["quantity"] == 1
    assert intents == pair_intents(timestamps, left, right, window=5, entry_z=0.5)


def test_strategy_config_validation_is_strict():
    with pytest.raises(ValueError, match="lookback"):
        momentum_signal(CLOSES, lookback=1)
    with pytest.raises(ValueError, match="threshold_bps"):
        mean_reversion_signal(CLOSES, lookback=5, threshold_bps=float("inf"))
    with pytest.raises(ValueError, match="aligned"):
        spread_series([Decimal("1")], [])
