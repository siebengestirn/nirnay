"""Content-addressed run records and fresh-process replay comparison.

The producer never writes a ``pnl_basis`` claim.  The verifier derives it from a
committed fill stream and marks, and excludes P&L unless it can recompute every
cost and the resulting P&L from that committed evidence.
"""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path
from typing import Any, Mapping, Sequence

from .core import EngineConfig, ReplaySeeds, SimulationEngine, SimResult
from .events import digest, event_from_dict, event_to_dict, to_jsonable
from .execution.base import BaselineParams, ConservativeMbpModel

VERIFIED_EQUAL_FIELDS: tuple[str, ...] = (
    "input_digest",
    "config_hash",
    "model_digests",
    "seeds_digest",
    "initial_state_hash",
    "final_state_hash",
    "emitted_digest",
    "fills_digest",
    "unmarked_residual",
)
P_AND_L_FIELDS: tuple[str, ...] = (
    "starting_cash",
    "ending_cash",
    "net_pnl",
    "gross_pnl",
    "fees",
    "impact_cost",
    "spread_cost",
)


@dataclass(frozen=True, slots=True)
class RunRecord:
    """Immutable, self-contained input and output evidence for one run."""

    schema_version: int
    run_id: str
    record_sha256: str
    input_events: tuple[dict[str, Any], ...]
    engine_config: dict[str, Any]
    model_config: dict[str, Any]
    seeds: dict[str, int]
    input_digest: str
    config_hash: str
    model_digests: tuple[str, ...]
    seeds_digest: str
    initial_state_hash: str
    final_state_hash: str
    emitted_digest: str
    fills_digest: str
    fill_rows: tuple[dict[str, Any], ...]
    marks: dict[str, str]
    starting_cash: str
    ending_cash: str
    positions: tuple[tuple[str, int], ...]
    net_pnl: str
    gross_pnl: str
    fees: str
    impact_cost: str
    spread_cost: str
    unmarked_residual: bool
    strategy_id: str


def _row(fill: Any) -> dict[str, Any]:
    return {
        "event_id": fill.event_id,
        "client_order_id": fill.client_order_id,
        "quantity": fill.quantity,
        "price": format(fill.price, "f"),
        "fees": format(fill.fees, "f"),
        "impact_cost": format(fill.impact_cost, "f"),
        "spread_cost": format(fill.spread_cost, "f"),
        "liquidity": fill.liquidity,
        "logical_time_ns": fill.logical_time_ns,
    }


def build_record(
    result: SimResult,
    *,
    engine_config: EngineConfig,
    model_config: dict[str, Any],
    seeds: ReplaySeeds,
    input_events: Sequence[dict[str, Any]] | None = None,
    run_id: str = "sim.run",
) -> RunRecord:
    """Build a content-addressed record from one engine result."""
    inputs = (
        tuple(event_to_dict(event) for event in result.consumed_events)
        if input_events is None
        else tuple(input_events)
    )
    fill_rows = tuple(_row(fill) for fill in result.fills)
    values = {
        "schema_version": 1,
        "run_id": run_id,
        "input_events": inputs,
        "engine_config": engine_config.to_dict(),
        "model_config": model_config,
        "seeds": seeds.to_dict(),
        "input_digest": digest(inputs),
        "config_hash": result.config_hash,
        "model_digests": result.model_digests,
        "seeds_digest": result.seeds_digest,
        "initial_state_hash": state_hash_from_config(engine_config, result),
        "final_state_hash": result.state_hash,
        "emitted_digest": digest([event_to_dict(item) for item in result.emitted]),
        "fills_digest": digest(fill_rows),
        "fill_rows": fill_rows,
        "marks": dict(sorted((k, format(v, "f")) for k, v in result.marks.items())),
        "starting_cash": format(result.final_state.starting_cash, "f"),
        "ending_cash": format(result.ending_cash, "f"),
        "positions": tuple(sorted(result.positions.items())),
        "net_pnl": format(result.net_pnl, "f"),
        "gross_pnl": format(result.gross_pnl, "f"),
        "fees": format(result.fees, "f"),
        "impact_cost": format(result.impact_cost, "f"),
        "spread_cost": format(result.spread_cost, "f"),
        "unmarked_residual": result.unmarked_residual,
        "strategy_id": result.strategy_id,
    }
    payload = to_jsonable(values)
    record_hash = hashlib.sha256(
        json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    ).hexdigest()
    values["record_sha256"] = record_hash
    return RunRecord(**values)


