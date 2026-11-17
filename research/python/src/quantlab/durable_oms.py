"""SQLite-backed event-sourced OMS store with a transactional outbox.

This module is the durable single-node reference implementation. It improves
on a JSONL journal by committing order acceptance, event history and outbox
commands in one SQLite transaction. It is not a distributed consensus system.
Live dispatch is fail-closed; the dispatcher is intended for deterministic
paper adapters and conformance testing until operational certification exists.
"""
from __future__ import annotations

import hashlib
import json
import os
import sqlite3
from contextlib import contextmanager
import uuid
from dataclasses import asdict, dataclass
from decimal import Decimal
from pathlib import Path
from typing import Any, Protocol

from .market import canonical_bytes
from .oms import BrokerOrderIntent, IdempotencyConflict, OmsError


class OutboxError(OmsError):
    pass


@dataclass(frozen=True, slots=True)
class OutboxCommand:
    outbox_id: str
    client_order_id: str
    broker: str
    command_type: str
    payload: dict[str, Any]
    state: str
    attempts: int
    lease_owner: str | None
    lease_until_ns: int | None


@dataclass(frozen=True, slots=True)
class DurableOrder:
    client_order_id: str
    broker: str
    status: str
    broker_order_id: str | None
    quantity: int
    filled_quantity: int
    average_fill_price: str | None
    intent: dict[str, Any]


