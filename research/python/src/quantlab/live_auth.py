"""Independent Ed25519 live-risk permit verification.

This verifier is fail-closed and cannot sign or mint permits.
Private keys must live in an independently controlled KMS/HSM process.  The
application remains globally live-disabled until a Rust authority service,
registered keys, broker certification and regulatory onboarding exist.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import fcntl
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .market import canonical_bytes

PERMIT_DOMAIN = b"NIRNAY-LIVE-RISK-PERMIT-V1\x00"
APPROVAL_DOMAIN = b"NIRNAY-LIVE-RISK-APPROVAL-V1\x00"
RUNTIME_EVIDENCE_DOMAIN = b"NIRNAY-RUNTIME-RISK-EVIDENCE-V1\x00"


class AuthorizationError(ValueError):
    pass


def _decode_key(value: str, field: str) -> bytes:
    try:
        decoded = base64.b64decode(value, validate=True)
    except Exception as exc:
        raise AuthorizationError(f"{field} must be canonical base64") from exc
    if len(decoded) != 32:
        raise AuthorizationError(f"{field} must decode to a 32-byte Ed25519 key")
    return decoded


def _decode_signature(value: str, field: str) -> bytes:
    try:
        decoded = base64.b64decode(value, validate=True)
    except Exception as exc:
        raise AuthorizationError(f"{field} must be base64") from exc
    if len(decoded) != 64:
        raise AuthorizationError(f"{field} must decode to a 64-byte Ed25519 signature")
    return decoded


def _verify(public_key: bytes, signature: bytes, payload: bytes) -> None:
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
        Ed25519PublicKey.from_public_bytes(public_key).verify(signature, payload)
    except ImportError as exc:
        raise AuthorizationError("Ed25519 verification requires cryptography; fail closed") from exc
    except Exception as exc:
        raise AuthorizationError("Ed25519 signature verification failed") from exc


@dataclass(frozen=True, slots=True)
class RuntimeEvidence:
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
    projected_exposure_paise: int
    projected_daily_loss_paise: int
    now_utc_ns: int
    mode_live_configured: bool
    kill_switch_clear: bool
    market_data_healthy: bool
    reconciliation_clean: bool
    broker_entitlement_current: bool
    trading_session_valid: bool
    clock_offset_ns: int
    risk_snapshot_age_ns: int
    risk_policy_sha256: str
    strategy_artifact_sha256: str
    risk_service_key_id: str
    attested_at_utc_ns: int
    expires_at_utc_ns: int
    signature_b64: str

    def signed_body(self) -> dict[str, Any]:
        """Exact canonical runtime facts covered by the independent risk signature."""
        from dataclasses import asdict
        value = asdict(self)
        value.pop("signature_b64")
        return value


@dataclass(frozen=True, slots=True)
class VerifiedAuthorization:
    permit_id: str
    permit_sha256: str
    issuer_key_id: str
    approver_ids: tuple[str, ...]
    expires_at_utc_ns: int
    notional_limit_paise: int
    exposure_limit_paise: int
    risk_service_key_id: str
    runtime_evidence_sha256: str


def verify_live_permit(envelope: dict[str, Any], registry_envelope: dict[str, Any],
                       registry_root_public_key_b64: str, evidence: RuntimeEvidence, *,
                       evaluation_time_utc_ns: int) -> VerifiedAuthorization:
    """Verify issuer + independent approver signatures and exact runtime scope."""
    if not isinstance(envelope, dict) or set(envelope) != {"body", "issuer_signature", "approvals"}:
        raise AuthorizationError("permit envelope has missing or unknown fields")
    body = envelope["body"]
    if not isinstance(body, dict):
        raise AuthorizationError("permit body must be an object")
    required = {
        "schema_version", "permit_id", "commit_sha", "broker", "account", "strategy",
        "instruments", "venues", "sides", "order_types", "products", "max_order_quantity",
        "max_order_notional_paise", "max_open_exposure_paise", "max_daily_loss_paise",
        "risk_policy_sha256", "strategy_artifact_sha256", "issued_at_utc_ns",
        "expires_at_utc_ns", "issuer_key_id", "required_approver_roles",
    }
    if set(body) != required:
        raise AuthorizationError(f"permit body field mismatch: missing={sorted(required-set(body))}, unknown={sorted(set(body)-required)}")
    if body["schema_version"] != 1:
        raise AuthorizationError("unsupported live permit schema version")
    for field in ("permit_id", "commit_sha", "broker", "account", "strategy", "issuer_key_id"):
        if not isinstance(body[field], str) or not body[field].strip():
            raise AuthorizationError(f"{field} must be a non-empty string")
    for field in ("instruments", "venues", "sides", "order_types", "products", "required_approver_roles"):
        value = body[field]
        if not isinstance(value, list) or not value or any(not isinstance(x, str) or not x for x in value):
            raise AuthorizationError(f"{field} must be a non-empty string array")
        if len(value) != len(set(value)):
            raise AuthorizationError(f"{field} contains duplicates")
    for field in ("max_order_quantity", "max_order_notional_paise", "max_open_exposure_paise", "max_daily_loss_paise", "issued_at_utc_ns", "expires_at_utc_ns"):
        if type(body[field]) is not int or body[field] <= 0:
            raise AuthorizationError(f"{field} must be a positive integer")
    for field in ("risk_policy_sha256", "strategy_artifact_sha256"):
        value = body[field]
        if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
            raise AuthorizationError(f"{field} must be lowercase SHA-256")
    if len(body["commit_sha"]) not in {40, 64} or any(c not in "0123456789abcdef" for c in body["commit_sha"]):
        raise AuthorizationError("commit_sha must be a lowercase Git SHA")
    if body["expires_at_utc_ns"] <= body["issued_at_utc_ns"]:
        raise AuthorizationError("permit expiry must follow issue time")
    if not isinstance(registry_envelope, dict) or set(registry_envelope) != {"body", "root_signature"}:
        raise AuthorizationError("trust registry must be a root-signed envelope")
    registry = registry_envelope["body"]
    if not isinstance(registry, dict) or registry.get("schema_version") != 1:
        raise AuthorizationError("invalid trust registry")
    registry_required = {"schema_version", "issued_at_utc_ns", "expires_at_utc_ns", "approval_threshold",
                         "max_clock_offset_ns", "max_risk_snapshot_age_ns", "max_runtime_evidence_ttl_ns",
                         "max_permit_ttl_ns", "issuers", "approvers", "risk_services", "control_sources"}
    if set(registry) != registry_required:
        raise AuthorizationError("trust registry field mismatch")
    root_public_key = _decode_key(registry_root_public_key_b64, "registry root key")
    _verify(root_public_key,
            _decode_signature(registry_envelope["root_signature"], "registry root signature"),
            b"NIRNAY-TRUST-REGISTRY-V1\x00" + canonical_bytes(registry))
    if type(evaluation_time_utc_ns) is not int or evaluation_time_utc_ns <= 0:
        raise AuthorizationError("verifier evaluation time must be a positive trusted UTC nanosecond timestamp")
    if evaluation_time_utc_ns < body["issued_at_utc_ns"] or evaluation_time_utc_ns >= body["expires_at_utc_ns"]:
        raise AuthorizationError("permit is not currently valid")
    if evidence.commit_sha != body["commit_sha"]:
        raise AuthorizationError("permit commit does not match running build")
    # Bind every dimension that an OMS adapter might otherwise reinterpret.
    for field, actual in (("broker", evidence.broker), ("account", evidence.account), ("strategy", evidence.strategy)):
        if actual != body[field]:
            raise AuthorizationError(f"permit {field} scope mismatch")
    scopes = (("instruments", evidence.instrument_id), ("venues", evidence.venue),
              ("sides", evidence.side), ("order_types", evidence.order_type), ("products", evidence.product))
    for field, actual in scopes:
        if actual not in body[field]:
            raise AuthorizationError(f"{field} scope does not permit {actual}")
    if evidence.quantity <= 0 or evidence.quantity > body["max_order_quantity"]:
        raise AuthorizationError("order quantity exceeds permit")
    if evidence.notional_paise <= 0 or evidence.notional_paise > body["max_order_notional_paise"]:
        raise AuthorizationError("order notional exceeds permit")
    if evidence.projected_exposure_paise < 0 or evidence.projected_exposure_paise > body["max_open_exposure_paise"]:
        raise AuthorizationError("projected exposure exceeds permit")
    if evidence.projected_daily_loss_paise < 0 or evidence.projected_daily_loss_paise > body["max_daily_loss_paise"]:
        raise AuthorizationError("projected daily loss exceeds permit")
    if evidence.risk_policy_sha256 != body["risk_policy_sha256"] or evidence.strategy_artifact_sha256 != body["strategy_artifact_sha256"]:
        raise AuthorizationError("risk policy or strategy artifact hash does not match permit")
    boolean_fields = ("mode_live_configured", "kill_switch_clear", "market_data_healthy",
                      "reconciliation_clean", "broker_entitlement_current", "trading_session_valid")
    if any(type(getattr(evidence, name)) is not bool for name in boolean_fields):
        raise AuthorizationError("runtime gate evidence must use strict booleans")
    numeric_fields = ("quantity", "notional_paise", "projected_exposure_paise",
                      "projected_daily_loss_paise", "now_utc_ns", "attested_at_utc_ns",
                      "expires_at_utc_ns", "clock_offset_ns", "risk_snapshot_age_ns")
    if any(type(getattr(evidence, name)) is not int for name in numeric_fields):
        raise AuthorizationError("runtime evidence numeric fields must be integers")
    if (evidence.quantity <= 0 or evidence.notional_paise <= 0 or
            evidence.now_utc_ns <= 0 or evidence.projected_exposure_paise < 0 or
            evidence.projected_daily_loss_paise < 0 or evidence.risk_snapshot_age_ns < 0):
        raise AuthorizationError("runtime risk quantities or timestamps are invalid")
    for field_name in ("broker", "account", "strategy", "instrument_id", "venue", "side",
                       "order_type", "product", "risk_service_key_id"):
        value = getattr(evidence, field_name)
        if not isinstance(value, str) or not value.strip():
            raise AuthorizationError(f"runtime evidence {field_name} must be a non-empty string")
    for field_name in ("commit_sha", "risk_policy_sha256", "strategy_artifact_sha256"):
        value = getattr(evidence, field_name)
        allowed_lengths = {40, 64} if field_name == "commit_sha" else {64}
        if not isinstance(value, str) or len(value) not in allowed_lengths or any(c not in "0123456789abcdef" for c in value):
            raise AuthorizationError(f"runtime evidence {field_name} must be lowercase hexadecimal")
    if not isinstance(evidence.risk_service_key_id, str) or not evidence.risk_service_key_id:
        raise AuthorizationError("runtime evidence risk-service key id is required")
    if not isinstance(evidence.signature_b64, str) or not evidence.signature_b64:
        raise AuthorizationError("runtime evidence signature is required")
    if evidence.attested_at_utc_ns <= 0 or evidence.expires_at_utc_ns <= evidence.attested_at_utc_ns:
        raise AuthorizationError("runtime evidence attestation window is invalid")
    max_evidence_ttl = registry.get("max_runtime_evidence_ttl_ns")
    if type(max_evidence_ttl) is not int or max_evidence_ttl <= 0 or evidence.expires_at_utc_ns - evidence.attested_at_utc_ns > max_evidence_ttl:
        raise AuthorizationError("runtime evidence lifetime exceeds trust-registry policy")
    if not (evidence.attested_at_utc_ns <= evaluation_time_utc_ns < evidence.expires_at_utc_ns):
        raise AuthorizationError("runtime evidence attestation is not currently valid")
    max_clock_offset = registry.get("max_clock_offset_ns")
    if type(max_clock_offset) is not int or max_clock_offset < 0:
        raise AuthorizationError("trust registry clock policy is invalid")
    if abs(evidence.now_utc_ns - evaluation_time_utc_ns) > max_clock_offset:
        raise AuthorizationError("risk service and verifier clocks exceed permitted skew")

    risk_services = registry.get("risk_services", {})
    risk_service = risk_services.get(evidence.risk_service_key_id) if isinstance(risk_services, dict) else None
    if (not isinstance(risk_service, dict) or risk_service.get("revoked") is True or
            evidence.risk_service_key_id in registry.get("issuers", {}) or
            evidence.risk_service_key_id in registry.get("approvers", {})):
        raise AuthorizationError("runtime risk-service signer is unknown, revoked or not independent")
    risk_service_public_key = _decode_key(risk_service.get("public_key_b64", ""), "risk-service public key")
    control_sources = registry.get("control_sources")
    required_control_roles = {"account_risk", "kill_switch", "market_data", "reconciliation",
                              "broker_entitlement", "trading_session", "clock_sync", "deployed_artifact"}
    if not isinstance(control_sources, dict) or not control_sources:
        raise AuthorizationError("trust registry must contain independently managed control_sources")
    control_roles: set[str] = set()
    control_principals: set[str] = set()
    control_public_keys: list[bytes] = []
    for source_id, source in control_sources.items():
        if (not isinstance(source_id, str) or not source_id or not isinstance(source, dict) or
                set(source) != {"role", "principal_id", "public_key_b64", "revoked"}):
            raise AuthorizationError("control-source trust record is malformed")
        role = source["role"]
        principal = source["principal_id"]
        if role not in required_control_roles or role in control_roles:
            raise AuthorizationError("control-source trust roles must be exactly one per required role")
        if not isinstance(principal, str) or not principal or principal in control_principals:
            raise AuthorizationError("control-source principals must be unique")
        if type(source["revoked"]) is not bool or source["revoked"]:
            raise AuthorizationError("control-source key is revoked or has invalid revocation flag")
        control_roles.add(role)
        control_principals.add(principal)
        control_public_keys.append(_decode_key(source["public_key_b64"], "control-source public key"))
    if control_roles != required_control_roles:
        raise AuthorizationError("trust registry is missing one or more independent control-source roles")
    if len(set(control_public_keys)) != len(control_public_keys):
        raise AuthorizationError("control-source public keys must be cryptographically distinct")
    runtime_body = evidence.signed_body()
    runtime_bytes = RUNTIME_EVIDENCE_DOMAIN + canonical_bytes(runtime_body)
    _verify(risk_service_public_key,
            _decode_signature(evidence.signature_b64, "runtime evidence signature"), runtime_bytes)

    gates = {
        "live mode": evidence.mode_live_configured,
        "kill switch": evidence.kill_switch_clear,
        "market data": evidence.market_data_healthy,
        "reconciliation": evidence.reconciliation_clean,
        "broker entitlement": evidence.broker_entitlement_current,
        "trading session": evidence.trading_session_valid,
    }
    failed = [name for name, passed in gates.items() if not passed]
    if failed:
        raise AuthorizationError("runtime gate failed: " + ", ".join(failed))
    if abs(evidence.clock_offset_ns) > max_clock_offset:
        raise AuthorizationError("clock synchronisation evidence is outside policy")
    max_snapshot_age = registry.get("max_risk_snapshot_age_ns")
    if type(max_snapshot_age) is not int or max_snapshot_age < 0 or evidence.risk_snapshot_age_ns > max_snapshot_age:
        raise AuthorizationError("risk snapshot is stale or has an invalid age")

    registry_issued = registry.get("issued_at_utc_ns")
    registry_expiry = registry.get("expires_at_utc_ns")
    if (type(registry_issued) is not int or type(registry_expiry) is not int or
            not (registry_issued <= body["issued_at_utc_ns"] <= evaluation_time_utc_ns < registry_expiry)):
        raise AuthorizationError("trust registry snapshot does not cover permit issuance and verification")
    issuer = registry.get("issuers", {}).get(body["issuer_key_id"])
    if not isinstance(issuer, dict) or issuer.get("revoked") is True:
        raise AuthorizationError("issuer is unknown or revoked")
    issuer_principal = issuer.get("principal_id")
    risk_principal = risk_service.get("principal_id")
    approver_principals = {a.get("approver_id") for a in registry.get("approvers", {}).values() if isinstance(a, dict)}
    if not isinstance(issuer_principal, str) or not issuer_principal or not isinstance(risk_principal, str) or not risk_principal:
        raise AuthorizationError("issuer and risk-service principal identities must be explicit")
    if len({issuer_principal, risk_principal, *approver_principals, *control_principals}) != 2 + len(approver_principals) + len(control_principals):
        raise AuthorizationError("issuer, risk service, approver and control-source principals must be independent")
    issuer_public_key = _decode_key(issuer.get("public_key_b64", ""), "issuer public key")
    max_permit_ttl = registry.get("max_permit_ttl_ns")
    if type(max_permit_ttl) is not int or max_permit_ttl <= 0 or body["expires_at_utc_ns"] - body["issued_at_utc_ns"] > max_permit_ttl:
        raise AuthorizationError("permit lifetime exceeds trust-registry policy")
    body_bytes = PERMIT_DOMAIN + canonical_bytes(body)
    _verify(issuer_public_key,
            _decode_signature(envelope["issuer_signature"], "issuer signature"), body_bytes)
    permit_digest = hashlib.sha256(body_bytes).digest()
    approvals = envelope["approvals"]
    if not isinstance(approvals, list):
        raise AuthorizationError("approvals must be an array")
    required_count = registry.get("approval_threshold")
    if type(required_count) is not int or required_count < 2:
        raise AuthorizationError("independent approval threshold must be at least two")
    if len(approvals) < required_count:
        raise AuthorizationError("insufficient independent approvals")
    trusted = registry.get("approvers", {})
    seen_ids: set[str] = set()
    seen_keys: set[str] = set()
    seen_roles: set[str] = set()
    approver_public_keys: list[bytes] = []
    for item in approvals:
        if not isinstance(item, dict) or set(item) != {"approver_id", "key_id", "signature"}:
            raise AuthorizationError("malformed approval attestation")
        approver_id, key_id = item["approver_id"], item["key_id"]
        if not all(isinstance(x, str) and x for x in (approver_id, key_id, item["signature"])):
            raise AuthorizationError("approval identities and signatures must be non-empty strings")
        if approver_id in seen_ids or key_id in seen_keys:
            raise AuthorizationError("approver identities and keys must be distinct")
        seen_ids.add(approver_id); seen_keys.add(key_id)
        approver = trusted.get(key_id)
        if not isinstance(approver, dict) or approver.get("approver_id") != approver_id or approver.get("revoked") is True:
            raise AuthorizationError("approver key is unknown, mismatched or revoked")
        role = approver.get("role")
        if not isinstance(role, str) or role in seen_roles:
            raise AuthorizationError("approver roles must be distinct (separation of duties)")
        seen_roles.add(role)
        payload = APPROVAL_DOMAIN + permit_digest
        approver_public_key = _decode_key(approver.get("public_key_b64", ""), "approver public key")
        approver_public_keys.append(approver_public_key)
        _verify(approver_public_key,
                _decode_signature(item["signature"], "approval signature"), payload)
    required_roles = set(body["required_approver_roles"])
    if not required_roles.issubset(seen_roles):
        raise AuthorizationError("required independent approver roles are missing")
    if len(seen_ids) < required_count:
        raise AuthorizationError("not enough distinct approvers")
    # Distinct key labels are insufficient: the same Ed25519 key material must
    # never be trusted in two roles (including the offline registry root).
    all_public_keys = [root_public_key, risk_service_public_key, issuer_public_key, *approver_public_keys, *control_public_keys]
    if len(set(all_public_keys)) != len(all_public_keys):
        raise AuthorizationError("cryptographic public keys must be distinct across root, risk, issuer and approver roles")
    return VerifiedAuthorization(body["permit_id"], hashlib.sha256(canonical_bytes(body)).hexdigest(),
                                  body["issuer_key_id"], tuple(sorted(seen_ids)),
                                  body["expires_at_utc_ns"], body["max_order_notional_paise"],
                                  body["max_open_exposure_paise"], evidence.risk_service_key_id,
                                  hashlib.sha256(runtime_bytes).hexdigest())


class AuthorizationUseLedger:
    """Durable one-time permit-consumption ledger for a future live gateway.

    This is a replay-control primitive, not a live trading switch. A permit is
    consumed for one client order contract; the live OMS in this repository
    remains fail-closed and does not submit broker orders.
    """

    def __init__(self, path: str | Path):
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.touch(mode=0o600, exist_ok=True)
        if os.name == "posix":
            os.chmod(self.path, 0o600)
        self.lock_path = self.path.with_name(self.path.name + ".lock")
        self.lock_path.touch(mode=0o600, exist_ok=True)
        if os.name == "posix":
            os.chmod(self.lock_path, 0o600)

    def consume(self, authorization: VerifiedAuthorization, *, client_order_id: str,
                order_contract_sha256: str, now_utc_ns: int) -> dict[str, Any]:
        if not isinstance(authorization, VerifiedAuthorization):
            raise AuthorizationError("only a verified authorization can be consumed")
        if not isinstance(client_order_id, str) or not client_order_id.strip():
            raise AuthorizationError("client_order_id is required for permit consumption")
        if not isinstance(order_contract_sha256, str) or len(order_contract_sha256) != 64 or any(
                c not in "0123456789abcdef" for c in order_contract_sha256):
            raise AuthorizationError("order_contract_sha256 must be lowercase SHA-256")
        if type(now_utc_ns) is not int or now_utc_ns <= 0 or now_utc_ns >= authorization.expires_at_utc_ns:
            raise AuthorizationError("authorization is expired or use timestamp is invalid")
        self.lock_path.parent.mkdir(parents=True, exist_ok=True)
        with self.lock_path.open("a+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            try:
                raw = self.path.read_bytes()
                if raw and not raw.endswith(b"\n"):
                    raise AuthorizationError("permit-use ledger has a torn tail")
                previous = "0" * 64
                records = []
                for i, line in enumerate(raw.splitlines(), 1):
                    try:
                        row = json.loads(line)
                    except json.JSONDecodeError as exc:
                        raise AuthorizationError(f"permit-use ledger is malformed at line {i}") from exc
                    if not isinstance(row, dict) or set(row) != {"schema_version", "sequence", "payload", "previous_hash", "record_hash"}:
                        raise AuthorizationError(f"permit-use ledger envelope is invalid at line {i}")
                    if (type(row["schema_version"]) is not int or row["schema_version"] != 1 or
                            type(row["sequence"]) is not int or row["sequence"] != i or
                            row["previous_hash"] != previous or not isinstance(row["payload"], dict)):
                        raise AuthorizationError(f"permit-use ledger chain/sequence is invalid at line {i}")
                    expected = hashlib.sha256(canonical_bytes({k: v for k, v in row.items() if k != "record_hash"})).hexdigest()
                    if row["record_hash"] != expected:
                        raise AuthorizationError(f"permit-use ledger hash mismatch at line {i}")
                    records.append(row)
                    previous = expected
                if any(r["payload"].get("permit_id") == authorization.permit_id for r in records):
                    raise AuthorizationError("permit has already been consumed")
                payload = {"permit_id": authorization.permit_id, "permit_sha256": authorization.permit_sha256,
                           "client_order_id": client_order_id, "order_contract_sha256": order_contract_sha256,
                           "consumed_at_utc_ns": now_utc_ns}
                body = {"schema_version": 1, "sequence": len(records) + 1,
                        "payload": payload, "previous_hash": previous}
                body["record_hash"] = hashlib.sha256(canonical_bytes(body)).hexdigest()
                with self.path.open("ab") as stream:
                    stream.write(canonical_bytes(body) + b"\n")
                    stream.flush()
                    os.fsync(stream.fileno())
                return body
            finally:
                fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
