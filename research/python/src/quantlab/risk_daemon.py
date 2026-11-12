"""mTLS runtime-evidence service; this process never submits broker orders.

The daemon obtains control observations directly from configured independent
source endpoints, verifies their signatures through SignedEvidenceRiskGate,
and signs the resulting scoped RuntimeEvidence using AWS KMS. Its default
configuration refuses signing until an operator deliberately enables it.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import ssl
import sys
import time
import tomllib
from dataclasses import fields
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

from .kms_signing import AwsKmsEd25519Signer
from .risk_evidence import (OrderRiskRequest, SQLiteObservationReplayLedger,
                            SignedEvidenceRiskGate)
from .risk_gateway import MutualTlsObservationSource
from .risk_service import IndependentRiskService, RiskPolicy, RiskServiceError

MAX_REQUEST_BYTES = 16 * 1024


def _load_json_object(path: str, label: str) -> dict[str, Any]:
    value = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be a JSON object")
    return value


def _validate_daemon_config(config: dict[str, Any]) -> dict[str, Any]:
    required_sections = {"server", "registry", "risk_policy", "risk_signer",
                         "replay_ledger", "control_sources"}
    if not isinstance(config, dict) or set(config) != required_sections:
        raise ValueError(f"daemon config must contain exactly {sorted(required_sections)}")
    server = config["server"]
    if not isinstance(server, dict) or set(server) != {
            "bind_host", "port", "server_cert_file", "server_key_file",
            "client_ca_file", "allowed_client_cert_sha256", "risk_signing_enabled"}:
        raise ValueError("server config schema mismatch")
    if not isinstance(server["bind_host"], str) or not server["bind_host"]:
        raise ValueError("server bind_host is required")
    if type(server["port"]) is not int or not 1 <= server["port"] <= 65535:
        raise ValueError("server port must be in 1..65535")
    if type(server["risk_signing_enabled"]) is not bool:
        raise ValueError("risk_signing_enabled must be a boolean")
    fingerprints = server["allowed_client_cert_sha256"]
    if (not isinstance(fingerprints, list) or not fingerprints or
            any(not isinstance(x, str) or len(x) != 64 or any(c not in "0123456789abcdef" for c in x)
                for x in fingerprints) or len(set(fingerprints)) != len(fingerprints)):
        raise ValueError("allowed_client_cert_sha256 must contain unique lowercase SHA-256 fingerprints")
    for key in ("server_cert_file", "server_key_file", "client_ca_file"):
        if not isinstance(server[key], str) or not Path(server[key]).is_file():
            raise ValueError(f"server TLS file is missing: {key}")
    for section, key in (("registry", "path"), ("risk_policy", "path"),
                          ("replay_ledger", "path")):
        raw = config[section]
        if not isinstance(raw, dict) or set(raw) != {key} or not isinstance(raw[key], str) or not raw[key]:
            raise ValueError(f"{section}.{key} is required")
    registry = config["registry"]
    if set(registry) != {"path", "root_public_key_b64"} or not registry["root_public_key_b64"]:
        raise ValueError("registry must include path and root_public_key_b64")
    signer = config["risk_signer"]
    if not isinstance(signer, dict) or set(signer) != {"kms_key_arn", "region"}:
        raise ValueError("risk_signer config schema mismatch")
    if not all(isinstance(signer[x], str) and signer[x] for x in ("kms_key_arn", "region")):
        raise ValueError("risk signer KMS ARN and region are required")
    sources = config["control_sources"]
    if not isinstance(sources, dict) or set(sources) != {
            "source_urls", "ca_file", "client_cert_file", "client_key_file", "timeout_seconds"}:
        raise ValueError("control_sources config schema mismatch")
    if not all(isinstance(sources[x], str) and Path(sources[x]).is_file()
               for x in ("ca_file", "client_cert_file", "client_key_file")):
        raise ValueError("control-source mTLS CA/certificate/key files must exist")
    if not isinstance(sources["source_urls"], dict) or not sources["source_urls"]:
        raise ValueError("control_sources.source_urls must be a non-empty table")
    for source_id, url in sources["source_urls"].items():
        if not isinstance(source_id, str) or not isinstance(url, str) or not url.startswith("https://"):
            raise ValueError("every production control source endpoint must use HTTPS")
    timeout = sources["timeout_seconds"]
    if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not 0.1 <= timeout <= 10:
        raise ValueError("control-source timeout_seconds must be in 0.1..10")
    return config


class RuntimeEvidenceApplication:
    def __init__(self, *, gate: SignedEvidenceRiskGate, risk_signing_enabled: bool):
        self.gate = gate
        self.risk_signing_enabled = risk_signing_enabled

    def evaluate(self, body: bytes) -> tuple[int, dict[str, Any]]:
        if not self.risk_signing_enabled:
            return 503, {"error": "risk evidence signing is disabled by deployment policy"}
        if len(body) > MAX_REQUEST_BYTES:
            return 413, {"error": "request too large"}
        try:
            value = json.loads(body)
            allowed = {item.name for item in fields(OrderRiskRequest)}
            if not isinstance(value, dict) or set(value) != allowed:
                raise ValueError("request schema mismatch")
            request = OrderRiskRequest(**value)
            evidence = self.gate.authorize(request, self.source, live_mode_configured=True)
            from dataclasses import asdict
            return 200, {"runtime_evidence": asdict(evidence)}
        except (json.JSONDecodeError, UnicodeDecodeError, TypeError, ValueError):
            return 400, {"error": "invalid order-risk request"}
        except RiskServiceError as exc:
            # Risk rejections are generic to the caller; detailed reason codes
            # belong in protected service metrics/audit, not public responses.
            return 403, {"error": "runtime risk evidence refused", "reason_code": type(exc).__name__}

    @classmethod
    def from_config(cls, config: dict[str, Any]) -> "RuntimeEvidenceApplication":
        config = _validate_daemon_config(config)
        registry = _load_json_object(config["registry"]["path"], "trust registry")
        policy_raw = _load_json_object(config["risk_policy"]["path"], "risk policy")
        tuple_fields = {"allowed_brokers", "allowed_accounts", "allowed_strategies",
                        "allowed_instruments", "allowed_venues", "allowed_sides",
                        "allowed_order_types", "allowed_products"}
        for field in tuple_fields:
            if field in policy_raw:
                policy_raw[field] = tuple(policy_raw[field])
        policy = RiskPolicy(**policy_raw)
        signer = AwsKmsEd25519Signer.from_default_aws_credentials(
            config["risk_signer"]["kms_key_arn"], region_name=config["risk_signer"]["region"])
        service = IndependentRiskService(policy, key_id=config["risk_signer"]["kms_key_arn"], signer=signer)
        replay = SQLiteObservationReplayLedger(config["replay_ledger"]["path"])
        sources = config["control_sources"]
        observation_source = MutualTlsObservationSource(
            sources["source_urls"], ca_file=sources["ca_file"],
            client_cert_file=sources["client_cert_file"],
            client_key_file=sources["client_key_file"],
            timeout_seconds=sources["timeout_seconds"])
        registry_sources = registry.get("body", {}).get("control_sources", {})
        if set(sources["source_urls"]) != set(registry_sources):
            raise ValueError("control-source endpoint IDs must exactly match the root-signed trust registry")
        gate = SignedEvidenceRiskGate(service, registry_envelope=registry,
            registry_root_public_key_b64=config["registry"]["root_public_key_b64"],
            replay_ledger=replay, clock_ns=time.time_ns)
        app = cls(gate=gate, risk_signing_enabled=config["server"]["risk_signing_enabled"])
        app.source = observation_source
        return app


def create_server(config: dict[str, Any], app: RuntimeEvidenceApplication) -> ThreadingHTTPServer:
    config = _validate_daemon_config(config)
    server_config = config["server"]
    allowed_fingerprints = set(server_config["allowed_client_cert_sha256"])

    class Handler(BaseHTTPRequestHandler):
        server_version = "NirnayRiskEvidence/1"
        sys_version = ""

        def log_message(self, _format: str, *args: Any) -> None:
            # Never log request bodies, client data or auth material.
            return

        def _respond(self, status: int, value: dict[str, Any]) -> None:
            payload = json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("Pragma", "no-cache")
            self.end_headers()
            self.wfile.write(payload)

        def do_GET(self) -> None:
            self._respond(404, {"error": "not found"})

        def do_POST(self) -> None:
            if self.path != "/v1/runtime-evidence":
                self._respond(404, {"error": "not found"}); return
            peer = self.connection.getpeercert(binary_form=True)
            if not peer or hashlib.sha256(peer).hexdigest() not in allowed_fingerprints:
                self._respond(403, {"error": "client identity not authorized"}); return
            if self.headers.get("Content-Type", "").split(";", 1)[0].strip().lower() != "application/json":
                self._respond(415, {"error": "application/json required"}); return
            try:
                length = int(self.headers.get("Content-Length", "-1"))
            except ValueError:
                length = -1
            if length < 1 or length > MAX_REQUEST_BYTES:
                self._respond(413, {"error": "invalid content length"}); return
            body = self.rfile.read(length)
            if len(body) != length:
                self._respond(400, {"error": "incomplete request"}); return
            status, response = app.evaluate(body)
            self._respond(status, response)

    server = ThreadingHTTPServer((server_config["bind_host"], server_config["port"]), Handler)
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.minimum_version = ssl.TLSVersion.TLSv1_2
    tls.load_cert_chain(server_config["server_cert_file"], server_config["server_key_file"])
    tls.load_verify_locations(cafile=server_config["client_ca_file"])
    tls.verify_mode = ssl.CERT_REQUIRED
    server.socket = tls.wrap_socket(server.socket, server_side=True)
    return server


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Nirnay independent runtime-risk evidence service")
    parser.add_argument("--config", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        config = _validate_daemon_config(tomllib.loads(args.config.read_text(encoding="utf-8")))
        app = RuntimeEvidenceApplication.from_config(config)
        server = create_server(config, app)
    except Exception as exc:
        print(f"risk service startup refused: {type(exc).__name__}", file=sys.stderr)
        return 2
    try:
        server.serve_forever(poll_interval=0.5)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
