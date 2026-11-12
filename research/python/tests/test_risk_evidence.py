from __future__ import annotations

import base64
from dataclasses import replace

import pytest

from quantlab.live_auth import RUNTIME_EVIDENCE_DOMAIN
from quantlab.market import canonical_bytes
from quantlab.risk_evidence import (
    CONTROL_OBSERVATION_DOMAIN,
    REQUIRED_CONTROL_ROLES,
    OrderRiskRequest,
    RiskServiceError,
    SignedControlObservation,
    SignedEvidenceRiskGate,
    InMemoryObservationReplayLedger, SQLiteObservationReplayLedger,
)
from quantlab.risk_service import IndependentRiskService, RiskPolicy

cryptography = pytest.importorskip("cryptography")
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


NOW = 1_800_000_000_000_000_000


def _pub(private):
    return base64.b64encode(private.public_key().public_bytes(
        Encoding.Raw, PublicFormat.Raw)).decode("ascii")


class LocalSigner:
    def __init__(self, private=None):
        self.private = private or Ed25519PrivateKey.generate()
        self.public_key_b64 = _pub(self.private)

    def sign(self, message: bytes) -> bytes:
        return self.private.sign(message)


def _policy():
    return RiskPolicy(
        policy_id="nse-cash-risk-v1", max_order_quantity=10,
        max_order_notional_paise=100_000, max_open_exposure_paise=500_000,
        max_daily_loss_paise=10_000, max_clock_offset_ns=5_000_000,
        max_snapshot_age_ns=1_000_000_000, max_evidence_ttl_ns=5_000_000_000,
        allowed_brokers=("zerodha",), allowed_accounts=("acct-1",),
        allowed_strategies=("strategy-1",), allowed_instruments=("NSE:SBIN",),
        allowed_venues=("NSE",), allowed_sides=("buy",),
        allowed_order_types=("limit",), allowed_products=("CNC",),
    )


def _request():
    return OrderRiskRequest(
        commit_sha="a" * 40, broker="zerodha", account="acct-1",
        strategy="strategy-1", instrument_id="NSE:SBIN", venue="NSE",
        side="buy", order_type="limit", product="CNC", quantity=1,
        notional_paise=50_000,
    )


class FakeObservationSource:
    def __init__(self, keys, payloads=None, *, roles=REQUIRED_CONTROL_ROLES, source_sequence=1):
        self.keys = keys
        self.payloads = payloads or _payloads()
        self.roles = roles
        self.source_sequence = source_sequence

    def collect(self, request, *, now_utc_ns):
        return tuple(SignedControlObservation.sign(
            source_id=f"source-{role}", role=role, source_sequence=self.source_sequence,
            observed_at_utc_ns=NOW - 100_000,
            expires_at_utc_ns=NOW + 2_000_000_000,
            payload=self.payloads[role], signer=self.keys[role],
        ) for role in self.roles)


def _payloads():
    return {
        "account_risk": {"account": "acct-1", "projected_exposure_paise": 50_000,
                         "projected_daily_loss_paise": 0},
        "kill_switch": {"account": "acct-1", "enabled": False},
        "market_data": {"broker": "zerodha", "instrument_id": "NSE:SBIN",
                         "healthy": True, "gap_count": 0,
                         "last_event_at_utc_ns": NOW - 200_000},
        "reconciliation": {"broker": "zerodha", "account": "acct-1",
                            "clean": True, "break_count": 0},
        "broker_entitlement": {"broker": "zerodha", "account": "acct-1",
                               "instrument_id": "NSE:SBIN", "entitled": True,
                               "expires_at_utc_ns": NOW + 10_000_000_000},
        "trading_session": {"venue": "NSE", "valid": True, "session_id": "NSE-2026-11-12"},
        "clock_sync": {"offset_ns": 1_000},
        "deployed_artifact": {"commit_sha": "a" * 40, "strategy": "strategy-1",
                              "strategy_artifact_sha256": "b" * 64},
    }