def state_hash_from_config(config: EngineConfig, result: SimResult) -> str:
    """Recompute the starting state hash from the config and model params."""
    from .state import empty_state, state_hash

    return state_hash(
        empty_state(config.starting_cash, result.final_state.params, config.risk_budgets)
    )


def record_to_dict(record: RunRecord) -> dict[str, Any]:
    """Serialize a run record deterministically."""
    return to_jsonable(record)


def record_from_dict(data: Mapping[str, Any]) -> RunRecord:
    """Strictly reconstruct a run record and verify its content address."""
    if not isinstance(data, Mapping):
        raise ValueError("run record must be an object")
    expected = {field for field in RunRecord.__dataclass_fields__}
    if set(data) != expected:
        raise ValueError(
            f"run record fields differ: missing={sorted(expected - set(data))}, "
            f"unknown={sorted(set(data) - expected)}"
        )
    payload = to_jsonable({key: value for key, value in data.items() if key != "record_sha256"})
    claimed = data["record_sha256"]
    computed = hashlib.sha256(
        json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    ).hexdigest()
    if claimed != computed:
        raise ValueError("run record content address does not match its payload")
    if data["schema_version"] != 1:
        raise ValueError("unsupported run record schema version")
    restored = dict(data)
    for name in ("input_events", "model_digests", "fill_rows"):
        restored[name] = tuple(restored[name])
    restored["positions"] = tuple(tuple(row) for row in restored["positions"])
    return RunRecord(**restored)


def write_record(record: RunRecord, directory: Path, *, overwrite: bool = False) -> Path:
    """Write a no-overwrite, content-addressed record and return its path."""
    if overwrite:
        raise ValueError("replay records are immutable; overwrite is not permitted")
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / f"{record.record_sha256}.json"
    payload = json.dumps(
        record_to_dict(record),
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    )
    if path.exists():
        if path.read_text(encoding="utf-8") != payload:
            raise ValueError("content address collision: record differs from existing file")
        return path
    path.write_text(payload, encoding="utf-8")
    return path


def read_record(path: Path) -> RunRecord:
    """Read and validate a content-addressed run record."""
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    record = record_from_dict(data)
    if path.name != f"{record.record_sha256}.json":
        raise ValueError("run record filename does not match its content address")
    return record


@dataclass(frozen=True, slots=True)
class ComparisonResult:
    """Verifier-derived comparison outcome; P&L basis is never producer-supplied."""

    verified_equal: dict[str, bool]
    excluded_fields: tuple[str, ...]
    pnl_basis: str
    verified: bool


def verifier_recompute(record: RunRecord) -> tuple[dict[str, str], str, tuple[str, ...]]:
    """Recompute P&L and costs from the committed fill stream and marks."""
    totals = {
        "fees": Decimal("0"),
        "impact_cost": Decimal("0"),
        "spread_cost": Decimal("0"),
    }
    for row in record.fill_rows:
        for name in totals:
            totals[name] += Decimal(row[name])
    marked_value = Decimal("0")
    residual: list[str] = []
    for instrument, quantity in record.positions:
        if quantity == 0:
            continue
        if instrument in record.marks:
            marked_value += Decimal(quantity) * Decimal(record.marks[instrument])
        else:
            residual.append(instrument)
    net = (
        Decimal(record.ending_cash)
        + marked_value
        - Decimal(record.starting_cash)
    )
    gross = net + totals["fees"] + totals["impact_cost"] + totals["spread_cost"]
    recomputed = {
        "starting_cash": format(Decimal(record.starting_cash), "f"),
        "ending_cash": format(Decimal(record.ending_cash), "f"),
        "net_pnl": format(net, "f"),
        "gross_pnl": format(gross, "f"),
        "fees": format(totals["fees"], "f"),
        "impact_cost": format(totals["impact_cost"], "f"),
        "spread_cost": format(totals["spread_cost"], "f"),
    }
    mismatches = tuple(
        name for name in P_AND_L_FIELDS if getattr(record, name) != recomputed[name]
    )
    if residual and not record.unmarked_residual:
        mismatches += ("unmarked_residual",)
    basis = "recomputed" if not mismatches else "excluded"
    return recomputed, basis, mismatches


