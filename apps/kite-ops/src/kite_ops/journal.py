from __future__ import annotations

import fcntl
import hashlib
import json
import os
from pathlib import Path
from typing import Any

_ZERO = "0" * 64


def canonical(value: Any) -> bytes:
    return json.dumps(value, allow_nan=False, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


class LockedHashJournal:
    """Append-only operator journal with an exclusive process lock and fsync."""

    def __init__(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        self.path = path
        self.lock_path = path.with_suffix(path.suffix + ".lock")
        os.chmod(path.parent, 0o700)
        self._lock_fd = os.open(self.lock_path, os.O_CREAT | os.O_RDWR, 0o600)
        os.chmod(self.lock_path, 0o600)
        try:
            fcntl.flock(self._lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            os.close(self._lock_fd)
            raise RuntimeError("another Kite operator process holds the session lock") from error
        try:
            file_fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_APPEND, 0o600)
            self._file = os.fdopen(file_fd, "a+b", buffering=0)
            os.chmod(path, 0o600)
            directory_fd = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
            self._head = _ZERO
            self.records = self._verify_existing()
        except BaseException:
            file_obj = getattr(self, "_file", None)
            if file_obj is not None and not file_obj.closed:
                file_obj.close()
            fcntl.flock(self._lock_fd, fcntl.LOCK_UN)
            os.close(self._lock_fd)
            self._lock_fd = -1
            raise

    def _verify_existing(self) -> int:
        self._file.seek(0)
        count = 0
        for number, line in enumerate(self._file, 1):
            if not line.endswith(b"\n"):
                raise RuntimeError(f"torn Kite journal tail at record {number}; operator review required")
            record = json.loads(line)
            if not isinstance(record, dict):
                raise RuntimeError(f"Kite journal record {number} is not a JSON object")
            record_hash = record.pop("record_hash", None)
            if record.get("previous_hash") != self._head:
                raise RuntimeError(f"Kite journal chain mismatch at record {number}")
            expected = hashlib.sha256(bytes.fromhex(self._head) + canonical(record)).hexdigest()
            if record_hash != expected:
                raise RuntimeError(f"Kite journal hash mismatch at record {number}")
            self._head = expected
            count += 1
        self._file.seek(0, os.SEEK_END)
        return count

    def append(self, event: dict[str, Any]) -> str:
        record = dict(event)
        record["previous_hash"] = self._head
        digest = hashlib.sha256(bytes.fromhex(self._head) + canonical(record)).hexdigest()
        record["record_hash"] = digest
        self._file.write(canonical(record) + b"\n")
        os.fsync(self._file.fileno())
        self._head = digest
        self.records += 1
        return digest

    def read_events(self) -> list[dict[str, Any]]:
        self._file.flush()
        self._file.seek(0)
        items = [json.loads(line) for line in self._file]
        self._file.seek(0, os.SEEK_END)
        return items

    def close(self) -> None:
        if not self._file.closed:
            self._file.flush()
            os.fsync(self._file.fileno())
            self._file.close()
        if self._lock_fd >= 0:
            fcntl.flock(self._lock_fd, fcntl.LOCK_UN)
            os.close(self._lock_fd)
            self._lock_fd = -1

    def __enter__(self) -> "LockedHashJournal":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()
