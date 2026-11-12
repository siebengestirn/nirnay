"""Independent risk-service decision core for signed runtime evidence.

This module creates a bounded, domain-separated signature over the exact
runtime facts consumed by `verify_live_permit`. It deliberately does not issue
permits, approve orders, connect to brokers, or enable live routing. Production
independence requires deploying it as a separately administered service with
separate IAM/KMS policy, network identity, audit retention and operator roles.
"""
from __future__ import annotations

import base64
import hashlib
from dataclasses import asdict, dataclass
from typing import Protocol

from .live_auth import RUNTIME_EVIDENCE_DOMAIN, RuntimeEvidence
from .market import canonical_bytes


class RiskServiceError(RuntimeError):
    pass


# Module-private token used only by the signed-observation gate. It is not a
# security boundary; production must isolate this service in its own process.
_GATE_SIGNING_CAPABILITY = object()


class EvidenceSigner(Protocol):
    def sign(self, message: bytes) -> bytes: ...


@dataclass(frozen=True, slots=True)
class RiskPolicy:
    policy_id: str
    max_order_quantity: int
    max_order_notional_paise: int
    max_open_exposure_paise: int
    max_daily_loss_paise: int
    max_clock_offset_ns: int
    max_snapshot_age_ns: int
    max_evidence_ttl_ns: int
    allowed_brokers: tuple[str, ...]
    allowed_accounts: tuple[str, ...]
    allowed_strategies: tuple[str, ...]
    allowed_instruments: tuple[str, ...]
    allowed_venues: tuple[str, ...]
    allowed_sides: tuple[str, ...]
    allowed_order_types: tuple[str, ...]
    allowed_products: tuple[str, ...]

    def validate(self) -> None:
        if not self.policy_id.strip():
            raise RiskServiceError("policy_id is required")
        positive = (self.max_order_quantity, self.max_order_notional_paise,
                    self.max_open_exposure_paise, self.max_daily_loss_paise,
                    self.max_evidence_ttl_ns)
        if any(type(x) is not int or x <= 0 for x in positive):
            raise RiskServiceError("risk policy caps and evidence TTL must be positive integers")
        nonnegative = (self.max_clock_offset_ns, self.max_snapshot_age_ns)
        if any(type(x) is not int or x < 0 for x in nonnegative):
            raise RiskServiceError("clock and snapshot age limits must be non-negative integers")
        for name in ("allowed_brokers", "allowed_accounts", "allowed_strategies", "allowed_instruments",
                     "allowed_venues", "allowed_sides", "allowed_order_types", "allowed_products"):
            values = getattr(self, name)
            if not values or any(not isinstance(x, str) or not x.strip() for x in values):
                raise RiskServiceError(f"{name} must be a non-empty tuple of non-empty strings")
            if len(values) != len(set(values)):
                raise RiskServiceError(f"{name} contains duplicates")

    @property
    def sha256(self) -> str:
        return hashlib.sha256(canonical_bytes(asdict(self))).hexdigest()


@dataclass(frozen=True, slots=True)
class RiskFacts:
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
    kill_switch_clear: bool
    market_data_healthy: bool
    reconciliation_clean: bool
    broker_entitlement_current: bool
    trading_session_valid: bool
    clock_offset_ns: int
    risk_snapshot_age_ns: int
    strategy_artifact_sha256: str
    attested_at_utc_ns: int
    expires_at_utc_ns: int


