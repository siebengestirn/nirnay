from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
from typing import Any


_ZERO_HASH = "0" * 64


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


class HashChainJournal:
    """Single-writer, create-new JSONL journal for decoded SDK callbacks.

    FYERS' Python Data Socket callback gives the application decoded JSON.
    These records are therefore SDK observations, not original WebSocket bytes.
    The chain protects the saved callback stream from undetected edits; it does
    not prove the provider sent every update.
    """

    def __init__(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        self.path = path
        self._file = path.open("xb", buffering=0)
        self._previous_hash = _ZERO_HASH
        self.records = 0
        self.gaps = 0
        self._closed = False

    @property
    def head(self) -> str:
        return self._previous_hash

    def append(self, event: dict[str, Any]) -> str:
        if self._closed:
            raise RuntimeError("journal is closed")
        payload = dict(event)
        payload["previous_hash"] = self._previous_hash
        digest = hashlib.sha256(bytes.fromhex(self._previous_hash) + canonical_json(payload)).hexdigest()
        record = dict(payload)
        record["record_hash"] = digest
        self._file.write(canonical_json(record) + b"\n")
        os.fsync(self._file.fileno())
        self._previous_hash = digest
        self.records += 1
        if event.get("event_type") == "gap":
            self.gaps += 1
        return digest

    def close(self) -> None:
        if not self._closed:
            self._file.flush()
            os.fsync(self._file.fileno())
            self._file.close()
            self._closed = True


def verify(path: Path) -> tuple[int, str]:
    previous = _ZERO_HASH
    count = 0
    with path.open("rb") as source:
        for line_number, line in enumerate(source, 1):
            if not line.endswith(b"\n"):
                raise ValueError(f"torn journal tail at line {line_number}")
            record = json.loads(line)
            record_hash = record.pop("record_hash", None)
            if record.get("previous_hash") != previous:
                raise ValueError(f"hash-chain predecessor mismatch at line {line_number}")
            expected = hashlib.sha256(bytes.fromhex(previous) + canonical_json(record)).hexdigest()
            if record_hash != expected:
                raise ValueError(f"record hash mismatch at line {line_number}")
            previous = expected
            count += 1
    return count, previous
