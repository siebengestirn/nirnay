"""Independent-source attestation gate for risk-service runtime evidence.

The signer does not accept caller-supplied safety booleans. It requires fresh,
source-signed observations from distinct control authorities and an offline
root-signed trust registry. Provider/database clients which collect these
observations are deployment-specific; without them this component refuses to
produce live runtime evidence.
"""
from __future__ import annotations

import base64
import hashlib
import os
import sqlite3
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Protocol, Sequence

from .live_auth import (
    RUNTIME_EVIDENCE_DOMAIN,
    AuthorizationError,
    _decode_key,
    _decode_signature,
    _verify,
)
from .market import canonical_bytes
from .risk_service import (IndependentRiskService, RiskFacts, RiskServiceError,
                           _GATE_SIGNING_CAPABILITY)

CONTROL_OBSERVATION_DOMAIN = b"NIRNAY-CONTROL-OBSERVATION-V1\x00"
REQUIRED_CONTROL_ROLES = (
    "account_risk", "kill_switch", "market_data", "reconciliation",
    "broker_entitlement", "trading_session", "clock_sync", "deployed_artifact",
)
ROLE_PAYLOAD_FIELDS = {
    "account_risk": {"account", "projected_exposure_paise", "projected_daily_loss_paise"},
    "kill_switch": {"account", "enabled"},
    "market_data": {"broker", "instrument_id", "healthy", "gap_count", "last_event_at_utc_ns"},
    "reconciliation": {"broker", "account", "clean", "break_count"},
    "broker_entitlement": {"broker", "account", "instrument_id", "entitled", "expires_at_utc_ns"},
    "trading_session": {"venue", "valid", "session_id"},
    "clock_sync": {"offset_ns"},
    "deployed_artifact": {"commit_sha", "strategy", "strategy_artifact_sha256"},
}


class ObservationSource(Protocol):
    def collect(self, request: "OrderRiskRequest", *, now_utc_ns: int) -> Sequence["SignedControlObservation"]: ...


class ObservationReplayLedger(Protocol):
    # Implementations must declare their persistence/consistency scope. This is
    # a configuration guard, not a proof of deployment isolation.
    durability_scope: str
    def consume(self, observations: Sequence["SignedControlObservation"]) -> None: ...


class InMemoryObservationReplayLedger:
    """Test-only replay ledger. Do not use for risk authorization."""

    durability_scope = "test_only_volatile"

    def __init__(self):
        self._last: dict[str, tuple[int, int]] = {}

    def consume(self, observations: Sequence["SignedControlObservation"]) -> None:
        seen: set[str] = set()
        for item in observations:
            if item.source_id in seen:
                raise RiskServiceError("duplicate control source in replay ledger batch")
            seen.add(item.source_id)
            prior = self._last.get(item.source_id)
            if prior is not None and item.source_sequence <= prior[0]:
                raise RiskServiceError(f"replayed or regressed control source sequence: {item.source_id}")
            if prior is not None and item.observed_at_utc_ns < prior[1]:
                raise RiskServiceError(f"control source observation time regressed: {item.source_id}")
        for item in observations:
            self._last[item.source_id] = (item.source_sequence, item.observed_at_utc_ns)


