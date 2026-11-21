from __future__ import annotations

import hashlib
import importlib.metadata
import json
import os
import queue
import threading
import time
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

from .journal import HashChainJournal, canonical_json, verify
from .rights import CAPTURE, STORE, ProductKey, authorize

_STOP = object()

FYERS_PRODUCT_KEY = ProductKey("fyers", "data_socket", "NSE", "Cash")


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


@dataclass(frozen=True)
class RecorderConfig:
    symbols: tuple[str, ...]
    entitlement_ref: str
    output_root: Path
    registry_path: Path
    queue_capacity: int = 16_384
    reconnect_retry: int = 10
    include_depth: bool = True
    max_symbols: int = 100

    def validate(self) -> None:
        # Rights are checked before any run directory, journal or writer thread
        # exists. A denial must leave no file behind.
        authorize(self.registry_path, FYERS_PRODUCT_KEY, CAPTURE)
        authorize(self.registry_path, FYERS_PRODUCT_KEY, STORE)
        if not self.entitlement_ref.strip():
            raise ValueError("an entitlement reference is required")
        if not self.symbols:
            raise ValueError("at least one FYERS symbol is required")
        if len(set(self.symbols)) != len(self.symbols):
            raise ValueError("watchlist contains duplicate symbols")
        if type(self.max_symbols) is not int or not 1 <= self.max_symbols <= 5_000:
            raise ValueError("max_symbols must be an integer between 1 and the documented Prime ceiling of 5000")
        if len(self.symbols) > self.max_symbols:
            raise ValueError(
                f"watchlist has {len(self.symbols)} symbols but the operator-declared entitlement limit is {self.max_symbols}"
            )
        if any(not symbol.startswith(("NSE:", "BSE:")) for symbol in self.symbols):
            raise ValueError("symbols must use an explicit NSE: or BSE: prefix")
        if self.queue_capacity < 64:
            raise ValueError("queue_capacity must be at least 64")
        if not 0 <= self.reconnect_retry <= 50:
            raise ValueError("reconnect_retry must be between 0 and 50")


