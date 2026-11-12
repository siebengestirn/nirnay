from __future__ import annotations

import csv
import json
from datetime import datetime
from decimal import Decimal
from pathlib import Path
from zoneinfo import ZoneInfo

import pytest

from kite_ops.client import KiteClient, KiteCredentials
from kite_ops.errors import KiteUnknownOutcome, RiskRejected
from kite_ops.journal import LockedHashJournal
from kite_ops.service import KiteOperations, KitePolicy

IST = ZoneInfo("Asia/Kolkata")


class FakeResponse:
    def __init__(self, status_code=200, body=None, text=""):
        self.status_code = status_code
        self._body = body
        self.text = text

    def json(self):
        if isinstance(self._body, Exception):
            raise self._body
        return self._body


class FakeSession:
    def __init__(self, responses):
        self.responses = list(responses)
        self.calls = []

    def request(self, method, url, **kwargs):
        self.calls.append((method, url, kwargs))
        return self.responses.pop(0)


class FakeKite:
    def __init__(self, root: Path, orders=None, positions=None, cash="5000", user_id="TESTUSER", submit=None):
        self.root = root
        self._orders = orders or []
        self._positions = positions or {"net": []}
        self.cash = cash
        self.user_id = user_id
        self.submit_result = submit or "ORDER-1"
        self.place_calls = []

    def profile(self):
        return {"user_id": self.user_id}

    def margins(self):
        return {"available": {"cash": self.cash}}

    def orders(self):
        return self._orders

    def positions(self):
        return self._positions

    def instruments_nse_csv(self):
        raise AssertionError("the test installs the same-day instrument master")

    def place_cnc_buy_limit(self, order):
        self.place_calls.append(dict(order))
        if isinstance(self.submit_result, Exception):
            raise self.submit_result
        self._orders.append({
            "order_id": self.submit_result,
            **dict(order),
            "variety": "regular",
            "quantity": int(order["quantity"]),
            "status": "COMPLETE",
            "filled_quantity": int(order["quantity"]),
            "pending_quantity": 0,
        })
        return self.submit_result

    def order_history(self, order_id):
        return [{"order_id": order_id, "status": "COMPLETE", "pending_quantity": 0}]

    def order_trades(self, order_id):
        return [{"order_id": order_id, "quantity": 1, "price": "100.00"}]


def policy(**overrides):
    values = dict(
        expected_user_id="TESTUSER",
        allowed_instruments=("SBIN",),
        live_order_enabled=False,
        static_ip_verified_for_ist_date="",
        daily_auth_verified_for_ist_date="",
        market_session_verified_for_ist_date="",
        max_order_notional_inr=Decimal("1000"),
        max_quantity=100,
    )
    values.update(overrides)
    return KitePolicy(**values)


def set_regular_market_time(monkeypatch):
    fixed = datetime(2026, 11, 13, 10, 0, tzinfo=IST)
    monkeypatch.setattr(KiteOperations, "_ist_now", staticmethod(lambda: fixed))


def instrument_master(root: Path, *, tick="0.05", lot="1", instrument_type="EQ"):
    today = KiteOperations._ist_now().date().isoformat()
    folder = root / "instruments"
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / f"kite-nse-{today}.csv"
    fields = ["instrument_token", "exchange_token", "tradingsymbol", "name", "last_price", "expiry", "strike", "tick_size", "lot_size", "instrument_type", "segment", "exchange"]
    row = {
        "instrument_token": "12345", "exchange_token": "123", "tradingsymbol": "SBIN", "name": "STATE BANK OF INDIA",
        "last_price": "0", "expiry": "", "strike": "0", "tick_size": tick, "lot_size": lot,
        "instrument_type": instrument_type, "segment": "NSE", "exchange": "NSE",
    }
    with path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        writer.writerow(row)
    import hashlib
    data = path.read_bytes()
    (folder / f"kite-nse-{today}.manifest.json").write_text(json.dumps({
        "effective_session_date_ist": today, "sha256": hashlib.sha256(data).hexdigest(), "row_count": 1
    }), encoding="utf-8")


