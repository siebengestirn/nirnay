from __future__ import annotations

import base64
import hashlib
import urllib.parse
from unittest.mock import patch

import pytest
import requests

from kite_ops.errors import KiteOpsError
from kite_ops import oauth
from kite_ops.keychain import _invoke


class FakeResponse:
    status_code = 200

    def __init__(self, body):
        self._body = body

    def json(self):
        return self._body


class FakeSession:
    def __init__(self, body=None, *, error=None):
        self.body = body or {"status": "success", "data": {
            "api_key": "api-key", "access_token": "daily-token", "user_id": "AB1234",
            "user_name": "Test User",
        }}
        self.error = error
        self.calls = []

    def post(self, url, **kwargs):
        self.calls.append((url, kwargs))
        if self.error:
            raise self.error
        return FakeResponse(self.body)


def test_login_url_carries_random_state_through_redirect_params():
    url = oauth.make_login_url("api-key", "a" * 43)
    parsed = urllib.parse.urlparse(url)
    query = urllib.parse.parse_qs(parsed.query)
    assert parsed.scheme == "https"
    assert parsed.netloc == "kite.zerodha.com"
    assert query["api_key"] == ["api-key"]
    redirect_params = urllib.parse.parse_qs(query["redirect_params"][0])
    assert redirect_params["state"] == ["a" * 43]


def test_callback_requires_success_valid_state_and_single_request_token():
    token = oauth._parse_callback("/?status=success&state=correct&request_token=req1", "correct")
    assert token == "req1"
    with pytest.raises(KiteOpsError, match="state mismatch"):
        oauth._parse_callback("/?status=success&state=wrong&request_token=req1", "correct")
    with pytest.raises(KiteOpsError, match="not successful"):
        oauth._parse_callback("/?status=error&state=correct", "correct")
    with pytest.raises(KiteOpsError, match="one valid"):
        oauth._parse_callback("/?status=success&state=correct&request_token=x&request_token=y", "correct")


def test_request_token_checksum_matches_official_sha256_concatenation():
    expected = hashlib.sha256(b"api-keyrequest-tokenapi-secret").hexdigest()
    assert oauth.request_token_checksum("api-key", "request-token", "api-secret") == expected


def test_exchange_sends_checksum_without_redirects_and_validates_app_binding():
    session = FakeSession()
    data = oauth.exchange_request_token("api-key", "api-secret", "request-token", session=session)
    assert data["access_token"] == "daily-token"
    url, kwargs = session.calls[0]
    assert url == oauth.TOKEN_URL
    assert kwargs["allow_redirects"] is False
    assert kwargs["data"]["checksum"] == hashlib.sha256(b"api-keyrequest-tokenapi-secret").hexdigest()
    bad = FakeSession({"status": "success", "data": {
        "api_key": "other-key", "access_token": "daily-token", "user_id": "AB1234"}})
    with pytest.raises(KiteOpsError, match="did not match"):
        oauth.exchange_request_token("api-key", "api-secret", "request-token", session=bad)


def test_login_stores_token_only_after_user_identity_matches():
    saved = []
    session = FakeSession()
    with patch("kite_ops.oauth.wait_for_request_token", return_value="one-time-request-token") as receive:
        result = oauth.login(
            expected_user_id="AB1234", keychain_get=lambda service, account: {
                "NIRNAY_KITE_API_KEY": "api-key", "NIRNAY_KITE_API_SECRET": "api-secret"}[service],
            keychain_set=lambda service, secret, account: saved.append((service, secret, account)),
            session=session, opener=lambda _url: True,
        )
    assert receive.call_count == 1
    assert result.user_id == "AB1234" and result.token_saved
    assert saved == [("NIRNAY_KITE_ACCESS_TOKEN", "daily-token", "ZERODHA")]


def test_login_does_not_save_token_for_wrong_zerodha_account():
    saved = []
    with patch("kite_ops.oauth.wait_for_request_token", return_value="one-time-request-token"):
        with pytest.raises(KiteOpsError, match="does not match"):
            oauth.login(
                expected_user_id="OTHER",
                keychain_get=lambda service, account: "api-key" if service.endswith("API_KEY") else "api-secret",
                keychain_set=lambda *args: saved.append(args), session=FakeSession(), opener=lambda _url: True,
            )
    assert saved == []


def test_keychain_helper_fails_closed_on_non_macos(monkeypatch):
    monkeypatch.setattr("kite_ops.keychain.os.name", "posix")
    monkeypatch.setattr("kite_ops.keychain.os.uname", lambda: type("U", (), {"sysname": "Linux"})())
    with pytest.raises(KiteOpsError, match="only on macOS"):
        _invoke("get", "NIRNAY_KITE_API_KEY", "ZERODHA")
