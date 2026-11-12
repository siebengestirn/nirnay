"""Reproducible synthetic market session for the deterministic EMA strategy."""
from __future__ import annotations

import json
from decimal import Decimal

from .backtest import BacktestConfig, EventDrivenBacktester
from .market import BookLevel, MarketEvent, raw_hash
from .strategies import DeterministicEmaCross


def synthetic_events() -> list[MarketEvent]:
    # Fully synthetic path with a down leg, rebound and second down leg.  These
    # fixtures test state transitions and execution assumptions, not alpha.
    prices = [100, 100, 100, 100, 99, 98, 97, 96, 97, 98, 99, 100, 101, 102,
              103, 103, 102, 101, 100, 99, 98, 97, 96, 96, 97, 98]
    events: list[MarketEvent] = []
    base_ns = 1_800_000_000_000_000_000
    for i, raw in enumerate(prices):
        mid = Decimal(raw)
        payload = {"fixture": "synthetic-ema-cross-v1", "sequence": i, "mid": str(mid)}
        events.append(MarketEvent(
            schema_version=1, event_id=f"synthetic:{i:06d}", provider="synthetic",
            provider_instrument_id="SYN:DEMO", instrument_id="fixture.demo",
            venue="NSE", event_kind="quote", received_utc_ns=base_ns + i * 1_000_000_000,
            received_monotonic_ns=i * 1_000_000_000, local_sequence=i + 1,
            source_timestamp_ns=None, source_timestamp_semantics=None,
            last_trade_time_ns=None, last_trade_time_semantics=None,
            last_price=mid, last_quantity=None, cumulative_volume=i * 100,
            aggressor_side=None,
            bids=(BookLevel(mid - Decimal("0.05"), 100),),
            asks=(BookLevel(mid + Decimal("0.05"), 100),),
            raw_sha256=raw_hash(payload), quality_flags=("synthetic_fixture",)))
    return events


def run_demo() -> dict[str, object]:
    strategy = DeterministicEmaCross(instrument_id="fixture.demo", fast=2, slow=5,
                                     quantity=10, aggressive_limit_bps=Decimal("100"))
    config = BacktestConfig(starting_cash=Decimal("100000"), order_latency_ns=100_000_000,
                            cancel_latency_ns=50_000_000, fee_bps=Decimal("2.0"),
                            impact_coefficient_bps=Decimal("1.0"),
                            max_participation=Decimal("0.20"), seed=42)
    result = EventDrivenBacktester(config).run(synthetic_events(), strategy)
    return {"fixture": "synthetic-ema-cross-v1", "not_real_market_data": True,
            "not_alpha_evidence": True, **result.summary(),
            "fills": [{"client_order_id": f.client_order_id, "event_id": f.event_id,
                       "timestamp_ns": f.timestamp_ns, "side": f.side,
                       "quantity": f.quantity, "price": str(f.price),
                       "fees": str(f.fees), "impact_cost": str(f.impact_cost),
                       "spread_cost": str(f.spread_cost), "liquidity": f.liquidity} for f in result.fills]}


if __name__ == "__main__":
    print(json.dumps(run_demo(), sort_keys=True, indent=2))