class FyersRecorder:
    """Capture FYERS Data Socket callback messages in a local session archive."""

    def __init__(
        self,
        config: RecorderConfig,
        access_token: str,
        socket_factory: Callable[..., Any] | None = None,
    ) -> None:
        config.validate()
        if not access_token or ":" not in access_token:
            raise ValueError("FYERS_ACCESS_TOKEN must use appid:accesstoken format")
        self.config = config
        self._access_token = access_token
        self._socket_factory = socket_factory or self._load_socket_factory()
        self._queue: queue.Queue[Any] = queue.Queue(maxsize=config.queue_capacity)
        self._lock = threading.Lock()
        self._sequence = 0
        self._pending_gap_first: int | None = None
        self._pending_gap_last: int | None = None
        self._pending_gap_count = 0
        self._callback_count = 0
        self._dropped_count = 0
        self._error_count = 0
        self._connect_count = 0
        self._pending_subscribe = False
        self._closed = False
        self._writer_error: BaseException | None = None
        self._stop_event = threading.Event()
        self._started_utc = datetime.now(timezone.utc)
        self._started_monotonic_ns = time.monotonic_ns()
        self._run_dir = self._new_run_dir()
        self._journal_path = self._run_dir / "fyers-sdk-observations.jsonl"
        self._journal = HashChainJournal(self._journal_path)
        self._writer = threading.Thread(target=self._writer_main, name="fyers-journal-writer", daemon=False)
        self._writer.start()
        self._socket: Any | None = None

    @staticmethod
    def _load_socket_factory() -> Callable[..., Any]:
        try:
            from fyers_apiv3.FyersWebsocket import data_ws
        except ImportError as error:
            raise RuntimeError("install apps/fyers-recorder with pip before running") from error
        return data_ws.FyersDataSocket

    def _new_run_dir(self) -> Path:
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
        self.config.output_root.mkdir(parents=True, exist_ok=True)
        if os.name == "posix":
            os.chmod(self.config.output_root, 0o700)
        run_dir = self.config.output_root / f"fyers-{stamp}-{uuid.uuid4().hex[:8]}"
        run_dir.mkdir(mode=0o700, parents=True, exist_ok=False)
        if os.name == "posix":
            os.chmod(run_dir, 0o700)
        return run_dir

    @staticmethod
    def _normalize_payload(message: Any) -> Any:
        if isinstance(message, str):
            try:
                return json.loads(message)
            except json.JSONDecodeError:
                return {"_sdk_text": message}
        if isinstance(message, (dict, list, int, float, bool)) or message is None:
            # Validate JSON compatibility and reject NaN/Infinity before enqueue.
            canonical_json(message)
            return message
        raise TypeError(f"unsupported SDK callback payload type: {type(message).__name__}")

    def _event(self, event_type: str, payload: Any, sequence: int | None = None) -> dict[str, Any]:
        return {
            "schema_version": 1,
            "capture_layer": "fyers-python-sdk-callback-json",
            "event_type": event_type,
            "capture_sequence": sequence,
            "received_utc_ns": time.time_ns(),
            "received_monotonic_ns": time.monotonic_ns(),
            "payload": payload,
        }

    def _mark_drop_locked(self, sequence: int) -> None:
        if self._pending_gap_first is None:
            self._pending_gap_first = sequence
        self._pending_gap_last = sequence
        self._pending_gap_count += 1
        self._dropped_count += 1

    def _pending_gap_event_locked(self) -> dict[str, Any] | None:
        if self._pending_gap_count == 0:
            return None
        payload = {
            "first_missing_local_sequence": self._pending_gap_first,
            "last_missing_local_sequence": self._pending_gap_last,
            "dropped_callback_count": self._pending_gap_count,
            "reason": "bounded_callback_queue_full",
        }
        return self._event("gap", payload)

    def _clear_pending_gap_locked(self) -> None:
        self._pending_gap_first = None
        self._pending_gap_last = None
        self._pending_gap_count = 0

    def _take_pending_gap_locked(self) -> dict[str, Any] | None:
        gap = self._pending_gap_event_locked()
        if gap is not None:
            self._clear_pending_gap_locked()
        return gap

    def _emit(self, event_type: str, payload: Any, *, market_data: bool = False, error: bool = False) -> None:
        # Sequence assignment and queue insertion share one lock. The journal
        # order therefore matches the local sequence order, including GAP rows.
        with self._lock:
            if self._closed:
                return
            self._sequence += 1
            sequence = self._sequence
            if market_data:
                self._callback_count += 1
            if error:
                self._error_count += 1

            gap = self._pending_gap_event_locked()
            if gap is not None:
                try:
                    self._queue.put_nowait(gap)
                except queue.Full:
                    # Keep the prior gap intact; the current callback joins it.
                    self._mark_drop_locked(sequence)
                    self._error_count += 1
                    return
                else:
                    self._clear_pending_gap_locked()

            event = self._event(event_type, payload, sequence)
            try:
                self._queue.put_nowait(event)
            except queue.Full:
                self._mark_drop_locked(sequence)
                self._error_count += 1

    def on_message(self, message: Any) -> None:
        try:
            payload = self._normalize_payload(message)
            self._emit("market_data", payload, market_data=True)
        except (TypeError, ValueError) as error:
            self._emit("malformed_sdk_callback", {"error": str(error)}, error=True)

    def on_error(self, message: Any) -> None:
        self._emit("socket_error", self._normalize_error(message), error=True)

    def on_close(self, message: Any) -> None:
        self._emit("socket_closed", self._normalize_error(message))

    @staticmethod
    def _normalize_error(message: Any) -> Any:
        if isinstance(message, (str, int, float, bool)) or message is None:
            return message
        try:
            canonical_json(message)
            return message
        except (TypeError, ValueError):
            return {"message": str(message)}

    def on_connect(self) -> None:
        with self._lock:
            self._connect_count += 1
            connect_number = self._connect_count
        self._emit("socket_connected", {"connect_number": connect_number})
        self._subscribe()

    def _subscribe(self) -> None:
        with self._lock:
            socket = self._socket
            if socket is None:
                self._pending_subscribe = True
                return
            self._pending_subscribe = False
        socket.subscribe(symbols=list(self.config.symbols), data_type="SymbolUpdate")
        if self.config.include_depth:
            socket.subscribe(symbols=list(self.config.symbols), data_type="DepthUpdate")

    def _writer_main(self) -> None:
        try:
            while True:
                item = self._queue.get()
                if item is _STOP:
                    with self._lock:
                        gap = self._take_pending_gap_locked()
                    if gap is not None:
                        self._journal.append(gap)
                    break
                self._journal.append(item)
        except BaseException as error:  # retain the writer fault for the operator
            self._writer_error = error
            self._stop_event.set()
        finally:
            try:
                self._journal.close()
            except BaseException as error:
                self._writer_error = self._writer_error or error

    def run(self) -> Path:
        runner_errors: list[BaseException] = []
        try:
            socket = self._socket_factory(
                access_token=self._access_token,
                log_path="",
                litemode=False,
                write_to_file=False,
                reconnect=True,
                on_connect=self.on_connect,
                on_close=self.on_close,
                on_error=self.on_error,
                on_message=self.on_message,
                reconnect_retry=self.config.reconnect_retry,
            )
            with self._lock:
                self._socket = socket
                pending_subscribe = self._pending_subscribe
            if pending_subscribe:
                self._subscribe()

            # Keep the SDK loop separate from the durable writer supervisor. A
            # writer failure must close the socket rather than leave callbacks
            # accumulating in a dead queue.
            def socket_loop() -> None:
                try:
                    socket.keep_running()
                except BaseException as error:
                    runner_errors.append(error)
                finally:
                    self._stop_event.set()

            runner = threading.Thread(target=socket_loop, name="fyers-sdk-socket", daemon=False)
            runner.start()
            while runner.is_alive():
                if self._writer_error is not None:
                    self._stop_event.set()
                if self._stop_event.wait(timeout=0.1):
                    if runner.is_alive():
                        try:
                            socket.close_connection()
                        except Exception:
                            pass
                    break
            runner.join(timeout=30)
            if runner.is_alive():
                try:
                    socket.close_connection()
                except Exception:
                    pass
                runner.join(timeout=5)
            if runner.is_alive():
                raise RuntimeError("FYERS SDK socket loop did not stop within 35 seconds")
            if runner_errors:
                raise RuntimeError("FYERS SDK socket loop failed") from runner_errors[0]
        finally:
            self.close()
        return self._run_dir

    def close(self) -> Path:
        self._stop_event.set()
        socket = self._socket
        if socket is not None:
            try:
                socket.close_connection()
            except Exception:
                pass
        with self._lock:
            if self._closed:
                return self._run_dir
            self._closed = True
        if self._writer.is_alive():
            try:
                self._queue.put(_STOP, timeout=30)
            except queue.Full as error:
                raise RuntimeError("FYERS callback queue did not drain within 30 seconds") from error
        self._writer.join(timeout=30)
        if self._writer.is_alive():
            raise RuntimeError("FYERS journal writer did not stop within 30 seconds")
        if self._writer_error is not None:
            self._write_manifest(writer_error=str(self._writer_error))
            raise RuntimeError("FYERS journal writer failed") from self._writer_error
        self._write_manifest()
        return self._run_dir

    def _write_manifest(self, writer_error: str | None = None) -> None:
        ended = datetime.now(timezone.utc)
        try:
            journal_count, head = verify(self._journal_path)
            journal_valid = True
        except (OSError, ValueError, json.JSONDecodeError):
            journal_count, head, journal_valid = self._journal.records, self._journal.head, False
        try:
            sdk_version = importlib.metadata.version("fyers-apiv3")
        except importlib.metadata.PackageNotFoundError:
            sdk_version = "not-installed-test-double"
        manifest = {
            "schema_version": 1,
            "provider": "FYERS",
            "capture_layer": "Python SDK callback JSON; not original WebSocket wire bytes",
            "run_id": self._run_dir.name,
            "started_at_utc": self._started_utc.isoformat(),
            "ended_at_utc": ended.isoformat(),
            "duration_ns": time.monotonic_ns() - self._started_monotonic_ns,
            "symbols": list(self.config.symbols),
            "subscriptions": ["SymbolUpdate"] + (["DepthUpdate"] if self.config.include_depth else []),
            "configured_symbol_limit": self.config.max_symbols,
            "subscription_capacity_assertion": "operator_declared_entitlement_bound_not_broker_verified",
            "official_plan_reference": "https://fyers.in/api-docs/v3/api/websockets/websocket-overview/",
            "official_plan_reference_last_updated": "2026-10-14",
            "documented_data_socket_capacity": {"standard": 100, "prime": 5000},
            "tbt_socket_supported_by_this_recorder": False,
            "entitlement_ref": self.config.entitlement_ref,
            "sdk_version": sdk_version,
            "callback_count": self._callback_count,
            "record_count": journal_count,
            "socket_connect_count": self._connect_count,
            "dropped_callback_count": self._dropped_count,
            "error_count": self._error_count,
            "journal_file": self._journal_path.name,
            "journal_sha256": _sha256_file(self._journal_path),
            "journal_head_hash": head,
            "journal_chain_valid": journal_valid,
            "writer_error": writer_error,
            "provider_sequence_available": False,
            "research_grade": False,
            "reason_not_research_grade": "SDK callback delivery has no verified provider sequence and captures decoded callback JSON, not wire frames.",
        }
        manifest_path = self._run_dir / "manifest.json"
        temp_path = self._run_dir / f"manifest.json.{uuid.uuid4().hex}.tmp"
        with temp_path.open("xb") as target:
            if os.name == "posix":
                os.chmod(temp_path, 0o600)
            target.write(canonical_json(manifest) + b"\n")
            target.flush()
            os.fsync(target.fileno())
        os.replace(temp_path, manifest_path)
        directory_fd = os.open(self._run_dir, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