def _fixture():
    root = Ed25519PrivateKey.generate()
    risk_signer = LocalSigner()
    source_keys = {role: LocalSigner() for role in REQUIRED_CONTROL_ROLES}
    issuer = LocalSigner()
    approver_risk = LocalSigner()
    approver_ops = LocalSigner()
    registry_body = {
        "schema_version": 1,
        "issued_at_utc_ns": NOW - 1_000_000_000,
        "expires_at_utc_ns": NOW + 60_000_000_000,
        "approval_threshold": 2,
        "max_clock_offset_ns": 5_000_000,
        "max_risk_snapshot_age_ns": 1_000_000_000,
        "max_runtime_evidence_ttl_ns": 5_000_000_000,
        "max_permit_ttl_ns": 30_000_000_000,
        "issuers": {"issuer-1": {"public_key_b64": issuer.public_key_b64,
                                  "principal_id": "release-manager", "revoked": False}},
        "approvers": {
            "approver-risk": {"approver_id": "risk-approver", "public_key_b64": approver_risk.public_key_b64,
                               "role": "risk", "revoked": False},
            "approver-ops": {"approver_id": "ops-approver", "public_key_b64": approver_ops.public_key_b64,
                              "role": "operations", "revoked": False},
        },
        "risk_services": {"risk-service-1": {"public_key_b64": risk_signer.public_key_b64,
                                              "principal_id": "independent-risk-service", "revoked": False}},
        "control_sources": {
            f"source-{role}": {"role": role, "principal_id": f"independent-{role}",
                               "public_key_b64": source_keys[role].public_key_b64, "revoked": False}
            for role in REQUIRED_CONTROL_ROLES
        },
    }
    registry = {
        "body": registry_body,
        "root_signature": base64.b64encode(root.sign(
            b"NIRNAY-TRUST-REGISTRY-V1\x00" + canonical_bytes(registry_body))).decode("ascii"),
    }
    service = IndependentRiskService(_policy(), key_id="risk-service-1", signer=risk_signer)
    # The authorization gate is never constructed with a volatile replay
    # ledger, even in unit tests: replay protection must survive process restart.
    import tempfile
    from pathlib import Path
    directory = tempfile.TemporaryDirectory(prefix="nirnay-risk-test-")
    replay = SQLiteObservationReplayLedger(Path(directory.name) / "replay.sqlite3")
    replay._test_directory = directory
    gate = SignedEvidenceRiskGate(service, registry_envelope=registry,
                                  registry_root_public_key_b64=_pub(root),
                                  replay_ledger=replay, clock_ns=lambda: NOW)
    return gate, source_keys, risk_signer, registry


def test_signed_evidence_gate_builds_runtime_evidence_from_all_independent_sources():
    gate, keys, risk_signer, _ = _fixture()
    evidence = gate.authorize(_request(), FakeObservationSource(keys), live_mode_configured=True)
    risk_signer.private.public_key().verify(
        base64.b64decode(evidence.signature_b64),
        RUNTIME_EVIDENCE_DOMAIN + canonical_bytes(evidence.signed_body()),
    )
    assert evidence.risk_service_key_id == "risk-service-1"
    assert evidence.kill_switch_clear is True
    assert evidence.market_data_healthy is True
    assert evidence.reconciliation_clean is True
    assert evidence.broker_entitlement_current is True
    assert evidence.trading_session_valid is True
    assert evidence.expires_at_utc_ns <= NOW + 2_000_000_000


def test_signed_evidence_gate_refuses_disabled_live_mode_and_missing_sources():
    gate, keys, _, _ = _fixture()
    source = FakeObservationSource(keys)
    with pytest.raises(RiskServiceError, match="not configured"):
        gate.authorize(_request(), source, live_mode_configured=False)
    with pytest.raises(RiskServiceError, match="missing independent control observations"):
        gate.authorize(_request(), FakeObservationSource(keys, roles=REQUIRED_CONTROL_ROLES[:-1]),
                       live_mode_configured=True)


def test_signed_evidence_gate_refuses_tampered_and_stale_source_observations():
    gate, keys, _, _ = _fixture()
    source = FakeObservationSource(keys)
    good = source.collect(_request(), now_utc_ns=NOW)
    tampered = list(good)
    tampered[0] = replace(tampered[0], payload={**tampered[0].payload, "projected_exposure_paise": 1})
    class StaticSource:
        def collect(self, request, *, now_utc_ns): return tuple(tampered)
    with pytest.raises(RiskServiceError, match="signature verification failed"):
        gate.authorize(_request(), StaticSource(), live_mode_configured=True)
    stale = list(good)
    stale[0] = SignedControlObservation.sign(
        source_id=stale[0].source_id, role=stale[0].role, source_sequence=1,
        observed_at_utc_ns=NOW - 2_000_000_000,
        expires_at_utc_ns=NOW + 2_000_000_000,
        payload=stale[0].payload, signer=keys[stale[0].role])
    class StaleSource:
        def collect(self, request, *, now_utc_ns): return tuple(stale)
    with pytest.raises(RiskServiceError, match="stale"):
        gate.authorize(_request(), StaleSource(), live_mode_configured=True)