class SQLiteObservationReplayLedger:
    """Persistent one-time observation sequence ledger for an isolated risk service.

    This is a single-node persistence primitive. A clustered risk service needs
    a transactional shared store with equivalent compare-and-set semantics.
    """

    durability_scope = "single_node_persistent"

    def __init__(self, path: str | Path):
        requested = Path(path)
        requested.parent.mkdir(parents=True, exist_ok=True)
        if requested.is_symlink():
            raise RiskServiceError("observation replay ledger path must not be a symlink")
        self.path = requested.parent.resolve() / requested.name
        if os.name == "posix":
            os.chmod(self.path.parent, 0o700)
        with self._connect() as db:
            db.execute("PRAGMA journal_mode=WAL")
            db.execute("""CREATE TABLE IF NOT EXISTS consumed_control_observations (
                source_id TEXT PRIMARY KEY,
                last_sequence INTEGER NOT NULL CHECK(last_sequence > 0),
                last_observed_at_utc_ns INTEGER NOT NULL CHECK(last_observed_at_utc_ns > 0),
                last_observation_sha256 TEXT NOT NULL
            )""")
        self._secure_permissions()

    def _secure_permissions(self) -> None:
        if os.name == "posix":
            if self.path.exists(): os.chmod(self.path, 0o600)
            for suffix in ("-wal", "-shm"):
                sidecar = Path(str(self.path) + suffix)
                if sidecar.exists(): os.chmod(sidecar, 0o600)

    def _connect(self) -> sqlite3.Connection:
        db = sqlite3.connect(self.path, timeout=15.0, isolation_level=None)
        db.execute("PRAGMA synchronous=FULL")
        db.execute("PRAGMA busy_timeout=15000")
        db.execute("PRAGMA trusted_schema=OFF")
        db.execute("PRAGMA secure_delete=ON")
        self._secure_permissions()
        return db

    def consume(self, observations: Sequence["SignedControlObservation"]) -> None:
        if not observations:
            raise RiskServiceError("replay ledger cannot consume an empty observation batch")
        digests = []
        seen: set[str] = set()
        for item in observations:
            if item.source_id in seen:
                raise RiskServiceError("duplicate control source in replay ledger batch")
            seen.add(item.source_id)
            digest = hashlib.sha256(
                canonical_bytes(item.signed_body()) + item.signature_b64.encode("ascii")
            ).hexdigest()
            digests.append((item, digest))
        with self._connect() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                for item, _digest in digests:
                    old = db.execute(
                        "SELECT last_sequence,last_observed_at_utc_ns "
                        "FROM consumed_control_observations WHERE source_id=?",
                        (item.source_id,),
                    ).fetchone()
                    if old is not None and item.source_sequence <= old[0]:
                        raise RiskServiceError(f"replayed or regressed control source sequence: {item.source_id}")
                    if old is not None and item.observed_at_utc_ns < old[1]:
                        raise RiskServiceError(f"control source observation time regressed: {item.source_id}")
                for item, digest in digests:
                    db.execute(
                        "INSERT INTO consumed_control_observations "
                        "(source_id,last_sequence,last_observed_at_utc_ns,last_observation_sha256) "
                        "VALUES(?,?,?,?) ON CONFLICT(source_id) DO UPDATE SET "
                        "last_sequence=excluded.last_sequence, "
                        "last_observed_at_utc_ns=excluded.last_observed_at_utc_ns, "
                        "last_observation_sha256=excluded.last_observation_sha256",
                        (item.source_id, item.source_sequence, item.observed_at_utc_ns, digest),
                    )
                db.commit()
            except Exception:
                db.rollback()
                raise
        self._secure_permissions()


@dataclass(frozen=True, slots=True)
class OrderRiskRequest:
    commit_sha: str
    broker: str
    account: str
    strategy: str
    instrument_id: str
    venue: str
    side: str
    order_type: str
    product: str
    quantity: int
    notional_paise: int

    def validate(self) -> None:
        for name in ("commit_sha", "broker", "account", "strategy", "instrument_id", "venue", "side", "order_type", "product"):
            value = getattr(self, name)
            if not isinstance(value, str) or not value.strip():
                raise RiskServiceError(f"order risk request {name} must be non-empty")
        if len(self.commit_sha) not in {40, 64} or any(c not in "0123456789abcdef" for c in self.commit_sha):
            raise RiskServiceError("order risk request commit_sha must be lowercase Git SHA")
        if type(self.quantity) is not int or self.quantity <= 0:
            raise RiskServiceError("order risk request quantity must be positive")
        if type(self.notional_paise) is not int or self.notional_paise <= 0:
            raise RiskServiceError("order risk request notional must be positive")