class DurableOMSStore:
    """SQLite WAL store with FULL synchronous durability and tamper-evident events."""

    def __init__(self, path: str | Path):
        requested = Path(path)
        requested.parent.mkdir(parents=True, exist_ok=True)
        self.path = requested.parent.resolve() / requested.name
        if self.path.is_symlink():
            raise OmsError("OMS database path must not be a symlink")
        if os.name == "posix":
            os.chmod(self.path.parent, 0o700)
        self._initialize()
        if os.name == "posix":
            os.chmod(self.path, 0o600)
            for suffix in ("-wal", "-shm"):
                sidecar = Path(str(self.path) + suffix)
                if sidecar.exists():
                    os.chmod(sidecar, 0o600)

    def _connect(self) -> sqlite3.Connection:
        db = sqlite3.connect(self.path, timeout=15.0, isolation_level=None)
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA foreign_keys=ON")
        db.execute("PRAGMA busy_timeout=15000")
        db.execute("PRAGMA synchronous=FULL")
        db.execute("PRAGMA trusted_schema=OFF")
        db.execute("PRAGMA secure_delete=ON")
        db.execute("PRAGMA wal_autocheckpoint=1000")
        if os.name == "posix":
            os.chmod(self.path, 0o600)
            for suffix in ("-wal", "-shm"):
                sidecar = Path(str(self.path) + suffix)
                if sidecar.exists():
                    os.chmod(sidecar, 0o600)
        return db

    @contextmanager
    def _db(self):
        db = self._connect()
        try:
            yield db
        finally:
            db.close()

    def _initialize(self) -> None:
        with self._db() as db:
            db.execute("PRAGMA journal_mode=WAL")
            db.executescript("""
                CREATE TABLE IF NOT EXISTS oms_events (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    event_id TEXT NOT NULL UNIQUE,
                    event_type TEXT NOT NULL,
                    entity_id TEXT NOT NULL,
                    timestamp_ns INTEGER NOT NULL CHECK(timestamp_ns > 0),
                    payload_json TEXT NOT NULL,
                    previous_hash TEXT NOT NULL,
                    record_hash TEXT NOT NULL UNIQUE
                );
                CREATE TABLE IF NOT EXISTS oms_orders (
                    client_order_id TEXT PRIMARY KEY,
                    broker TEXT NOT NULL,
                    status TEXT NOT NULL,
                    broker_order_id TEXT,
                    intent_json TEXT NOT NULL,
                    quantity INTEGER NOT NULL CHECK(quantity > 0),
                    filled_quantity INTEGER NOT NULL DEFAULT 0 CHECK(filled_quantity >= 0),
                    average_fill_price TEXT,
                    last_update_sequence INTEGER NOT NULL DEFAULT 0 CHECK(last_update_sequence >= 0),
                    updated_at_ns INTEGER NOT NULL CHECK(updated_at_ns > 0),
                    UNIQUE(broker, broker_order_id)
                );
                CREATE TABLE IF NOT EXISTS oms_outbox (
                    outbox_id TEXT PRIMARY KEY,
                    client_order_id TEXT NOT NULL REFERENCES oms_orders(client_order_id),
                    broker TEXT NOT NULL,
                    command_type TEXT NOT NULL CHECK(command_type IN ('SUBMIT', 'CANCEL')),
                    payload_json TEXT NOT NULL,
                    state TEXT NOT NULL CHECK(state IN ('PENDING','CLAIMED','DISPATCHING','ACKNOWLEDGED','UNKNOWN','REJECTED')),
                    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
                    lease_owner TEXT,
                    lease_until_ns INTEGER,
                    created_at_ns INTEGER NOT NULL CHECK(created_at_ns > 0),
                    UNIQUE(client_order_id, command_type)
                );
                CREATE TABLE IF NOT EXISTS oms_broker_events (
                    broker TEXT NOT NULL,
                    event_id TEXT NOT NULL,
                    digest TEXT NOT NULL,
                    client_order_id TEXT NOT NULL REFERENCES oms_orders(client_order_id),
                    created_at_ns INTEGER NOT NULL CHECK(created_at_ns > 0),
                    PRIMARY KEY(broker, event_id)
                );
                CREATE TABLE IF NOT EXISTS oms_execution_ids (
                    broker TEXT NOT NULL,
                    execution_id TEXT NOT NULL,
                    client_order_id TEXT NOT NULL REFERENCES oms_orders(client_order_id),
                    created_at_ns INTEGER NOT NULL CHECK(created_at_ns > 0),
                    PRIMARY KEY(broker, execution_id)
                );
                CREATE INDEX IF NOT EXISTS oms_outbox_ready
                    ON oms_outbox(state, created_at_ns);
                CREATE INDEX IF NOT EXISTS oms_orders_status
                    ON oms_orders(status, broker);
            """)
            columns = {row["name"] for row in db.execute("PRAGMA table_info(oms_orders)").fetchall()}
            if "last_update_sequence" not in columns:
                db.execute("ALTER TABLE oms_orders ADD COLUMN last_update_sequence INTEGER NOT NULL DEFAULT 0")

    @staticmethod
    def _intent_dict(intent: BrokerOrderIntent) -> dict[str, Any]:
        intent.validate()
        raw = asdict(intent)
        raw["limit_price"] = None if intent.limit_price is None else format(intent.limit_price, "f")
        return raw

    @staticmethod
    def _event(db: sqlite3.Connection, event_type: str, entity_id: str,
               payload: dict[str, Any], timestamp_ns: int) -> dict[str, Any]:
        if type(timestamp_ns) is not int or timestamp_ns <= 0:
            raise ValueError("event timestamp must be positive integer UTC nanoseconds")
        previous = db.execute("SELECT record_hash FROM oms_events ORDER BY sequence DESC LIMIT 1").fetchone()
        previous_hash = previous[0] if previous else "0" * 64
        body = {
            "event_id": str(uuid.uuid4()), "event_type": event_type,
            "entity_id": entity_id, "timestamp_ns": timestamp_ns,
            "payload": payload, "previous_hash": previous_hash,
        }
        digest = hashlib.sha256(canonical_bytes(body)).hexdigest()
        db.execute(
            "INSERT INTO oms_events(event_id,event_type,entity_id,timestamp_ns,payload_json,previous_hash,record_hash) "
            "VALUES(?,?,?,?,?,?,?)",
            (body["event_id"], event_type, entity_id, timestamp_ns,
             json.dumps(payload, sort_keys=True, separators=(",", ":"), allow_nan=False),
             previous_hash, digest),
        )
        return {**body, "record_hash": digest}

    def accept_intent(self, intent: BrokerOrderIntent, *, broker: str, now_ns: int) -> DurableOrder:
        raw = self._intent_dict(intent)
        if not isinstance(broker, str) or not broker.strip():
            raise ValueError("broker identity is required")
        encoded = json.dumps(raw, sort_keys=True, separators=(",", ":"), allow_nan=False)
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                existing = db.execute("SELECT * FROM oms_orders WHERE client_order_id=?", (intent.client_order_id,)).fetchone()
                if existing:
                    if existing["broker"] != broker or existing["intent_json"] != encoded:
                        raise IdempotencyConflict("client_order_id reused with a different order contract")
                    db.commit()
                    return self._row_order(existing)
                self._event(db, "INTENT_ACCEPTED", intent.client_order_id,
                            {"broker": broker, "intent": raw}, now_ns)
                db.execute(
                    "INSERT INTO oms_orders(client_order_id,broker,status,intent_json,quantity,updated_at_ns) "
                    "VALUES(?,?,?,?,?,?)",
                    (intent.client_order_id, broker, "PENDING_NEW", encoded, intent.quantity, now_ns),
                )
                outbox_id = str(uuid.uuid4())
                db.execute(
                    "INSERT INTO oms_outbox(outbox_id,client_order_id,broker,command_type,payload_json,state,created_at_ns) "
                    "VALUES(?,?,?,?,?,'PENDING',?)",
                    (outbox_id, intent.client_order_id, broker, "SUBMIT", encoded, now_ns),
                )
                self._event(db, "OUTBOX_CREATED", intent.client_order_id,
                            {"outbox_id": outbox_id, "command_type": "SUBMIT"}, now_ns)
                row = db.execute("SELECT * FROM oms_orders WHERE client_order_id=?", (intent.client_order_id,)).fetchone()
                db.commit()
                return self._row_order(row)
            except Exception:
                db.rollback()
                raise

    def claim_next(self, *, worker_id: str, now_ns: int, lease_ns: int = 5_000_000_000) -> OutboxCommand | None:
        if not worker_id or type(now_ns) is not int or now_ns <= 0 or type(lease_ns) is not int or lease_ns <= 0:
            raise ValueError("worker_id, positive now_ns and lease_ns are required")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                # Expired CLAIMED commands were not yet marked dispatching, so
                # reclaiming them cannot duplicate an external side effect.
                db.execute("UPDATE oms_outbox SET state='PENDING',lease_owner=NULL,lease_until_ns=NULL "
                           "WHERE state='CLAIMED' AND lease_until_ns<=?", (now_ns,))
                # DISPATCHING means a side effect may have escaped. It is never
                # automatically requeued; expiry converts it to UNKNOWN below.
                expired = db.execute("SELECT outbox_id,client_order_id,command_type FROM oms_outbox "
                                     "WHERE state='DISPATCHING' AND lease_until_ns<=?", (now_ns,)).fetchall()
                for row in expired:
                    db.execute("UPDATE oms_outbox SET state='UNKNOWN',lease_owner=NULL,lease_until_ns=NULL WHERE outbox_id=?",
                               (row["outbox_id"],))
                    current = db.execute("SELECT status FROM oms_orders WHERE client_order_id=?",
                                         (row["client_order_id"],)).fetchone()
                    # A late cancel timeout must not erase a terminal fill/cancel
                    # already established by an authoritative broker update.
                    if current and current["status"] not in {"FILLED", "CANCELLED", "REJECTED", "EXPIRED"}:
                        db.execute("UPDATE oms_orders SET status='UNKNOWN',updated_at_ns=? WHERE client_order_id=?",
                                   (now_ns, row["client_order_id"]))
                    event_type = ("SUBMISSION_OUTCOME_UNKNOWN" if row["command_type"] == "SUBMIT"
                                  else "CANCEL_OUTCOME_UNKNOWN")
                    self._event(db, event_type, row["client_order_id"],
                                {"outbox_id": row["outbox_id"], "command_type": row["command_type"],
                                 "reason": "dispatcher lease expired after dispatch began"}, now_ns)
                row = db.execute("SELECT * FROM oms_outbox WHERE state='PENDING' ORDER BY created_at_ns,outbox_id LIMIT 1").fetchone()
                if not row:
                    db.commit()
                    return None
                until = now_ns + lease_ns
                db.execute("UPDATE oms_outbox SET state='CLAIMED',attempts=attempts+1,lease_owner=?,lease_until_ns=? WHERE outbox_id=?",
                           (worker_id, until, row["outbox_id"]))
                self._event(db, "OUTBOX_CLAIMED", row["client_order_id"],
                            {"outbox_id": row["outbox_id"], "worker_id": worker_id, "lease_until_ns": until}, now_ns)
                current = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (row["outbox_id"],)).fetchone()
                db.commit()
                return self._row_outbox(current)
            except Exception:
                db.rollback()
                raise

    def request_cancel(self, client_order_id: str, *, now_ns: int) -> OutboxCommand:
        """Durably enqueue one cancel request; cancellation is not complete until broker confirmation."""
        if not isinstance(client_order_id, str) or not client_order_id.strip():
            raise ValueError("client_order_id is required")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                row = db.execute("SELECT * FROM oms_orders WHERE client_order_id=?", (client_order_id,)).fetchone()
                if not row:
                    raise OutboxError("cannot cancel an unknown local order")
                if row["status"] == "UNKNOWN":
                    raise OutboxError("unknown order/cancel outcome must be reconciled before another cancel request")
                if row["status"] not in {"WORKING", "PARTIALLY_FILLED"} or not row["broker_order_id"]:
                    raise OutboxError("only a broker-acknowledged working order can be cancelled")
                existing = db.execute("SELECT * FROM oms_outbox WHERE client_order_id=? AND command_type='CANCEL'",
                                      (client_order_id,)).fetchone()
                if existing:
                    if existing["state"] == "UNKNOWN":
                        raise OutboxError("unknown cancel outcome must be reconciled before another cancel request")
                    if existing["state"] == "REJECTED":
                        # A definitive broker rejection establishes that the prior
                        # cancel command did not take effect. A new explicit caller
                        # request may reuse the unique per-order cancel slot. The
                        # attempt counter remains cumulative across claims.
                        payload = {"client_order_id": client_order_id,
                                   "broker_order_id": row["broker_order_id"]}
                        db.execute("UPDATE oms_outbox SET payload_json=?,state='PENDING',"
                                   "lease_owner=NULL,lease_until_ns=NULL,created_at_ns=? WHERE outbox_id=?",
                                   (json.dumps(payload, sort_keys=True, separators=(",", ":")),
                                    now_ns, existing["outbox_id"]))
                        db.execute("UPDATE oms_orders SET status='CANCEL_PENDING',updated_at_ns=? WHERE client_order_id=?",
                                   (now_ns, client_order_id))
                        self._event(db, "CANCEL_RETRY_REQUESTED", client_order_id,
                                    {"outbox_id": existing["outbox_id"], "prior_state": "REJECTED"}, now_ns)
                        refreshed = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?",
                                                (existing["outbox_id"],)).fetchone()
                        db.commit()
                        return self._row_outbox(refreshed)
                    return self._row_outbox(existing)
                outbox_id = str(uuid.uuid4())
                payload = {"client_order_id": client_order_id, "broker_order_id": row["broker_order_id"]}
                db.execute("INSERT INTO oms_outbox(outbox_id,client_order_id,broker,command_type,payload_json,state,created_at_ns) "
                           "VALUES(?,?,?,'CANCEL',?,'PENDING',?)",
                           (outbox_id, client_order_id, row["broker"],
                            json.dumps(payload, sort_keys=True, separators=(",", ":")), now_ns))
                db.execute("UPDATE oms_orders SET status='CANCEL_PENDING',updated_at_ns=? WHERE client_order_id=?",
                           (now_ns, client_order_id))
                self._event(db, "CANCEL_REQUESTED", client_order_id,
                            {"outbox_id": outbox_id, **payload}, now_ns)
                result = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (outbox_id,)).fetchone()
                db.commit()
                return self._row_outbox(result)
            except Exception:
                db.rollback()
                raise

    def acknowledge_cancel_request(self, outbox_id: str, *, now_ns: int) -> None:
        """Record broker acceptance of a cancel request, not terminal cancellation."""
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                row = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (outbox_id,)).fetchone()
                if not row or row["command_type"] != "CANCEL" or row["state"] != "DISPATCHING":
                    raise OutboxError("only a dispatching cancel request can be acknowledged")
                self._event(db, "CANCEL_REQUEST_ACKNOWLEDGED", row["client_order_id"],
                            {"outbox_id": outbox_id}, now_ns)
                db.execute("UPDATE oms_outbox SET state='ACKNOWLEDGED',lease_owner=NULL,lease_until_ns=NULL WHERE outbox_id=?",
                           (outbox_id,))
                # Keep OMS status CANCEL_PENDING until an order update confirms
                # CANCELLED or a race confirms a fill/working state.
                db.execute("UPDATE oms_orders SET updated_at_ns=? WHERE client_order_id=?",
                           (now_ns, row["client_order_id"]))
                db.commit()
            except Exception:
                db.rollback()
                raise

    def mark_dispatch_started(self, outbox_id: str, *, worker_id: str, now_ns: int,
                              dispatch_lease_ns: int = 15_000_000_000) -> OutboxCommand:
        if type(dispatch_lease_ns) is not int or dispatch_lease_ns <= 0:
            raise ValueError("dispatch_lease_ns must be positive")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                row = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (outbox_id,)).fetchone()
                if not row or row["state"] != "CLAIMED" or row["lease_owner"] != worker_id or row["lease_until_ns"] <= now_ns:
                    raise OutboxError("outbox lease is absent, expired or owned by another worker")
                new_until = now_ns + dispatch_lease_ns
                db.execute("UPDATE oms_outbox SET state='DISPATCHING',lease_until_ns=? WHERE outbox_id=?",
                           (new_until, outbox_id))
                self._event(db, "SUBMISSION_ATTEMPT_STARTED", row["client_order_id"],
                            {"outbox_id": outbox_id, "worker_id": worker_id}, now_ns)
                current = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (outbox_id,)).fetchone()
                db.commit()
                return self._row_outbox(current)
            except Exception:
                db.rollback()
                raise

    def acknowledge_submission(self, outbox_id: str, broker_order_id: str, *, now_ns: int) -> DurableOrder:
        if not broker_order_id:
            raise ValueError("broker_order_id is required")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                row = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (outbox_id,)).fetchone()
                if not row or row["command_type"] != "SUBMIT" or row["state"] != "DISPATCHING":
                    raise OutboxError("only a dispatching SUBMIT command can be acknowledged as a submission")
                self._event(db, "SUBMISSION_ACKNOWLEDGED", row["client_order_id"],
                            {"outbox_id": outbox_id, "broker_order_id": broker_order_id}, now_ns)
                db.execute("UPDATE oms_orders SET status='WORKING',broker_order_id=?,updated_at_ns=? WHERE client_order_id=?",
                           (broker_order_id, now_ns, row["client_order_id"]))
                db.execute("UPDATE oms_outbox SET state='ACKNOWLEDGED',lease_owner=NULL,lease_until_ns=NULL WHERE outbox_id=?",
                           (outbox_id,))
                order = db.execute("SELECT * FROM oms_orders WHERE client_order_id=?", (row["client_order_id"],)).fetchone()
                db.commit()
                return self._row_order(order)
            except sqlite3.IntegrityError as exc:
                db.rollback()
                raise IdempotencyConflict("broker order id is already bound to another local order") from exc
            except Exception:
                db.rollback()
                raise

    def mark_unknown(self, outbox_id: str, *, now_ns: int, reason_code: str) -> None:
        if not reason_code:
            raise ValueError("reason_code is required")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                row = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (outbox_id,)).fetchone()
                if not row or row["state"] not in {"DISPATCHING", "UNKNOWN"}:
                    raise OutboxError("only a dispatching command can become unknown")
                unknown_event = ("SUBMISSION_OUTCOME_UNKNOWN" if row["command_type"] == "SUBMIT"
                                 else "CANCEL_OUTCOME_UNKNOWN")
                self._event(db, unknown_event, row["client_order_id"],
                            {"outbox_id": outbox_id, "reason_code": reason_code,
                             "command_type": row["command_type"]}, now_ns)
                db.execute("UPDATE oms_outbox SET state='UNKNOWN',lease_owner=NULL,lease_until_ns=NULL WHERE outbox_id=?",
                           (outbox_id,))
                current = db.execute("SELECT status FROM oms_orders WHERE client_order_id=?",
                                     (row["client_order_id"],)).fetchone()
                if current and current["status"] not in {"FILLED", "CANCELLED", "REJECTED", "EXPIRED"}:
                    db.execute("UPDATE oms_orders SET status='UNKNOWN',updated_at_ns=? WHERE client_order_id=?",
                               (now_ns, row["client_order_id"]))
                db.commit()
            except Exception:
                db.rollback()
                raise

    def record_definitive_rejection(self, outbox_id: str, *, now_ns: int, broker_reason_code: str) -> None:
        if not broker_reason_code:
            raise ValueError("broker_reason_code is required")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                row = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (outbox_id,)).fetchone()
                if not row or row["state"] not in {"CLAIMED", "DISPATCHING"}:
                    raise OutboxError("only a claimed or dispatching command can be rejected")
                self._event(db, "BROKER_COMMAND_REJECTED", row["client_order_id"],
                            {"outbox_id": outbox_id, "command_type": row["command_type"],
                             "reason_code": broker_reason_code}, now_ns)
                db.execute("UPDATE oms_outbox SET state='REJECTED',lease_owner=NULL,lease_until_ns=NULL WHERE outbox_id=?",
                           (outbox_id,))
                order_row = db.execute("SELECT status,filled_quantity,quantity FROM oms_orders WHERE client_order_id=?",
                                       (row["client_order_id"],)).fetchone()
                if row["command_type"] == "SUBMIT":
                    rejected_status = "REJECTED"
                elif order_row["status"] in {"FILLED", "CANCELLED", "REJECTED", "EXPIRED"}:
                    # A cancel rejection may race with an already-applied fill or
                    # terminal broker event. Never roll a terminal order backwards.
                    rejected_status = order_row["status"]
                elif order_row["filled_quantity"] >= order_row["quantity"]:
                    rejected_status = "FILLED"
                else:
                    rejected_status = "PARTIALLY_FILLED" if order_row["filled_quantity"] else "WORKING"
                db.execute("UPDATE oms_orders SET status=?,updated_at_ns=? WHERE client_order_id=?",
                           (rejected_status, now_ns, row["client_order_id"]))
                db.commit()
            except Exception:
                db.rollback()
                raise

    def apply_broker_event(self, *, broker: str, client_order_id: str, event_id: str,
                           status: str, sequence: int, fill_quantity: int,
                           fill_price: str | None, execution_id: str | None,
                           broker_order_id: str, now_ns: int) -> DurableOrder:
        if not all(isinstance(x, str) and x for x in (broker, client_order_id, event_id, status, broker_order_id)):
            raise ValueError("broker event identity fields are required")
        if type(sequence) is not int or sequence <= 0 or type(fill_quantity) is not int or fill_quantity < 0:
            raise ValueError("broker event sequence/fill quantity is invalid")
        if fill_quantity and (not execution_id or fill_price is None):
            raise ValueError("fill event requires execution id and price")
        if not fill_quantity and (execution_id is not None or fill_price is not None):
            raise ValueError("zero-fill event must not carry execution data")
        if status == "PARTIALLY_FILLED" and fill_quantity == 0:
            # Cumulative partial status may legitimately repeat with zero delta,
            # but a no-change update is represented as WORKING by this contract.
            raise ValueError("PARTIALLY_FILLED broker event must carry a positive fill delta")
        payload = {"broker": broker, "client_order_id": client_order_id, "event_id": event_id,
                   "status": status, "sequence": sequence, "fill_quantity": fill_quantity,
                   "fill_price": fill_price, "execution_id": execution_id,
                   "broker_order_id": broker_order_id}
        digest = hashlib.sha256(canonical_bytes(payload)).hexdigest()
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                old_event = db.execute("SELECT digest,client_order_id FROM oms_broker_events WHERE broker=? AND event_id=?",
                                       (broker, event_id)).fetchone()
                if old_event:
                    if old_event["digest"] != digest or old_event["client_order_id"] != client_order_id:
                        raise IdempotencyConflict("broker event id replayed with different content")
                    row = db.execute("SELECT * FROM oms_orders WHERE client_order_id=?", (client_order_id,)).fetchone()
                    db.commit()
                    return self._row_order(row)
                row = db.execute("SELECT * FROM oms_orders WHERE client_order_id=?", (client_order_id,)).fetchone()
                if not row or row["broker"] != broker or row["broker_order_id"] != broker_order_id:
                    raise OutboxError("broker event does not resolve to the expected broker order")
                if sequence <= row["last_update_sequence"]:
                    raise OutboxError("broker event sequence must be strictly increasing")
                allowed_transitions = {
                    "PENDING_NEW": {"WORKING", "PARTIALLY_FILLED", "FILLED", "CANCELLED", "REJECTED", "EXPIRED", "UNKNOWN"},
                    "WORKING": {"WORKING", "PARTIALLY_FILLED", "FILLED", "CANCEL_PENDING", "CANCELLED", "REJECTED", "EXPIRED", "UNKNOWN"},
                    "PARTIALLY_FILLED": {"PARTIALLY_FILLED", "FILLED", "CANCEL_PENDING", "CANCELLED", "EXPIRED", "UNKNOWN"},
                    "CANCEL_PENDING": {"CANCEL_PENDING", "WORKING", "PARTIALLY_FILLED", "FILLED", "CANCELLED", "UNKNOWN"},
                    "UNKNOWN": {"WORKING", "PARTIALLY_FILLED", "FILLED", "CANCEL_PENDING", "CANCELLED", "REJECTED", "EXPIRED", "UNKNOWN"},
                    "FILLED": {"FILLED"}, "CANCELLED": {"CANCELLED"},
                    "REJECTED": {"REJECTED"}, "EXPIRED": {"EXPIRED"},
                }
                previous_status = row["status"]
                if status not in allowed_transitions.get(previous_status, set()):
                    raise OutboxError(f"invalid broker order status transition {previous_status} -> {status}")
                filled = row["filled_quantity"] + fill_quantity
                if filled > row["quantity"]:
                    raise OutboxError("broker reported an overfill")
                if fill_quantity:
                    try:
                        price = Decimal(fill_price or "")
                    except Exception as exc:
                        raise OutboxError("broker fill price is invalid") from exc
                    if not price.is_finite() or price <= 0:
                        raise OutboxError("broker fill price must be positive finite")
                    old_value = Decimal(row["average_fill_price"] or "0") * row["filled_quantity"]
                    average = (old_value + price * fill_quantity) / filled
                    average_text = format(average, "f")
                else:
                    average_text = row["average_fill_price"]
                if status not in {"WORKING", "PARTIALLY_FILLED", "FILLED", "CANCEL_PENDING", "CANCELLED", "REJECTED", "EXPIRED", "UNKNOWN"}:
                    raise OutboxError("unsupported broker order status")
                if status == "FILLED" and filled != row["quantity"]:
                    raise OutboxError("FILLED requires cumulative fill equal to order quantity")
                if status == "PARTIALLY_FILLED" and not (0 < filled < row["quantity"]):
                    raise OutboxError("PARTIALLY_FILLED requires a strict partial cumulative fill")
                if status == "WORKING" and filled != 0:
                    raise OutboxError("WORKING cannot contain executed quantity; use PARTIALLY_FILLED")
                if status == "CANCEL_PENDING" and filled >= row["quantity"]:
                    raise OutboxError("CANCEL_PENDING requires an unfilled remainder")
                if status == "REJECTED" and filled != 0:
                    raise OutboxError("REJECTED cannot contain executed quantity")
                if status == "CANCELLED" and filled > row["quantity"]:
                    raise OutboxError("CANCELLED cannot contain overfilled quantity")
                if execution_id:
                    try:
                        db.execute("INSERT INTO oms_execution_ids(broker,execution_id,client_order_id,created_at_ns) VALUES(?,?,?,?)",
                                   (broker, execution_id, client_order_id, now_ns))
                    except sqlite3.IntegrityError as exc:
                        raise IdempotencyConflict("execution_id was reused") from exc
                self._event(db, "BROKER_EVENT_APPLIED", client_order_id, payload, now_ns)
                db.execute("INSERT INTO oms_broker_events(broker,event_id,digest,client_order_id,created_at_ns) VALUES(?,?,?,?,?)",
                           (broker, event_id, digest, client_order_id, now_ns))
                db.execute("UPDATE oms_orders SET status=?,filled_quantity=?,average_fill_price=?,last_update_sequence=?,updated_at_ns=? WHERE client_order_id=?",
                           (status, filled, average_text, sequence, now_ns, client_order_id))
                result = db.execute("SELECT * FROM oms_orders WHERE client_order_id=?", (client_order_id,)).fetchone()
                db.commit()
                return self._row_order(result)
            except Exception:
                db.rollback()
                raise

    def order(self, client_order_id: str) -> DurableOrder | None:
        with self._db() as db:
            row = db.execute("SELECT * FROM oms_orders WHERE client_order_id=?", (client_order_id,)).fetchone()
            return None if row is None else self._row_order(row)

    def outbox(self, outbox_id: str) -> OutboxCommand | None:
        with self._db() as db:
            row = db.execute("SELECT * FROM oms_outbox WHERE outbox_id=?", (outbox_id,)).fetchone()
            return None if row is None else self._row_outbox(row)

    def verify_event_chain(self) -> dict[str, Any]:
        with self._db() as db:
            rows = db.execute("SELECT * FROM oms_events ORDER BY sequence").fetchall()
        previous = "0" * 64
        for expected_seq, row in enumerate(rows, 1):
            if row["sequence"] != expected_seq or row["previous_hash"] != previous:
                raise OmsError(f"OMS event chain sequence/predecessor mismatch at {expected_seq}")
            payload = json.loads(row["payload_json"])
            body = {"event_id": row["event_id"], "event_type": row["event_type"],
                    "entity_id": row["entity_id"], "timestamp_ns": row["timestamp_ns"],
                    "payload": payload, "previous_hash": row["previous_hash"]}
            digest = hashlib.sha256(canonical_bytes(body)).hexdigest()
            if digest != row["record_hash"]:
                raise OmsError(f"OMS event record hash mismatch at {expected_seq}")
            previous = digest
        return {"status": "verified_internal_integrity", "event_count": len(rows),
                "last_hash": previous, "signature": False}

    @staticmethod
    def _row_order(row: sqlite3.Row) -> DurableOrder:
        return DurableOrder(row["client_order_id"], row["broker"], row["status"],
                            row["broker_order_id"], row["quantity"], row["filled_quantity"],
                            row["average_fill_price"], json.loads(row["intent_json"]))

    @staticmethod
    def _row_outbox(row: sqlite3.Row) -> OutboxCommand:
        return OutboxCommand(row["outbox_id"], row["client_order_id"], row["broker"],
                             row["command_type"], json.loads(row["payload_json"]), row["state"],
                             row["attempts"], row["lease_owner"], row["lease_until_ns"])


