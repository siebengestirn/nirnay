#!/usr/bin/env python3
"""Generate golden parity vectors for the rational EMA strategy.

This is the research-side reference using fractions.Fraction with the same
rules as the Rust runtime. It is a systems fixture, not validated alpha.
"""
from __future__ import annotations

import json
from decimal import ROUND_HALF_UP, Decimal
from fractions import Fraction
from pathlib import Path

HERE = Path(__file__).resolve().parent
FAST, SLOW, QUANTITY, MIN_OBS, BPS = 2, 3, 2, 3, 100
PRICES = [10, 9, 8, 7, 8, 9, 10, 9, 8, 7]
BID_OFFSET = 5
ASK_OFFSET = 5

ARTIFACT = {
    "candidate_id": "fixture.rational-ema.v1",
    "hypothesis": "Deterministic systems fixture for strategy-runtime parity",
    "return_source": "momentum",
    "universe": ["IN:TEST"],
    "entry_rule": "rational_ema_cross_v1",
    "exit_rule": "opposite_cross",
    "rebalance_frequency": "intraday",
    "experiment_ids": ["exp-fixture-0001"],
    "validation": {
        "walk_forward_windows": 2,
        "holdout_locked": True,
        "multiple_testing_correction": "bonferroni",
        "deflated_sharpe": "0.000000",
        "cost_model": "fixture-no-costs",
        "probability_of_backtest_overfit": "1.000000",
    },
    "status": "approved_for_paper",
}


def limit_price(side: str, reference: int, bps: int) -> int:
    basis = Fraction(10_000 + (bps if side == "buy" else -bps), 10_000)
    value = Fraction(reference) * basis
    return int(
        (Decimal(value.numerator) / Decimal(value.denominator)).quantize(
            Decimal("1"), rounding=ROUND_HALF_UP
        )
    )


def update(previous, value, span):
    if previous is None:
        return value
    alpha = Fraction(2, span + 1)
    return alpha * value + (1 - alpha) * previous


def run() -> dict:
    fast_ema = slow_ema = previous_fast = previous_slow = None
    observations = 0
    intent_sequence = 0
    position = 0
    has_open_order = False
    vectors = []
    for sequence, mid in enumerate(PRICES, 1):
        midpoint = Fraction(mid, 1)
        event = {
            "sequence": sequence,
            "instrument": "IN:TEST",
            "midpoint_num": midpoint.numerator,
            "midpoint_den": midpoint.denominator,
            "best_bid": (mid - 1) * 100 - BID_OFFSET,
            "best_ask": (mid + 1) * 100 + ASK_OFFSET,
            "position": position,
            "has_open_order": has_open_order,
        }
        previous_fast, previous_slow = fast_ema, slow_ema
        fast_ema = update(fast_ema, midpoint, FAST)
        slow_ema = update(slow_ema, midpoint, SLOW)
        observations += 1

        intent = None
        ready = (
            observations >= MIN_OBS
            and not has_open_order
            and previous_fast is not None
            and previous_slow is not None
        )
        if ready:
            crossed_up = previous_fast <= previous_slow and fast_ema > slow_ema
            crossed_down = previous_fast >= previous_slow and fast_ema < slow_ema
            if crossed_up and position == 0:
                side, qty, ref = "buy", QUANTITY, event["best_ask"]
            elif crossed_down and position > 0:
                side, qty, ref = "sell", position, event["best_bid"]
            else:
                side = None
            if side is not None:
                intent_sequence += 1
                intent = {
                    "intent_id": f"{ARTIFACT['candidate_id']}:{intent_sequence:06}",
                    "candidate_id": ARTIFACT["candidate_id"],
                    "instrument_id": "IN:TEST",
                    "side": side,
                    "quantity": qty,
                    "limit_price_paise": limit_price(side, ref, BPS),
                }
                if side == "buy":
                    position += qty
                else:
                    position = 0
        event["intent"] = intent
        vectors.append(event)
    return {
        "artifact": ARTIFACT,
        "params": {
            "fast": FAST,
            "slow": SLOW,
            "quantity": QUANTITY,
            "min_observations": MIN_OBS,
            "aggressive_limit_bps": BPS,
        },
        "events": vectors,
    }


if __name__ == "__main__":
    (HERE / "golden" / "artifact-v1.json").write_text(
        json.dumps(ARTIFACT, indent=2) + "\n"
    )
    (HERE / "golden" / "ema-rational-v1.json").write_text(
        json.dumps(run(), indent=2) + "\n"
    )
