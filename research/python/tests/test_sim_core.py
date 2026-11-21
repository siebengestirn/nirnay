"""Synthetic-event tests for the typed simulation core.

Every input below is fabricated in-process.  No provider payload, credential,
live service or canonical schema record is consumed by these tests.
"""
from __future__ import annotations

import json
import tempfile
import unittest
from dataclasses import replace
from decimal import Decimal
from pathlib import Path
from typing import Any

from quantlab.market import BookLevel, MarketEvent, raw_hash
from quantlab.sim import (
    BookDelta,
    BookSnapshot,
    ConfigChange,
    ConservativeMbpModel,
    EngineConfig,
    ExecutionReport,
    FeeEntry,
    FeeSchedule,
    ImpactConfig,
    MarketStatus,
    OrderAdd,
    OrderCancel,
    QueueTruthUnavailable,
    ReplaySeeds,
    SimBookLevel,
    SimulationEngine,
    SlippageEntry,
    SlippageSchedule,
    TradePrint,
    OrderIntent,
    BaselineParams,
    CostModel,
    default_cost_model,
    empty_state,
    event_to_dict,
    market_event_to_sim_events,
    quarantine_reason,
    reduce,
    state_hash,
)
from quantlab.sim.replay import (
    build_record,
    compare_records,
    read_record,
    record_to_dict,
    replay_in_fresh_process,
    write_record,
)
from quantlab.sim.state import to_jsonable

SHA = "a" * 64
CERT = "b" * 64
INSTRUMENT = "NSE:EQ:SYN"
VENUE = "NSE"


def envelope(
    event_id: str,
    logical_ns: int,
    *,
    seq: int = 0,
    origin: str = "sdk_callback",
    available_ns: int | None = None,
    venue: str = VENUE,
    instrument: str = INSTRUMENT,
) -> dict[str, Any]:
    """Return the common event envelope with explicit lineage."""
    return {
        "event_id": event_id,
        "source_frame_sha256": SHA,
        "instrument_id": instrument,
        "venue": venue,
        "logical_time_ns": logical_ns,
        "available_time_ns": logical_ns if available_ns is None else available_ns,
        "origin_class": origin,
        "derived_from": (event_id,),
        "source_sequence": seq,
    }


def levels(rows: tuple[tuple[str, int], ...]) -> tuple[SimBookLevel, ...]:
    """Build synthetic book levels from string prices and integer sizes."""
    return tuple(SimBookLevel(Decimal(price), size) for price, size in rows)


def snapshot(
    event_id: str,
    logical_ns: int,
    bids: tuple[tuple[str, int], ...] = (("99", 100),),
    asks: tuple[tuple[str, int], ...] = (("101", 100),),
    **kwargs: Any,
) -> BookSnapshot:
    """Build a synthetic two-sided book snapshot."""
    return BookSnapshot(
        **envelope(event_id, logical_ns, **kwargs),
        bids=levels(bids),
        asks=levels(asks),
    )


def order_add(event_id: str, logical_ns: int, *, price: str = "102",
              quantity: int = 10, seq: int = 1) -> OrderAdd:
    """Build a synthetic order submission."""
    return OrderAdd(
        **envelope(event_id, logical_ns, seq=seq, origin="simulated"),
        client_order_id=event_id,
        side="buy",
        quantity=quantity,
        limit_price=Decimal(price),
    )


def trade(event_id: str, logical_ns: int, price: str, quantity: int,
          *, aggressor: str | None = "sell", origin: str = "sdk_callback",
          seq: int = 5) -> TradePrint:
    """Build a synthetic trade print with an explicit aggressor or unknown."""
    return TradePrint(
        **envelope(event_id, logical_ns, seq=seq, origin=origin),
        price=Decimal(price),
        quantity=quantity,
        aggressor_side=aggressor,
    )


def cost_model(*, boundary: int | None = None, impact_bps: str = "0") -> CostModel:
    """Build a fully declared (non-calibrated) cost model for tests."""
    early = FeeEntry(1, "*", "*", "*", "*", Decimal("10"),
                     assumption="synthetic early fee; not calibrated")
    rows = (early,)
    if boundary is not None:
        late = FeeEntry(boundary, "*", "*", "*", "*", Decimal("20"),
                        assumption="synthetic late fee; not calibrated")
        rows = (early, late)
    return CostModel(
        fee_schedule=FeeSchedule(rows),
        slippage_schedule=SlippageSchedule((
            SlippageEntry(1, "*", "*", Decimal("0"),
                          assumption="synthetic zero slippage; not calibrated"),
        )),
        impact=ImpactConfig("sqrt", Decimal(impact_bps), assumption="synthetic impact assumption"),
    )