@dataclass(frozen=True, slots=True)
class SignedControlObservation:
    source_id: str
    role: str
    source_sequence: int
    observed_at_utc_ns: int
    expires_at_utc_ns: int
    payload: dict[str, Any]
    signature_b64: str

    def signed_body(self) -> dict[str, Any]:
        return {
            "schema_version": 1,
            "source_id": self.source_id,
            "role": self.role,
            "source_sequence": self.source_sequence,
            "observed_at_utc_ns": self.observed_at_utc_ns,
            "expires_at_utc_ns": self.expires_at_utc_ns,
            "payload": self.payload,
        }

    @classmethod
    def sign(cls, *, source_id: str, role: str, source_sequence: int,
             observed_at_utc_ns: int, expires_at_utc_ns: int,
             payload: dict[str, Any], signer: Any) -> "SignedControlObservation":
        """Convenience for independent control agents; private keys stay in their own service."""
        if type(source_sequence) is not int or source_sequence <= 0:
            raise RiskServiceError("control source sequence must be a positive integer")
        provisional = cls(source_id, role, source_sequence, observed_at_utc_ns,
                          expires_at_utc_ns, payload, "")
        signature = signer.sign(CONTROL_OBSERVATION_DOMAIN + canonical_bytes(provisional.signed_body()))
        if not isinstance(signature, bytes) or len(signature) != 64:
            raise RiskServiceError("control source signer must return a 64-byte Ed25519 signature")
        return cls(source_id, role, source_sequence, observed_at_utc_ns,
                   expires_at_utc_ns, payload, base64.b64encode(signature).decode("ascii"))


@dataclass(frozen=True, slots=True)
class _TrustedSource:
    role: str
    principal_id: str
    public_key: bytes


def _verified_control_sources(registry_envelope: dict[str, Any], root_public_key_b64: str,
                              now_utc_ns: int) -> tuple[dict[str, _TrustedSource], dict[str, Any]]:
    if not isinstance(registry_envelope, dict) or set(registry_envelope) != {"body", "root_signature"}:
        raise RiskServiceError("control trust registry must be a root-signed envelope")
    body = registry_envelope["body"]
    required = {
        "schema_version", "issued_at_utc_ns", "expires_at_utc_ns", "approval_threshold",
        "max_clock_offset_ns", "max_risk_snapshot_age_ns", "max_runtime_evidence_ttl_ns",
        "max_permit_ttl_ns", "issuers", "approvers", "risk_services", "control_sources",
    }
    if not isinstance(body, dict) or set(body) != required or body.get("schema_version") != 1:
        raise RiskServiceError("root trust registry schema mismatch")
    if type(now_utc_ns) is not int or now_utc_ns <= 0:
        raise RiskServiceError("risk service trusted clock is invalid")
    numeric_limits = ("approval_threshold", "max_clock_offset_ns", "max_risk_snapshot_age_ns",
                      "max_runtime_evidence_ttl_ns", "max_permit_ttl_ns")
    if any(type(body[name]) is not int for name in numeric_limits):
        raise RiskServiceError("root trust registry limits must be strict integers")
    if (body["approval_threshold"] < 2 or body["max_clock_offset_ns"] < 0 or
            body["max_risk_snapshot_age_ns"] <= 0 or
            body["max_runtime_evidence_ttl_ns"] <= 0 or body["max_permit_ttl_ns"] <= 0):
        raise RiskServiceError("root trust registry contains invalid risk or approval limits")
    if (not isinstance(body["issuers"], dict) or not body["issuers"] or
            not isinstance(body["approvers"], dict) or len(body["approvers"]) < body["approval_threshold"] or
            not isinstance(body["risk_services"], dict) or not body["risk_services"]):
        raise RiskServiceError("root trust registry is missing issuer, approver or risk-service records")
    if type(body["issued_at_utc_ns"]) is not int or type(body["expires_at_utc_ns"]) is not int or not (
            body["issued_at_utc_ns"] <= now_utc_ns < body["expires_at_utc_ns"]):
        raise RiskServiceError("root trust registry is not currently valid")
    root = _decode_key(root_public_key_b64, "registry root key")
    _verify(root, _decode_signature(registry_envelope["root_signature"], "registry root signature"),
            b"NIRNAY-TRUST-REGISTRY-V1\x00" + canonical_bytes(body))
    raw_sources = body["control_sources"]
    if not isinstance(raw_sources, dict) or not raw_sources:
        raise RiskServiceError("root trust registry must contain control_sources")
    trusted: dict[str, _TrustedSource] = {}
    roles, principals, keys = set(), set(), set()
    for source_id, record in raw_sources.items():
        if not isinstance(source_id, str) or not source_id or not isinstance(record, dict) or set(record) != {
                "role", "principal_id", "public_key_b64", "revoked"}:
            raise RiskServiceError("control source trust record schema mismatch")
        role, principal = record["role"], record["principal_id"]
        if role not in REQUIRED_CONTROL_ROLES or role in roles:
            raise RiskServiceError("control source roles must be exactly one per required role")
        if not isinstance(principal, str) or not principal or principal in principals:
            raise RiskServiceError("control source principal identities must be distinct")
        if type(record["revoked"]) is not bool or record["revoked"]:
            raise RiskServiceError("control source is revoked or has invalid revocation flag")
        public = _decode_key(record["public_key_b64"], f"control source {source_id} public key")
        if public in keys:
            raise RiskServiceError("control source public keys must be cryptographically distinct")
        roles.add(role); principals.add(principal); keys.add(public)
        trusted[source_id] = _TrustedSource(role, principal, public)
    if roles != set(REQUIRED_CONTROL_ROLES):
        raise RiskServiceError("control trust registry is missing one or more required control roles")
    return trusted, body


