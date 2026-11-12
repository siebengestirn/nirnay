"""Zerodha Kite Connect local desktop OAuth flow.

The one-time request_token is exchanged immediately and never printed or
persisted to disk. The registered redirect URL must be http://127.0.0.1:8000 (exact dashboard match).
A random state is carried through Kite's documented redirect_params mechanism.
"""
from __future__ import annotations

import hashlib
import hmac
import secrets
import time
import urllib.parse
import webbrowser
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Callable

import requests

from .errors import KiteOpsError

LOGIN_BASE = "https://kite.zerodha.com/connect/login"
TOKEN_URL = "https://api.kite.trade/session/token"
REDIRECT_URI = "http://127.0.0.1:8000"


@dataclass(frozen=True)
class OAuthResult:
    user_id: str
    user_name: str
    token_saved: bool


def make_login_url(api_key: str, state: str) -> str:
    if not isinstance(api_key, str) or not api_key.strip() or not isinstance(state, str) or len(state) < 32:
        raise ValueError("api_key and a strong random state are required")
    redirect_params = urllib.parse.urlencode({"state": state})
    return LOGIN_BASE + "?" + urllib.parse.urlencode({
        "v": "3", "api_key": api_key, "redirect_params": redirect_params,
    })


def request_token_checksum(api_key: str, request_token: str, api_secret: str) -> str:
    if not all(isinstance(x, str) and x for x in (api_key, request_token, api_secret)):
        raise ValueError("Kite checksum inputs must be non-empty")
    return hashlib.sha256((api_key + request_token + api_secret).encode("utf-8")).hexdigest()


def _parse_callback(path: str, expected_state: str) -> str:
    parsed = urllib.parse.urlparse(path)
    if parsed.path not in {"", "/"}:
        raise KiteOpsError("unexpected OAuth callback path")
    values = urllib.parse.parse_qs(parsed.query, strict_parsing=True)
    if values.get("status", [None])[0] != "success":
        raise KiteOpsError("Kite login was not successful; no token was exchanged")
    states = values.get("state", [])
    if len(states) != 1 or not hmac.compare_digest(states[0], expected_state):
        raise KiteOpsError("Kite OAuth state mismatch; refusing token exchange")
    tokens = values.get("request_token", [])
    if len(tokens) != 1 or not tokens[0] or len(tokens[0]) > 512:
        raise KiteOpsError("Kite callback did not contain one valid request_token")
    return tokens[0]


class _CallbackServer(HTTPServer):
    allow_reuse_address = False

    def __init__(self, expected_state: str):
        self.expected_state = expected_state
        self.request_token: str | None = None
        self.callback_error: Exception | None = None
        super().__init__(("127.0.0.1", 8000), _Handler)


class _Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - stdlib callback API
        server: _CallbackServer = self.server  # type: ignore[assignment]
        try:
            server.request_token = _parse_callback(self.path, server.expected_state)
            body = b"Nirnay authentication received. You may close this tab and return to Terminal."
            self.send_response(200)
        except Exception as exc:
            server.callback_error = exc
            body = b"Nirnay could not validate this login callback. Return to Terminal."
            self.send_response(400)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        # The request line contains request_token; never allow BaseHTTPRequestHandler
        # to print it to stdout/stderr or a terminal log.
        return


def wait_for_request_token(expected_state: str, *, timeout_seconds: int = 180,
                           opener: Callable[[str], object] = webbrowser.open,
                           login_url: str = "") -> str:
    server = _CallbackServer(expected_state)
    try:
        if login_url:
            opener(login_url)
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            server.timeout = min(1.0, max(0.05, deadline - time.monotonic()))
            server.handle_request()
            if server.callback_error:
                raise KiteOpsError(str(server.callback_error)) from server.callback_error
            if server.request_token:
                return server.request_token
        raise KiteOpsError("Kite login callback timed out; restart login and do not reuse an old request token")
    finally:
        server.server_close()


def exchange_request_token(api_key: str, api_secret: str, request_token: str,
                           *, session: requests.Session | None = None) -> dict:
    checksum = request_token_checksum(api_key, request_token, api_secret)
    client = session or requests.Session()
    try:
        response = client.post(
            TOKEN_URL, headers={"X-Kite-Version": "3", "Accept": "application/json"},
            data={"api_key": api_key, "request_token": request_token, "checksum": checksum},
            timeout=(3.05, 10.0), allow_redirects=False,
        )
    except requests.RequestException as exc:
        raise KiteOpsError(f"Kite token exchange transport failure: {type(exc).__name__}; restart login") from exc
    if response.status_code != 200:
        raise KiteOpsError(f"Kite token exchange rejected with HTTP {response.status_code}")
    try:
        body = response.json()
    except Exception as exc:
        raise KiteOpsError("Kite token exchange response was malformed") from exc
    if not isinstance(body, dict) or body.get("status") != "success" or not isinstance(body.get("data"), dict):
        raise KiteOpsError("Kite token exchange returned an unsuccessful response")
    data = body["data"]
    if data.get("api_key") != api_key or not isinstance(data.get("access_token"), str) or not data["access_token"]:
        raise KiteOpsError("Kite token exchange response did not match the requested app")
    if not isinstance(data.get("user_id"), str) or not data["user_id"]:
        raise KiteOpsError("Kite token exchange omitted authenticated user identity")
    return data


def login(*, expected_user_id: str, timeout_seconds: int = 180,
          keychain_get: Callable[[str, str], str] | None = None,
          keychain_set: Callable[[str, str, str], None] | None = None,
          session: requests.Session | None = None,
          opener: Callable[[str], object] = webbrowser.open) -> OAuthResult:
    if not expected_user_id:
        raise KiteOpsError("set expected_user_id in data/zerodha-live.json before login")
    if keychain_get is None or keychain_set is None:
        from .keychain import get_secret, set_secret
        keychain_get = keychain_get or get_secret
        keychain_set = keychain_set or set_secret
    api_key = keychain_get("NIRNAY_KITE_API_KEY", "ZERODHA")
    api_secret = keychain_get("NIRNAY_KITE_API_SECRET", "ZERODHA")
    state = secrets.token_urlsafe(32)
    token = wait_for_request_token(state, timeout_seconds=timeout_seconds,
                                   opener=opener, login_url=make_login_url(api_key, state))
    data = exchange_request_token(api_key, api_secret, token, session=session)
    if not hmac.compare_digest(data["user_id"], expected_user_id):
        raise KiteOpsError("authenticated Kite user does not match expected_user_id; token was not saved")
    access_token = data["access_token"]
    keychain_set("NIRNAY_KITE_ACCESS_TOKEN", access_token, "ZERODHA")
    return OAuthResult(user_id=data["user_id"], user_name=str(data.get("user_name", "")), token_saved=True)
