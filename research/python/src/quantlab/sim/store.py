"""Thin append-only trial store backed by hash-named files, with no database.

A key is the SHA-256 of the canonical payload.  A second write at an existing
key is refused and a differing payload at that key fails closed; no record is
ever truncated, replaced or silently deduplicated.
"""
from __future__ import annotations

import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from .events import canonical_bytes, digest, is_sha256, to_jsonable


class TrialStoreError(ValueError):
    """Base class for trial-store invariant violations."""


class DuplicateTrial(TrialStoreError):
    """Raised when a key already has a record."""


class TrialConflict(TrialStoreError):
    """Raised when a key maps to different content."""


@dataclass(frozen=True, slots=True)
class TrialStore:
    """Append-only, content-addressed trial records on the local filesystem."""

    root: Path

    def __post_init__(self) -> None:
        if not isinstance(self.root, Path):
            raise ValueError("trial store root must be a Path")

    def _path(self, key: str) -> Path:
        if not is_sha256(key):
            raise TrialStoreError("trial key must be a lowercase SHA-256 digest")
        return self.root / f"{key}.json"

    def put(self, key: str, payload: Mapping[str, Any]) -> Path:
        """Append one record; refuse any second write at the same key."""
        path = self._path(key)
        body = canonical_bytes(to_jsonable(payload)).decode("utf-8")
        self.root.mkdir(parents=True, exist_ok=True)
        if path.exists():
            if path.read_text(encoding="utf-8") != body:
                raise TrialConflict(f"differing payload at trial key {key}")
            raise DuplicateTrial(f"trial key {key} already exists")
        if digest(to_jsonable(payload)) != key:
            raise TrialConflict("trial payload does not hash to its key")
        try:
            with path.open("xb") as stream:
                stream.write(body.encode("utf-8"))
                stream.flush()
                os.fsync(stream.fileno())
        except FileExistsError as exc:
            raise DuplicateTrial(f"trial key {key} was written concurrently") from exc
        return path

    def get(self, key: str) -> dict[str, Any]:
        """Read one record, failing closed if the file no longer matches its key."""
        path = self._path(key)
        if not path.is_file():
            raise TrialStoreError(f"trial key {key} has no record")
        data: Any = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, dict) or digest(to_jsonable(data)) != key:
            raise TrialConflict(f"trial record {key} fails its content address")
        return data

    def keys(self) -> tuple[str, ...]:
        """Return all well-formed content addresses currently stored."""
        if not self.root.is_dir():
            return ()
        return tuple(sorted(
            item.stem for item in self.root.glob("*.json") if is_sha256(item.stem)
        ))
