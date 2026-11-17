"""Broker-neutral OMS contract and append-only local event journal.

The included implementation is a deterministic state-machine/reference OMS.
It has no live broker adapter. Live submission always fails closed in this
release; the signed permit verifier is an additional gate, not a live switch.
"""
from __future__ import annotations

import fcntl
import copy
import hashlib
import json
import os
from contextlib import contextmanager
from dataclasses import asdict, dataclass, field
from decimal import Decimal
from pathlib import Path
from typing import Any, Protocol

from .market import canonical_bytes
from .live_auth import VerifiedAuthorization


class OmsError(RuntimeError): pass
class IdempotencyConflict(OmsError): pass
class LiveRoutingDisabled(OmsError): pass
class InvalidTransition(OmsError): pass


@dataclass(frozen=True, slots=True)
class BrokerOrderIntent:
    client_order_id: str
    strategy_id: str
    instrument_id: str
    venue: str
    side: str
    order_type: str
    product: str
    quantity: int
    limit_price: Decimal | None
    time_in_force: str

    def validate(self) -> None:
        if not all(isinstance(x, str) and x.strip() for x in
                   (self.client_order_id, self.strategy_id, self.instrument_id, self.venue, self.product)):
            raise ValueError("order identity and routing fields are required")
        if self.side not in {"buy", "sell"} or self.order_type not in {"market", "limit"}:
            raise ValueError("reference OMS supports only explicit market and limit order contracts")
        if type(self.quantity) is not int or self.quantity <= 0 or self.time_in_force not in {"day", "ioc", "fok", "gtc"}:
            raise ValueError("invalid quantity or time in force")
        if self.order_type == "limit" and (not isinstance(self.limit_price, Decimal) or
                not self.limit_price.is_finite() or self.limit_price <= 0):
            raise ValueError("limit order requires a positive finite Decimal limit price")
        if self.limit_price is not None and (not isinstance(self.limit_price, Decimal) or
                not self.limit_price.is_finite() or self.limit_price <= 0):
            raise ValueError("limit price must be a positive finite Decimal")
        if self.order_type == "market" and self.limit_price is not None:
            raise ValueError("market order must not carry a limit price")


@dataclass(slots=True)
class OrderProjection:
    intent: BrokerOrderIntent
    broker: str
    status: str = "pending_new"
    broker_order_id: str | None = None
    filled_quantity: int = 0
    average_fill_price: Decimal | None = None
    execution_ids: set[str] = field(default_factory=set)
    last_update_sequence: int = 0
    cancel_request_id: str | None = None
    last_broker_event_id: str | None = None
    broker_event_digests: dict[str, str] = field(default_factory=dict)

    @property
    def remaining_quantity(self) -> int:
        return self.intent.quantity - self.filled_quantity


class BrokerAdapter(Protocol):
    """Required adapter boundary; implementations must declare capabilities."""
    broker_id: str
    capabilities: dict[str, Any]
    def submit(self, intent: BrokerOrderIntent) -> str: ...
    def cancel(self, broker_order_id: str) -> None: ...
    def query_order(self, broker_order_id: str) -> dict[str, Any]: ...