class SignedEvidenceRiskGate:
    """Builds runtime evidence only from independently signed, fresh observations."""

    def __init__(self, risk_service: IndependentRiskService, *,
                 registry_envelope: dict[str, Any], registry_root_public_key_b64: str,
                 replay_ledger: ObservationReplayLedger,
                 clock_ns=time.time_ns):
        if (replay_ledger is None or not callable(getattr(replay_ledger, "consume", None)) or
                getattr(replay_ledger, "durability_scope", None) not in {
                    "single_node_persistent", "cluster_transactional"
                }):
            raise ValueError("a persistent transactional control-observation replay ledger is required")
        self.risk_service = risk_service
        self.registry_envelope = registry_envelope
        self.registry_root_public_key_b64 = registry_root_public_key_b64
        self.replay_ledger = replay_ledger
        self.clock_ns = clock_ns

    def authorize(self, request: OrderRiskRequest, source: ObservationSource, *,
                  live_mode_configured: bool) -> Any:
        request.validate()
        if type(live_mode_configured) is not bool or not live_mode_configured:
            raise RiskServiceError("live mode is not configured; no runtime evidence will be signed")
        now = self.clock_ns()
        if type(now) is not int or now <= 0:
            raise RiskServiceError("risk service trusted clock is invalid")
        trusted, registry = _verified_control_sources(self.registry_envelope,
                                                       self.registry_root_public_key_b64, now)
        effective_clock_limit = min(self.risk_service.policy.max_clock_offset_ns,
                                    registry["max_clock_offset_ns"])
        effective_snapshot_age = min(self.risk_service.policy.max_snapshot_age_ns,
                                     registry["max_risk_snapshot_age_ns"])
        effective_evidence_ttl = min(self.risk_service.policy.max_evidence_ttl_ns,
                                     registry["max_runtime_evidence_ttl_ns"])
        risk_record = registry["risk_services"].get(self.risk_service.key_id)
        if (not isinstance(risk_record, dict) or
                set(risk_record) != {"public_key_b64", "principal_id", "revoked"} or
                type(risk_record["revoked"]) is not bool or risk_record["revoked"]):
            raise RiskServiceError("risk-service signer is not independently trusted or is revoked")
        risk_principal = risk_record["principal_id"]
        if not isinstance(risk_principal, str) or not risk_principal:
            raise RiskServiceError("risk-service principal identity is invalid")
        risk_public = _decode_key(risk_record["public_key_b64"], "trusted risk-service public key")
        advertised_public = getattr(self.risk_service.signer, "public_key_b64", None)
        if not isinstance(advertised_public, str) or _decode_key(advertised_public, "risk signer public key") != risk_public:
            raise RiskServiceError("configured risk-service signer does not match root trust registry")
        # Control authorities, risk service, release issuers and human approvers
        # must be separate principals and use separate key material. This check
        # is repeated by permit verification, but the signer must not issue
        # evidence under an identity collision in the first place.
        principals = {risk_principal}
        keys = {risk_public, _decode_key(self.registry_root_public_key_b64, "registry root key")}
        for trusted_source_record in trusted.values():
            if trusted_source_record.principal_id in principals or trusted_source_record.public_key in keys:
                raise RiskServiceError("risk and control source principals/keys must be cryptographically independent")
            principals.add(trusted_source_record.principal_id)
            keys.add(trusted_source_record.public_key)
        for issuer in registry["issuers"].values():
            if not isinstance(issuer, dict):
                raise RiskServiceError("issuer trust record is malformed")
            principal = issuer.get("principal_id")
            if not isinstance(principal, str) or not principal or principal in principals:
                raise RiskServiceError("issuer principal must be independent")
            public = _decode_key(issuer.get("public_key_b64", ""), "issuer public key")
            if public in keys:
                raise RiskServiceError("issuer key material must be independent")
            principals.add(principal); keys.add(public)
        for approver in registry["approvers"].values():
            if not isinstance(approver, dict):
                raise RiskServiceError("approver trust record is malformed")
            principal = approver.get("approver_id")
            if not isinstance(principal, str) or not principal or principal in principals:
                raise RiskServiceError("approver principal must be independent")
            public = _decode_key(approver.get("public_key_b64", ""), "approver public key")
            if public in keys:
                raise RiskServiceError("approver key material must be independent")
            principals.add(principal); keys.add(public)
        observations = tuple(source.collect(request, now_utc_ns=now))
        by_role: dict[str, SignedControlObservation] = {}
        principals: set[str] = set()
        public_keys: set[bytes] = set()
        risk_signer_raw = risk_public
        if not observations:
            raise RiskServiceError("independent control source returned no observations")
        for item in observations:
            if not isinstance(item, SignedControlObservation):
                raise RiskServiceError("control source returned an untyped observation")
            trusted_source = trusted.get(item.source_id)
            if trusted_source is None or trusted_source.role != item.role:
                raise RiskServiceError("control observation source is not trusted for its declared role")
            if item.role in by_role:
                raise RiskServiceError("duplicate control observation role")
            if (type(item.source_sequence) is not int or item.source_sequence <= 0 or
                    type(item.observed_at_utc_ns) is not int or type(item.expires_at_utc_ns) is not int or not (
                    item.observed_at_utc_ns <= now < item.expires_at_utc_ns)):
                raise RiskServiceError(f"{item.role} observation is expired or from the future")
            if now - item.observed_at_utc_ns > effective_snapshot_age:
                raise RiskServiceError(f"{item.role} observation is stale")
            if item.expires_at_utc_ns - item.observed_at_utc_ns > effective_evidence_ttl:
                raise RiskServiceError(f"{item.role} observation TTL exceeds policy")
            if not isinstance(item.payload, dict) or set(item.payload) != ROLE_PAYLOAD_FIELDS[item.role]:
                raise RiskServiceError(f"{item.role} observation payload schema mismatch")
            try:
                _verify(trusted_source.public_key,
                        _decode_signature(item.signature_b64, f"{item.role} observation signature"),
                        CONTROL_OBSERVATION_DOMAIN + canonical_bytes(item.signed_body()))
            except AuthorizationError as exc:
                raise RiskServiceError(f"{item.role} observation signature verification failed") from exc
            if trusted_source.principal_id in principals or trusted_source.public_key in public_keys:
                raise RiskServiceError("independent control-source roles must have distinct principals and key material")
            principals.add(trusted_source.principal_id); public_keys.add(trusted_source.public_key)
            by_role[item.role] = item
        if set(by_role) != set(REQUIRED_CONTROL_ROLES):
            missing = sorted(set(REQUIRED_CONTROL_ROLES) - set(by_role))
            raise RiskServiceError(f"missing independent control observations: {missing}")
        if risk_signer_raw is not None and risk_signer_raw in public_keys:
            raise RiskServiceError("risk-service signing key must be distinct from every control-source key")

        def payload(role: str) -> dict[str, Any]:
            return by_role[role].payload
        risk = payload("account_risk")
        kill = payload("kill_switch")
        market = payload("market_data")
        recon = payload("reconciliation")
        entitlement = payload("broker_entitlement")
        session = payload("trading_session")
        clock = payload("clock_sync")
        artifact = payload("deployed_artifact")

        if risk["account"] != request.account or kill["account"] not in {request.account, "GLOBAL"}:
            raise RiskServiceError("account risk/kill-switch evidence scope mismatch")
        if market["broker"] != request.broker or market["instrument_id"] != request.instrument_id:
            raise RiskServiceError("market-data health evidence scope mismatch")
        if recon["broker"] != request.broker or recon["account"] != request.account:
            raise RiskServiceError("reconciliation evidence scope mismatch")
        if entitlement["broker"] != request.broker or entitlement["account"] != request.account or entitlement["instrument_id"] != request.instrument_id:
            raise RiskServiceError("broker entitlement evidence scope mismatch")
        if session["venue"] != request.venue or not isinstance(session["session_id"], str) or not session["session_id"]:
            raise RiskServiceError("trading-session evidence scope mismatch")
        if artifact["commit_sha"] != request.commit_sha or artifact["strategy"] != request.strategy:
            raise RiskServiceError("deployed strategy artifact scope mismatch")
        artifact_hash = artifact["strategy_artifact_sha256"]
        if not isinstance(artifact_hash, str) or len(artifact_hash) != 64 or any(c not in "0123456789abcdef" for c in artifact_hash):
            raise RiskServiceError("deployed strategy artifact hash is invalid")
        for name in ("projected_exposure_paise", "projected_daily_loss_paise"):
            if type(risk[name]) is not int or risk[name] < 0:
                raise RiskServiceError(f"account risk {name} must be a non-negative integer")
        if type(kill["enabled"]) is not bool or kill["enabled"]:
            raise RiskServiceError("global/account kill switch is enabled or invalid")
        if type(market["healthy"]) is not bool or not market["healthy"] or type(market["gap_count"]) is not int or market["gap_count"] != 0:
            raise RiskServiceError("market-data health is degraded or has unresolved gaps")
        if type(market["last_event_at_utc_ns"]) is not int or not (0 < market["last_event_at_utc_ns"] <= now):
            raise RiskServiceError("market-data last event timestamp is invalid")
        if now - market["last_event_at_utc_ns"] > effective_snapshot_age:
            raise RiskServiceError("market-data last event is stale")
        if type(recon["clean"]) is not bool or not recon["clean"] or type(recon["break_count"]) is not int or recon["break_count"] != 0:
            raise RiskServiceError("broker reconciliation is not clean")
        if type(entitlement["entitled"]) is not bool or not entitlement["entitled"] or type(entitlement["expires_at_utc_ns"]) is not int or entitlement["expires_at_utc_ns"] <= now:
            raise RiskServiceError("broker entitlement is absent or expired")
        if type(session["valid"]) is not bool or not session["valid"]:
            raise RiskServiceError("venue trading session is not valid")
        if type(clock["offset_ns"]) is not int or abs(clock["offset_ns"]) > effective_clock_limit:
            raise RiskServiceError("clock synchronisation evidence is outside policy")

        latest_attestation = max(x.observed_at_utc_ns for x in observations)
        expiry = min(min(x.expires_at_utc_ns for x in observations),
                     latest_attestation + effective_evidence_ttl)
        if expiry <= now:
            raise RiskServiceError("aggregate runtime evidence expires before authorization")
        risk_observed = by_role["account_risk"].observed_at_utc_ns
        facts = RiskFacts(
            commit_sha=request.commit_sha, broker=request.broker, account=request.account,
            strategy=request.strategy, instrument_id=request.instrument_id, venue=request.venue,
            side=request.side, order_type=request.order_type, product=request.product,
            quantity=request.quantity, notional_paise=request.notional_paise,
            projected_exposure_paise=risk["projected_exposure_paise"],
            projected_daily_loss_paise=risk["projected_daily_loss_paise"],
            now_utc_ns=now, kill_switch_clear=not kill["enabled"],
            market_data_healthy=market["healthy"] and market["gap_count"] == 0,
            reconciliation_clean=recon["clean"] and recon["break_count"] == 0,
            broker_entitlement_current=entitlement["entitled"] and entitlement["expires_at_utc_ns"] > now,
            trading_session_valid=session["valid"], clock_offset_ns=clock["offset_ns"],
            risk_snapshot_age_ns=now - risk_observed,
            strategy_artifact_sha256=artifact_hash,
            attested_at_utc_ns=latest_attestation, expires_at_utc_ns=expiry,
        )
        # Consume sequences transactionally only after all signatures, roles,
        # scopes and payloads have passed. A signing failure intentionally burns
        # the observations; source agents must issue fresh attestations.
        self.replay_ledger.consume(observations)
        return self.risk_service._sign_after_verified_sources(
            facts, capability=_GATE_SIGNING_CAPABILITY)
