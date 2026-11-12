import base64
from dataclasses import replace
import copy
import hashlib
import tempfile
import unittest
from pathlib import Path
from decimal import Decimal

from quantlab.live_auth import (APPROVAL_DOMAIN, PERMIT_DOMAIN, RUNTIME_EVIDENCE_DOMAIN,
    AuthorizationError, RuntimeEvidence, AuthorizationUseLedger, verify_live_permit)
from quantlab.market import canonical_bytes
from quantlab.oms import (BrokerOrderIntent, IdempotencyConflict, InvalidTransition,
    LiveRoutingDisabled, OrderManagementSystem)


def _pub(private):
    from cryptography.hazmat.primitives import serialization
    return base64.b64encode(private.public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw)).decode()


def signed_authorisation(now=1_800_000_000_000_000_000):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    import hashlib
    root = Ed25519PrivateKey.generate()
    issuer = Ed25519PrivateKey.generate()
    risk = Ed25519PrivateKey.generate()
    ops = Ed25519PrivateKey.generate()
    risk_service = Ed25519PrivateKey.generate()
    control_role_names = ("account_risk", "kill_switch", "market_data", "reconciliation",
                          "broker_entitlement", "trading_session", "clock_sync", "deployed_artifact")
    control_keys = {role: Ed25519PrivateKey.generate() for role in control_role_names}
    registry = {
        "schema_version": 1, "issued_at_utc_ns": now - 1_000_000_000,
        "expires_at_utc_ns": now + 60_000_000_000,
        "approval_threshold": 2, "max_clock_offset_ns": 1_000_000,
        "max_risk_snapshot_age_ns": 1_000_000_000,
        "max_runtime_evidence_ttl_ns": 5_000_000_000,
        "max_permit_ttl_ns": 30_000_000_000,
        "issuers": {"release-1": {"public_key_b64": _pub(issuer), "principal_id": "release-manager", "revoked": False}},
        "risk_services": {"risk-service-1": {"public_key_b64": _pub(risk_service), "principal_id": "risk-service", "revoked": False}},
        "control_sources": {f"control-{role}": {"role": role, "principal_id": f"authority-{role}",
            "public_key_b64": _pub(key), "revoked": False} for role, key in control_keys.items()},
        "approvers": {
            "risk-key": {"approver_id": "risk-a", "public_key_b64": _pub(risk), "role": "risk", "revoked": False},
            "ops-key": {"approver_id": "ops-b", "public_key_b64": _pub(ops), "role": "operations", "revoked": False},
        },
    }
    registry_envelope = {"body": registry,
        "root_signature": base64.b64encode(root.sign(b"NIRNAY-TRUST-REGISTRY-V1\x00" + canonical_bytes(registry))).decode()}
    body = {
        "schema_version": 1, "permit_id": "permit-001", "commit_sha": "a" * 40,
        "broker": "upstox", "account": "acct-test", "strategy": "fixture.ema-cross.v1",
        "instruments": ["in.sbin"], "venues": ["NSE"], "sides": ["buy"],
        "order_types": ["limit"], "products": ["CNC"],
        "max_order_quantity": 10, "max_order_notional_paise": 1000000,
        "max_open_exposure_paise": 5000000, "max_daily_loss_paise": 100000,
        "risk_policy_sha256": "b" * 64, "strategy_artifact_sha256": "c" * 64,
        "issued_at_utc_ns": now - 100, "expires_at_utc_ns": now + 10_000_000_000,
        "issuer_key_id": "release-1", "required_approver_roles": ["risk", "operations"],
    }
    issuer_payload = PERMIT_DOMAIN + canonical_bytes(body)
    digest = hashlib.sha256(issuer_payload).digest()
    envelope = {
        "body": body,
        "issuer_signature": base64.b64encode(issuer.sign(issuer_payload)).decode(),
        "approvals": [
            {"approver_id": "risk-a", "key_id": "risk-key",
             "signature": base64.b64encode(risk.sign(APPROVAL_DOMAIN + digest)).decode()},
            {"approver_id": "ops-b", "key_id": "ops-key",
             "signature": base64.b64encode(ops.sign(APPROVAL_DOMAIN + digest)).decode()},
        ],
    }
    evidence = RuntimeEvidence(
        commit_sha="a"*40, broker="upstox", account="acct-test", strategy="fixture.ema-cross.v1",
        instrument_id="in.sbin", venue="NSE", side="buy", order_type="limit", product="CNC",
        quantity=1, notional_paise=50000, projected_exposure_paise=50000,
        projected_daily_loss_paise=0, now_utc_ns=now, mode_live_configured=True,
        kill_switch_clear=True, market_data_healthy=True, reconciliation_clean=True,
        broker_entitlement_current=True, trading_session_valid=True, clock_offset_ns=-1000,
        risk_snapshot_age_ns=1000, risk_policy_sha256="b"*64, strategy_artifact_sha256="c"*64,
        risk_service_key_id="risk-service-1", attested_at_utc_ns=now - 1_000_000_000,
        expires_at_utc_ns=now + 4_000_000_000, signature_b64="")
    evidence = replace(evidence, signature_b64=base64.b64encode(
        risk_service.sign(RUNTIME_EVIDENCE_DOMAIN + canonical_bytes(evidence.signed_body()))).decode())
    return root, registry_envelope, envelope, evidence, risk_service


