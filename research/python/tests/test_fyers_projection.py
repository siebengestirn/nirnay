"""FYERS canonical projection over the real Python hash-chain journal."""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

from quantlab.cli import main
from quantlab.market import canonical_bytes

ROOT = Path(__file__).resolve().parents[3]
JOURNAL = (ROOT / "apps" / "fyers-importer" / "tests" / "fixtures"
           / "session" / "fyers-sdk-observations.jsonl")
SRC = Path(__file__).resolve().parents[1] / "src"


def _reference_revision(path: Path, instruments: dict[str, str]) -> str:
    body = {"schema_version": 1, "instruments": instruments, "venues": {}}
    digest = hashlib.sha256(canonical_bytes(body)).hexdigest()
    body["revision_sha256"] = digest
    path.write_text(json.dumps(body, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return digest


def _prepare(tmp_path: Path, instruments: dict[str, str]) -> tuple[Path, str]:
    (tmp_path / "instrument-map.json").write_text(
        json.dumps(instruments), encoding="utf-8")
    reference = tmp_path / "reference.json"
    digest = _reference_revision(reference, instruments)
    return reference, digest


def _args(tmp_path: Path, reference: Path, output: Path) -> list[str]:
    return ["normalize", "--provider", "fyers", "--input", str(JOURNAL),
            "--instrument-map", str(tmp_path / "instrument-map.json"),
            "--reference-revision", str(reference), "--allow-gaps",
            "--output", str(output)]


def test_fyers_projection_resolves_instruments_and_records_versions(tmp_path):
    reference, digest = _prepare(tmp_path, {"SBIN-EQ": "in.sbin"})
    output = tmp_path / "events.jsonl"
    assert main(_args(tmp_path, reference, output)) == 0
    events = [json.loads(line) for line in output.read_text().splitlines()]
    assert len(events) == 2, "two market observations are projected"
    assert events[0]["instrument_id"] == "in.sbin"
    assert events[0]["provider_instrument_id"] == "SBIN-EQ"
    assert events[1]["instrument_id"] is None, "an unknown instrument stays unresolved"
    first_record = json.loads(JOURNAL.read_text().splitlines()[1])
    assert events[0]["source_frame_sha256"] == first_record["record_hash"]
    report = json.loads(output.with_suffix(".jsonl.quality.json").read_text())
    assert report["reference_revision_sha256"] == digest
    assert report["normalizer_version"] == "fyers-v3-v1"
    assert report["event_schema_version"] == 1
    assert report["explicit_capture_gaps"] == 1
    assert report["source_hash_chain_verified"] is True
    assert report["research_grade"] is False


def test_fyers_gap_without_allow_flag_fails_closed(tmp_path):
    reference, _ = _prepare(tmp_path, {"SBIN-EQ": "in.sbin"})
    output = tmp_path / "events.jsonl"
    args = _args(tmp_path, reference, output)
    args.remove("--allow-gaps")
    assert main(args) == 2
    assert not output.exists()


def test_fyers_reference_drift_fails_closed(tmp_path):
    # The CLI map differs from the map bound inside the revision file.
    (tmp_path / "instrument-map.json").write_text(
        json.dumps({"SBIN-EQ": "in.sbin"}), encoding="utf-8")
    reference = tmp_path / "reference.json"
    _reference_revision(reference, {"SBIN-EQ": "other.id"})
    output = tmp_path / "events.jsonl"
    assert main(_args(tmp_path, reference, output)) == 2
    assert not output.exists()


def test_fyers_fresh_process_replay_is_byte_identical(tmp_path):
    reference, _ = _prepare(tmp_path, {"SBIN-EQ": "in.sbin"})
    outputs = []
    reports = []
    for name in ("first", "second"):
        output = tmp_path / f"{name}.jsonl"
        env = dict(os.environ, PYTHONPATH=str(SRC))
        result = subprocess.run(
            [sys.executable, "-m", "quantlab.cli"]
            + _args(tmp_path, reference, output),
            env=env, capture_output=True, text=True, check=False)
        assert result.returncode == 0, result.stderr
        outputs.append(output.read_bytes())
        reports.append(output.with_suffix(".jsonl.quality.json").read_bytes())
    assert outputs[0] == outputs[1]
    assert reports[0] == reports[1]