def test_journal_hash_chain_reopens_and_detects_tampering(tmp_path):
    path = tmp_path / "session.jsonl"
    with LockedHashJournal(path) as journal:
        journal.append({"event_type": "INTENT_DURABLE", "tag": "abc"})
    with LockedHashJournal(path) as journal:
        events = journal.read_events()
        assert len(events) == 1
        assert events[0]["event_type"] == "INTENT_DURABLE"
    lines = path.read_text().splitlines()
    value = json.loads(lines[0])
    value["tag"] = "changed"
    path.write_text(json.dumps(value) + "\n")
    with pytest.raises(RuntimeError, match="hash mismatch"):
        LockedHashJournal(path)


def test_plan_enforces_tick_lot_and_notional(tmp_path):
    instrument_master(tmp_path)
    ops = KiteOperations(FakeKite(tmp_path), policy(), tmp_path)
    with pytest.raises(RiskRejected, match="₹1,000"):
        ops.plan("SBIN", 10, "100.05")
    with pytest.raises(RiskRejected, match="tick_size"):
        ops.plan("SBIN", 1, "100.03")
    with pytest.raises(RiskRejected, match="allowed_instruments"):
        ops.plan("RELIANCE", 1, "100")


def test_plan_rejects_cash_user_and_existing_orders(tmp_path):
    instrument_master(tmp_path)
    with pytest.raises(RiskRejected, match="reserve"):
        KiteOperations(FakeKite(tmp_path, cash="100"), policy(), tmp_path).plan("SBIN", 1, "100")
    with pytest.raises(RiskRejected, match="user_id"):
        KiteOperations(FakeKite(tmp_path, user_id="OTHER"), policy(), tmp_path).plan("SBIN", 1, "100")
    with pytest.raises(RiskRejected, match="existing Kite order"):
        KiteOperations(FakeKite(tmp_path, orders=[{"status": "CANCELLED"}]), policy(), tmp_path).plan("SBIN", 1, "100")


def test_manual_submission_is_unconditionally_hard_disabled(tmp_path, monkeypatch):
    set_regular_market_time(monkeypatch)
    instrument_master(tmp_path)
    client = FakeKite(tmp_path)
    # Even the most permissive private config cannot make the client submit.
    ops = KiteOperations(
        client,
        policy(live_order_enabled=True,
               static_ip_verified_for_ist_date="2026-11-13",
               daily_auth_verified_for_ist_date="2026-11-13",
               market_session_verified_for_ist_date="2026-11-13"),
        tmp_path,
    )
    plan = ops.plan("SBIN", 1, "100.00")
    with pytest.raises(RiskRejected, match="LIVE_ROUTING_HARD_DISABLED"):
        ops.submit_manual(plan, "PLACE LIVE ORDER SBIN 1 @ 100.00 INR")
    assert client.place_calls == []
    with ops._order_journal() as journal:
        assert journal.read_events() == []


def test_kite_client_never_sends_trading_writes():
    session = FakeSession([])
    client = KiteClient(KiteCredentials("key", "token"), session=session)
    with pytest.raises(Exception, match="hard-disabled"):
        client.place_cnc_buy_limit({"tradingsymbol": "SBIN"})
    assert session.calls == []


def test_doctor_can_discover_account_id_before_private_policy_is_pinned(tmp_path):
    report = KiteOperations(FakeKite(tmp_path), policy(expected_user_id=""), tmp_path).doctor()
    assert report["profile_user_id"] == "TESTUSER"
    assert report["user_id_match"] is False


