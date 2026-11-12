from __future__ import annotations

import csv
import hashlib
import json
import os
import uuid
from dataclasses import dataclass
from datetime import date, datetime, time, timezone
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo

from .client import KiteClient
from .errors import KiteOpsError, KiteRejected, KiteUnknownOutcome, RiskRejected
from .journal import LockedHashJournal

IST = ZoneInfo("Asia/Kolkata")
_ABSOLUTE_MAX_NOTIONAL = Decimal("1000")
_ABSOLUTE_MAX_QUANTITY = 100
_OPEN_STATUSES = {
    "OPEN", "OPEN PENDING", "VALIDATION PENDING", "TRIGGER PENDING",
    "MODIFY PENDING", "MODIFY VALIDATION PENDING", "CANCEL PENDING",
    "PUT ORDER REQ RECEIVED", "AMO REQ RECEIVED",
}
_TERMINAL_STATUSES = {"COMPLETE", "CANCELLED", "REJECTED", "EXPIRED"}


def _sync_directory(path: Path) -> None:
    directory_fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)


def _create_file_atomically(path: Path, payload: bytes) -> None:
    """Publish a private file without replacing an existing path."""
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    fd = os.open(temporary, flags, 0o600 if os.name == "posix" else 0o666)
    try:
        with os.fdopen(fd, "wb", closefd=True) as target:
            target.write(payload)
            target.flush()
            os.fsync(target.fileno())
        # link() fails if the destination exists; os.replace() would permit a
        # race to replace another operator's instrument master.
        os.link(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
    _sync_directory(path.parent)


@dataclass(frozen=True)
class KitePolicy:
    expected_user_id: str
    allowed_instruments: tuple[str, ...]
    live_order_enabled: bool
    static_ip_verified_for_ist_date: str
    daily_auth_verified_for_ist_date: str
    market_session_verified_for_ist_date: str
    max_order_notional_inr: Decimal
    max_quantity: int

    @classmethod
    def load(cls, path: Path) -> "KitePolicy":
        raw = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(raw, dict):
            raise KiteOpsError("Kite policy must be a JSON object")
        allowed = {
            "schema_version", "expected_user_id", "allowed_instruments",
            "live_order_enabled", "static_ip_verified_for_ist_date", "daily_auth_verified_for_ist_date",
            "market_session_verified_for_ist_date", "max_order_notional_inr", "max_quantity", "market", "exchange",
            "product", "order_type", "validity", "variety",
        }
        unknown = set(raw) - allowed
        missing = allowed - set(raw)
        if unknown or missing:
            raise KiteOpsError(f"Kite policy fields invalid; missing={sorted(missing)}, unknown={sorted(unknown)}")
        if type(raw["schema_version"]) is not int or raw["schema_version"] != 1:
            raise KiteOpsError("unsupported Kite policy schema_version")
        if raw["market"] != "NSE_CASH" or raw["exchange"] != "NSE":
            raise KiteOpsError("manual order tool is restricted to NSE cash equity")
        if raw["product"] != "CNC" or raw["order_type"] != "LIMIT" or raw["validity"] != "DAY" or raw["variety"] != "regular":
            raise KiteOpsError("manual order tool permits only regular CNC BUY LIMIT DAY")
        expected = raw["expected_user_id"]
        if not isinstance(expected, str):
            raise KiteOpsError("expected_user_id must be a string")
        allowed_instruments = raw["allowed_instruments"]
        if not isinstance(allowed_instruments, list) or any(not isinstance(s, str) or not s for s in allowed_instruments):
            raise KiteOpsError("allowed_instruments must be a list of symbols")
        if not isinstance(raw["max_order_notional_inr"], str):
            raise KiteOpsError("max_order_notional_inr must be a decimal string")
        try:
            notional = Decimal(raw["max_order_notional_inr"])
        except InvalidOperation as error:
            raise KiteOpsError("max_order_notional_inr must be a decimal string") from error
        if not notional.is_finite() or notional <= 0 or notional > _ABSOLUTE_MAX_NOTIONAL:
            raise KiteOpsError("max_order_notional_inr must be in (0, 1000]")
        quantity = raw["max_quantity"]
        if not isinstance(quantity, int) or isinstance(quantity, bool) or not 1 <= quantity <= _ABSOLUTE_MAX_QUANTITY:
            raise KiteOpsError("max_quantity must be in [1, 100]")
        if not isinstance(raw["live_order_enabled"], bool):
            raise KiteOpsError("live_order_enabled must be boolean")
        for field in (
            "static_ip_verified_for_ist_date",
            "daily_auth_verified_for_ist_date",
            "market_session_verified_for_ist_date",
        ):
            value = raw[field]
            if not isinstance(value, str):
                raise KiteOpsError(f"{field} must be an ISO date string")
            if value:
                try:
                    parsed_date = date.fromisoformat(value)
                except ValueError as error:
                    raise KiteOpsError(f"{field} must use YYYY-MM-DD") from error
                if parsed_date.isoformat() != value:
                    raise KiteOpsError(f"{field} must use YYYY-MM-DD")
        return cls(
            expected_user_id=expected,
            allowed_instruments=tuple(sorted(set(allowed_instruments))),
            live_order_enabled=raw["live_order_enabled"],
            static_ip_verified_for_ist_date=raw["static_ip_verified_for_ist_date"],
            daily_auth_verified_for_ist_date=raw["daily_auth_verified_for_ist_date"],
            market_session_verified_for_ist_date=raw["market_session_verified_for_ist_date"],
            max_order_notional_inr=notional,
            max_quantity=quantity,
        )


@dataclass(frozen=True)
class Instrument:
    tradingsymbol: str
    instrument_token: str
    instrument_type: str
    segment: str
    tick_size: Decimal
    lot_size: int


@dataclass(frozen=True)
class OrderPlan:
    symbol: str
    quantity: int
    price: Decimal
    notional: Decimal
    tick_size: Decimal
    lot_size: int
    tag: str
    user_id: str


class KiteOperations:
    def __init__(self, client: KiteClient, policy: KitePolicy, data_dir: Path) -> None:
        self.client = client
        self.policy = policy
        self.data_dir = data_dir

    @staticmethod
    def _ist_now() -> datetime:
        return datetime.now(IST)

    @staticmethod
    def _money(value: Any, field: str) -> Decimal:
        try:
            parsed = Decimal(str(value))
        except (InvalidOperation, ValueError) as error:
            raise RiskRejected(f"Kite {field} is not a valid decimal") from error
        if not parsed.is_finite():
            raise RiskRejected(f"Kite {field} is not finite")
        return parsed

    def doctor(self) -> dict[str, Any]:
        profile = self.client.profile()
        margins = self.client.margins()
        orders = self.client.orders()
        positions = self.client.positions()
        user_id = profile.get("user_id")
        user_id_match = bool(self.policy.expected_user_id) and user_id == self.policy.expected_user_id
        return {
            "user_id_match": user_id_match,
            "profile_user_id": user_id,
            "equity_available_cash": margins.get("available", {}).get("cash"),
            "order_count_today": len(orders),
            "open_order_count": sum(order.get("status") in _OPEN_STATUSES for order in orders),
            "net_position_rows": len(positions.get("net", [])),
            "static_ip_verified_for_today": self.policy.static_ip_verified_for_ist_date == self._session_date_iso(),
            "daily_auth_verified_for_today": self.policy.daily_auth_verified_for_ist_date == self._session_date_iso(),
            "market_session_verified_for_today": self.policy.market_session_verified_for_ist_date == self._session_date_iso(),
            "live_order_enabled_in_policy": self.policy.live_order_enabled,
            "market_data_entitlement": "not inferred from Kite Personal order access",
        }

    def refresh_instruments(self) -> Path:
        csv_text = self.client.instruments_nse_csv()
        rows = list(csv.DictReader(csv_text.splitlines()))
        if not rows:
            raise KiteOpsError("Kite instrument master is empty")
        required = {"tradingsymbol", "instrument_token", "instrument_type", "segment", "tick_size", "lot_size", "exchange"}
        if not required.issubset(rows[0]):
            raise KiteOpsError(f"Kite instrument master lacks fields: {sorted(required - set(rows[0]))}")
        today = self._ist_now().date().isoformat()
        out_dir = self.data_dir / "instruments"
        out_dir.mkdir(parents=True, exist_ok=True)
        if os.name == "posix":
            os.chmod(out_dir, 0o700)
        csv_path = out_dir / f"kite-nse-{today}.csv"
        manifest_path = out_dir / f"kite-nse-{today}.manifest.json"
        payload = csv_text.encode("utf-8")
        digest = hashlib.sha256(payload).hexdigest()

        csv_exists = csv_path.exists()
        manifest_exists = manifest_path.exists()
        if csv_exists:
            existing = csv_path.read_bytes()
            if hashlib.sha256(existing).hexdigest() != digest:
                raise KiteOpsError("today's instrument master already exists with different content; inspect before replacing")
        if manifest_exists:
            try:
                existing_manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as error:
                raise KiteOpsError("today's instrument manifest is unreadable; inspect before replacing") from error
            if (
                existing_manifest.get("effective_session_date_ist") != today
                or existing_manifest.get("sha256") != digest
                or existing_manifest.get("row_count") != len(rows)
            ):
                raise KiteOpsError("today's instrument manifest does not match the current provider response")

        if not csv_exists:
            _create_file_atomically(csv_path, payload)
        if not manifest_exists:
            manifest = {
                "schema_version": 1,
                "provider": "Zerodha Kite Connect",
                "exchange": "NSE",
                "retrieved_at_utc": datetime.now(timezone.utc).isoformat(),
                "effective_session_date_ist": today,
                "row_count": len(rows),
                "sha256": digest,
                "source": "GET /instruments/NSE",
            }
            _create_file_atomically(
                manifest_path,
                json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode() + b"\n",
            )
        return csv_path

    def _current_instrument(self, symbol: str) -> Instrument:
        today = self._ist_now().date().isoformat()
        csv_path = self.data_dir / "instruments" / f"kite-nse-{today}.csv"
        manifest_path = self.data_dir / "instruments" / f"kite-nse-{today}.manifest.json"
        if not csv_path.exists() or not manifest_path.exists():
            raise RiskRejected("today's Kite NSE instrument master is required; run instruments-refresh")
        raw = csv_path.read_bytes()
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if hashlib.sha256(raw).hexdigest() != manifest.get("sha256"):
            raise RiskRejected("Kite instrument master hash mismatch")
        if manifest.get("effective_session_date_ist") != today:
            raise RiskRejected("Kite instrument master is not for today's IST session")
        rows = list(csv.DictReader(raw.decode("utf-8").splitlines()))
        matches = [row for row in rows if row.get("tradingsymbol") == symbol and row.get("exchange") == "NSE"]
        if len(matches) != 1:
            raise RiskRejected(f"expected one NSE instrument-master row for {symbol}, found {len(matches)}")
        row = matches[0]
        try:
            tick = Decimal(row["tick_size"])
            lot = int(row["lot_size"])
        except (KeyError, ValueError, InvalidOperation) as error:
            raise RiskRejected(f"instrument master fields invalid for {symbol}") from error
        if not tick.is_finite() or tick <= 0 or lot <= 0:
            raise RiskRejected(f"instrument tick size or lot size invalid for {symbol}")
        if row.get("instrument_type") != "EQ" or row.get("segment") not in {"NSE", "NSE-EQ"}:
            raise RiskRejected(f"{symbol} is not an NSE cash equity")
        return Instrument(symbol, row["instrument_token"], row["instrument_type"], row["segment"], tick, lot)

    @staticmethod
    def _session_date() -> str:
        return KiteOperations._ist_now().date().strftime("%Y%m%d")

    @staticmethod
    def _session_date_iso() -> str:
        return KiteOperations._ist_now().date().isoformat()

    def _order_journal(self) -> LockedHashJournal:
        return LockedHashJournal(self.data_dir / "kite-orders" / f"{self._session_date()}.jsonl")

    def plan(self, symbol: str, quantity: int, price_text: str) -> OrderPlan:
        if symbol not in self.policy.allowed_instruments:
            raise RiskRejected(f"{symbol} is not in the private allowed_instruments list")
        with self._order_journal() as journal:
            if journal.read_events():
                raise RiskRejected("a Kite order attempt is already journalled for this session; do not retry")
        if not isinstance(quantity, int) or isinstance(quantity, bool) or quantity < 1:
            raise RiskRejected("quantity must be a positive integer")
        if quantity > self.policy.max_quantity:
            raise RiskRejected("quantity exceeds the configured absolute limit")
        price = self._money(price_text, "limit price")
        if price <= 0:
            raise RiskRejected("limit price must be positive")

        instrument = self._current_instrument(symbol)
        if quantity % instrument.lot_size != 0:
            raise RiskRejected(f"quantity must be a multiple of lot_size={instrument.lot_size}")
        if price % instrument.tick_size != 0:
            raise RiskRejected(f"price must be a multiple of tick_size={instrument.tick_size}")
        notional = price * quantity
        if notional > self.policy.max_order_notional_inr or notional > _ABSOLUTE_MAX_NOTIONAL:
            raise RiskRejected("order notional exceeds the ₹1,000 absolute cap")

        profile = self.client.profile()
        user_id = profile.get("user_id")
        if not self.policy.expected_user_id or user_id != self.policy.expected_user_id:
            raise RiskRejected("authenticated Kite user_id does not match the configured private policy")
        margins = self.client.margins()
        available_raw = margins.get("available", {}).get("cash")
        if available_raw is None:
            raise RiskRejected("Kite did not provide available cash; cannot authorize this order")
        available_cash = self._money(available_raw, "available cash")
        if available_cash < (notional * Decimal("1.03")):
            raise RiskRejected("available cash does not cover notional plus a 3% reserve")

        orders = self.client.orders()
        open_orders = [item for item in orders if item.get("status") in _OPEN_STATUSES]
        if open_orders:
            raise RiskRejected("Kite has open orders; reconcile them before any new order")
        if orders:
            raise RiskRejected("the one-order-per-session control found an existing Kite order today")
        positions = self.client.positions().get("net", [])
        if any(
            item.get("exchange") == "NSE"
            and item.get("tradingsymbol") == symbol
            and int(item.get("quantity", 0)) != 0
            for item in positions
        ):
            raise RiskRejected(f"an NSE position already exists in {symbol}")

        return OrderPlan(
            symbol=symbol,
            quantity=quantity,
            price=price,
            notional=notional,
            tick_size=instrument.tick_size,
            lot_size=instrument.lot_size,
            tag=uuid.uuid4().hex[:12],
            user_id=user_id,
        )

    def submit_manual(self, plan: OrderPlan, confirmation: str) -> str:
        """Fail-closed placeholder; config flags and CLI confirmation cannot enable trading."""
        del plan, confirmation
        raise RiskRejected(
            "LIVE_ROUTING_HARD_DISABLED: production risk gateway, signed deployment policy, "
            "broker certification and exchange/regulatory onboarding are not implemented; no order was sent"
        )

    def reconcile(self) -> list[dict[str, Any]]:
        with self._order_journal() as journal:
            events = journal.read_events()
            intents = [event for event in events if event.get("event_type") == "INTENT_DURABLE"]
            if not intents:
                return []
            intent = intents[-1]
            ack = [event for event in events if event.get("event_type") == "BROKER_ACKNOWLEDGED"]
            expected_order_id = ack[-1].get("order_id") if ack else None
            broker_orders = self.client.orders()
            matches = [
                order for order in broker_orders
                if order.get("tag") == intent.get("tag")
                or (expected_order_id and order.get("order_id") == expected_order_id)
            ]
            if len(matches) > 1:
                journal.append({
                    "schema_version": 1,
                    "event_type": "RECONCILIATION_CONFLICT",
                    "tag": intent["tag"],
                    "match_count": len(matches),
                })
                raise RiskRejected("multiple broker orders match the local intent; manual review required")
            if not matches:
                journal.append({
                    "schema_version": 1,
                    "event_type": "RECONCILIATION_UNRESOLVED",
                    "tag": intent["tag"],
                    "expected_order_id": expected_order_id,
                    "reason": "no broker order row matches the durable intent or acknowledged order_id",
                })
                return [{
                    "state": "unresolved",
                    "tag": intent["tag"],
                    "action": "do not retry; inspect Kite order book and support evidence",
                }]

            order = matches[0]
            order_id = order.get("order_id")
            if not isinstance(order_id, str) or not order_id:
                journal.append({
                    "schema_version": 1,
                    "event_type": "RECONCILIATION_UNRESOLVED",
                    "tag": intent["tag"],
                    "reason": "matched broker row has no order_id",
                })
                return [{"state": "unresolved", "tag": intent["tag"], "action": "matched order has no order_id"}]
            if expected_order_id and order_id != expected_order_id:
                journal.append({
                    "schema_version": 1,
                    "event_type": "RECONCILIATION_CONFLICT",
                    "tag": intent["tag"],
                    "expected_order_id": expected_order_id,
                    "observed_order_id": order_id,
                })
                raise RiskRejected("broker order_id differs from the durable acknowledgement; manual review required")

            # A tag match identifies a candidate, not proof that the broker row
            # represents this exact instruction. Compare every order contract
            # field before accepting its status or trades.
            expected_fields = {
                "exchange": intent["exchange"],
                "tradingsymbol": intent["tradingsymbol"],
                "transaction_type": intent["transaction_type"],
                "product": intent["product"],
                "order_type": intent["order_type"],
                "validity": intent["validity"],
                "variety": intent["variety"],
                "tag": intent["tag"],
            }
            mismatches = {
                key: {"expected": expected, "observed": order.get(key)}
                for key, expected in expected_fields.items()
                if order.get(key) != expected
            }
            try:
                observed_quantity = int(order.get("quantity"))
                observed_price = self._money(order.get("price"), "broker order price")
                requested_price = self._money(intent.get("price_inr"), "journalled limit price")
            except (TypeError, ValueError, RiskRejected):
                mismatches["quantity_or_price"] = {"expected": "valid quantity and price", "observed": "malformed"}
            else:
                if observed_quantity != int(intent["quantity"]):
                    mismatches["quantity"] = {"expected": int(intent["quantity"]), "observed": observed_quantity}
                if observed_price != requested_price:
                    mismatches["price"] = {"expected": format(requested_price, "f"), "observed": format(observed_price, "f")}
            if mismatches:
                journal.append({
                    "schema_version": 1,
                    "event_type": "RECONCILIATION_CONFLICT",
                    "tag": intent["tag"],
                    "order_id": order_id,
                    "mismatches": mismatches,
                })
                raise RiskRejected("broker order does not match the durable intent; manual review required")

            history = self.client.order_history(order_id)
            trades = self.client.order_trades(order_id)
            latest = history[-1] if history else order
            if latest.get("order_id") not in (None, order_id):
                journal.append({
                    "schema_version": 1,
                    "event_type": "RECONCILIATION_CONFLICT",
                    "tag": intent["tag"],
                    "order_id": order_id,
                    "history_order_id": latest.get("order_id"),
                })
                raise RiskRejected("Kite order history returned a different order_id; manual review required")
            try:
                for trade in trades:
                    if trade.get("order_id") not in (None, order_id):
                        raise ValueError("trade row belongs to a different order_id")
                    if trade.get("exchange") not in (None, intent["exchange"]):
                        raise ValueError("trade row belongs to a different exchange")
                    if trade.get("tradingsymbol") not in (None, intent["tradingsymbol"]):
                        raise ValueError("trade row belongs to a different instrument")
                trade_quantities = [int(trade.get("quantity")) for trade in trades]
                if any(quantity <= 0 for quantity in trade_quantities):
                    raise ValueError("trade quantity is not positive")
                filled_quantity = sum(trade_quantities)
                if filled_quantity > int(intent["quantity"]):
                    raise ValueError("reported trade quantity exceeds the order quantity")
                trade_prices = [
                    self._money(trade.get("average_price", trade.get("price")), "trade price")
                    for trade in trades
                ]
                if any(price <= 0 or price > requested_price for price in trade_prices):
                    raise ValueError("BUY trade price is outside the submitted limit")
                traded_value = sum(
                    (price * quantity for price, quantity in zip(trade_prices, trade_quantities)),
                    Decimal("0"),
                )
            except (TypeError, ValueError, RiskRejected) as error:
                journal.append({
                    "schema_version": 1,
                    "event_type": "RECONCILIATION_CONFLICT",
                    "tag": intent["tag"],
                    "order_id": order_id,
                    "reason": str(error),
                })
                raise RiskRejected("broker trade records violate the durable order contract; manual review required") from error
            observed = {
                "schema_version": 1,
                "event_type": "BROKER_STATE_OBSERVED",
                "tag": intent["tag"],
                "order_id": order_id,
                "broker_status": latest.get("status", order.get("status")),
                "filled_quantity": filled_quantity,
                "pending_quantity": latest.get("pending_quantity", order.get("pending_quantity")),
                "trade_count": len(trades),
                "trade_value_inr": format(traded_value, "f"),
                "meaning": "broker order/trade query result; fills are recorded only from returned trade records",
            }
            journal.append(observed)
            return [observed]
