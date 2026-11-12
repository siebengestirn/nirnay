# Strategies

## Deterministic reference strategy

`quantlab.strategies.DeterministicEmaCross` is the shipped deterministic streaming EMA-cross example. It accepts canonical market events, maintains causal recursive EMA state, emits long-only `OrderIntent` values and has no broker, OMS, risk-service, cash or position-authority access. Its behavior is exercised by the synthetic `quantlab demo` fixture.

```bash
python3 -m pip install -e 'research/python[dev]'
PYTHONPATH=research/python/src python3 -m quantlab.cli demo
```

The demo is a contract/replay test only. It reports `not_real_market_data=true` and `not_alpha_evidence=true`. It is not an endorsed strategy, investment recommendation or evidence of profitability.

## Strategy admission

A strategy is added here only after its economic hypothesis, falsifier, universe, point-in-time data requirements, cost assumptions, benchmarks, validation protocol and evidence bundle have been registered. Strategy modules emit intents; they never access broker SDKs, OMS mutation methods, authoritative cash/positions, risk permits or live-routing switches.

Candidate promotion requires, at minimum: immutable dataset and strategy hashes; chronological train/validation/untouched holdout; explicit embargo/purge policy; multiple-testing accounting; effective dated brokerage/exchange/tax charges; spread, slippage, market-impact and capacity assumptions; event-level simulation; regime and sensitivity analysis; paper evidence; and independent review. A positive bar-close screen is not alpha certification.