class LiveAuthorizationTests(unittest.TestCase):
    def test_independent_root_issuer_and_role_distinct_approvals_verify(self):
        root, registry, envelope, evidence, risk_service = signed_authorisation()
        verified = verify_live_permit(envelope, registry, _pub(root), evidence,
                                      evaluation_time_utc_ns=evidence.now_utc_ns)
        self.assertEqual(verified.permit_id, "permit-001")
        self.assertEqual(verified.approver_ids, ("ops-b", "risk-a"))
        self.assertEqual(verified.notional_limit_paise, 1000000)

    def test_tampered_body_fails_issuer_signature(self):
        root, registry, envelope, evidence, risk_service = signed_authorisation()
        changed = copy.deepcopy(envelope)
        changed["body"]["max_order_quantity"] = 100
        with self.assertRaises(AuthorizationError):
            verify_live_permit(changed, registry, _pub(root), evidence,
                               evaluation_time_utc_ns=evidence.now_utc_ns)

    def test_untrusted_registry_fails_root_signature(self):
        root, registry, envelope, evidence, risk_service = signed_authorisation()
        registry["body"]["approval_threshold"] = 1
        with self.assertRaises(AuthorizationError):
            verify_live_permit(envelope, registry, _pub(root), evidence,
                               evaluation_time_utc_ns=evidence.now_utc_ns)

    def test_stale_reconciliation_gate_fails_closed(self):
        root, registry, envelope, evidence, risk_service = signed_authorisation()
        invalid = replace(evidence, reconciliation_clean=False)
        invalid = replace(invalid, signature_b64=base64.b64encode(
            risk_service.sign(RUNTIME_EVIDENCE_DOMAIN + canonical_bytes(invalid.signed_body()))).decode())
        with self.assertRaisesRegex(AuthorizationError, "reconciliation"):
            verify_live_permit(envelope, registry, _pub(root), invalid,
                               evaluation_time_utc_ns=invalid.now_utc_ns)

    def test_unsigned_or_tampered_runtime_risk_evidence_fails(self):
        root, registry, envelope, evidence, risk_service = signed_authorisation()
        changed = replace(evidence, kill_switch_clear=False)
        with self.assertRaisesRegex(AuthorizationError, "signature"):
            verify_live_permit(envelope, registry, _pub(root), changed,
                               evaluation_time_utc_ns=evidence.now_utc_ns)

    def test_runtime_evidence_expiry_and_clock_binding_are_enforced(self):
        root, registry, envelope, evidence, risk_service = signed_authorisation()
        with self.assertRaisesRegex(AuthorizationError, "not currently valid"):
            verify_live_permit(envelope, registry, _pub(root), evidence,
                               evaluation_time_utc_ns=evidence.expires_at_utc_ns)
        with self.assertRaisesRegex(AuthorizationError, "clocks exceed"):
            verify_live_permit(envelope, registry, _pub(root), evidence,
                               evaluation_time_utc_ns=evidence.now_utc_ns + 10_000_000)

    def test_expired_permit_fails(self):
        root, registry, envelope, evidence, risk_service = signed_authorisation(now=1_800_000_000_000_000_000)
        expired = replace(evidence, now_utc_ns=envelope["body"]["expires_at_utc_ns"])
        with self.assertRaisesRegex(AuthorizationError, "not currently valid"):
            verify_live_permit(envelope, registry, _pub(root), expired,
                               evaluation_time_utc_ns=expired.now_utc_ns)