def params(*, boundary: int | None = None, participation: str = "1",
           latency: int = 1) -> BaselineParams:
    """Build baseline parameters with deterministic synthetic assumptions."""
    return BaselineParams(
        order_latency_ns=latency,
        cancel_latency_ns=latency,
        max_participation=Decimal(participation),
        max_book_age_ns=1_000_000_000,
        cost_model=cost_model(boundary=boundary),
    )


def config(*, boundary: int | None = None) -> EngineConfig:
    """Build a small cash account for synthetic runs."""
    return EngineConfig(
        starting_cash=Decimal("100000"),
        order_latency_ns=1,
        cancel_latency_ns=1,
        max_participation=Decimal("1"),
    )


def run(events: tuple[Any, ...], *, boundary: int | None = None,
        seeds: ReplaySeeds | None = None):
    """Run a deterministic replay with the synthetic baseline model."""
    model = ConservativeMbpModel(params(boundary=boundary))
    return SimulationEngine(config(boundary=boundary)).run(
        events, models=model, seeds=seeds or ReplaySeeds(1, 2, 3, 4)
    )


def market_event(kind: str, seq: int, *, last: Decimal | None = None,
                 quantity: int | None = None, aggressor: str | None = None,
                 bids: tuple[BookLevel, ...] = (),
                 asks: tuple[BookLevel, ...] = ()) -> MarketEvent:
    """Build a canonical MarketEvent for adapter-only tests."""
    received = 1_000_000_000 + seq
    last_trade_time = received if kind == "last_trade_observation" else None
    return MarketEvent(
        schema_version=1,
        event_id=f"m:{seq}",
        provider="synthetic",
        provider_instrument_id="SYN:X",
        instrument_id=INSTRUMENT,
        venue=VENUE,
        event_kind=kind,
        received_utc_ns=received,
        received_monotonic_ns=None,
        local_sequence=seq,
        source_timestamp_ns=None,
        source_timestamp_semantics=None,
        last_trade_time_ns=last_trade_time,
        last_trade_time_semantics=None,
        last_price=last,
        last_quantity=quantity,
        cumulative_volume=None,
        aggressor_side=aggressor,
        bids=bids,
        asks=asks,
        raw_sha256=raw_hash({"kind": kind, "seq": seq}),
    )


class ReplayDeterminismTests(unittest.TestCase):
    def test_deterministic_replay_byte_identical_in_fresh_process(self) -> None:
        events = (
            snapshot("b1", 1),
            order_add("o1", 2, quantity=6),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
            trade("t1", 4, "101", 8, seq=3),
        )
        cfg = config()
        model = ConservativeMbpModel(params())
        seeds = ReplaySeeds(7, 11, 13, 17)
        result = SimulationEngine(cfg).run(events, models=model, seeds=seeds)
        record = build_record(result, engine_config=cfg,
                              model_config=model.params.to_dict(), seeds=seeds)
        original_bytes = json.dumps(record_to_dict(record), sort_keys=True).encode()
        with tempfile.TemporaryDirectory() as directory:
            written = write_record(record, Path(directory))
            self.assertEqual(read_record(written), record)
            for hash_seed in ("1", "2", "12345", "random"):
                fresh = replay_in_fresh_process(record, python_hash_seed=hash_seed)
                fresh_bytes = json.dumps(record_to_dict(fresh), sort_keys=True).encode()
                self.assertEqual(fresh_bytes, original_bytes)
                comparison = compare_records(record, fresh)
                self.assertTrue(comparison.verified)
                self.assertEqual(comparison.pnl_basis, "recomputed")
                self.assertEqual(fresh.final_state_hash, record.final_state_hash)

    def test_verifier_excludes_copied_pnl_and_recording_has_no_basis_field(self) -> None:
        result = run((snapshot("b1", 1), order_add("o1", 2, quantity=3),
                      snapshot("b2", 3, asks=(("101", 110),), seq=2)))
        cfg = config()
        seeds = ReplaySeeds(1, 2, 3, 4)
        record = build_record(result, engine_config=cfg,
                              model_config=params().to_dict(), seeds=seeds)
        self.assertNotIn("pnl_basis", record_to_dict(record))
        tampered = replace(record, net_pnl="0", record_sha256="0" * 64)
        comparison = compare_records(record, tampered)
        self.assertFalse(comparison.verified)
        self.assertIn("net_pnl", comparison.excluded_fields)

    def test_state_hash_is_deterministic_and_exact(self) -> None:
        first = empty_state(Decimal("1000"), params())
        second = empty_state(Decimal("1000"), params())
        self.assertEqual(state_hash(first), state_hash(second))
        changed = first.updated(cash=Decimal("1000.000000000000000001"))
        self.assertNotEqual(state_hash(first), state_hash(changed))
        self.assertFalse(any(isinstance(value, float) for value in to_jsonable(first)))

    def test_event_ordering_key_is_venue_scoped_and_five_part(self) -> None:
        first = snapshot("a", 10, seq=3)
        other_venue = snapshot("b", 10, seq=1, venue="BSE")
        later_seq = snapshot("c", 10, seq=4)
        self.assertEqual(len(first.ordering_key()), 5)
        self.assertLess(first.ordering_key(), later_seq.ordering_key())
        self.assertNotEqual(first.ordering_key(), other_venue.ordering_key())
        self.assertEqual(first.ordering_key()[0], 10)


