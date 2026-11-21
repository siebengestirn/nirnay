"""Append-only SQLite store for frozen experiment records.

The durability shape mirrors ``durable_oms.py``: WAL with FULL synchronous
writes, no symlink path, restricted permissions, a predecessor hash chain and
storage-level UPDATE/DELETE triggers. A UNIQUE claim index makes the first
selection claim of a holdout atomic; later selection claims fail closed.
"""
from __future__ import annotations

import base64
import json
import os
import sqlite3
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .holdout import assert_holdout_key, validate_claim
from .schema import (ExperimentError, HoldoutClaim, Receipt, content_hash,
                     freeze_registered_experiment, validate_complete_trial_accounting,
                     validate_registered_experiment)
from .verify import signed_head_message, verify_chain, verify_dataset_binding, verify_head_signature


class DuplicateExperimentError(ExperimentError):
    """Raised when an experiment_id is registered more than once."""


@dataclass(frozen=True, slots=True)
class StoredRecord:
    sequence: int
    experiment_id: str
    status: str
    registered_at_utc: str
    holdout_key: str
    body: dict[str, Any]
    previous_hash: str
    record_hash: str
    body_sha256: str


class ExperimentRegistry:
    """Single-node research registry: insert-only records and holdout claims."""

    def __init__(self, path: str | Path):
        requested = Path(path)
        requested.parent.mkdir(parents=True, exist_ok=True)
        self.path = requested.parent.resolve() / requested.name
        if self.path.is_symlink():
            raise ExperimentError("experiment registry path must not be a symlink")
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
                CREATE TABLE IF NOT EXISTS experiment_records (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    experiment_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    registered_at_utc TEXT NOT NULL,
                    holdout_key TEXT NOT NULL,
                    body_json TEXT NOT NULL,
                    body_sha256 TEXT NOT NULL,
                    previous_hash TEXT NOT NULL,
                    record_hash TEXT NOT NULL UNIQUE,
                    UNIQUE(experiment_id, body_sha256)
                );
                CREATE TABLE IF NOT EXISTS holdout_claims (
                    holdout_key TEXT NOT NULL,
                    claim_kind TEXT NOT NULL
                        CHECK(claim_kind IN ('selection','verification_only')),
                    experiment_id TEXT NOT NULL,
                    registered_at_utc TEXT NOT NULL,
                    created_sequence INTEGER NOT NULL CHECK(created_sequence > 0),
                    PRIMARY KEY(holdout_key, claim_kind)
                );
                CREATE UNIQUE INDEX IF NOT EXISTS holdout_claim_unique
                    ON holdout_claims(holdout_key, claim_kind);
                CREATE TABLE IF NOT EXISTS registry_head_signatures (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    head_hash TEXT NOT NULL,
                    key_id TEXT NOT NULL,
                    signature_b64 TEXT NOT NULL
                );
                CREATE TRIGGER IF NOT EXISTS experiment_records_no_update
                    BEFORE UPDATE ON experiment_records
                    BEGIN SELECT RAISE(ABORT, 'experiment records are append-only'); END;
                CREATE TRIGGER IF NOT EXISTS experiment_records_no_delete
                    BEFORE DELETE ON experiment_records
                    BEGIN SELECT RAISE(ABORT, 'experiment records are append-only'); END;
                CREATE TRIGGER IF NOT EXISTS holdout_claims_no_update
                    BEFORE UPDATE ON holdout_claims
                    BEGIN SELECT RAISE(ABORT, 'holdout claims are append-only'); END;
                CREATE TRIGGER IF NOT EXISTS holdout_claims_no_delete
                    BEFORE DELETE ON holdout_claims
                    BEGIN SELECT RAISE(ABORT, 'holdout claims are append-only'); END;
                CREATE TRIGGER IF NOT EXISTS head_signatures_no_update
                    BEFORE UPDATE ON registry_head_signatures
                    BEGIN SELECT RAISE(ABORT, 'head signatures are append-only'); END;
                CREATE TRIGGER IF NOT EXISTS head_signatures_no_delete
                    BEFORE DELETE ON registry_head_signatures
                    BEGIN SELECT RAISE(ABORT, 'head signatures are append-only'); END;
            """)

    @staticmethod
    def _last_hash(db: sqlite3.Connection) -> str:
        previous = db.execute(
            "SELECT record_hash FROM experiment_records ORDER BY sequence DESC LIMIT 1"
        ).fetchone()
        return "0" * 64 if previous is None else previous["record_hash"]

    def _append(self, db: sqlite3.Connection, record: dict[str, Any]) -> Receipt:
        body, _ = freeze_registered_experiment(record)
        body_sha256 = content_hash(body)
        existing = db.execute(
            "SELECT * FROM experiment_records WHERE experiment_id=? AND body_sha256=?",
            (body["experiment_id"], body_sha256),
        ).fetchone()
        if existing is not None:
            return self._receipt(existing)
        previous_hash = self._last_hash(db)
        record_hash = content_hash({"body": body, "previous_hash": previous_hash})
        try:
            cursor = db.execute(
                "INSERT INTO experiment_records(experiment_id,status,registered_at_utc,"
                "holdout_key,body_json,body_sha256,previous_hash,record_hash) "
                "VALUES(?,?,?,?,?,?,?,?)",
                (body["experiment_id"], body["status"], body["registered_at_utc"],
                 body["holdout_key"], json.dumps(body, sort_keys=True, separators=(",", ":")),
                 body_sha256, previous_hash, record_hash),
            )
        except sqlite3.IntegrityError as exc:
            raise ExperimentError("experiment record conflicts with an existing record") from exc
        row = db.execute(
            "SELECT * FROM experiment_records WHERE sequence=?", (cursor.lastrowid,)
        ).fetchone()
        return self._receipt(row)

    @staticmethod
    def _receipt(row: sqlite3.Row) -> Receipt:
        return Receipt(row["experiment_id"], row["status"], row["sequence"],
                       row["holdout_key"], row["record_hash"], row["previous_hash"])

    @staticmethod
    def _claim(row: sqlite3.Row) -> HoldoutClaim:
        return HoldoutClaim(row["holdout_key"], row["claim_kind"], row["experiment_id"],
                            row["registered_at_utc"])

    def _selection_claim(self, db: sqlite3.Connection,
                         holdout_key_value: str) -> HoldoutClaim | None:
        row = db.execute(
            "SELECT * FROM holdout_claims WHERE holdout_key=? AND claim_kind='selection'",
            (holdout_key_value,),
        ).fetchone()
        return None if row is None else self._claim(row)

    def register(self, record: dict[str, Any]) -> Receipt:
        """Persist a pre-registered record and atomically claim its holdout."""
        validate_registered_experiment(record)
        if record["status"] != "registered":
            raise ExperimentError("register() accepts only status='registered'")
        assert_holdout_key(record)
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                if db.execute(
                    "SELECT 1 FROM experiment_records WHERE experiment_id=? LIMIT 1",
                    (record["experiment_id"],),
                ).fetchone() is not None:
                    raise DuplicateExperimentError("experiment_id was already registered")
                kind = validate_claim(record, existing_selection=self._selection_claim(
                    db, record["holdout_key"]))
                receipt = self._append(db, record)
                db.execute(
                    "INSERT INTO holdout_claims(holdout_key,claim_kind,experiment_id,"
                    "registered_at_utc,created_sequence) VALUES(?,?,?,?,?)",
                    (record["holdout_key"], kind, record["experiment_id"],
                     record["registered_at_utc"], receipt.sequence),
                )
                db.commit()
            except Exception:
                db.rollback()
                raise
        return receipt

    def claim_holdout(self, record: dict[str, Any], *, claim_kind: str | None = None,
                      replicates: str | None = None) -> HoldoutClaim:
        """Add a holdout claim, normally a verification-only replication."""
        validate_registered_experiment(record)
        assert_holdout_key(record)
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                kind = claim_kind or ("selection" if record.get("holdout_use", "selection")
                                      == "selection" else "verification_only")
                if kind not in {"selection", "verification_only"}:
                    raise ExperimentError("claim_kind must be selection or verification_only")
                existing = db.execute(
                    "SELECT * FROM holdout_claims WHERE holdout_key=? AND claim_kind=?",
                    (record["holdout_key"], kind),
                ).fetchone()
                if existing is not None and existing["experiment_id"] == record["experiment_id"]:
                    db.commit()
                    return self._claim(existing)
                selection = self._selection_claim(db, record["holdout_key"])
                kind = validate_claim(record, existing_selection=selection,
                                      replicates=replicates)
                try:
                    db.execute(
                        "INSERT INTO holdout_claims(holdout_key,claim_kind,experiment_id,"
                        "registered_at_utc,created_sequence) VALUES(?,?,?,?,?)",
                        (record["holdout_key"], kind, record["experiment_id"],
                         record["registered_at_utc"], self._last_sequence(db) + 1),
                    )
                except sqlite3.IntegrityError as exc:
                    raise ExperimentError("holdout claim already exists") from exc
                row = db.execute(
                    "SELECT * FROM holdout_claims WHERE holdout_key=? AND claim_kind=?",
                    (record["holdout_key"], kind),
                ).fetchone()
                db.commit()
            except Exception:
                db.rollback()
                raise
        return self._claim(row)

    @staticmethod
    def _last_sequence(db: sqlite3.Connection) -> int:
        row = db.execute("SELECT MAX(sequence) AS value FROM experiment_records").fetchone()
        return 0 if row["value"] is None else row["value"]

    def append_result(self, record: dict[str, Any]) -> Receipt:
        """Append a terminal record without mutating the registered record."""
        validate_registered_experiment(record)
        if record["status"] not in {"completed", "candidate", "rejected"}:
            raise ExperimentError("append_result requires a terminal status")
        assert_holdout_key(record)
        validate_complete_trial_accounting(record)
        from ..validation.leakage_detection import assert_no_holdout_leakage

        assert_no_holdout_leakage(record)
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                registered = db.execute(
                    "SELECT body_json FROM experiment_records WHERE experiment_id=? "
                    "AND status='registered' ORDER BY sequence LIMIT 1",
                    (record["experiment_id"],),
                ).fetchone()
                if registered is None:
                    raise ExperimentError("terminal result requires a prior registered record")
                if json.loads(registered["body_json"])["registered_at_utc"] != record[
                    "registered_at_utc"
                ]:
                    raise ExperimentError(
                        "terminal record must retain the original registration time"
                    )
                claim = db.execute(
                    "SELECT 1 FROM holdout_claims WHERE experiment_id=? AND holdout_key=? LIMIT 1",
                    (record["experiment_id"], record["holdout_key"]),
                ).fetchone()
                if claim is None:
                    raise ExperimentError("terminal result requires a claimed holdout")
                receipt = self._append(db, record)
                db.commit()
            except Exception:
                db.rollback()
                raise
        return receipt

    def records(self) -> list[StoredRecord]:
        """Return all stored records in append order."""
        with self._db() as db:
            rows = db.execute("SELECT * FROM experiment_records ORDER BY sequence").fetchall()
        return [StoredRecord(row["sequence"], row["experiment_id"], row["status"],
                             row["registered_at_utc"], row["holdout_key"],
                             json.loads(row["body_json"]), row["previous_hash"],
                             row["record_hash"], row["body_sha256"]) for row in rows]

    def record(self, experiment_id: str, *, terminal_only: bool = False) -> StoredRecord | None:
        """Return the latest record for an experiment id, if present."""
        with self._db() as db:
            if terminal_only:
                row = db.execute(
                    "SELECT * FROM experiment_records WHERE experiment_id=? "
                    "AND status IN ('completed','candidate','rejected') "
                    "ORDER BY sequence DESC LIMIT 1",
                    (experiment_id,),
                ).fetchone()
            else:
                row = db.execute(
                    "SELECT * FROM experiment_records WHERE experiment_id=? "
                    "ORDER BY sequence DESC LIMIT 1",
                    (experiment_id,),
                ).fetchone()
        if row is None:
            return None
        return StoredRecord(row["sequence"], row["experiment_id"], row["status"],
                            row["registered_at_utc"], row["holdout_key"],
                            json.loads(row["body_json"]), row["previous_hash"],
                            row["record_hash"], row["body_sha256"])

    def sign_head(self, signer: Any, key_id: str) -> dict[str, Any]:
        """Optionally sign the current chain head with a domain-separated signer."""
        if not isinstance(key_id, str) or not key_id.strip():
            raise ExperimentError("signing key_id is required")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                head = self._last_hash(db)
                signature = signer.sign(signed_head_message(head))
                if not isinstance(signature, bytes) or len(signature) != 64:
                    raise ExperimentError("registry signer must return a 64-byte signature")
                encoded = base64.b64encode(signature).decode("ascii")
                db.execute(
                    "INSERT INTO registry_head_signatures(head_hash,key_id,signature_b64) "
                    "VALUES(?,?,?)",
                    (head, key_id, encoded),
                )
                db.commit()
            except Exception:
                db.rollback()
                raise
        return {"head_hash": head, "key_id": key_id, "signature_b64": encoded}

    def verify_integrity(self, *, trusted_public_keys: dict[str, str] | None = None,
                         dataset_dir: str | Path | None = None) -> dict[str, Any]:
        """Recompute the chain, digests, claim uniqueness and optional head signature."""
        stored = self.records()
        last_hash = verify_chain([{
            "sequence": row.sequence,
            "body": row.body,
            "body_sha256": row.body_sha256,
            "previous_hash": row.previous_hash,
            "record_hash": row.record_hash,
        } for row in stored])
        with self._db() as db:
            claims = db.execute(
                "SELECT * FROM holdout_claims ORDER BY holdout_key, claim_kind"
            ).fetchall()
            duplicates = db.execute(
                "SELECT holdout_key, claim_kind, COUNT(*) AS copies FROM holdout_claims "
                "GROUP BY holdout_key, claim_kind HAVING copies > 1"
            ).fetchall()
            signature = db.execute(
                "SELECT * FROM registry_head_signatures ORDER BY sequence DESC LIMIT 1"
            ).fetchone()
        if duplicates:
            raise ExperimentError("duplicate holdout claims detected")
        known = {(row.experiment_id, row.holdout_key) for row in stored}
        for claim in claims:
            if claim["claim_kind"] == "selection" and (
                claim["experiment_id"], claim["holdout_key"]
            ) not in known:
                raise ExperimentError("selection claim references an unregistered experiment")
        signature_verified = False
        if signature is not None:
            if signature["head_hash"] != last_hash:
                raise ExperimentError("registry head signature does not cover the current head")
            signature_verified = verify_head_signature(
                signature["head_hash"], signature["signature_b64"],
                signature["key_id"], trusted_public_keys,
            )
        if dataset_dir is not None:
            digests = {row.body["dataset_manifest_sha256"] for row in stored}
            if len(digests) > 1:
                raise ExperimentError("registry records bind more than one dataset manifest")
            if stored:
                verify_dataset_binding(stored[0].body, dataset_dir)
        return {"status": "verified_internal_integrity", "record_count": len(stored),
                "last_hash": last_hash, "signature_verified": signature_verified,
                "holdout_claims": len(claims), "duplicate_holdout_claims": 0,
                "quality_or_entitlement_certified": False}
