"""Low-level canonical digest helper with no package-level dependencies.

This is the JSON encoding used by the experiment schema, exposed without
importing the experiments package so pure feature, data and validation modules
cannot create an import cycle when they are loaded first.
"""
from __future__ import annotations

import hashlib

from .market import canonical_bytes


def content_hash(value: object) -> str:
    """Return the SHA-256 digest of the canonical JSON representation."""
    return hashlib.sha256(canonical_bytes(value)).hexdigest()