def test_live_submit_is_blocked_even_if_cash_or_session_changes(tmp_path, monkeypatch):
    set_regular_market_time(monkeypatch)
    instrument_master(tmp_path)
    client = FakeKite(tmp_path, cash="5000")
    ops = KiteOperations(
        client,
        policy(live_order_enabled=True,
               static_ip_verified_for_ist_date="2026-11-13",
               daily_auth_verified_for_ist_date="2026-11-13",
               market_session_verified_for_ist_date="2026-11-13"),
        tmp_path,
    )
    plan = ops.plan("SBIN", 1, "100.00")
    client.cash = "1"
    with pytest.raises(RiskRejected, match="LIVE_ROUTING_HARD_DISABLED"):
        ops.submit_manual(plan, "PLACE LIVE ORDER SBIN 1 @ 100.00 INR")
    assert client.place_calls == []
    with ops._order_journal() as journal:
        assert journal.read_events() == []


def test_live_submit_is_hard_disabled_outside_regular_market_session(tmp_path, monkeypatch):
    fixed = datetime(2026, 11, 13, 8, 59, tzinfo=IST)
    monkeypatch.setattr(KiteOperations, "_ist_now", staticmethod(lambda: fixed))
    instrument_master(tmp_path)
    client = FakeKite(tmp_path)
    ops = KiteOperations(client, policy(live_order_enabled=True), tmp_path)
    plan = ops.plan("SBIN", 1, "100.00")
    with pytest.raises(RiskRejected, match="LIVE_ROUTING_HARD_DISABLED"):
        ops.submit_manual(plan, "PLACE LIVE ORDER SBIN 1 @ 100.00 INR")
    assert client.place_calls == []


def test_live_submit_is_hard_disabled_even_with_current_attestations(tmp_path, monkeypatch):
    set_regular_market_time(monkeypatch)
    instrument_master(tmp_path)
    client = FakeKite(tmp_path)
    ops = KiteOperations(
        client,
        policy(live_order_enabled=True,
               static_ip_verified_for_ist_date="2026-11-13",
               daily_auth_verified_for_ist_date="2026-11-13",
               market_session_verified_for_ist_date="2026-11-13"),
        tmp_path,
    )
    plan = ops.plan("SBIN", 1, "100.00")
    with pytest.raises(RiskRejected, match="LIVE_ROUTING_HARD_DISABLED"):
        ops.submit_manual(plan, "PLACE LIVE ORDER SBIN 1 @ 100.00 INR")
    assert client.place_calls == []


def test_reconcile_rejects_broker_order_that_does_not_match_intent(tmp_path, monkeypatch):
    set_regular_market_time(monkeypatch)
    instrument_master(tmp_path)
    client = FakeKite(tmp_path)
    ops = KiteOperations(client, policy(), tmp_path)
    plan = ops.plan("SBIN", 1, "100.00")
    with ops._order_journal() as journal:
        journal.append({
            "schema_version": 1, "event_type": "INTENT_DURABLE", "tag": plan.tag,
            "user_id": plan.user_id, "exchange": "NSE", "tradingsymbol": "SBIN",
            "transaction_type": "BUY", "quantity": 1, "product": "CNC",
            "order_type": "LIMIT", "price_inr": "100.00", "validity": "DAY", "variety": "regular",
        })
        journal.append({"schema_version": 1, "event_type": "SUBMISSION_ATTEMPTED", "tag": plan.tag})
    client._orders.append({
        "order_id": "OTHER-ORDER", "tag": plan.tag, "exchange": "NSE", "tradingsymbol": "RELIANCE",
        "transaction_type": "BUY", "quantity": 1, "product": "CNC", "order_type": "LIMIT",
        "price": "100.00", "validity": "DAY", "variety": "regular", "status": "COMPLETE",
    })
    with pytest.raises(RiskRejected, match="does not match the durable intent"):
        ops.reconcile()
    with ops._order_journal() as journal:
        assert journal.read_events()[-1]["event_type"] == "RECONCILIATION_CONFLICT"


