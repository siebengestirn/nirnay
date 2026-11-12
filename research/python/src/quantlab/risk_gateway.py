"""Mutual-TLS client for independently operated runtime control sources.

The risk-service process calls these source endpoints itself. An OMS caller can
submit an OrderRiskRequest but cannot supply observation payloads or signatures.
"""
from __future__ import annotations

import json
import ssl
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import asdict
from typing import Any, Mapping
from urllib.parse import urlparse

from .risk_evidence import OrderRiskRequest, SignedControlObservation
from .risk_service import RiskServiceError
from .market import canonical_bytes

MAX_SOURCE_RESPONSE_BYTES = 32 * 1024


class MutualTlsObservationSource:
    """Collect one independently signed observation from each configured source.

    Production URLs must be HTTPS. HTTP is allowed only for literal loopback
    addresses in explicit unit tests; it is never enabled by the daemon config.
    One worker identity certificate is used for mTLS client authentication to
    the source services. Each response is independently signature-verified by
    SignedEvidenceRiskGate against the offline root-signed trust registry.
    """

    def __init__(self, source_urls: Mapping[str, str], *, ca_file: str,
                 client_cert_file: str, client_key_file: str,
                 timeout_seconds: float = 2.0,
                 allow_insecure_loopback_for_tests: bool = False):
        if not isinstance(source_urls, Mapping) or not source_urls:
            raise ValueError("source_urls must map trusted source IDs to endpoints")
        if not 0.1 <= float(timeout_seconds) <= 10.0:
            raise ValueError("control source timeout must be between 0.1 and 10 seconds")
        self.source_urls = dict(source_urls)
        for source_id, value in self.source_urls.items():
            if not isinstance(source_id, str) or not source_id or not isinstance(value, str):
                raise ValueError("control source IDs and URLs must be non-empty strings")
            parsed = urlparse(value)
            loopback_http = parsed.scheme == "http" and parsed.hostname in {"127.0.0.1", "::1", "localhost"}
            if parsed.scheme != "https" and not (allow_insecure_loopback_for_tests and loopback_http):
                raise ValueError(f"control source {source_id} must use HTTPS with certificate verification")
            if parsed.username or parsed.password or parsed.fragment:
                raise ValueError(f"control source {source_id} URL must not embed credentials or fragments")
        self.timeout_seconds = float(timeout_seconds)
        self._context = ssl.create_default_context(ssl.Purpose.SERVER_AUTH, cafile=ca_file)
        self._context.minimum_version = ssl.TLSVersion.TLSv1_2
        self._context.load_cert_chain(certfile=client_cert_file, keyfile=client_key_file)

    def _collect_one(self, source_id: str, endpoint: str, request: OrderRiskRequest,
                     now_utc_ns: int) -> SignedControlObservation:
        body = canonical_bytes({
            "schema_version": 1,
            "source_id": source_id,
            "now_utc_ns": now_utc_ns,
            "order_request": asdict(request),
        })
        req = urllib.request.Request(endpoint, data=body,
                                     headers={"Content-Type": "application/json",
                                              "Accept": "application/json"},
                                     method="POST")
        try:
            with urllib.request.urlopen(req, context=self._context,
                                        timeout=self.timeout_seconds) as response:
                if response.status != 200:
                    raise RiskServiceError(f"control source {source_id} returned HTTP {response.status}")
                raw = response.read(MAX_SOURCE_RESPONSE_BYTES + 1)
        except urllib.error.HTTPError as exc:
            raise RiskServiceError(f"control source {source_id} returned HTTP {exc.code}") from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            # Avoid embedding URLs, headers, response bodies or credentials in logs/errors.
            raise RiskServiceError(f"control source {source_id} unavailable ({type(exc).__name__})") from exc
        if len(raw) > MAX_SOURCE_RESPONSE_BYTES:
            raise RiskServiceError(f"control source {source_id} response exceeds size limit")
        try:
            value = json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise RiskServiceError(f"control source {source_id} returned malformed JSON") from exc
        expected = {"schema_version", "source_id", "role", "source_sequence",
                    "observed_at_utc_ns", "expires_at_utc_ns", "payload", "signature_b64"}
        if not isinstance(value, dict) or set(value) != expected or value.get("schema_version") != 1:
            raise RiskServiceError(f"control source {source_id} response schema mismatch")
        if value.get("source_id") != source_id:
            raise RiskServiceError(f"control source endpoint {source_id} returned a different source identity")
        try:
            return SignedControlObservation(
                source_id=value["source_id"], role=value["role"],
                source_sequence=value["source_sequence"],
                observed_at_utc_ns=value["observed_at_utc_ns"],
                expires_at_utc_ns=value["expires_at_utc_ns"],
                payload=value["payload"], signature_b64=value["signature_b64"],
            )
        except (TypeError, KeyError) as exc:
            raise RiskServiceError(f"control source {source_id} response fields are invalid") from exc

    def collect(self, request: OrderRiskRequest, *, now_utc_ns: int) -> tuple[SignedControlObservation, ...]:
        request.validate()
        if type(now_utc_ns) is not int or now_utc_ns <= 0:
            raise RiskServiceError("risk service trusted time is invalid")
        collected: dict[str, SignedControlObservation] = {}
        with ThreadPoolExecutor(max_workers=min(8, len(self.source_urls))) as pool:
            futures = {
                pool.submit(self._collect_one, source_id, endpoint, request, now_utc_ns): source_id
                for source_id, endpoint in self.source_urls.items()
            }
            try:
                for future in as_completed(futures):
                    source_id = futures[future]
                    collected[source_id] = future.result()
            except Exception:
                for future in futures:
                    future.cancel()
                raise
        return tuple(collected[source_id] for source_id in sorted(collected))
