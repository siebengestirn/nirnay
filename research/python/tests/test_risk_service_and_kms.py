from __future__ import annotations

import base64
from dataclasses import replace

import pytest

from quantlab.kms_signing import AwsKmsEd25519Signer, KmsSigningError
from quantlab.risk_service import IndependentRiskService, RiskFacts, RiskPolicy, RiskServiceError
from quantlab.live_auth import RUNTIME_EVIDENCE_DOMAIN
from quantlab.market import canonical_bytes

cryptography = pytest.importorskip("cryptography")
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat, PrivateFormat, NoEncryption


class FakeKms:
    def __init__(self, private=None, *, key_spec="ECC_NIST_EDWARDS25519", key_state="Enabled"):
        self.private = private or Ed25519PrivateKey.generate()
        self.key_spec = key_spec
        self.key_state = key_state
        self.calls = []
    def describe_key(self, *, KeyId):
        return {"KeyMetadata": {"KeyId": KeyId, "Arn": "arn:aws:kms:ap-south-1:123:key/test",
            "KeySpec": self.key_spec, "KeyUsage": "SIGN_VERIFY", "KeyState": self.key_state,
            "KeyManager": "CUSTOMER"}}
    def get_public_key(self, *, KeyId):
        return {"PublicKey": self.private.public_key().public_bytes(Encoding.DER, PublicFormat.SubjectPublicKeyInfo)}
    def sign(self, **kwargs):
        self.calls.append(kwargs)
        return {"Signature": self.private.sign(kwargs["Message"])}


class LocalSigner:
    def __init__(self): self.private = Ed25519PrivateKey.generate()
    @property
    def public_key_b64(self):
        raw = self.private.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        return base64.b64encode(raw).decode("ascii")
    def sign(self, message: bytes) -> bytes:
        return self.private.sign(message)


def test_kms_signer_validates_key_metadata_and_verifies_every_signature():
    fake = FakeKms()
    signer = AwsKmsEd25519Signer(fake, "risk-key")
    message = b"NIRNAY-TEST-DOMAIN\x00payload"
    signature = signer.sign(message)
    signer.verify(message, signature)
    assert len(base64.b64decode(signer.public_key_b64)) == 32
    assert fake.calls[0]["MessageType"] == "RAW"
    assert fake.calls[0]["SigningAlgorithm"] == "ED25519_SHA_512"
    with pytest.raises(KmsSigningError):
        signer.verify(message + b"tamper", signature)


def test_kms_signer_rejects_wrong_key_spec_and_non_ed25519_signature():
    with pytest.raises(KmsSigningError, match="must use"):
        AwsKmsEd25519Signer(FakeKms(key_spec="ECC_NIST_P256"), "bad")
    fake = FakeKms()
    signer = AwsKmsEd25519Signer(fake, "risk-key")
    fake.sign = lambda **_kwargs: {"Signature": b"x" * 64}
    with pytest.raises(KmsSigningError, match="failed local"):
        signer.sign(b"payload")


def _policy():
    return RiskPolicy(
        policy_id="nse-cash-risk-v1", max_order_quantity=10,
        max_order_notional_paise=100_000, max_open_exposure_paise=500_000,
        max_daily_loss_paise=10_000, max_clock_offset_ns=5_000_000,
        max_snapshot_age_ns=1_000_000_000, max_evidence_ttl_ns=5_000_000_000,
        allowed_brokers=("zerodha",), allowed_accounts=("acct-1",),
        allowed_strategies=("strategy-1",), allowed_instruments=("NSE:SBIN",),
        allowed_venues=("NSE",), allowed_sides=("buy", "sell"),
        allowed_order_types=("limit",), allowed_products=("CNC",),
    )


def _facts():
    now = 1_800_000_000_000_000_000
    return RiskFacts(
        commit_sha="a" * 40, broker="zerodha", account="acct-1", strategy="strategy-1",
        instrument_id="NSE:SBIN", venue="NSE", side="buy", order_type="limit", product="CNC",
        quantity=1, notional_paise=50_000, projected_exposure_paise=50_000,
        projected_daily_loss_paise=0, now_utc_ns=now, kill_switch_clear=True,
        market_data_healthy=True, reconciliation_clean=True, broker_entitlement_current=True,
        trading_session_valid=True, clock_offset_ns=1000, risk_snapshot_age_ns=1000,
        strategy_artifact_sha256="b" * 64, attested_at_utc_ns=now-100_000_000,
        expires_at_utc_ns=now+2_000_000_000,
    )


def test_direct_riskfacts_signing_is_blocked_even_for_valid_facts():
    service = IndependentRiskService(_policy(), key_id="risk-service-key-1", signer=LocalSigner())
    with pytest.raises(RiskServiceError, match="direct RiskFacts signing is disabled"):
        service.evaluate_and_sign(_facts(), live_mode_configured=True)
    # The non-signing validator remains testable; the signature capability is
    # held exclusively by SignedEvidenceRiskGate after source verification.
    unsigned = service._validate_facts(_facts(), live_mode_configured=True)
    assert unsigned.signature_b64 == ""
    assert unsigned.risk_service_key_id == "risk-service-key-1"
    assert unsigned.risk_policy_sha256 == _policy().sha256
    assert unsigned.mode_live_configured is True


def test_risk_service_refuses_disabled_mode_failed_control_and_out_of_scope_order():
    service = IndependentRiskService(_policy(), key_id="risk-service-key-1", signer=LocalSigner())
    with pytest.raises(RiskServiceError, match="not configured"):
        service._validate_facts(_facts(), live_mode_configured=False)
    with pytest.raises(RiskServiceError, match="runtime control"):
        service._validate_facts(replace(_facts(), reconciliation_clean=False), live_mode_configured=True)
    with pytest.raises(RiskServiceError, match="notional"):
        service._validate_facts(replace(_facts(), notional_paise=100_001), live_mode_configured=True)
    with pytest.raises(RiskServiceError, match="instrument"):
        service._validate_facts(replace(_facts(), instrument_id="NSE:UNKNOWN"), live_mode_configured=True)


def test_risk_service_refuses_stale_snapshots_and_oversized_ttl():
    service = IndependentRiskService(_policy(), key_id="risk-service-key-1", signer=LocalSigner())
    with pytest.raises(RiskServiceError, match="stale"):
        service._validate_facts(replace(_facts(), risk_snapshot_age_ns=2_000_000_000), live_mode_configured=True)
    facts = _facts()
    with pytest.raises(RiskServiceError, match="lifetime"):
        service._validate_facts(replace(facts, expires_at_utc_ns=facts.attested_at_utc_ns+6_000_000_000), live_mode_configured=True)
