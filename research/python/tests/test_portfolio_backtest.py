from __future__ import annotations

from dataclasses import replace
from decimal import Decimal as D

import pytest

from quantlab.backtest import OrderIntent
from quantlab.market import BookLevel, MarketEvent, raw_hash
from quantlab.portfolio_backtest import MultiInstrumentPortfolioBacktester, PortfolioBacktestConfig


def event(seq: int, instrument: str, *, provider="synthetic", kind="quote", bid=D("99.9"), ask=D("100.1"),
          bid_qty=100, ask_qty=100, last=None, qty=None, aggressor=None, recv=None, local_seq=None):
    receive = recv if recv is not None else 1_000_000_000 + seq * 100_000_000
    bids = (BookLevel(bid, bid_qty),) if bid is not None and bid_qty else ()
    asks = (BookLevel(ask, ask_qty),) if ask is not None and ask_qty else ()
    payload = {"seq": seq, "instrument": instrument, "kind": kind, "bid": str(bid) if bid is not None else None,
               "ask": str(ask) if ask is not None else None, "bq": bid_qty, "aq": ask_qty,
               "last": str(last) if last is not None else None, "qty": qty, "aggressor": aggressor}
    return MarketEvent(1, f"{provider}:{instrument}:{seq}", provider, instrument, instrument, "NSE", kind,
        receive, seq * 100_000_000, seq if local_seq is None else local_seq, None, None, None, None,
        last, qty, None, aggressor, bids, asks, raw_hash(payload), ("synthetic_fixture",))


class BuyFirstPerInstrument:
    strategy_id = "test.portfolio.buy-first"
    def __init__(self): self.done = set()
    def on_event(self, event, context):
        if event.instrument_id in self.done: return None
        self.done.add(event.instrument_id)
        return OrderIntent(f"buy:{event.instrument_id}", event.instrument_id, "buy", 1, D("101"), 5_000_000_000)


def test_portfolio_backtest_handles_two_symbols_and_marks_equity():
    events = [event(0, "NSE:AAA"), event(1, "NSE:BBB"),
              event(2, "NSE:AAA", ask_qty=101), event(3, "NSE:BBB", ask_qty=101)]
    cfg = PortfolioBacktestConfig(starting_cash=D("1000"), order_latency_ns=1,
        max_participation=D("1"), fee_bps=D("0"), impact_coefficient_bps=D("0"),
        max_order_notional=D("200"), max_gross_exposure=D("500"))
    result = MultiInstrumentPortfolioBacktester(cfg).run(events, BuyFirstPerInstrument())
    assert result.orders == 2
    assert result.positions == {"NSE:AAA": 1, "NSE:BBB": 1}
    assert result.ending_equity == D("999.8")  # two buys cross the 10 bp half-spread
    assert len(result.fills) == 2


def test_portfolio_refuses_to_double_count_two_provider_feeds():
    one = event(0, "NSE:AAA", provider="upstox")
    two = replace(one, event_id="fyers:AAA:0", provider="fyers", provider_instrument_id="FYERS:AAA")
    with pytest.raises(ValueError, match="mixed-provider"):
        MultiInstrumentPortfolioBacktester(PortfolioBacktestConfig()).run([one, two], BuyFirstPerInstrument())


def test_portfolio_enforces_participation_without_minimum_one_share_override():
    events = [event(0, "NSE:AAA", ask_qty=1), event(1, "NSE:AAA", ask_qty=2)]
    cfg = PortfolioBacktestConfig(starting_cash=D("1000"), order_latency_ns=1,
        max_participation=D("0.1"), fee_bps=D("0"), impact_coefficient_bps=D("0"),
        max_order_notional=D("200"), max_gross_exposure=D("500"))
    result = MultiInstrumentPortfolioBacktester(cfg).run(events, BuyFirstPerInstrument())
    assert sum(x.quantity for x in result.fills) == 0


def test_portfolio_reserves_cash_for_all_open_buy_orders():
    class BuyTwo:
        strategy_id = "test.two-buys"
        def __init__(self): self.done=set()
        def on_event(self, event, context):
            if event.instrument_id in self.done: return None
            self.done.add(event.instrument_id)
            return OrderIntent(f"buy:{event.instrument_id}", event.instrument_id, "buy", 1, D("100"), 5_000_000_000)
    events = [event(0, "NSE:A"), event(1, "NSE:B"), event(2, "NSE:A", ask_qty=100), event(3, "NSE:B", ask_qty=100)]
    cfg = PortfolioBacktestConfig(starting_cash=D("150"), order_latency_ns=10_000_000_000,
        max_participation=D("1"), fee_bps=D("0"), impact_coefficient_bps=D("0"),
        max_order_notional=D("100"), max_gross_exposure=D("500"))
    result = MultiInstrumentPortfolioBacktester(cfg).run(events, BuyTwo())
    assert len(result.rejections) == 1
    assert result.orders == 1