class OMSTests(unittest.TestCase):
    def intent(self, cid="c1"):
        return BrokerOrderIntent(cid, "fixture", "in.sbin", "NSE", "buy", "limit", "CNC", 10,
                                 Decimal("500"), "day")

    def test_authorization_permit_consumption_is_one_time_and_durable(self):
        from quantlab.live_auth import VerifiedAuthorization
        auth = VerifiedAuthorization("one-shot", "a"*64, "issuer", ("risk", "ops"), 1000,
            10000, 20000, "risk-service", "b"*64)
        with tempfile.TemporaryDirectory() as tmp:
            ledger = AuthorizationUseLedger(Path(tmp) / "permit-uses.jsonl")
            ledger.consume(auth, client_order_id="client-1", order_contract_sha256="c"*64, now_utc_ns=999)
            with self.assertRaisesRegex(AuthorizationError, "already been consumed"):
                ledger.consume(auth, client_order_id="client-2", order_contract_sha256="d"*64, now_utc_ns=999)
            with self.assertRaisesRegex(AuthorizationError, "expired"):
                ledger.consume(VerifiedAuthorization("expired", "a"*64, "issuer", ("risk", "ops"), 999,
                    10000, 20000, "risk-service", "b"*64), client_order_id="client-3",
                    order_contract_sha256="e"*64, now_utc_ns=999)

    def test_event_sourced_order_state_partial_fills_and_replay(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "orders.jsonl"
            oms = OrderManagementSystem(path, mode="paper")
            order = oms.submit(self.intent(), broker="upstox", now_ns=1)
            self.assertEqual(order.status, "pending_new")
            self.assertEqual(oms.submit(self.intent(), broker="upstox", now_ns=2).status, "pending_new")
            oms.mark_submitted("c1", "broker-1", now_ns=3)
            oms.apply_broker_update("c1", {"event_id": "evt-1", "status": "partially_filled", "execution_id": "e1",
                "last_fill_quantity": 4, "last_fill_price": "499.5", "sequence": 1,
                "broker_order_id": "broker-1"}, now_ns=4)
            oms.apply_broker_update("c1", {"event_id": "evt-2", "status": "filled", "execution_id": "e2",
                "last_fill_quantity": 6, "last_fill_price": "500", "sequence": 2,
                "broker_order_id": "broker-1"}, now_ns=5)
            self.assertEqual(oms.orders["c1"].filled_quantity, 10)
            self.assertEqual(oms.orders["c1"].status, "filled")
            restored = OrderManagementSystem(path, mode="paper")
            self.assertEqual(restored.orders["c1"].filled_quantity, 10)
            self.assertEqual(restored.orders["c1"].average_fill_price, Decimal("499.8"))

    def test_reused_client_id_with_changed_contract_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            oms = OrderManagementSystem(Path(tmp) / "orders.jsonl", mode="paper")
            oms.submit(self.intent(), broker="upstox", now_ns=1)
            with self.assertRaises(IdempotencyConflict):
                oms.submit(self.intent().__class__("c1", "other", "in.sbin", "NSE", "buy", "limit", "CNC", 10, Decimal("500"), "day"),
                           broker="upstox", now_ns=2)

    def test_duplicate_execution_and_overfill_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            oms = OrderManagementSystem(Path(tmp) / "orders.jsonl", mode="paper")
            oms.submit(self.intent(), broker="upstox", now_ns=1)
            oms.mark_submitted("c1", "broker-1", now_ns=2)
            update = {"event_id": "evt-1", "status": "partially_filled", "execution_id": "e1", "last_fill_quantity": 4,
                      "last_fill_price": "500", "sequence": 1, "broker_order_id": "broker-1"}
            oms.apply_broker_update("c1", update, now_ns=3)
            # Exact event replay is idempotent; a distinct event reusing an execution ID is not.
            oms.apply_broker_update("c1", update, now_ns=4)
            duplicate_execution = {**update, "event_id": "evt-1b", "sequence": 2}
            with self.assertRaises(InvalidTransition): oms.apply_broker_update("c1", duplicate_execution, now_ns=5)
            overfill = {"event_id": "evt-2", "status": "filled", "execution_id": "e2", "last_fill_quantity": 7,
                        "last_fill_price": "500", "sequence": 2, "broker_order_id": "broker-1"}
            with self.assertRaises(InvalidTransition): oms.apply_broker_update("c1", overfill, now_ns=6)

    def test_nonconsecutive_duplicate_broker_event_is_idempotent_but_conflict_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            oms = OrderManagementSystem(Path(tmp) / "orders.jsonl", mode="paper")
            oms.submit(self.intent(), broker="upstox", now_ns=1)
            oms.mark_submitted("c1", "broker-1", now_ns=2)
            first = {"event_id": "evt-1", "status": "working", "execution_id": None,
                "last_fill_quantity": 0, "last_fill_price": None, "sequence": 1,
                "broker_order_id": "broker-1"}
            second = {**first, "event_id": "evt-2", "sequence": 2}
            oms.apply_broker_update("c1", first, now_ns=3)
            oms.apply_broker_update("c1", second, now_ns=4)
            oms.apply_broker_update("c1", first, now_ns=5)
            self.assertEqual(oms.orders["c1"].last_update_sequence, 2)
            changed = {**first, "status": "cancelled"}
            with self.assertRaises(IdempotencyConflict):
                oms.apply_broker_update("c1", changed, now_ns=6)
            restored = OrderManagementSystem(Path(tmp) / "orders.jsonl", mode="paper")
            self.assertEqual(restored.orders["c1"].last_update_sequence, 2)

    def test_cancel_rejection_releases_cancel_id_for_new_request(self):
        with tempfile.TemporaryDirectory() as tmp:
            oms = OrderManagementSystem(Path(tmp) / "orders.jsonl", mode="paper")
            oms.submit(self.intent(), broker="upstox", now_ns=1)
            oms.mark_submitted("c1", "broker-1", now_ns=2)
            oms.request_cancel("c1", cancel_request_id="cancel-1", now_ns=3)
            oms.apply_broker_update("c1", {"event_id": "evt-1", "status": "working", "execution_id": None,
                "last_fill_quantity": 0, "last_fill_price": None, "sequence": 1,
                "broker_order_id": "broker-1"}, now_ns=4)
            self.assertIsNone(oms.orders["c1"].cancel_request_id)
            self.assertEqual(oms.request_cancel("c1", cancel_request_id="cancel-2", now_ns=5).status, "pending_cancel")

    def test_order_intent_rejects_boolean_quantity_and_float_price(self):
        bad_qty = BrokerOrderIntent("c", "s", "i", "NSE", "buy", "limit", "CNC", True, Decimal("1"), "day")
        with self.assertRaises(ValueError): bad_qty.validate()
        bad_price = BrokerOrderIntent("c", "s", "i", "NSE", "buy", "limit", "CNC", 1, 1.0, "day")
        with self.assertRaises(ValueError): bad_price.validate()

    def test_journal_rejects_torn_final_line(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "orders.jsonl"
            oms = OrderManagementSystem(path, mode="paper")
            oms.submit(self.intent(), broker="upstox", now_ns=1)
            with path.open("ab") as stream: stream.write(b"{\"torn\":")
            with self.assertRaisesRegex(Exception, "torn final record"):
                OrderManagementSystem(path, mode="paper")

    def test_submit_ack_does_not_consume_broker_update_sequence(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "orders.jsonl"
            oms = OrderManagementSystem(path, mode="paper")
            oms.submit(self.intent(), broker="upstox", now_ns=1)
            oms.mark_submitted("c1", "broker-1", now_ns=2)
            self.assertEqual(oms.orders["c1"].last_update_sequence, 0)
            oms.apply_broker_update("c1", {"event_id": "first-provider-event", "status": "working",
                "execution_id": None, "last_fill_quantity": 0, "last_fill_price": None,
                "sequence": 1, "broker_order_id": "broker-1"}, now_ns=3)
            restored = OrderManagementSystem(path, mode="paper")
            self.assertEqual(restored.orders["c1"].last_update_sequence, 1)
            self.assertEqual(restored.orders["c1"].last_broker_event_id, "first-provider-event")

    def test_broker_order_id_cannot_be_reused_for_two_client_orders(self):
        with tempfile.TemporaryDirectory() as tmp:
            oms = OrderManagementSystem(Path(tmp) / "orders.jsonl", mode="paper")
            oms.submit(self.intent("c1"), broker="upstox", now_ns=1)
            oms.submit(self.intent("c2"), broker="upstox", now_ns=2)
            oms.mark_submitted("c1", "same-broker-id", now_ns=3)
            with self.assertRaises(IdempotencyConflict):
                oms.mark_submitted("c2", "same-broker-id", now_ns=4)

    def test_live_mode_never_routes_even_with_permit(self):
        from quantlab.live_auth import VerifiedAuthorization
        with tempfile.TemporaryDirectory() as tmp:
            oms = OrderManagementSystem(Path(tmp) / "orders.jsonl", mode="live")
            with self.assertRaises(LiveRoutingDisabled):
                oms.submit(self.intent(), broker="upstox", now_ns=1)
            permit = VerifiedAuthorization("p", "a"*64, "issuer", ("risk", "ops"), 99,
                999999, 999999, "risk-service", "b"*64)
            with self.assertRaisesRegex(LiveRoutingDisabled, "not implemented/certified"):
                oms.submit(self.intent(), broker="upstox", now_ns=1, authorization=permit)


if __name__ == "__main__": unittest.main()