def compare_records(left: RunRecord, right: RunRecord) -> ComparisonResult:
    """Compare only verifier-derived fields; exclude unproven P&L."""
    equal = {
        name: getattr(left, name) == getattr(right, name)
        for name in VERIFIED_EQUAL_FIELDS
    }
    excluded: list[str] = []
    bases: list[str] = []
    for record in (left, right):
        recomputed, basis, mismatches = verifier_recompute(record)
        bases.append(basis)
        for name in P_AND_L_FIELDS:
            if name in mismatches:
                equal[name] = False
                excluded.append(name)
                continue
            equal[name] = (
                getattr(record, name) == recomputed[name]
                and getattr(left, name) == getattr(right, name)
            )
    basis = "recomputed" if bases == ["recomputed", "recomputed"] else "excluded"
    if basis == "excluded":
        for name in P_AND_L_FIELDS:
            excluded.append(name)
            equal[name] = False
    return ComparisonResult(
        verified_equal=equal,
        excluded_fields=tuple(sorted(set(excluded))),
        pnl_basis=basis,
        verified=all(equal.values()),
    )


def _source_dir() -> Path:
    return Path(__file__).resolve().parents[2]


def _fresh_entry(record_path: str, out_dir: str) -> None:
    """Re-execute a recorded run in this (fresh) interpreter."""
    record = read_record(Path(record_path))
    events = tuple(event_from_dict(item) for item in record.input_events)
    config = EngineConfig.from_dict(record.engine_config)
    params = BaselineParams.from_dict(record.model_config)
    model = ConservativeMbpModel(params)
    seeds = ReplaySeeds.from_dict(record.seeds)
    result = SimulationEngine(config).run(events, strategy=None, models=model, seeds=seeds)
    rebuilt = build_record(
        result,
        engine_config=config,
        model_config=record.model_config,
        seeds=seeds,
        input_events=record.input_events,
    )
    write_record(rebuilt, Path(out_dir))


def replay_in_fresh_process(
    record: RunRecord,
    *,
    python_hash_seed: str = "1",
    directory: Path | None = None,
) -> RunRecord:
    """Replay a record in a fresh process with a controlled hash seed."""
    code = "\n".join(
        [
            "import sys",
            "from quantlab.sim.replay import _fresh_entry",
            "_fresh_entry(sys.argv[1], sys.argv[2])",
        ]
    )
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(directory) if directory is not None else Path(temporary)
        root.mkdir(parents=True, exist_ok=True)
        source = root / f"{record.record_sha256}.json"
        source.write_text(
            json.dumps(record_to_dict(record), sort_keys=True, separators=(",", ":")),
            encoding="utf-8",
        )
        out = root / "out"
        out.mkdir(exist_ok=True)
        env = os.environ.copy()
        env["PYTHONHASHSEED"] = python_hash_seed
        source_dir = str(_source_dir())
        env["PYTHONPATH"] = source_dir if not env.get("PYTHONPATH") else (
            source_dir + os.pathsep + env["PYTHONPATH"]
        )
        completed = subprocess.run(
            [sys.executable, "-c", code, str(source), str(out)],
            capture_output=True,
            check=False,
            env=env,
            text=True,
        )
        if completed.returncode != 0:
            raise RuntimeError(
                f"fresh-process replay failed: {completed.stderr.strip()}"
            )
        written = sorted(out.glob("*.json"))
        if len(written) != 1:
            raise RuntimeError("fresh-process replay did not emit exactly one record")
        return read_record(written[0])
