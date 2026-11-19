#!/usr/bin/env python3
"""Generate a synthetic FYERS session with the real Python hash-chain journal.

The output is a static fixture for the Rust importer tests. It is not a live
provider session and contains no account identifiers.
"""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "apps" / "fyers-recorder" / "src"))

from fyers_recorder.journal import HashChainJournal, canonical_json  # noqa: E402


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    session = ROOT / "apps" / "fyers-importer" / "tests" / "fixtures" / "session"
    session.mkdir(parents=True, exist_ok=True)
    journal_path = session / "fyers-sdk-observations.jsonl"
    if journal_path.exists():
        journal_path.unlink()
    journal = HashChainJournal(journal_path)
    records = [
        {"schema_version": 1, "event_type": "socket_connected", "capture_sequence": 1,
         "symbol": None, "payload": {"attempt": 1}},
        {"schema_version": 1, "event_type": "market_data", "capture_sequence": 2,
         "symbol": "SBIN-EQ", "payload": {"type": "sf", "ltp": 812.5}},
        {"schema_version": 1, "event_type": "gap", "capture_sequence": 4,
         "symbol": None, "payload": {"first_missing_local_sequence": 3,
                                     "last_missing_local_sequence": 3}},
        {"schema_version": 1, "event_type": "market_data", "capture_sequence": 5,
         "symbol": "TCS-EQ", "payload": {"type": "sf", "ltp": 4012.0}},
    ]
    for record in records:
        journal.append(record)
    journal.close()
    _, head = __import__("fyers_recorder.journal", fromlist=["verify"]).verify(journal_path)
    manifest = {
        "schema_version": 1,
        "provider": "FYERS",
        "capture_layer": "Python SDK callback JSON; not original WebSocket wire bytes",
        "run_id": "synthetic-fyers-session",
        "entitlement_ref": "entitlement:synthetic:fyers",
        "journal_file": journal_path.name,
        "journal_sha256": sha256_file(journal_path),
        "journal_head_hash": head,
        "journal_chain_valid": True,
        "record_count": len(records),
        "writer_error": None,
        "provider_sequence_available": False,
    }
    (session / "manifest.json").write_bytes(canonical_json(manifest) + b"\n")


if __name__ == "__main__":
    main()
