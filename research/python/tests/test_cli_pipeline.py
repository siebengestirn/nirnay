from __future__ import annotations

import json
from pathlib import Path

from quantlab.cli import main


def _dump(path: Path, rows):
    path.write_text("".join(json.dumps(row, sort_keys=True) + "\n" for row in rows), encoding="utf-8")


def test_upstox_normalize_cli_links_canonical_events_to_raw_frames(tmp_path):
    raw = tmp_path / "decoded.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    mapping.write_text(json.dumps({"NSE_EQ|X": "in.x"}), encoding="utf-8")
    payload = {
        "currentTs": "1794372000000",
        "feeds": {"NSE_EQ|X": {"fullFeed": {"marketFF": {
            "ltpc": {"ltp": "100", "ltt": "1794371999000", "ltq": "2"},
            "marketLevel": {"bidAskQuote": [{"bidP": "99.9", "bidQ": "10", "askP": "100.1", "askQ": "11"}]},
            "vtt": "1000",
        }}}},
    }
    _dump(raw, [{"received_at_utc_ns": 1794372000000000000,
                 "received_monotonic_ns": 12345, "capture_sequence": 1,
                 "payload_sha256": "a" * 64, "message": payload}])
    assert main(["normalize", "--provider", "upstox", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 0
    events = [json.loads(line) for line in output.read_text().splitlines()]
    assert len(events) == 1  # First snapshot seeds the last-trade observation baseline.
    assert events[0]["instrument_id"] == "in.x"
    assert events[0]["source_frame_sha256"] == "a" * 64
    report = json.loads(output.with_suffix(".jsonl.quality.json").read_text())
    assert report["output_sha256"]


def test_fyers_normalize_refuses_journal_gap_by_default(tmp_path):
    raw = tmp_path / "fyers.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    mapping.write_text(json.dumps({"NSE:SBIN-EQ": "in.sbin"}), encoding="utf-8")
    _dump(raw, [{"event_type": "gap", "received_utc_ns": 10,
                 "payload": {"first_missing_local_sequence": 3, "last_missing_local_sequence": 4,
                             "dropped_callback_count": 2}}])
    assert main(["normalize", "--provider", "fyers", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 2
    assert not output.exists()


def test_reconcile_cli_is_deterministic_and_serializes_decimal_results(tmp_path):
    events = tmp_path / "events.jsonl"
    output = tmp_path / "result.json"
    common = {"schema_version": 1, "instrument_id": "in.sbin", "venue": "NSE",
        "event_kind": "quote", "received_utc_ns": 100, "received_monotonic_ns": None,
        "local_sequence": 1, "source_timestamp_ns": None, "source_timestamp_semantics": None,
        "last_trade_time_ns": 90, "last_trade_time_semantics": "last_trade",
        "last_price": "100", "last_quantity": 1, "cumulative_volume": None,
        "aggressor_side": None, "bids": [{"price": "99.9", "quantity": 10, "order_count": None}],
        "asks": [{"price": "100.1", "quantity": 10, "order_count": None}],
        "quality_flags": [], "source_frame_sha256": None}
    rows = [dict(common, event_id="up", provider="upstox", provider_instrument_id="NSE_EQ|X", raw_sha256="a"*64),
            dict(common, event_id="fy", provider="fyers", provider_instrument_id="NSE:X-EQ", raw_sha256="b"*64)]
    _dump(events, rows)
    assert main(["reconcile", "--events", str(events), "--now-utc-ns", "101",
                 "--output", str(output)]) == 0
    report = json.loads(output.read_text())
    assert report["results"][0]["status"] == "consistent_within_tolerance"
    assert report["results"][0]["price_difference_bps"] == "0"


def test_normalize_cli_group_publication_refuses_existing_sidecar_without_partial_output(tmp_path):
    raw = tmp_path / "decoded.jsonl"
    mapping = tmp_path / "instrument-map.json"
    output = tmp_path / "canonical.jsonl"
    sidecar = output.with_suffix(".jsonl.quality.json")
    mapping.write_text(json.dumps({"NSE_EQ|X": "in.x"}), encoding="utf-8")
    payload = {"currentTs": "1794372000000", "feeds": {"NSE_EQ|X": {"fullFeed": {"marketFF": {
        "ltpc": {"ltp": "100", "ltt": "1794371999000", "ltq": "2"},
        "marketLevel": {"bidAskQuote": [{"bidP": "99.9", "bidQ": "10", "askP": "100.1", "askQ": "11"}]},
        "vtt": "1000"}}}}}
    _dump(raw, [{"received_at_utc_ns": 1794372000000000000, "capture_sequence": 1,
                 "payload_sha256": "a" * 64, "message": payload}])
    sidecar.write_text("reserved", encoding="utf-8")
    assert main(["normalize", "--provider", "upstox", "--input", str(raw),
                 "--instrument-map", str(mapping), "--output", str(output)]) == 2
    assert not output.exists()
    assert sidecar.read_text(encoding="utf-8") == "reserved"


def test_jsonl_reader_rejects_blank_lines_as_ambiguous_source_records(tmp_path):
    from quantlab.cli import _read_jsonl
    source = tmp_path / "source.jsonl"
    source.write_text('{"x":1}\n\n', encoding="utf-8")
    import pytest
    with pytest.raises(ValueError, match="blank lines"):
        _read_jsonl(source)
