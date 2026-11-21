from __future__ import annotations

import json
from pathlib import Path

import pytest

from fyers_recorder.journal import HashChainJournal, verify
from fyers_recorder.recorder import FyersRecorder, RecorderConfig
from fyers_recorder.recorder import FYERS_PRODUCT_KEY
from fyers_recorder.rights import ALL_OPERATIONS, RightsDenied, authorize

ROOT = Path(__file__).resolve().parents[3]
DENY_REGISTRY = ROOT / "configs" / "providers" / "source-products.v1.json"
PERMIT_REGISTRY = Path(__file__).parent / "fixtures" / "registry-permitted.test.json"


class FakeSocket:
    def __init__(self, **kwargs):
        self.callbacks = kwargs
        self.subscriptions: list[tuple[tuple[str, ...], str]] = []
        self.closed = False

    def subscribe(self, *, symbols, data_type):
        self.subscriptions.append((tuple(symbols), data_type))

    def keep_running(self):
        self.callbacks["on_connect"]()
        self.callbacks["on_message"]({"type": "sf", "symbol": "NSE:SBIN-EQ", "ltp": 812.5})
        self.callbacks["on_message"]({"type": "dp", "symbol": "NSE:SBIN-EQ", "bid": [[812.4, 10]]})

    def close_connection(self):
        self.closed = True


def config(tmp_path: Path, **changes) -> RecorderConfig:
    values = dict(
        symbols=("NSE:SBIN-EQ", "NSE:RELIANCE-EQ"),
        entitlement_ref="fyers-entitlement-2026-11",
        output_root=tmp_path,
        registry_path=PERMIT_REGISTRY,
        queue_capacity=64,
        include_depth=True,
    )
    values.update(changes)
    return RecorderConfig(**values)


def test_fyers_session_captures_sdk_callbacks_and_closes_manifest(tmp_path):
    created = {}

    def factory(**kwargs):
        sock = FakeSocket(**kwargs)
        created["socket"] = sock
        return sock

    recorder = FyersRecorder(config(tmp_path), "app-id:access-token", factory)
    run_dir = recorder.run()

    journal_path = run_dir / "fyers-sdk-observations.jsonl"
    count, head = verify(journal_path)
    events = [json.loads(line) for line in journal_path.read_text().splitlines()]
    types = [event["event_type"] for event in events]
    manifest = json.loads((run_dir / "manifest.json").read_text())

    assert "socket_connected" in types
    assert types.count("market_data") == 2
    assert manifest["record_count"] == count
    assert manifest["journal_head_hash"] == head
    assert manifest["journal_chain_valid"] is True
    assert manifest["provider_sequence_available"] is False
    assert manifest["capture_layer"].startswith("Python SDK callback JSON")
    assert manifest["configured_symbol_limit"] == 100
    assert manifest["tbt_socket_supported_by_this_recorder"] is False
    assert created["socket"].subscriptions == [
        (("NSE:SBIN-EQ", "NSE:RELIANCE-EQ"), "SymbolUpdate"),
        (("NSE:SBIN-EQ", "NSE:RELIANCE-EQ"), "DepthUpdate"),
    ]
    assert created["socket"].closed is True


def test_hash_chain_detects_payload_mutation(tmp_path):
    path = tmp_path / "records.jsonl"
    journal = HashChainJournal(path)
    journal.append({"schema_version": 1, "event_type": "market_data", "payload": {"ltp": 10}})
    journal.close()
    count, _ = verify(path)
    assert count == 1

    record = json.loads(path.read_text())
    record["payload"]["ltp"] = 11
    path.write_text(json.dumps(record) + "\n")
    with pytest.raises(ValueError, match="record hash mismatch"):
        verify(path)


def test_watchlist_limits_and_duplicates_fail_closed(tmp_path):
    with pytest.raises(ValueError, match="duplicate"):
        config(tmp_path, symbols=("NSE:SBIN-EQ", "NSE:SBIN-EQ")).validate()
    with pytest.raises(ValueError, match="entitlement limit"):
        config(tmp_path, symbols=tuple(f"NSE:S{i}-EQ" for i in range(101))).validate()
    # Prime's documented Data Socket ceiling is 5,000 symbols. This is only
    # accepted as an operator-declared cap; the API cannot verify the account tier.
    prime = config(tmp_path, symbols=tuple(f"NSE:S{i}-EQ" for i in range(101)), max_symbols=5_000)
    prime.validate()
    with pytest.raises(ValueError, match="max_symbols"):
        config(tmp_path, max_symbols=5_001).validate()


def test_token_must_include_app_id_prefix(tmp_path):
    with pytest.raises(ValueError, match="appid:accesstoken"):
        FyersRecorder(config(tmp_path), "token-only", lambda **kwargs: None)