class OutboxBroker(Protocol):
    broker_id: str
    def submit(self, intent: BrokerOrderIntent, *, client_order_id: str) -> str: ...
    def cancel(self, broker_order_id: str, *, client_order_id: str) -> None: ...


class PaperOutboxDispatcher:
    """One-shot dispatcher for paper adapters; live mode is always refused."""

    def __init__(self, store: DurableOMSStore, adapter: OutboxBroker, *, mode: str = "paper"):
        if mode not in {"paper", "live"}:
            raise ValueError("mode must be paper or live")
        self.store, self.adapter, self.mode = store, adapter, mode

    def dispatch_one(self, *, worker_id: str, now_ns: int) -> DurableOrder | None:
        if self.mode == "live":
            raise OutboxError("LIVE_ROUTING_HARD_DISABLED: no live dispatcher is admitted in this build")
        command = self.store.claim_next(worker_id=worker_id, now_ns=now_ns)
        if command is None:
            return None
        if command.broker != self.adapter.broker_id:
            self.store.record_definitive_rejection(command.outbox_id, now_ns=now_ns,
                                                   broker_reason_code="adapter_mismatch")
            raise OutboxError("outbox command does not match the configured broker adapter")
        self.store.mark_dispatch_started(command.outbox_id, worker_id=worker_id, now_ns=now_ns)
        if command.command_type == "CANCEL":
            broker_order_id = command.payload.get("broker_order_id")
            client_order_id = command.client_order_id
            if not isinstance(broker_order_id, str) or not broker_order_id:
                self.store.record_definitive_rejection(command.outbox_id, now_ns=now_ns + 1,
                                                       broker_reason_code="missing_broker_order_id")
                raise OutboxError("cancel command has no broker_order_id")
            try:
                self.adapter.cancel(broker_order_id, client_order_id=client_order_id)
            except Exception as exc:
                self.store.mark_unknown(command.outbox_id, now_ns=now_ns + 1,
                                        reason_code=type(exc).__name__)
            else:
                self.store.acknowledge_cancel_request(command.outbox_id, now_ns=now_ns + 1)
            return self.store.order(client_order_id)
        if command.command_type != "SUBMIT":
            self.store.record_definitive_rejection(command.outbox_id, now_ns=now_ns + 1,
                                                   broker_reason_code="unsupported_command")
            raise OutboxError("unsupported outbox command")
        intent_raw = dict(command.payload)
        intent_raw["limit_price"] = None if intent_raw["limit_price"] is None else Decimal(intent_raw["limit_price"])
        intent = BrokerOrderIntent(**intent_raw)
        intent.validate()
        try:
            broker_order_id = self.adapter.submit(intent, client_order_id=intent.client_order_id)
        except Exception as exc:
            # Once a broker call might have started, an arbitrary exception is
            # ambiguous. Never auto-retry; require explicit reconciliation.
            self.store.mark_unknown(command.outbox_id, now_ns=now_ns + 1,
                                    reason_code=type(exc).__name__)
            return self.store.order(intent.client_order_id)
        return self.store.acknowledge_submission(command.outbox_id, broker_order_id, now_ns=now_ns + 1)