class HashChainJournal:
    def __init__(self, path: str | Path):
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        if os.name == "posix": os.chmod(self.path.parent, 0o700)
        self.path.touch(mode=0o600, exist_ok=True)
        if os.name == "posix": os.chmod(self.path, 0o600)
        self.lock_path = self.path.with_name(self.path.name + ".lock")
        self.lock_path.touch(mode=0o600, exist_ok=True)
        if os.name == "posix": os.chmod(self.lock_path, 0o600)
        self._last_hash = "0" * 64
        self._sequence = 0
        for record in self.read_verified():
            self._last_hash = record["record_hash"]
            self._sequence = record["sequence"]

    @contextmanager
    def transaction(self):
        """Serialize OMS projection-check-append transactions across processes."""
        with self.lock_path.open("a+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock.fileno(), fcntl.LOCK_UN)

    def read_verified(self) -> list[dict[str, Any]]:
        output: list[dict[str, Any]] = []
        previous = "0" * 64
        if not self.path.exists(): return output
        raw_bytes = self.path.read_bytes()
        if raw_bytes and not raw_bytes.endswith(b"\n"):
            raise OmsError("journal has a torn final record; manual recovery is required")
        for line_number, line in enumerate(raw_bytes.splitlines(), 1):
            try: row = json.loads(line)
            except json.JSONDecodeError as exc: raise OmsError(f"journal line {line_number} is malformed") from exc
            if not isinstance(row, dict) or set(row) != {"schema_version", "sequence", "timestamp_ns", "event_type", "payload", "previous_hash", "record_hash"}:
                raise OmsError(f"journal line {line_number} has an invalid event envelope")
            if (type(row.get("schema_version")) is not int or row.get("schema_version") != 1 or
                    type(row.get("sequence")) is not int or row["sequence"] != line_number):
                raise OmsError(f"journal sequence/schema mismatch at line {line_number}")
            if type(row.get("timestamp_ns")) is not int or row["timestamp_ns"] <= 0:
                raise OmsError(f"journal timestamp is invalid at line {line_number}")
            if not isinstance(row.get("event_type"), str) or not row["event_type"] or not isinstance(row.get("payload"), dict):
                raise OmsError(f"journal event body is invalid at line {line_number}")
            if row.get("previous_hash") != previous:
                raise OmsError(f"journal hash chain breaks at line {line_number}")
            expected = hashlib.sha256(canonical_bytes({k: v for k, v in row.items() if k != "record_hash"})).hexdigest()
            if row.get("record_hash") != expected:
                raise OmsError(f"journal digest mismatch at line {line_number}")
            output.append(row); previous = expected
        return output

    def append(self, event_type: str, payload: dict[str, Any], timestamp_ns: int) -> dict[str, Any]:
        if not event_type or not isinstance(payload, dict) or type(timestamp_ns) is not int or timestamp_ns <= 0:
            raise ValueError("journal event type, object payload and positive timestamp are required")
        with self.path.open("ab") as stream:
            fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
            # Re-read under the file lock: more than one process may own an OMS
            # object, so cached sequence/hash values are not authoritative.
            latest = self.read_verified()
            self._sequence = latest[-1]["sequence"] if latest else 0
            self._last_hash = latest[-1]["record_hash"] if latest else "0" * 64
            self._sequence += 1
            body = {"schema_version": 1, "sequence": self._sequence,
                    "timestamp_ns": timestamp_ns, "event_type": event_type,
                    "payload": payload, "previous_hash": self._last_hash}
            body["record_hash"] = hashlib.sha256(canonical_bytes(body)).hexdigest()
            stream.write(canonical_bytes(body) + b"\n"); stream.flush(); os.fsync(stream.fileno())
            fcntl.flock(stream.fileno(), fcntl.LOCK_UN)
            self._last_hash = body["record_hash"]
            return body


class OrderManagementSystem:
    def __init__(self, journal_path: str | Path, *, mode: str = "paper"):
        if mode not in {"paper", "simulated", "live"}: raise ValueError("invalid OMS mode")
        self.mode = mode
        self.journal = HashChainJournal(journal_path)
        self.orders: dict[str, OrderProjection] = {}
        with self.journal.transaction():
            self._restore()

    def _restore(self) -> None:
        # Reconstruct only lifecycle events written by this module.
        self.orders.clear()
        for record in self.journal.read_verified():
            payload = record["payload"]
            if record["event_type"] == "intent_accepted":
                raw = payload["intent"]
                raw["limit_price"] = None if raw["limit_price"] is None else Decimal(raw["limit_price"])
                if not isinstance(raw, dict) or set(raw) != set(BrokerOrderIntent.__dataclass_fields__):
                    raise OmsError("journal contains an invalid order intent contract")
                intent = BrokerOrderIntent(**raw)
                intent.validate()
                broker = payload.get("broker")
                if not isinstance(broker, str) or not broker.strip():
                    raise OmsError("journal contains an invalid broker identity")
                if intent.client_order_id in self.orders:
                    raise OmsError("journal reuses a client_order_id")
                self.orders[intent.client_order_id] = OrderProjection(intent, broker)
            elif record["event_type"] == "submission_acknowledged":
                order = self.orders.get(payload["client_order_id"])
                if order is None: raise OmsError("submit acknowledgement precedes intent")
                broker_order_id = payload.get("broker_order_id")
                if not isinstance(broker_order_id, str) or not broker_order_id:
                    raise OmsError("journal contains an invalid submit acknowledgement")
                if order.status not in {"pending_new", "unknown"}:
                    raise OmsError("journal contains an invalid submit acknowledgement transition")
                if any(other.broker == order.broker and other.broker_order_id == broker_order_id
                       for other in self.orders.values() if other is not order):
                    raise OmsError("journal reuses a broker order id")
                order.status = "working"
                order.broker_order_id = broker_order_id
            elif record["event_type"] == "submission_unknown":
                order = self.orders.get(payload["client_order_id"])
                if order is None: raise OmsError("unknown submission precedes intent")
                if order.status != "pending_new":
                    raise OmsError("journal contains an invalid unknown submission transition")
                order.status = "unknown"
            elif record["event_type"] == "broker_update":
                order = self.orders.get(payload["client_order_id"])
                if order is None: raise OmsError("broker update precedes intent")
                event_id = payload.get("event_id")
                if any(other.broker == order.broker and event_id in other.broker_event_digests
                       for other in self.orders.values() if other is not order):
                    raise OmsError("journal reuses a broker event id across orders")
                self._apply_update(order, payload, restore=True)
            elif record["event_type"] == "cancel_requested":
                order = self.orders.get(payload["client_order_id"])
                if order is None: raise OmsError("cancel request precedes intent")
                if order.status not in {"working", "partially_filled", "pending_cancel"}:
                    raise OmsError("journal contains an invalid cancel request transition")
                if order.cancel_request_id and order.cancel_request_id != payload["cancel_request_id"]:
                    raise OmsError("journal contains conflicting cancel request ids")
                order.cancel_request_id = payload["cancel_request_id"]
                order.status = "pending_cancel"
            else:
                raise OmsError(f"journal contains unsupported event type: {record['event_type']}")

    def submit(self, intent: BrokerOrderIntent, *, broker: str,
               now_ns: int, adapter: BrokerAdapter | None = None,
               authorization: VerifiedAuthorization | None = None) -> OrderProjection:
        intent.validate()
        if not broker:
            raise ValueError("broker id is required")
        with self.journal.transaction():
            self._restore()  # Refresh the projection while holding the cross-process lock.
            existing = self.orders.get(intent.client_order_id)
            if existing:
                if existing.intent != intent or existing.broker != broker:
                    raise IdempotencyConflict("client_order_id was reused with a different order contract")
                return existing
            if self.mode == "live":
                # No live adapter has been independently certified in this repository.
                # A valid permit is necessary but not sufficient.
                if authorization is None:
                    raise LiveRoutingDisabled("live order requires independent signed authorisation")
                raise LiveRoutingDisabled("live routing is not implemented/certified; permit cannot enable it")
            if adapter is not None:
                raise OmsError("broker adapter side effects are not enabled; this OMS is a paper/reference state machine")
            payload = {"intent": {**asdict(intent), "limit_price": None if intent.limit_price is None else str(intent.limit_price)},
                       "broker": broker}
            self.journal.append("intent_accepted", payload, now_ns)
            order = OrderProjection(intent, broker)
            self.orders[intent.client_order_id] = order
            return order

    def mark_submitted(self, client_order_id: str, broker_order_id: str, *, now_ns: int) -> None:
        if not broker_order_id:
            raise InvalidTransition("broker order id is required")
        with self.journal.transaction():
            self._restore()
            order = self._get(client_order_id)
            if order.status == "working" and order.broker_order_id == broker_order_id:
                return  # Idempotent replay of the same transport acknowledgement.
            if order.status not in {"pending_new", "unknown"}:
                raise InvalidTransition("invalid submit acknowledgement")
            if any(other.broker == order.broker and other.broker_order_id == broker_order_id
                   for other in self.orders.values() if other is not order):
                raise IdempotencyConflict("broker_order_id is already bound to another client order")
            self.journal.append("submission_acknowledged", {
                "client_order_id": client_order_id, "broker_order_id": broker_order_id,
            }, now_ns)
            order.status = "working"
            order.broker_order_id = broker_order_id

    def mark_submission_unknown(self, client_order_id: str, *, now_ns: int) -> None:
        """Record an indeterminate submit outcome; caller must reconcile, never resend blindly."""
        with self.journal.transaction():
            self._restore()
            order = self._get(client_order_id)
            if order.status == "unknown":
                return
            if order.status != "pending_new":
                raise InvalidTransition("only an unacknowledged new order can become unknown")
            self.journal.append("submission_unknown", {"client_order_id": client_order_id}, now_ns)
            order.status = "unknown"

    def request_cancel(self, client_order_id: str, *, cancel_request_id: str, now_ns: int) -> OrderProjection:
        """Durably record cancel intent before any adapter is allowed to send it."""
        if not cancel_request_id:
            raise ValueError("cancel_request_id is required for idempotency")
        with self.journal.transaction():
            self._restore()
            order = self._get(client_order_id)
            if order.cancel_request_id == cancel_request_id:
                return order
            if order.status not in {"working", "partially_filled"} or not order.broker_order_id:
                raise InvalidTransition("only a broker-acknowledged working order can be cancelled")
            if order.cancel_request_id is not None:
                raise IdempotencyConflict("a different cancel request is already pending")
            payload = {"client_order_id": client_order_id, "cancel_request_id": cancel_request_id,
                       "broker_order_id": order.broker_order_id, "expected_update_sequence": order.last_update_sequence}
            self.journal.append("cancel_requested", payload, now_ns)
            order.cancel_request_id = cancel_request_id
            order.status = "pending_cancel"
            return order

    def apply_broker_update(self, client_order_id: str, update: dict[str, Any], *, now_ns: int) -> None:
        required = {"event_id", "status", "execution_id", "last_fill_quantity", "last_fill_price", "sequence", "broker_order_id"}
        if set(update) != required: raise ValueError("broker update fields do not match the adapter contract")
        if not isinstance(update["event_id"], str) or not update["event_id"]:
            raise ValueError("broker update requires a stable event_id")
        with self.journal.transaction():
            self._restore()
            order = self._get(client_order_id)
            event_digest = hashlib.sha256(canonical_bytes(update)).hexdigest()
            prior_digest = order.broker_event_digests.get(update["event_id"])
            if prior_digest is not None:
                if prior_digest != event_digest:
                    raise IdempotencyConflict("broker event_id was replayed with a different payload")
                return  # Exact same broker event is idempotent, even after later events.
            if any(x.broker == order.broker and update["event_id"] in x.broker_event_digests
                   for x in self.orders.values() if x is not order):
                raise IdempotencyConflict("broker event_id was reused across orders")
            broker_order_id = update["broker_order_id"]
            if not isinstance(broker_order_id, str) or not broker_order_id:
                raise InvalidTransition("broker update must be bound to a broker order id")
            if any(x.broker == order.broker and x.broker_order_id == broker_order_id
                   for x in self.orders.values() if x is not order):
                raise IdempotencyConflict("broker_order_id is already bound to another client order")
            payload = {"client_order_id": client_order_id, **update}
            self._update(order, payload, now_ns)

    def _update(self, order: OrderProjection, payload: dict[str, Any], now_ns: int) -> None:
        # Validate on a detached projection, fsync the event, then publish the
        # new projection. A failed commit cannot create an in-memory success.
        candidate = copy.deepcopy(order)
        self._apply_update(candidate, payload, restore=False)
        self.journal.append("broker_update", payload, now_ns)
        order.status = candidate.status
        order.broker_order_id = candidate.broker_order_id
        order.filled_quantity = candidate.filled_quantity
        order.average_fill_price = candidate.average_fill_price
        order.execution_ids = candidate.execution_ids
        order.last_update_sequence = candidate.last_update_sequence
        order.last_broker_event_id = candidate.last_broker_event_id
        order.broker_event_digests = candidate.broker_event_digests
        order.cancel_request_id = candidate.cancel_request_id

    @staticmethod
    def _apply_update(order: OrderProjection, payload: dict[str, Any], *, restore: bool) -> None:
        sequence = payload["sequence"]
        if type(sequence) is not int or sequence <= order.last_update_sequence:
            raise InvalidTransition("broker update sequence must be strictly increasing")
        event_id = payload.get("event_id")
        if not isinstance(event_id, str) or not event_id:
            raise InvalidTransition("broker update requires a stable event_id")
        if event_id in order.broker_event_digests:
            raise InvalidTransition("duplicate broker event id in journal")
        status = payload["status"]
        allowed = {"working", "partially_filled", "filled", "pending_cancel", "cancelled", "rejected", "expired", "unknown"}
        if status not in allowed: raise InvalidTransition("unknown broker order status")
        transitions = {
            "pending_new": {"working", "rejected", "unknown"},
            "working": {"working", "partially_filled", "filled", "pending_cancel", "cancelled", "rejected", "expired", "unknown"},
            "partially_filled": {"partially_filled", "filled", "pending_cancel", "cancelled", "expired", "unknown"},
            # A broker may reject a cancel request while the original order remains live.
            "pending_cancel": {"working", "partially_filled", "pending_cancel", "filled", "cancelled", "unknown"},
            "unknown": {"working", "partially_filled", "filled", "pending_cancel", "cancelled", "rejected", "expired", "unknown"},
        }
        if status not in transitions.get(order.status, set()):
            raise InvalidTransition(f"invalid order transition {order.status} -> {status}")
        qty = payload.get("last_fill_quantity", 0)
        execution_id = payload.get("execution_id")
        if type(qty) is not int or qty < 0: raise InvalidTransition("invalid fill quantity")
        if qty:
            if not isinstance(execution_id, str) or not execution_id: raise InvalidTransition("fill requires unique execution_id")
            if execution_id in order.execution_ids: raise InvalidTransition("duplicate execution_id")
            if qty > order.remaining_quantity: raise InvalidTransition("broker overfill exceeds order quantity")
            price = Decimal(str(payload["last_fill_price"]))
            if not price.is_finite() or price <= 0: raise InvalidTransition("fill price must be positive and finite")
            old_value = (order.average_fill_price or Decimal(0)) * order.filled_quantity
            order.filled_quantity += qty
            order.average_fill_price = (old_value + price * qty) / order.filled_quantity
            order.execution_ids.add(execution_id)
        elif payload.get("last_fill_price") is not None:
            raise InvalidTransition("fill price without fill quantity")
        elif execution_id is not None:
            raise InvalidTransition("execution id without fill quantity")
        if status == "filled" and order.remaining_quantity != 0:
            raise InvalidTransition("filled state requires the full original quantity")
        if status == "partially_filled" and not (0 < order.filled_quantity < order.intent.quantity):
            raise InvalidTransition("partially_filled state requires a strict partial fill")
        if order.filled_quantity == order.intent.quantity and status not in {"filled", "unknown"}:
            raise InvalidTransition("fully executed order must be marked filled")
        broker_order_id = payload.get("broker_order_id")
        if not isinstance(broker_order_id, str) or not broker_order_id:
            raise InvalidTransition("broker update must include broker_order_id")
        if order.broker_order_id and order.broker_order_id != broker_order_id:
            raise InvalidTransition("broker order id cannot change")
        if order.status == "pending_cancel" and status in {"working", "partially_filled"}:
            # A broker's working/partial status after a cancel request is a
            # cancel reject or still-open acknowledgement; a new cancel id may
            # be used after this explicit response.
            order.cancel_request_id = None
        order.broker_order_id = broker_order_id
        order.last_broker_event_id = event_id
        order.broker_event_digests[event_id] = hashlib.sha256(canonical_bytes({
            key: payload[key] for key in ("event_id", "status", "execution_id", "last_fill_quantity",
                                          "last_fill_price", "sequence", "broker_order_id")
        })).hexdigest()
        order.status = status
        order.last_update_sequence = sequence

    def _get(self, client_order_id: str) -> OrderProjection:
        try: return self.orders[client_order_id]
        except KeyError as exc: raise OmsError("unknown client order id") from exc