class LiquidityDisciplineTests(unittest.TestCase):
    def test_no_free_liquidity_from_identical_snapshots(self) -> None:
        events = (
            snapshot("b1", 1),
            order_add("o1", 2, quantity=10),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
            snapshot("b3", 4, asks=(("101", 110),), seq=3),
            snapshot("b4", 5, asks=(("101", 110),), seq=4),
        )
        result = run(events)
        self.assertEqual(len(result.fills), 1)
        self.assertEqual(result.fills[0].quantity, 10)
        self.assertEqual(result.positions[INSTRUMENT], 10)

    def test_book_delta_zero_quantity_deletes_a_level_without_new_liquidity(self) -> None:
        base = snapshot("b1", 1)
        delta = BookDelta(
            **envelope("d1", 2, seq=1),
            side="ask",
            updates=levels((("101", 0), ("102", 5))),
        )
        result = reduce(empty_state(Decimal("1000"), params()), delta)
        self.assertEqual(result[0].books[(VENUE, INSTRUMENT)].asks[0].price, Decimal("102"))
        self.assertEqual(base.bids[0].price, Decimal("99"))

    def test_unknown_aggressor_never_queue_advances(self) -> None:
        book = snapshot("b1", 1, bids=(("100", 4),))
        add = order_add("o1", 2, price="100", quantity=10)
        activation = snapshot("b2", 3, bids=(("100", 4),), seq=2)
        unknown = trade("t1", 4, "100", 10, aggressor=None, seq=3)
        reconstructed = trade("t2", 5, "100", 10, origin="reconstructed", seq=4)
        self.assertEqual(len(run((book, add, activation, unknown)).fills), 0)
        self.assertEqual(len(run((book, add, activation, reconstructed)).fills), 0)
        confirmed = trade("t3", 6, "100", 10, seq=5)
        self.assertEqual(len(run((book, add, activation, confirmed)).fills), 1)

    def test_partial_fill_from_trade_print_is_emitted_once(self) -> None:
        events = (
            snapshot("b1", 1, bids=(("100", 4),)),
            order_add("o1", 2, price="100", quantity=10),
            snapshot("b2", 3, bids=(("100", 4),), seq=2),
            trade("t1", 4, "100", 10, seq=3),
        )
        result = run(events)
        self.assertEqual(len(result.fills), 1)
        self.assertEqual(result.fills[0].quantity, 6)
        self.assertEqual(result.fills[0].liquidity, "maker")
        self.assertEqual(result.positions[INSTRUMENT], 6)

    def test_execution_reports_are_the_only_cash_and_position_channel(self) -> None:
        state = empty_state(Decimal("1000"), params())
        book_state, book_out = reduce(state, snapshot("b1", 1, asks=(("101", 110),)))
        self.assertEqual(book_state.cash, state.cash)
        self.assertEqual(book_state.positions, state.positions)
        add_state, add_out = reduce(book_state, order_add("o1", 2, quantity=5))
        self.assertEqual(add_state.cash, state.cash)
        second_book, book_out2 = reduce(add_state, snapshot("b2", 3, asks=(("101", 120),), seq=2))
        fills = [item for item in book_out2 if item.report_type == "fill"]
        self.assertTrue(fills)
        self.assertEqual(second_book.cash, state.cash)
        self.assertEqual(second_book.positions, state.positions)
        applied, empty = reduce(second_book, fills[0])
        self.assertLess(applied.cash, state.cash)
        self.assertEqual(applied.positions[INSTRUMENT], fills[0].quantity)
        self.assertEqual(empty, ())
        again, _ = reduce(applied, fills[0])
        self.assertEqual(again.cash, applied.cash)

    def test_unmarked_residual_is_flagged_never_zero_valued(self) -> None:
        one_sided = snapshot("b1", 1, bids=(), asks=(("101", 100),))
        events = (
            one_sided,
            order_add("o1", 2, quantity=4),
            snapshot("b2", 3, bids=(), asks=(("101", 110),), seq=2),
        )
        result = run(events)
        self.assertTrue(result.unmarked_residual)
        self.assertEqual(len(result.unmarked_details), 1)
        self.assertNotIn(INSTRUMENT, result.marks)
        self.assertEqual(result.positions[INSTRUMENT], 4)

    def test_cancel_exposure_persists_until_cancel_ack(self) -> None:
        events = (
            snapshot("b1", 1, bids=(("100", 4),)),
            order_add("o1", 2, price="100", quantity=10),
            snapshot("b2", 3, bids=(("100", 4),), seq=2),
            OrderCancel(
                **envelope("c1", 4, seq=3, origin="simulated"),
                client_order_id="o1",
            ),
            trade("t1", 4, "100", 10, seq=4),
        )
        self.assertEqual(len(run(events).fills), 1)
        later = (
            *events[:4],
            trade("t2", 100, "100", 10, seq=9),
        )
        self.assertEqual(len(run(later).fills), 0)


