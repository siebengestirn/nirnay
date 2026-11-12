from __future__ import annotations

import base64
import json
from dataclasses import asdict

import pytest

from quantlab.risk_evidence import OrderRiskRequest, SignedControlObservation
from quantlab.risk_gateway import MutualTlsObservationSource
from quantlab.risk_daemon import RuntimeEvidenceApplication, _validate_daemon_config
from quantlab.risk_service import RiskServiceError


class _FakeTlsContext:
    minimum_version = None
    def load_cert_chain(self, **_kwargs):
        return None


class _FakeResponse:
    status = 200
    def __init__(self, body): self.body = body
    def __enter__(self): return self
    def __exit__(self, *_args): return False
    def read(self, size=-1): return self.body[:size]


@pytest.fixture
def control_files(tmp_path):
    paths = {}
    for name in ("ca.pem", "client.pem", "client.key"):
        path = tmp_path / name
        path.write_text("test-only placeholder")
        paths[name] = str(path)
    return paths


def _request():
    return OrderRiskRequest(
        commit_sha="a" * 40, broker="zerodha", account="acct-1",
        strategy="strategy-1", instrument_id="NSE:SBIN", venue="NSE",
        side="buy", order_type="limit", product="CNC", quantity=1,
        notional_paise=50_000,
    )


def test_mutual_tls_source_collects_typed_observation_and_passes_it_to_gate(monkeypatch, control_files):
    import quantlab.risk_gateway as gateway_module
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from quantlab.risk_evidence import CONTROL_OBSERVATION_DOMAIN
    from quantlab.market import canonical_bytes

    private = Ed25519PrivateKey.generate()
    provisional = SignedControlObservation(
        source_id="account-risk-source", role="account_risk", source_sequence=3,
        observed_at_utc_ns=1_800_000_000_000_000_000,
        expires_at_utc_ns=1_800_000_002_000_000_000,
        payload={"account": "acct-1", "projected_exposure_paise": 50_000,
                 "projected_daily_loss_paise": 0}, signature_b64="")
    signature = private.sign(CONTROL_OBSERVATION_DOMAIN + canonical_bytes(provisional.signed_body()))
    response = {**provisional.signed_body(),
                "signature_b64": base64.b64encode(signature).decode("ascii")}
    monkeypatch.setattr(gateway_module.ssl, "create_default_context", lambda *_a, **_k: _FakeTlsContext())
    monkeypatch.setattr(gateway_module.urllib.request, "urlopen",
                        lambda *_a, **_k: _FakeResponse(json.dumps(response).encode()))
    source = MutualTlsObservationSource(
        {"account-risk-source": "https://risk-source.internal/v1/observation"},
        ca_file=control_files["ca.pem"], client_cert_file=control_files["client.pem"],
        client_key_file=control_files["client.key"])
    observations = source.collect(_request(), now_utc_ns=1_800_000_001_000_000_000)
    assert len(observations) == 1
    assert observations[0].source_id == "account-risk-source"
    assert observations[0].source_sequence == 3


def test_mutual_tls_source_rejects_non_https_and_embedded_credentials(control_files):
    for endpoint in ("http://example.com/observation", "https://user:pass@example.com/observation"):
        with pytest.raises(ValueError):
            MutualTlsObservationSource({"source": endpoint}, ca_file=control_files["ca.pem"],
                client_cert_file=control_files["client.pem"], client_key_file=control_files["client.key"])


def test_mutual_tls_source_rejects_endpoint_identity_mismatch(monkeypatch, control_files):
    import quantlab.risk_gateway as gateway_module
    body = {"schema_version": 1, "source_id": "unexpected", "role": "account_risk",
            "source_sequence": 1, "observed_at_utc_ns": 1, "expires_at_utc_ns": 2,
            "payload": {}, "signature_b64": "AA=="}
    monkeypatch.setattr(gateway_module.ssl, "create_default_context", lambda *_a, **_k: _FakeTlsContext())
    monkeypatch.setattr(gateway_module.urllib.request, "urlopen",
                        lambda *_a, **_k: _FakeResponse(json.dumps(body).encode()))
    source = MutualTlsObservationSource({"expected": "https://source.internal/obs"},
        ca_file=control_files["ca.pem"], client_cert_file=control_files["client.pem"],
        client_key_file=control_files["client.key"])
    with pytest.raises(RiskServiceError, match="different source identity"):
        source.collect(_request(), now_utc_ns=10)


def test_daemon_refuses_config_without_exact_client_certificate_allowlist(control_files):
    config = {
        "server": {"bind_host": "127.0.0.1", "port": 9443,
                   "server_cert_file": control_files["client.pem"],
                   "server_key_file": control_files["client.key"],
                   "client_ca_file": control_files["ca.pem"],
                   "allowed_client_cert_sha256": [], "risk_signing_enabled": False},
        "registry": {"path": "/tmp/registry.json", "root_public_key_b64": "x"},
        "risk_policy": {"path": "/tmp/policy.json"},
        "risk_signer": {"kms_key_arn": "arn:aws:kms:ap-south-1:123:key/abc", "region": "ap-south-1"},
        "replay_ledger": {"path": "/tmp/replay.sqlite"},
        "control_sources": {"source_urls": {"s": "https://source.internal"},
                            "ca_file": control_files["ca.pem"],
                            "client_cert_file": control_files["client.pem"],
                            "client_key_file": control_files["client.key"], "timeout_seconds": 2},
    }
    with pytest.raises(ValueError, match="fingerprints"):
        _validate_daemon_config(config)


def test_runtime_evidence_http_application_rejects_disabled_and_extra_request_fields():
    class FakeGate:
        source = object()
        def authorize(self, *_args, **_kwargs):
            raise AssertionError("disabled gate must not authorize")
    disabled = RuntimeEvidenceApplication(gate=FakeGate(), risk_signing_enabled=False)
    status, body = disabled.evaluate(json.dumps(asdict(_request())).encode())
    assert status == 503 and "disabled" in body["error"]

    enabled = RuntimeEvidenceApplication(gate=FakeGate(), risk_signing_enabled=True)
    invalid = {**asdict(_request()), "runtime_evidence": True}
    status, body = enabled.evaluate(json.dumps(invalid).encode())
    assert status == 400 and body["error"] == "invalid order-risk request"
