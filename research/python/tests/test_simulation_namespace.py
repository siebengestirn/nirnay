"""Simulation namespace shims re-export canonical sim behavior without duplication."""
from __future__ import annotations

from dataclasses import replace
from decimal import Decimal

from quantlab.sim import (BaselineParams as SimBaselineParams,
                          ConservativeMbpModel as SimConservativeMbpModel,
                          EngineConfig, ReplaySeeds, SimulationEngine, default_cost_model)
from quantlab.sim import compare_records as sim_compare_records
from quantlab.sim import read_record as sim_read_record
from quantlab.sim.execution import BaselineParams as ExecBaselineParams
from quantlab.sim.execution import CostModel as ExecCostModel
from quantlab.sim.replay import build_record, record_to_dict, write_record
from quantlab.simulation.event_replay import (
    RunRecord,
    build_record as shim_build_record,
    compare_records as shim_compare_records,
    read_record as shim_read_record,
    write_record as shim_write_record,
)
from quantlab.simulation.execution import BaselineParams, CostModel, default_cost_model as shim_cost


def _record() -> RunRecord:
    config = EngineConfig(starting_cash=Decimal("100000"), order_latency_ns=1,
                          cancel_latency_ns=1, max_participation=Decimal("1"))
    params = BaselineParams(order_latency_ns=1, cancel_latency_ns=1,
                            max_participation=Decimal("1"),
                            max_book_age_ns=1_000_000_000,
                            cost_model=default_cost_model())
    seeds = ReplaySeeds(1, 2, 3, 4)
    model = SimConservativeMbpModel(params)
    result = SimulationEngine(config).run(tuple(), models=model, seeds=seeds)
    return build_record(result, engine_config=config, model_config=params.to_dict(),
                        seeds=seeds)


def test_namespace_symbols_are_identical_objects():
    assert shim_build_record is build_record
    assert shim_compare_records is sim_compare_records
    assert shim_read_record is sim_read_record
    assert shim_write_record is write_record
    assert BaselineParams is SimBaselineParams is ExecBaselineParams
    assert CostModel is ExecCostModel
    assert shim_cost is default_cost_model


def test_shim_write_and_read_record_round_trip(tmp_path):
    record = _record()
    path = shim_write_record(record, tmp_path)
    assert shim_read_record(path) == record


def test_shim_record_round_trip_and_tampered_fill_digest():
    record = _record()
    assert shim_build_record is build_record
    assert record_to_dict(record)["schema_version"] == 1
    comparison = shim_compare_records(record, record)
    assert comparison.verified is True
    assert comparison.pnl_basis == "recomputed"
    tampered = replace(record, fills_digest="1" * 64)
    failed = shim_compare_records(record, tampered)
    assert failed.verified is False
    assert failed.verified_equal["fills_digest"] is False
