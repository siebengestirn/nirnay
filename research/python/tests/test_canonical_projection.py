"""Canonical projection: exact reference binding, versions and fresh-process replay."""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

from quantlab.cli import main
from quantlab.market import canonical_bytes

SRC = Path(__file__).resolve().parents[1] / "src"


def _dump(path: Path, rows: list[dict]) -> None:
    path.write_text("".join(json.dumps(row, sort_keys=True) + "\n" for row in rows),
                    encoding="utf-8")


def _upstox_input(path: Path) -> None:
    payload = {
        "currentTs": "1794372000000",
        "feeds": {"NSE_EQ|X": {"fullFeed": {"marketFF": {
            "ltpc": {"ltp": "100", "ltt": "1794371999000", "ltq": "2"},
            "marketLevel": {"bidAskQuote": [{"bidP": "99.9", "bidQ": "10",
                                              "askP": "100.1", "askQ": "11"}]},
        }}}},
    }
    _dump(path, [{"schema_version": 1, "provider": "upstox", "capture_sequence": 1,
                  "payload_sha256": "a" * 64, "received_at_utc_ns": 1794372000000000000,
                  "message": payload}])


def _reference_revision(path: Path, instruments: dict[str, str]) -> str:
    body = {"schema_version": 1, "instruments": instruments, "venues": {}}
    digest = hashlib.sha256(canonical_bytes(body)).hexdigest()
    body["revision_sha256"] = digest
    path.write_text(json.dumps(body, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return digest


def _args(tmp_path: Path, reference: Path) -> list[str]:
    return ["normalize", "--provider", "upstox", "--input", str(tmp_path / "decoded.jsonl"),
            "--instrument-map", str(tmp_path / "instrument-map.json"),
            "--reference-revision", str(reference)]


def test_projection_binds_reference_revision_and_records_versions(tmp_path):
    _upstox_input(tmp_path / "decoded.jsonl")
    (tmp_path / "instrument-map.json").write_text(
        json.dumps({"NSE_EQ|X": "in.x"}), encoding="utf-8")
    expected = _reference_revision(tmp_path / "reference.json", {"NSE_EQ|X": "in.x"})
    output = tmp_path / "canonical.jsonl"
    assert main(_args(tmp_path, tmp_path / "reference.json") + ["--output", str(output)]) == 0
    events = [json.loads(line) for line in output.read_text().splitlines()]
    assert len(events) == 1
    assert events[0]["instrument_id"] == "in.x"
    assert events[0]["source_frame_sha256"] == "a" * 64
    report = json.loads(output.with_suffix(".jsonl.quality.json").read_text())
    assert report["reference_revision_sha256"] == expected
    assert report["normalizer_version"] == "upstox-v3-v1"
    assert report["event_schema_version"] == 1


def test_unknown_instrument_stays_unresolved(tmp_path):
    _upstox_input(tmp_path / "decoded.jsonl")
    (tmp_path / "instrument-map.json").write_text("{}", encoding="utf-8")
    _reference_revision(tmp_path / "reference.json", {})
    output = tmp_path / "canonical.jsonl"
    assert main(_args(tmp_path, tmp_path / "reference.json") + ["--output", str(output)]) == 0
    event = json.loads(output.read_text().splitlines()[0])
    assert event["instrument_id"] is None
    assert event["provider_instrument_id"] == "NSE_EQ|X"


def test_reference_drift_fails_closed(tmp_path):
    _upstox_input(tmp_path / "decoded.jsonl")
    (tmp_path / "instrument-map.json").write_text(
        json.dumps({"NSE_EQ|X": "in.x"}), encoding="utf-8")
    reference = tmp_path / "reference.json"
    _reference_revision(reference, {"NSE_EQ|X": "other.id"})
    assert main(_args(tmp_path, reference) + ["--output", str(tmp_path / "out.jsonl")]) == 2
    assert not (tmp_path / "out.jsonl").exists()


def test_fresh_process_replay_is_byte_identical(tmp_path):
    _upstox_input(tmp_path / "decoded.jsonl")
    (tmp_path / "instrument-map.json").write_text(
        json.dumps({"NSE_EQ|X": "in.x"}), encoding="utf-8")
    reference = tmp_path / "reference.json"
    _reference_revision(reference, {"NSE_EQ|X": "in.x"})
    outputs = []
    sidecars = []
    for run in ("first", "second"):
        output = tmp_path / f"{run}.jsonl"
        env = dict(os.environ, PYTHONPATH=str(SRC))
        result = subprocess.run(
            [sys.executable, "-m", "quantlab.cli"]
            + _args(tmp_path, reference) + ["--output", str(output)],
            env=env, capture_output=True, text=True, check=False)
        assert result.returncode == 0, result.stderr
        outputs.append(output.read_bytes())
        sidecars.append(output.with_suffix(".jsonl.quality.json").read_bytes())
    assert outputs[0] == outputs[1], "replay in a fresh process is byte-identical"
    assert sidecars[0] == sidecars[1], "quality reports are byte-identical too"
