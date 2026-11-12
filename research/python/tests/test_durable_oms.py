from __future__ import annotations

import json
import sqlite3
from decimal import Decimal

import pytest

from quantlab.durable_oms import (
    DurableOMSStore, OutboxError, PaperOutboxDispatcher,
)
from quantlab.oms import BrokerOrderIntent, IdempotencyConflict, OmsError


class FakePaperBroker:
    broker_id = "paper"
    def __init__(self, *, submit_error=None, cancel_error=None):
        self.submit_error = submit_error
        self.cancel_error = cancel_error
        self.submits = []
        self.cancels = []
    def submit(self, intent, *, client_order_id):
        self.submits.append((intent, client_order_id))
        if self.submit_error: raise self.submit_error
        return f"broker-{client_order_id}"
    def cancel(self, broker_order_id, *, client_order_id):
        self.cancels.append((broker_order_id, client_order_id))
        if self.cancel_error: raise self.cancel_error


def intent(cid="c-1", *, quantity=10):
    return BrokerOrderIntent(cid, "strategy.v1", "NSE:SBIN", "NSE", "buy", "limit", "CNC",
                             quantity, Decimal("500.00"), "day")


def test_accept_intent_is_transactional_idempotent_and_detects_contract_reuse(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    first = store.accept_intent(intent(), broker="paper", now_ns=1)
    again = store.accept_intent(intent(), broker="paper", now_ns=2)
    assert first.client_order_id == again.client_order_id == "c-1"
    assert store.claim_next(worker_id="w1", now_ns=3).state == "CLAIMED"
    with pytest.raises(IdempotencyConflict):
        store.accept_intent(intent(quantity=9), broker="paper", now_ns=4)
    assert store.verify_event_chain()["event_count"] == 3


def test_paper_dispatcher_acknowledges_once_and_live_mode_refuses_before_claim(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    store.accept_intent(intent(), broker="paper", now_ns=1)
    fake = FakePaperBroker()
    with pytest.raises(OutboxError, match="LIVE_ROUTING_HARD_DISABLED"):
        PaperOutboxDispatcher(store, fake, mode="live").dispatch_one(worker_id="w", now_ns=2)
    assert fake.submits == []
    order = PaperOutboxDispatcher(store, fake).dispatch_one(worker_id="w", now_ns=2)
    assert order.status == "WORKING"
    assert order.broker_order_id == "broker-c-1"
    assert len(fake.submits) == 1
    assert PaperOutboxDispatcher(store, fake).dispatch_one(worker_id="w", now_ns=3) is None
    assert store.verify_event_chain()["status"] == "verified_internal_integrity"


def test_expired_claimed_command_can_be_reclaimed_before_side_effect(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    store.accept_intent(intent(), broker="paper", now_ns=1)
    first = store.claim_next(worker_id="dead-worker", now_ns=2, lease_ns=10)
    assert first.attempts == 1
    second = store.claim_next(worker_id="healthy-worker", now_ns=13, lease_ns=10)
    assert second.outbox_id == first.outbox_id
    assert second.attempts == 2
    assert second.lease_owner == "healthy-worker"


def test_expired_dispatching_command_becomes_unknown_and_is_never_requeued(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    store.accept_intent(intent(), broker="paper", now_ns=1)
    command = store.claim_next(worker_id="w", now_ns=2, lease_ns=10)
    store.mark_dispatch_started(command.outbox_id, worker_id="w", now_ns=3, dispatch_lease_ns=10)
    assert store.claim_next(worker_id="w2", now_ns=14) is None
    assert store.outbox(command.outbox_id).state == "UNKNOWN"
    assert store.order("c-1").status == "UNKNOWN"


def test_ambiguous_submit_failure_is_unknown_and_not_automatically_retried(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    store.accept_intent(intent(), broker="paper", now_ns=1)
    broker = FakePaperBroker(submit_error=TimeoutError("ambiguous after send"))
    order = PaperOutboxDispatcher(store, broker).dispatch_one(worker_id="w", now_ns=2)
    assert order.status == "UNKNOWN"
    assert store.claim_next(worker_id="w2", now_ns=10) is None
    assert len(broker.submits) == 1


def _working_order(store: DurableOMSStore):
    store.accept_intent(intent(), broker="paper", now_ns=1)
    broker = FakePaperBroker()
    PaperOutboxDispatcher(store, broker).dispatch_one(worker_id="w", now_ns=2)


def test_broker_events_are_idempotent_sequenced_and_execution_ids_unique(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    kwargs = dict(broker="paper", client_order_id="c-1", broker_order_id="broker-c-1",
        event_id="evt-1", status="PARTIALLY_FILLED", sequence=1, fill_quantity=4,
        fill_price="499.50", execution_id="exec-1", now_ns=10)
    partial = store.apply_broker_event(**kwargs)
    assert partial.filled_quantity == 4 and partial.status == "PARTIALLY_FILLED"
    assert store.apply_broker_event(**kwargs).filled_quantity == 4  # Exact event replay is idempotent.
    with pytest.raises(IdempotencyConflict):
        store.apply_broker_event(**{**kwargs, "event_id": "evt-2", "sequence": 2})
    with pytest.raises(OutboxError, match="strictly increasing"):
        store.apply_broker_event(**{**kwargs, "event_id": "evt-3", "sequence": 1,
                                    "execution_id": "exec-3", "fill_quantity": 1})
    filled = store.apply_broker_event(**{**kwargs, "event_id": "evt-2", "sequence": 2,
        "status": "FILLED", "fill_quantity": 6, "fill_price": "500", "execution_id": "exec-2", "now_ns": 11})
    assert filled.status == "FILLED" and filled.filled_quantity == 10
    with pytest.raises(OutboxError, match="invalid broker order status transition"):
        store.apply_broker_event(**{**kwargs, "event_id": "evt-3", "sequence": 3,
            "status": "WORKING", "fill_quantity": 0, "fill_price": None, "execution_id": None, "now_ns": 12})


def test_cancel_is_an_outbox_command_and_ack_is_not_cancel_confirmation(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    broker = FakePaperBroker()
    command = store.request_cancel("c-1", now_ns=5)
    assert command.command_type == "CANCEL" and command.state == "PENDING"
    assert store.order("c-1").status == "CANCEL_PENDING"
    order = PaperOutboxDispatcher(store, broker).dispatch_one(worker_id="w2", now_ns=6)
    assert order.status == "CANCEL_PENDING"
    assert broker.cancels == [("broker-c-1", "c-1")]
    assert store.outbox(command.outbox_id).state == "ACKNOWLEDGED"
    cancelled = store.apply_broker_event(broker="paper", client_order_id="c-1", event_id="cancel-ack",
        status="CANCELLED", sequence=1, fill_quantity=0, fill_price=None, execution_id=None,
        broker_order_id="broker-c-1", now_ns=7)
    assert cancelled.status == "CANCELLED"


def test_cancel_exception_becomes_unknown_not_auto_retried(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    command = store.request_cancel("c-1", now_ns=5)
    broker = FakePaperBroker(cancel_error=TimeoutError())
    order = PaperOutboxDispatcher(store, broker).dispatch_one(worker_id="w2", now_ns=6)
    assert order.status == "UNKNOWN"
    assert store.outbox(command.outbox_id).state == "UNKNOWN"
    assert store.claim_next(worker_id="w3", now_ns=100) is None


def test_sqlite_event_chain_detects_tampering_and_file_permissions(tmp_path):
    path = tmp_path / "oms.sqlite3"
    store = DurableOMSStore(path)
    store.accept_intent(intent(), broker="paper", now_ns=1)
    if hasattr(path, "stat"):
        assert path.stat().st_mode & 0o777 == 0o600
    with sqlite3.connect(path) as db:
        db.execute("UPDATE oms_events SET payload_json='{}' WHERE sequence=1")
    with pytest.raises(OmsError, match="record hash mismatch"):
        store.verify_event_chain()


def test_adapter_mismatch_is_a_definitive_pre_dispatch_rejection(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    store.accept_intent(intent(), broker="zerodha", now_ns=1)
    broker = FakePaperBroker()  # Configured paper adapter cannot service a Zerodha command.
    with pytest.raises(OutboxError, match="does not match"):
        PaperOutboxDispatcher(store, broker).dispatch_one(worker_id="w", now_ns=2)
    assert broker.submits == []
    assert store.order("c-1").status == "REJECTED"
    assert store.claim_next(worker_id="w2", now_ns=3) is None


def test_dispatch_cannot_start_at_exact_lease_expiry_boundary(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    store.accept_intent(intent(), broker="paper", now_ns=1)
    command = store.claim_next(worker_id="w", now_ns=2, lease_ns=10)
    with pytest.raises(OutboxError, match="expired"):
        store.mark_dispatch_started(command.outbox_id, worker_id="w", now_ns=12)
    assert store.outbox(command.outbox_id).state == "CLAIMED"
    # Claim expiry is safe to reclaim because no external side effect began.
    reclaimed = store.claim_next(worker_id="w2", now_ns=12, lease_ns=10)
    assert reclaimed.state == "CLAIMED" and reclaimed.lease_owner == "w2"


def test_definitive_cancel_rejection_restores_working_state_without_claiming_cancelled(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    command = store.request_cancel("c-1", now_ns=5)
    claimed = store.claim_next(worker_id="cancel-worker", now_ns=6)
    assert claimed.outbox_id == command.outbox_id
    store.record_definitive_rejection(command.outbox_id, now_ns=7,
                                      broker_reason_code="broker_rejected_cancel")
    assert store.outbox(command.outbox_id).state == "REJECTED"
    assert store.order("c-1").status == "WORKING"


def test_definitive_cancel_rejection_can_be_retried_only_by_new_explicit_request(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    first = store.request_cancel("c-1", now_ns=5)
    store.claim_next(worker_id="cancel-worker-1", now_ns=6)
    store.record_definitive_rejection(first.outbox_id, now_ns=7,
                                      broker_reason_code="broker_rejected_cancel")
    second = store.request_cancel("c-1", now_ns=8)
    assert second.outbox_id == first.outbox_id
    assert second.state == "PENDING"
    assert store.order("c-1").status == "CANCEL_PENDING"
    assert store.verify_event_chain()["event_count"] >= 6


def test_unknown_cancel_cannot_be_retried_without_reconciliation(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    command = store.request_cancel("c-1", now_ns=5)
    store.claim_next(worker_id="cancel-worker", now_ns=6)
    store.mark_dispatch_started(command.outbox_id, worker_id="cancel-worker", now_ns=6)
    store.mark_unknown(command.outbox_id, now_ns=7, reason_code="timeout")
    with pytest.raises(OutboxError, match="must be reconciled"):
        store.request_cancel("c-1", now_ns=8)


def test_submission_acknowledgement_cannot_acknowledge_cancel_command(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    command = store.request_cancel("c-1", now_ns=5)
    store.claim_next(worker_id="cancel-worker", now_ns=6)
    store.mark_dispatch_started(command.outbox_id, worker_id="cancel-worker", now_ns=6)
    with pytest.raises(OutboxError, match="SUBMIT"):
        store.acknowledge_submission(command.outbox_id, "should-not-bind", now_ns=7)
    assert store.outbox(command.outbox_id).state == "DISPATCHING"
    assert store.order("c-1").status == "CANCEL_PENDING"


def test_cancel_rejection_race_does_not_roll_terminal_fill_back_to_working(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    command = store.request_cancel("c-1", now_ns=5)
    store.claim_next(worker_id="cancel-worker", now_ns=6)
    store.mark_dispatch_started(command.outbox_id, worker_id="cancel-worker", now_ns=6)
    filled = store.apply_broker_event(broker="paper", client_order_id="c-1", event_id="fill-race",
        status="FILLED", sequence=1, fill_quantity=10, fill_price="500", execution_id="exec-race",
        broker_order_id="broker-c-1", now_ns=7)
    assert filled.status == "FILLED"
    store.record_definitive_rejection(command.outbox_id, now_ns=8,
                                      broker_reason_code="cancel_too_late_order_filled")
    assert store.order("c-1").status == "FILLED"


def test_cancel_timeout_after_terminal_fill_preserves_known_terminal_order_state(tmp_path):
    store = DurableOMSStore(tmp_path / "oms.sqlite3")
    _working_order(store)
    command = store.request_cancel("c-1", now_ns=5)
    store.claim_next(worker_id="cancel-worker", now_ns=6)
    store.mark_dispatch_started(command.outbox_id, worker_id="cancel-worker", now_ns=6)
    store.apply_broker_event(broker="paper", client_order_id="c-1", event_id="fill-before-timeout",
        status="FILLED", sequence=1, fill_quantity=10, fill_price="500", execution_id="exec-before-timeout",
        broker_order_id="broker-c-1", now_ns=7)
    store.mark_unknown(command.outbox_id, now_ns=8, reason_code="late_cancel_timeout")
    assert store.outbox(command.outbox_id).state == "UNKNOWN"
    assert store.order("c-1").status == "FILLED"