class IndependentRiskService:
    """Pure risk gate and signer; run separately from OMS/strategy in production."""

    def __init__(self, policy: RiskPolicy, *, key_id: str, signer: EvidenceSigner):
        policy.validate()
        if not key_id or not hasattr(signer, "sign"):
            raise RiskServiceError("independent signer and key_id are required")
        self.policy, self.key_id, self.signer = policy, key_id, signer

    def evaluate_and_sign(self, facts: RiskFacts, *, live_mode_configured: bool) -> RuntimeEvidence:
        """Reject direct caller-assembled facts.

        Runtime evidence must be issued through SignedEvidenceRiskGate, which
        verifies the root trust registry and source signatures before calling
        the private capability-gated signer. This public method intentionally
        remains as a fail-closed compatibility surface.
        """
        raise RiskServiceError(
            "direct RiskFacts signing is disabled; use SignedEvidenceRiskGate with independently signed controls"
        )

    def _validate_facts(self, facts: RiskFacts, *, live_mode_configured: bool) -> RuntimeEvidence:
        p = self.policy
        if type(live_mode_configured) is not bool or not live_mode_configured:
            raise RiskServiceError("live mode is not configured; no runtime risk evidence will be signed")
        scopes = {
            "broker": (facts.broker, p.allowed_brokers),
            "account": (facts.account, p.allowed_accounts),
            "strategy": (facts.strategy, p.allowed_strategies),
            "instrument": (facts.instrument_id, p.allowed_instruments),
            "venue": (facts.venue, p.allowed_venues),
            "side": (facts.side, p.allowed_sides),
            "order_type": (facts.order_type, p.allowed_order_types),
            "product": (facts.product, p.allowed_products),
        }
        for name, (value, allowed) in scopes.items():
            if value not in allowed:
                raise RiskServiceError(f"risk policy does not permit {name}")
        if type(facts.quantity) is not int or not (0 < facts.quantity <= p.max_order_quantity):
            raise RiskServiceError("order quantity exceeds risk policy")
        if type(facts.notional_paise) is not int or not (0 < facts.notional_paise <= p.max_order_notional_paise):
            raise RiskServiceError("order notional exceeds risk policy")
        if type(facts.projected_exposure_paise) is not int or not (0 <= facts.projected_exposure_paise <= p.max_open_exposure_paise):
            raise RiskServiceError("projected exposure exceeds risk policy")
        if type(facts.projected_daily_loss_paise) is not int or not (0 <= facts.projected_daily_loss_paise <= p.max_daily_loss_paise):
            raise RiskServiceError("projected daily loss exceeds risk policy")
        if any(type(x) is not bool or not x for x in (
                facts.kill_switch_clear, facts.market_data_healthy, facts.reconciliation_clean,
                facts.broker_entitlement_current, facts.trading_session_valid)):
            raise RiskServiceError("a runtime control is false or not a strict boolean")
        if type(facts.clock_offset_ns) is not int or abs(facts.clock_offset_ns) > p.max_clock_offset_ns:
            raise RiskServiceError("clock offset exceeds risk policy")
        if type(facts.risk_snapshot_age_ns) is not int or not (0 <= facts.risk_snapshot_age_ns <= p.max_snapshot_age_ns):
            raise RiskServiceError("risk snapshot is stale or invalid")
        for name in ("now_utc_ns", "attested_at_utc_ns", "expires_at_utc_ns"):
            if type(getattr(facts, name)) is not int or getattr(facts, name) <= 0:
                raise RiskServiceError(f"{name} must be a positive UTC nanosecond timestamp")
        if not facts.attested_at_utc_ns <= facts.now_utc_ns < facts.expires_at_utc_ns:
            raise RiskServiceError("risk attestation must cover the current trusted time")
        if facts.expires_at_utc_ns - facts.attested_at_utc_ns > p.max_evidence_ttl_ns:
            raise RiskServiceError("runtime evidence lifetime exceeds risk policy")
        if len(facts.commit_sha) not in {40, 64} or any(c not in "0123456789abcdef" for c in facts.commit_sha):
            raise RiskServiceError("commit_sha must be a lowercase Git SHA")
        if len(facts.strategy_artifact_sha256) != 64 or any(c not in "0123456789abcdef" for c in facts.strategy_artifact_sha256):
            raise RiskServiceError("strategy_artifact_sha256 must be lowercase SHA-256")
        return RuntimeEvidence(
            commit_sha=facts.commit_sha, broker=facts.broker, account=facts.account,
            strategy=facts.strategy, instrument_id=facts.instrument_id, venue=facts.venue,
            side=facts.side, order_type=facts.order_type, product=facts.product,
            quantity=facts.quantity, notional_paise=facts.notional_paise,
            projected_exposure_paise=facts.projected_exposure_paise,
            projected_daily_loss_paise=facts.projected_daily_loss_paise,
            now_utc_ns=facts.now_utc_ns, mode_live_configured=True,
            kill_switch_clear=facts.kill_switch_clear,
            market_data_healthy=facts.market_data_healthy,
            reconciliation_clean=facts.reconciliation_clean,
            broker_entitlement_current=facts.broker_entitlement_current,
            trading_session_valid=facts.trading_session_valid,
            clock_offset_ns=facts.clock_offset_ns,
            risk_snapshot_age_ns=facts.risk_snapshot_age_ns,
            risk_policy_sha256=p.sha256,
            strategy_artifact_sha256=facts.strategy_artifact_sha256,
            risk_service_key_id=self.key_id,
            attested_at_utc_ns=facts.attested_at_utc_ns,
            expires_at_utc_ns=facts.expires_at_utc_ns,
            signature_b64="",
        )

    def _sign_after_verified_sources(self, facts: RiskFacts, *, capability: object) -> RuntimeEvidence:
        """Internal signing path used only after SignedEvidenceRiskGate validation."""
        if capability is not _GATE_SIGNING_CAPABILITY:
            raise RiskServiceError("risk signer requires independently verified source evidence")
        unsigned = self._validate_facts(facts, live_mode_configured=True)
        try:
            signature = self.signer.sign(RUNTIME_EVIDENCE_DOMAIN + canonical_bytes(unsigned.signed_body()))
        except Exception as exc:
            raise RiskServiceError(f"risk evidence signing failed: {type(exc).__name__}") from exc
        if not isinstance(signature, bytes) or len(signature) != 64:
            raise RiskServiceError("risk signer returned an invalid Ed25519 signature")
        return RuntimeEvidence(**{**asdict(unsigned),
                                  "signature_b64": base64.b64encode(signature).decode("ascii")})