class ValidationTests(unittest.TestCase):
    def test_config_and_lineage_validation_fails_closed(self) -> None:
        fields = envelope("e1", 1)
        with self.assertRaises(ValueError):
            TradePrint(**{**fields, "source_frame_sha256": ""}, price=Decimal("1"), quantity=1)
        with self.assertRaises(ValueError):
            TradePrint(**{**fields, "source_frame_sha256": "xyz"}, price=Decimal("1"), quantity=1)
        with self.assertRaises(ValueError):
            TradePrint(**{**fields, "origin_class": "unknown"}, price=Decimal("1"), quantity=1)
        with self.assertRaises(ValueError):
            TradePrint(
                **{**fields, "derived_from": "not-a-tuple"}, price=Decimal("1"), quantity=1
            )
        with self.assertRaises(ValueError):
            TradePrint(**{**fields, "origin_class": "exchange_event"},
                       price=Decimal("1"), quantity=1)
        certified = TradePrint(
            **{**fields, "origin_class": "exchange_event", "certification_ref": CERT},
            price=Decimal("1"),
            quantity=1,
        )
        self.assertEqual(certified.origin_class, "exchange_event")
        with self.assertRaises(ValueError):
            ConfigChange(**envelope("c1", 1), config_key="fees", config_digest="xyz",
                         effective_ns=1)
        with self.assertRaises(ValueError):
            MarketStatus(**envelope("s1", 1), status="unknown")

    def test_ack_precedes_any_fill(self) -> None:
        events = (
            snapshot("b1", 1),
            order_add("o1", 2, quantity=5),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
        )
        result = run(events)
        kinds = [(item.report_type, item.logical_time_ns) for item in result.emitted]
        ack = next(index for index, item in enumerate(kinds) if item[0] == "ack")
        new = next(index for index, item in enumerate(kinds) if item[0] == "new")
        fill = next(index for index, item in enumerate(kinds) if item[0] == "fill")
        self.assertLess(ack, new)
        self.assertLess(new, fill)
        self.assertGreaterEqual(
            result.fills[0].logical_time_ns, result.emitted[new].logical_time_ns
        )

    def test_effective_dated_fees_change_across_boundary(self) -> None:
        boundary = 10_000
        events = (
            snapshot("b1", 1, asks=(("101", 100),)),
            order_add("o1", 2, quantity=5),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
            order_add("o2", 9_999, quantity=5, seq=3),
            snapshot("b3", 10_000, asks=(("101", 120),), seq=4),
        )
        result = run(events, boundary=boundary)
        self.assertEqual(len(result.fills), 2)
        self.assertEqual(result.fills[0].fees, Decimal("0.5050000000000000000000000"))
        self.assertEqual(result.fills[1].fees, Decimal("1.0100000000000000000000000"))
        self.assertNotEqual(result.fills[0].fees, result.fills[1].fees)

    def test_queue_truth_guard_refuses_fifo_claims(self) -> None:
        model = ConservativeMbpModel(params())
        self.assertEqual(model.queue_truth, "unavailable")
        self.assertIn("mbp", model.priority_model_id)
        with self.assertRaises(QueueTruthUnavailable):
            model.fifo_claim(VENUE, INSTRUMENT)
        result = run((snapshot("b1", 1), order_add("o1", 2, quantity=1),
                      snapshot("b2", 3, asks=(("101", 110),), seq=2)))
        self.assertFalse(any("fifo" in str(key).lower() for item in result.emitted
                             for key in event_to_dict(item)))
        with self.assertRaises(ValueError):
            market_event_to_sim_events(
                market_event("quote", 1, bids=(BookLevel(Decimal("99"), 10),)),
                origin_class="exchange_event",
            )

    def test_cost_assumptions_require_artifact_or_assumption(self) -> None:
        with self.assertRaises(ValueError):
            ImpactConfig("sqrt", Decimal("1"))
        with self.assertRaises(ValueError):
            ImpactConfig("sqrt", Decimal("1"), artifact_sha256="not-a-digest")
        assumed = ImpactConfig("sqrt", Decimal("1"), assumption="synthetic assumption")
        artifact = ImpactConfig("sqrt", Decimal("1"), artifact_sha256=SHA)
        self.assertNotEqual(assumed.impact_bps(Decimal("0.25")), Decimal("0"))
        self.assertEqual(assumed.impact_bps(Decimal("0.25")),
                         artifact.impact_bps(Decimal("0.25")))
        with self.assertRaises(ValueError):
            FeeEntry(1, "*", "*", "*", "*", Decimal("1"))