def test_reconcile_rejects_overfilled_or_over_limit_trade_records(tmp_path, monkeypatch):
    set_regular_market_time(monkeypatch)
    instrument_master(tmp_path)
    client = FakeKite(tmp_path)
    ops = KiteOperations(client, policy(), tmp_path)
    plan = ops.plan("SBIN", 1, "100.00")
    with ops._order_journal() as journal:
        journal.append({
            "schema_version": 1, "event_type": "INTENT_DURABLE", "tag": plan.tag,
            "user_id": plan.user_id, "exchange": "NSE", "tradingsymbol": "SBIN",
            "transaction_type": "BUY", "quantity": 1, "product": "CNC",
            "order_type": "LIMIT", "price_inr": "100.00", "validity": "DAY", "variety": "regular",
        })
        journal.append({"schema_version": 1, "event_type": "SUBMISSION_ATTEMPTED", "tag": plan.tag})
    client._orders.append({
        "order_id": "ORDER-1", "tag": plan.tag, "exchange": "NSE", "tradingsymbol": "SBIN",
        "transaction_type": "BUY", "quantity": 1, "product": "CNC", "order_type": "LIMIT",
        "price": "100.00", "validity": "DAY", "variety": "regular", "status": "COMPLETE",
    })
    client.order_trades = lambda order_id: [
        {"order_id": order_id, "quantity": 2, "average_price": "100.00"}
    ]
    with pytest.raises(RiskRejected, match="trade records violate"):
        ops.reconcile()
    with ops._order_journal() as journal:
        assert journal.read_events()[-1]["event_type"] == "RECONCILIATION_CONFLICT"


def test_instrument_refresh_recovers_a_csv_written_before_manifest(tmp_path, monkeypatch):
    set_regular_market_time(monkeypatch)
    instrument_master(tmp_path)
    today = KiteOperations._ist_now().date().isoformat()
    directory = tmp_path / "instruments"
    csv_path = directory / f"kite-nse-{today}.csv"
    manifest_path = directory / f"kite-nse-{today}.manifest.json"
    source = csv_path.read_bytes()
    manifest_path.unlink()
    client = FakeKite(tmp_path)
    client.instruments_nse_csv = lambda: source.decode("utf-8")
    result = KiteOperations(client, policy(), tmp_path).refresh_instruments()
    assert result == csv_path
    manifest = json.loads(manifest_path.read_text())
    assert manifest["sha256"] == __import__("hashlib").sha256(source).hexdigest()
    assert KiteOperations(client, policy(), tmp_path)._current_instrument("SBIN").tick_size == Decimal("0.05")


def test_instrument_refresh_reconstructs_missing_csv_from_matching_manifest(tmp_path, monkeypatch):
    set_regular_market_time(monkeypatch)
    instrument_master(tmp_path)
    today = KiteOperations._ist_now().date().isoformat()
    directory = tmp_path / "instruments"
    csv_path = directory / f"kite-nse-{today}.csv"
    manifest_path = directory / f"kite-nse-{today}.manifest.json"
    source = csv_path.read_bytes()
    manifest = json.loads(manifest_path.read_text())
    csv_path.unlink()
    client = FakeKite(tmp_path)
    client.instruments_nse_csv = lambda: source.decode("utf-8")
    result = KiteOperations(client, policy(), tmp_path).refresh_instruments()
    assert result.read_bytes() == source
    assert json.loads(manifest_path.read_text()) == manifest


def test_instrument_refresh_refuses_different_existing_master(tmp_path, monkeypatch):
    set_regular_market_time(monkeypatch)
    instrument_master(tmp_path)
    today = KiteOperations._ist_now().date().isoformat()
    directory = tmp_path / "instruments"
    csv_path = directory / f"kite-nse-{today}.csv"
    changed = csv_path.read_text().replace("SBIN", "RELIANCE")
    client = FakeKite(tmp_path)
    client.instruments_nse_csv = lambda: changed
    with pytest.raises(Exception, match="already exists with different content"):
        KiteOperations(client, policy(), tmp_path).refresh_instruments()
