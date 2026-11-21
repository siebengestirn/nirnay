"""Research-only tests for the simulation completion extensions.

All inputs are fabricated in-process.  No provider payload, credential,
licensed feed, live service or canonical provider record is consumed here.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from dataclasses import replace
from decimal import Decimal
from pathlib import Path
from typing import Any

from quantlab.sim import (
    ArtifactError,
    ArtifactResolver,
    AuctionPhase,
    BaselineParams,
    BorrowTerms,
    ConservativeMbpModel,
    CostModel,
    DuplicateTrial,
    EngineConfig,
    FeeEntry,
    FeeSchedule,
    FundingTerms,
    ImpactConfig,
    InstrumentChange,
    LatencyDistribution,
    MarginTerms,
    MarketStatus,
    OrderAdd,
    ReplaySeeds,
    SimBookLevel,
    SimulationEngine,
    SlippageEntry,
    SlippageSchedule,
    TrialConflict,
    TrialStore,
)
from quantlab.sim.calibration import calibration_statement, cost_provenance
from quantlab.sim.events import digest
from quantlab.sim.replay import (
    build_record,
    compare_records,
    record_to_dict,
    replay_in_fresh_process,
    verifier_recompute,
    write_record,
)

SHA = "a" * 64
INSTRUMENT = "NSE:EQ:SYN"
VENUE = "NSE"
SRC = Path(__file__).resolve().parents[1] / "src"
DAY_NS = 86_400_000_000_000


def envelope(event_id: str, logical_ns: int, *, seq: int = 0,
             origin: str = "sdk_callback") -> dict[str, Any]:
    """Return the common lineage-bearing event envelope."""
    return {
        "event_id": event_id,
        "source_frame_sha256": SHA,
        "instrument_id": INSTRUMENT,
        "venue": VENUE,
        "logical_time_ns": logical_ns,
        "available_time_ns": logical_ns,
        "origin_class": origin,
        "derived_from": (event_id,),
        "source_sequence": seq,
    }


def levels(rows: tuple[tuple[str, int], ...]) -> tuple[SimBookLevel, ...]:
    """Build synthetic displayed levels."""
    return tuple(SimBookLevel(Decimal(price), size) for price, size in rows)


def snapshot(event_id: str, logical_ns: int, *, bids: tuple[tuple[str, int], ...] = (("99", 100),),
             asks: tuple[tuple[str, int], ...] = (("101", 100),), seq: int = 0) -> Any:
    """Build a two-sided book snapshot."""
    from quantlab.sim import BookSnapshot

    return BookSnapshot(
        **envelope(event_id, logical_ns, seq=seq), bids=levels(bids), asks=levels(asks)
    )


def order_add(event_id: str, logical_ns: int, *, side: str = "buy", price: str = "102",
              quantity: int = 5, seq: int = 1) -> OrderAdd:
    """Build one simulated order."""
    return OrderAdd(
        **envelope(event_id, logical_ns, seq=seq, origin="simulated"),
        client_order_id=event_id,
        side=side,
        quantity=quantity,
        limit_price=Decimal(price),
    )


def trade(event_id: str, logical_ns: int, price: str, quantity: int, *,
          aggressor: str | None = "sell", seq: int = 5) -> Any:
    """Build one confirmed trade print."""
    from quantlab.sim import TradePrint

    return TradePrint(
        **envelope(event_id, logical_ns, seq=seq),
        price=Decimal(price),
        quantity=quantity,
        aggressor_side=aggressor,
    )


def status(event_id: str, logical_ns: int, value: str, *, seq: int = 6) -> MarketStatus:
    """Build one market-status transition."""
    return MarketStatus(**envelope(event_id, logical_ns, seq=seq), status=value)


def auction(event_id: str, logical_ns: int, reference_price: str, *, seq: int = 7) -> AuctionPhase:
    """Build one synthetic auction uncross with a declared reference price."""
    return AuctionPhase(
        **envelope(event_id, logical_ns, seq=seq),
        phase="uncross",
        reference_price=Decimal(reference_price),
    )


def base_cost(*, borrow: tuple[BorrowTerms, ...] = (), margin: MarginTerms | None = None,
              funding: tuple[FundingTerms, ...] = (), impact_artifact: str | None = None,
              impact_assumption: str | None = "synthetic zero impact; not calibrated") -> CostModel:
    """Build a fully declared, non-calibrated synthetic cost model."""
    fee = FeeEntry(1, "*", "*", "*", "*", Decimal("10"),
                   assumption="synthetic early fee; not calibrated")
    slippage = SlippageEntry(1, "*", "*", Decimal("0"),
                             assumption="synthetic zero slippage; not calibrated")
    impact = ImpactConfig(
        "sqrt", Decimal("0"), artifact_sha256=impact_artifact, assumption=impact_assumption
    )
    return CostModel(
        fee_schedule=FeeSchedule((fee,)),
        slippage_schedule=SlippageSchedule((slippage,)),
        impact=impact,
        borrow=borrow,
        margin=margin,
        funding=funding,
    )


def params(*, cost: CostModel | None = None, latency: LatencyDistribution | None = None,
           latency_ns: int = 1) -> BaselineParams:
    """Build baseline parameters with declared latency."""
    return BaselineParams(
        order_latency_ns=latency_ns,
        cancel_latency_ns=latency_ns,
        max_participation=Decimal("1"),
        max_book_age_ns=1_000_000_000_000_000,
        cost_model=cost or base_cost(),
        latency=latency,
    )


def engine_config() -> EngineConfig:
    """Build a small synthetic cash account."""
    return EngineConfig(
        starting_cash=Decimal("100000"),
        order_latency_ns=1,
        cancel_latency_ns=1,
        max_participation=Decimal("1"),
        max_book_age_ns=1_000_000_000_000_000,
    )


def run(events: tuple[Any, ...], *, model_params: BaselineParams | None = None,
        seeds: ReplaySeeds | None = None) -> Any:
    """Run one deterministic replay."""
    model = ConservativeMbpModel(model_params or params())
    return SimulationEngine(engine_config()).run(
        events, models=model, seeds=seeds or ReplaySeeds(1, 2, 3, 4)
    )


class LatencySamplerTests(unittest.TestCase):
    def test_seeded_draws_reproducible_and_latency_seed_is_consulted(self) -> None:
        declared = LatencyDistribution(
            "uniform", 1_000, 2_000_000, assumption="synthetic uniform; not calibrated"
        )
        model = ConservativeMbpModel(params(latency=declared))
        first = model.latency_sample(7, "o1", "order")
        self.assertEqual(first, model.latency_sample(7, "o1", "order"))
        self.assertNotEqual(first, model.latency_sample(8, "o1", "order"))
        self.assertNotEqual(first, model.latency_sample(7, "o2", "order"))
        self.assertLessEqual(1_000, first)
        self.assertLessEqual(first, 2_000_000)
        fixed = ConservativeMbpModel(params())
        self.assertEqual(fixed.latency_sample(7, "o1", "order"), 1)
        self.assertEqual(
            model.activation_time_ns(100, seed=7, context="o1"),
            100 + model.latency_sample(7, "o1", "order"),
        )

    def test_no_fill_before_sampled_activation_and_two_distributions_change_fills(self) -> None:
        events = (
            snapshot("b1", 1, asks=(("101", 100),)),
            order_add("o1", 2, quantity=5),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
        )
        fast = params(latency=LatencyDistribution(
            "uniform", 0, 1, assumption="synthetic short latency; not calibrated"
        ))
        slow = params(latency=LatencyDistribution(
            "uniform", 1_000_000_000, 2_000_000_000,
            assumption="synthetic long latency; not calibrated",
        ))
        short = run(events, model_params=fast)
        long = run(events, model_params=slow)
        self.assertEqual(len(short.fills), 1)
        self.assertEqual(len(long.fills), 0)
        kinds = [item.report_type for item in short.emitted]
        self.assertLess(kinds.index("ack"), kinds.index("latency_sample"))
        self.assertLess(kinds.index("latency_sample"), kinds.index("fill"))
        sample = next(item for item in short.emitted if item.report_type == "latency_sample")
        self.assertIsNotNone(sample.sampled_ns)
        self.assertIsNotNone(sample.effective_ns)
        self.assertGreaterEqual(short.fills[0].logical_time_ns, sample.effective_ns or 0)

    def test_fresh_replay_byte_identical_with_sampled_latency_across_hash_seeds(self) -> None:
        declared = LatencyDistribution(
            "uniform", 0, 10, assumption="synthetic seeded latency; not calibrated"
        )
        model = ConservativeMbpModel(params(latency=declared))
        events = (
            snapshot("b1", 1, asks=(("101", 100),)),
            order_add("o1", 2, quantity=5),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
        )
        seeds = ReplaySeeds(7, 11, 13, 17)
        result = SimulationEngine(engine_config()).run(events, models=model, seeds=seeds)
        record = build_record(
            result,
            engine_config=engine_config(),
            model_config=model.params.to_dict(),
            seeds=seeds,
        )
        original = json.dumps(record_to_dict(record), sort_keys=True).encode()
        for hash_seed in ("1", "2", "12345", "random"):
            fresh = replay_in_fresh_process(record, python_hash_seed=hash_seed)
            self.assertEqual(
                json.dumps(record_to_dict(fresh), sort_keys=True).encode(), original
            )
            comparison = compare_records(record, fresh)
            self.assertTrue(comparison.verified)
            self.assertEqual(comparison.pnl_basis, "recomputed")


class AuctionHaltTests(unittest.TestCase):
    def test_book_and_trade_fills_blocked_during_halt_then_resume(self) -> None:
        halted = (
            snapshot("b1", 1, bids=(("100", 4),), asks=(("101", 100),)),
            order_add("o1", 2, quantity=5),
            status("s1", 3, "halt"),
            snapshot("b2", 4, bids=(("100", 4),), asks=(("101", 110),), seq=2),
            trade("t1", 5, "100", 10, seq=3),
        )
        self.assertEqual(len(run(halted).fills), 0)
        resumed = (
            *halted,
            status("s2", 6, "resume", seq=8),
            snapshot("b3", 7, bids=(("100", 4),), asks=(("101", 120),), seq=9),
        )
        result = run(resumed)
        self.assertEqual(len(result.fills), 1)
        self.assertEqual(result.fills[0].logical_time_ns, 7)

    def test_orders_submitted_during_halt_are_rejected(self) -> None:
        result = run((status("s1", 1, "halt"), order_add("o1", 2)))
        rejects = [item for item in result.emitted if item.report_type == "reject"]
        self.assertEqual(len(rejects), 1)
        self.assertIn("halt", rejects[0].reason or "")
        self.assertNotIn("o1", result.final_state.orders)

    def test_synthetic_uncross_fills_once_at_declared_reference_price(self) -> None:
        events = (
            snapshot("b1", 1),
            order_add("o1", 2, price="105", quantity=6),
            auction("a1", 3, "102"),
            auction("a2", 3, "102", seq=8),
        )
        result = run(events)
        fills = [item for item in result.fills if item.reason and "uncross" in item.reason]
        self.assertEqual(len(fills), 1)
        self.assertEqual(fills[0].price, Decimal("102"))
        self.assertEqual(fills[0].liquidity, "taker")


class TickRoundingTests(unittest.TestCase):
    def test_effective_dated_tick_change_rounds_a_fill_price(self) -> None:
        change = InstrumentChange(
            **envelope("i1", 2, seq=1),
            field="tick_size",
            new_value="0.05",
            old_value="0.01",
            effective_ns=2,
        )
        events = (
            snapshot("b1", 1, asks=(("100.03", 100),)),
            change,
            order_add("o1", 3, price="101", quantity=5, seq=2),
            snapshot("b2", 4, asks=(("100.13", 110),), seq=3),
        )
        result = run(events)
        self.assertEqual(len(result.fills), 1)
        self.assertEqual(result.fills[0].price, Decimal("100.15"))
        self.assertEqual(result.fills[0].price % Decimal("0.05"), Decimal("0"))
        unrounded = run(tuple(item for item in events if item is not change))
        self.assertNotEqual(result.fills[0].price, unrounded.fills[0].price)

    def test_tick_change_effective_after_fill_does_not_round_it(self) -> None:
        change = InstrumentChange(
            **envelope("i1", 5, seq=1),
            field="tick_size",
            new_value="0.05",
            effective_ns=5,
        )
        events = (
            snapshot("b1", 1, asks=(("101", 100),)),
            order_add("o1", 2, quantity=5, seq=2),
            snapshot("b2", 3, asks=(("101", 110),), seq=3),
            change,
        )
        result = run(events)
        self.assertEqual(result.fills[0].price, Decimal("101"))


class ChargeMechanicsTests(unittest.TestCase):
    def test_short_with_and_without_borrow(self) -> None:
        events = (
            snapshot("b1", 1, bids=(("90", 100),)),
            order_add("o1", 2, side="sell", price="90", quantity=5),
            trade("t1", 3, "90", 10, aggressor="buy", seq=3),
        )
        with_borrow = BorrowTerms(
            INSTRUMENT, True, 10, Decimal("10000"),
            assumption="synthetic borrow rate; not calibrated",
        )
        allowed = run(events, model_params=params(cost=base_cost(borrow=(with_borrow,))))
        denied = run(events, model_params=params(cost=base_cost()))
        self.assertEqual(len(allowed.fills), 1)
        self.assertEqual(allowed.positions[INSTRUMENT], -5)
        self.assertEqual(len(denied.fills), 0)
        rejects = [item for item in denied.emitted if item.report_type == "reject"]
        self.assertEqual(len(rejects), 1)

    def test_borrow_cost_accrues_over_two_logical_times(self) -> None:
        terms = BorrowTerms(
            INSTRUMENT, True, 10, Decimal("10000"),
            assumption="synthetic borrow rate; not calibrated",
        )
        half = DAY_NS // 2
        events = (
            snapshot("b1", 1, bids=(("90", 100),)),
            order_add("o1", 2, side="sell", price="90", quantity=5),
            trade("t1", 3, "90", 10, aggressor="buy", seq=3),
            snapshot("b2", 3 + half, seq=4),
            snapshot("b3", 3 + DAY_NS, seq=5),
        )
        result = run(events, model_params=params(cost=base_cost(borrow=(terms,))))
        self.assertEqual(result.borrow_cost, Decimal("450"))
        charges = [item for item in result.charges if item.kind == "borrow"]
        self.assertEqual(len(charges), 2)
        self.assertEqual([item.amount for item in charges], [Decimal("225")] * 2)
        self.assertEqual([item.held_ns for item in charges], [half, half])

    def test_margin_breach_rejects_a_fill(self) -> None:
        tight = MarginTerms(
            Decimal("3000000"), Decimal("3000000"),
            assumption="synthetic margin floor; not calibrated",
        )
        events = (
            snapshot("b1", 1, asks=(("101", 100),)),
            order_add("o1", 2, quantity=5),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
        )
        self.assertEqual(len(run(events).fills), 1)
        blocked = run(events, model_params=params(cost=base_cost(margin=tight)))
        self.assertEqual(len(blocked.fills), 0)

    def test_funding_rate_changes_across_effective_boundary(self) -> None:
        boundary = 200
        early = FundingTerms(
            1, Decimal("10"), day_ns=1000,
            assumption="synthetic early funding; not calibrated",
        )
        late = FundingTerms(
            boundary, Decimal("20"), day_ns=1000,
            assumption="synthetic late funding; not calibrated",
        )
        events = (
            snapshot("b1", 1, asks=(("101", 100),)),
            order_add("o1", 2, quantity=5),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
            snapshot("b3", 100, seq=3),
            snapshot("b4", 300, seq=4),
        )
        result = run(events, model_params=params(cost=base_cost(funding=(early, late))))
        charges = [item for item in result.charges if item.kind == "funding"]
        self.assertEqual(len(charges), 2)
        self.assertEqual(
            [item.rate_bps_per_day for item in charges], [Decimal("10"), Decimal("20")]
        )
        self.assertNotEqual(charges[0].amount, charges[1].amount)
        self.assertEqual(result.funding_cost, sum((item.amount for item in charges), Decimal("0")))


class CalibrationArtifactTests(unittest.TestCase):
    def test_missing_or_mismatched_artifact_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            resolver = ArtifactResolver(Path(directory))
            with self.assertRaises(ArtifactError):
                resolver.resolve(SHA)
            (Path(directory) / f"{SHA}.json").write_text(
                json.dumps({"schema_version": 1, "artifact_sha256": SHA,
                            "dated_on": "2026-11-21", "payload": {"kind": "other"}}),
                encoding="utf-8",
            )
            with self.assertRaises(ArtifactError):
                resolver.resolve(SHA)

    def test_assumption_only_summary_never_claims_calibrated(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            statement = calibration_statement(ArtifactResolver(Path(directory)), base_cost())
            self.assertIn("not calibrated", statement)
            self.assertNotIn("calibrated; dated", statement)
            artifacts, assumptions = cost_provenance(base_cost())
            self.assertEqual(artifacts, ())
            self.assertTrue(assumptions)

    def test_dated_schema_probe_fixture_does_not_grant_calibrated(self) -> None:
        payload = {"kind": "schema_probe", "note": "synthetic; not a calibration result"}
        artifact_sha = digest(payload)
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / f"{artifact_sha}.json").write_text(
                json.dumps({
                    "schema_version": 1,
                    "artifact_sha256": artifact_sha,
                    "dated_on": "2026-11-21",
                    "payload": payload,
                }),
                encoding="utf-8",
            )
            cost = base_cost(impact_artifact=artifact_sha, impact_assumption=None)
            resolver = ArtifactResolver(Path(directory))
            self.assertEqual(resolver.resolve(artifact_sha).dated_on, "2026-11-21")
            self.assertIn("not calibrated", calibration_statement(resolver, cost))


class TrialStoreTests(unittest.TestCase):
    def test_duplicate_key_refused_and_differing_payload_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = TrialStore(Path(directory))
            payload = {"trial": "synthetic", "value": 1}
            key = digest(payload)
            path = store.put(key, payload)
            self.assertTrue(path.is_file())
            self.assertEqual(store.get(key), payload)
            self.assertEqual(store.keys(), (key,))
            with self.assertRaises(DuplicateTrial):
                store.put(key, payload)
            with self.assertRaises(TrialConflict):
                store.put(key, {"trial": "synthetic", "value": 2})
            path.write_text("{}\n", encoding="utf-8")
            with self.assertRaises(TrialConflict):
                store.get(key)


class VerifierCashTests(unittest.TestCase):
    def _record(self) -> Any:
        events = (
            snapshot("b1", 1, asks=(("101", 100),)),
            order_add("o1", 2, quantity=5),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
        )
        model = ConservativeMbpModel(params())
        seeds = ReplaySeeds(1, 2, 3, 4)
        result = SimulationEngine(engine_config()).run(events, models=model, seeds=seeds)
        return build_record(
            result,
            engine_config=engine_config(),
            model_config=model.params.to_dict(),
            seeds=seeds,
        )

    def test_ending_cash_recomputed_from_fills_detects_tampered_cash(self) -> None:
        record = self._record()
        recomputed, basis, mismatches = verifier_recompute(record)
        self.assertEqual(recomputed["ending_cash"], record.ending_cash)
        self.assertEqual(basis, "recomputed")
        self.assertEqual(mismatches, ())
        tampered = replace(record, ending_cash="0", record_sha256="0" * 64)
        comparison = compare_records(record, tampered)
        self.assertIn("ending_cash", comparison.excluded_fields)
        self.assertEqual(comparison.pnl_basis, "excluded")

    def test_charged_record_recomputes_ending_cash_from_cash_rows(self) -> None:
        terms = BorrowTerms(
            INSTRUMENT, True, 10, Decimal("10000"),
            assumption="synthetic borrow rate; not calibrated",
        )
        events = (
            snapshot("b1", 1, bids=(("90", 100),)),
            order_add("o1", 2, side="sell", price="90", quantity=5),
            trade("t1", 3, "90", 10, aggressor="buy", seq=3),
            snapshot("b2", 3 + DAY_NS, seq=4),
        )
        model = ConservativeMbpModel(params(cost=base_cost(borrow=(terms,))))
        seeds = ReplaySeeds(1, 2, 3, 4)
        result = SimulationEngine(engine_config()).run(events, models=model, seeds=seeds)
        record = build_record(
            result,
            engine_config=engine_config(),
            model_config=model.params.to_dict(),
            seeds=seeds,
        )
        self.assertTrue(any(row["kind"] == "charge" for row in record.cash_rows))
        recomputed, basis, mismatches = verifier_recompute(record)
        self.assertEqual(recomputed["ending_cash"], record.ending_cash)
        self.assertEqual(basis, "recomputed")
        self.assertEqual(mismatches, ())

    def test_tampered_fill_price_is_detected_by_recomputation(self) -> None:
        record = self._record()
        row = dict(record.fill_rows[0])
        row["price"] = "999"
        tampered = replace(
            record, fill_rows=(row,), record_sha256="0" * 64,
        )
        _, basis, mismatches = verifier_recompute(tampered)
        self.assertEqual(basis, "excluded")
        self.assertIn("ending_cash", mismatches)


class SimReplayCliTests(unittest.TestCase):
    def test_subprocess_replay_byte_identical_across_seeds(self) -> None:
        events = (
            snapshot("b1", 1, asks=(("101", 100),)),
            order_add("o1", 2, quantity=5),
            snapshot("b2", 3, asks=(("101", 110),), seq=2),
        )
        model = ConservativeMbpModel(params())
        seeds = ReplaySeeds(7, 11, 13, 17)
        result = SimulationEngine(engine_config()).run(events, models=model, seeds=seeds)
        record = build_record(
            result,
            engine_config=engine_config(),
            model_config=model.params.to_dict(),
            seeds=seeds,
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = write_record(record, root / "records")
            outputs: list[bytes] = []
            for outer_seed in ("1", "2", "random"):
                env = dict(os.environ, PYTHONPATH=str(SRC), PYTHONHASHSEED=outer_seed)
                completed = subprocess.run(
                    [sys.executable, "-m", "quantlab.cli", "sim-replay",
                     "--record", str(path), "--compare", "--hash-seed", "7"],
                    env=env, capture_output=True, check=False,
                )
                self.assertEqual(completed.returncode, 0, completed.stderr.decode())
                outputs.append(completed.stdout)
        self.assertEqual(outputs[0], outputs[1])
        self.assertEqual(outputs[1], outputs[2])
        payload = json.loads(outputs[0])
        comparison = payload["comparison"]
        self.assertTrue(all(comparison["verified_equal"].values()))
        self.assertEqual(comparison["pnl_basis"], "recomputed")
        self.assertEqual(payload["hash_seed"], "7")
        self.assertIn("PRECONDITION", payload["replay_run_gate"])


if __name__ == "__main__":
    unittest.main()