class AdapterTests(unittest.TestCase):
    def test_adapter_quarantines_ltp_and_unknown_aggressor(self) -> None:
        ltp = market_event("last_trade_observation", 1, last=Decimal("100"), quantity=5)
        converted = market_event_to_sim_events(ltp)
        self.assertEqual(len(converted), 1)
        self.assertEqual(converted[0].origin_class, "reconstructed")
        self.assertFalse(converted[0].queue_advancing)
        self.assertIsNotNone(quarantine_reason(ltp))
        unknown = market_event("trade", 2, last=Decimal("100"), quantity=5)
        self.assertFalse(market_event_to_sim_events(unknown)[0].queue_advancing)
        confirmed = market_event("trade", 3, last=Decimal("100"), quantity=5, aggressor="sell")
        self.assertTrue(market_event_to_sim_events(confirmed)[0].queue_advancing)
        quote = market_event("quote", 4, bids=(BookLevel(Decimal("99"), 10),))
        self.assertEqual(market_event_to_sim_events(quote)[0].depth_granularity, "mbp")
        self.assertEqual(market_event_to_sim_events(market_event("status", 5)), ())

    def test_mark_clears_stale_unmarked_residual(self) -> None:
        engine = SimulationEngine(config())
        add = order_add("add-1", 1)
        fill = ExecutionReport(
            **envelope("fill-1", 2, seq=2, origin="simulated"),
            client_order_id="add-1",
            report_type="fill",
            quantity=10,
            price=Decimal("100"),
            liquidity="taker",
        )
        state = reduce(reduce(empty_state(Decimal("1000"), params()), add)[0], fill)[0]
        self.assertTrue(state.unmarked_residual, "unmarked position after fill")
        marked = reduce(state, snapshot("book-1", 3))[0]
        self.assertFalse(marked.unmarked_residual, "mark clears stale residual")

if __name__ == "__main__":
    unittest.main()