def test_gap_record_follows_prior_accepted_callbacks(tmp_path):
    import threading
    import time

    recorder = FyersRecorder(config(tmp_path), "app-id:access-token", lambda **kwargs: FakeSocket(**kwargs))
    entered = threading.Event()
    release = threading.Event()
    original_append = recorder._journal.append

    def blocked_append(event):
        if event.get("capture_sequence") == 1:
            entered.set()
            assert release.wait(timeout=5)
        original_append(event)

    recorder._journal.append = blocked_append
    recorder.on_message({"n": 1})
    assert entered.wait(timeout=5)

    # The writer is blocked on sequence 1; fill its bounded queue with 2..65.
    for sequence in range(2, 66):
        recorder.on_message({"n": sequence})
    assert recorder._queue.full()
    recorder.on_message({"n": 66})  # explicit local sequence loss
    recorder.on_message({"n": 67})  # gap cannot be queued yet; extend the gap
    release.set()

    deadline = time.monotonic() + 5
    while not recorder._queue.empty() and time.monotonic() < deadline:
        time.sleep(0.001)
    assert recorder._queue.empty()
    recorder.on_message({"n": 68})  # emits GAP after 1..65, before 68
    run_dir = recorder.close()

    rows = [json.loads(line) for line in (run_dir / "fyers-sdk-observations.jsonl").read_text().splitlines()]
    types = [row["event_type"] for row in rows]
    assert types[:65] == ["market_data"] * 65
    assert rows[64]["capture_sequence"] == 65
    assert rows[65]["event_type"] == "gap"
    assert rows[65]["payload"]["first_missing_local_sequence"] == 66
    assert rows[65]["payload"]["last_missing_local_sequence"] == 67
    assert rows[65]["payload"]["dropped_callback_count"] == 2
    assert rows[66]["capture_sequence"] == 68


def test_connect_callback_during_socket_construction_is_deferred(tmp_path):
    created = {}

    class ConstructorConnectSocket(FakeSocket):
        def keep_running(self):
            self.callbacks["on_message"]({"type": "sf", "symbol": "NSE:SBIN-EQ", "ltp": 812.5})

    def factory(**kwargs):
        socket = ConstructorConnectSocket(**kwargs)
        created["socket"] = socket
        kwargs["on_connect"]()
        return socket

    recorder = FyersRecorder(config(tmp_path), "app-id:access-token", factory)
    run_dir = recorder.run()
    assert created["socket"].subscriptions == [
        (("NSE:SBIN-EQ", "NSE:RELIANCE-EQ"), "SymbolUpdate"),
        (("NSE:SBIN-EQ", "NSE:RELIANCE-EQ"), "DepthUpdate"),
    ]
    rows = [json.loads(line) for line in (run_dir / "fyers-sdk-observations.jsonl").read_text().splitlines()]
    assert rows[0]["event_type"] == "socket_connected"


def test_writer_failure_closes_the_sdk_socket(tmp_path):
    import time

    class WaitingSocket(FakeSocket):
        def keep_running(self):
            self.callbacks["on_connect"]()
            self.callbacks["on_message"]({"type": "sf", "symbol": "NSE:SBIN-EQ", "ltp": 812.5})
            while not self.closed:
                time.sleep(0.005)

    created = {}

    def factory(**kwargs):
        socket = WaitingSocket(**kwargs)
        created["socket"] = socket
        return socket

    recorder = FyersRecorder(config(tmp_path), "app-id:access-token", factory)

    def fail_append(_event):
        raise OSError("disk write failed")

    recorder._journal.append = fail_append
    with pytest.raises(RuntimeError, match="writer failed"):
        recorder.run()
    assert created["socket"].closed is True
    manifest = json.loads((recorder._run_dir / "manifest.json").read_text())
    assert manifest["writer_error"] == "disk write failed"
    assert manifest["research_grade"] is False


def test_deny_all_registry_creates_no_journal_with_stub_socket(tmp_path):
    calls = []

    def factory(**kwargs):
        calls.append(kwargs)
        return FakeSocket(**kwargs)

    denied = config(tmp_path, registry_path=DENY_REGISTRY)
    with pytest.raises(RightsDenied, match="not permitted"):
        FyersRecorder(denied, "app-id:access-token", factory)
    assert calls == [], "the stub socket is never constructed on denial"
    assert list(tmp_path.iterdir()) == [], "no run directory or journal was created"
    for operation in ALL_OPERATIONS:
        with pytest.raises(RightsDenied, match="not permitted"):
            authorize(DENY_REGISTRY, FYERS_PRODUCT_KEY, operation)


def test_rights_gate_reads_explicit_registry_path_only(tmp_path):
    for operation in ALL_OPERATIONS:
        authorize(PERMIT_REGISTRY, FYERS_PRODUCT_KEY, operation)
    with pytest.raises(RightsDenied, match="could not be read"):
        authorize(tmp_path / "missing.json", FYERS_PRODUCT_KEY, "capture")