@pytest.mark.parametrize(("role", "change"), [
    ("kill_switch", {"enabled": True}),
    ("market_data", {"gap_count": 1}),
    ("reconciliation", {"clean": False}),
    ("broker_entitlement", {"entitled": False}),
    ("trading_session", {"valid": False}),
])
def test_signed_evidence_gate_refuses_each_failed_operational_control(role, change):
    gate, keys, _, _ = _fixture()
    payloads = _payloads()
    payloads[role] = {**payloads[role], **change}
    with pytest.raises(RiskServiceError):
        gate.authorize(_request(), FakeObservationSource(keys, payloads), live_mode_configured=True)


def test_signed_evidence_gate_rejects_risk_signer_not_in_root_trust_registry():
    gate, keys, _, _ = _fixture()
    other = LocalSigner()
    # Reconstruct only the risk service with a key not in the root-signed registry.
    from quantlab.risk_service import IndependentRiskService
    bad_service = IndependentRiskService(_policy(), key_id="risk-service-1", signer=other)
    bad_gate = SignedEvidenceRiskGate(bad_service,
        registry_envelope=gate.registry_envelope,
        registry_root_public_key_b64=gate.registry_root_public_key_b64,
        replay_ledger=SQLiteObservationReplayLedger(__import__("pathlib").Path(__import__("tempfile").mkdtemp()) / "replay.sqlite3"), clock_ns=lambda: NOW)
    with pytest.raises(RiskServiceError, match="does not match root trust registry"):
        bad_gate.authorize(_request(), FakeObservationSource(keys), live_mode_configured=True)


def test_signed_control_observations_are_one_time_and_sequences_must_increase():
    gate, keys, _, _ = _fixture()
    source = FakeObservationSource(keys, source_sequence=1)
    first = gate.authorize(_request(), source, live_mode_configured=True)
    assert first.signature_b64
    with pytest.raises(RiskServiceError, match="replayed or regressed"):
        gate.authorize(_request(), source, live_mode_configured=True)
    fresh_source = FakeObservationSource(keys, source_sequence=2)
    second = gate.authorize(_request(), fresh_source, live_mode_configured=True)
    assert second.signature_b64


def test_sqlite_replay_ledger_is_persistent_atomic_and_private(tmp_path):
    from quantlab.risk_evidence import SQLiteObservationReplayLedger

    gate, keys, _, _ = _fixture()
    observations = FakeObservationSource(keys, source_sequence=1).collect(_request(), now_utc_ns=NOW)
    path = tmp_path / "risk" / "control-replay.sqlite3"
    ledger = SQLiteObservationReplayLedger(path)
    ledger.consume(observations)
    assert path.stat().st_mode & 0o777 == 0o600

    # Reconstructing the ledger must not make a previously consumed sequence valid.
    recovered = SQLiteObservationReplayLedger(path)
    with pytest.raises(RiskServiceError, match="replayed or regressed"):
        recovered.consume(observations)

    # A mixed batch must be all-or-nothing: one stale sequence cannot partially
    # advance the other seven source high-water marks.
    seq2 = list(FakeObservationSource(keys, source_sequence=2).collect(_request(), now_utc_ns=NOW))
    seq2[-1] = observations[-1]
    with pytest.raises(RiskServiceError, match="replayed or regressed"):
        recovered.consume(tuple(seq2))
    recovered.consume(FakeObservationSource(keys, source_sequence=2).collect(_request(), now_utc_ns=NOW))


def test_risk_gate_rejects_volatile_in_memory_replay_ledger():
    gate, _, _, _ = _fixture()
    with pytest.raises(ValueError, match="persistent transactional"):
        SignedEvidenceRiskGate(gate.risk_service,
            registry_envelope=gate.registry_envelope,
            registry_root_public_key_b64=gate.registry_root_public_key_b64,
            replay_ledger=InMemoryObservationReplayLedger(), clock_ns=lambda: NOW)
