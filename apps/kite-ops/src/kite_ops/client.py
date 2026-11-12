from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Protocol

import requests

from .errors import KiteOpsError, KiteRejected, KiteUnknownOutcome


class Response(Protocol):
    status_code: int
    text: str

    def json(self) -> Any: ...


class Session(Protocol):
    def request(self, method: str, url: str, **kwargs: Any) -> Response: ...


@dataclass(frozen=True)
class KiteCredentials:
    api_key: str
    access_token: str

    @classmethod
    def from_environment(cls) -> "KiteCredentials":
        """Explicit CI/test credential source; never mix partial env and Keychain."""
        import os
        api_key = os.environ.get("KITE_API_KEY", "")
        access_token = os.environ.get("KITE_ACCESS_TOKEN", "")
        if bool(api_key) != bool(access_token):
            raise KiteOpsError("provide both KITE_API_KEY and KITE_ACCESS_TOKEN, or neither")
        if not api_key:
            raise KiteOpsError("Kite environment credentials were not provided")
        return cls(api_key=api_key, access_token=access_token)

    @classmethod
    def from_environment_or_keychain(cls) -> "KiteCredentials":
        import os
        api_key = os.environ.get("KITE_API_KEY", "")
        access_token = os.environ.get("KITE_ACCESS_TOKEN", "")
        if bool(api_key) != bool(access_token):
            raise KiteOpsError("provide both KITE_API_KEY and KITE_ACCESS_TOKEN, or neither; refusing mixed credential sources")
        if api_key:
            return cls(api_key=api_key, access_token=access_token)
        from .keychain import get_secret
        return cls(
            api_key=get_secret("NIRNAY_KITE_API_KEY", "ZERODHA"),
            access_token=get_secret("NIRNAY_KITE_ACCESS_TOKEN", "ZERODHA"),
        )


class KiteClient:
    """Minimal Kite Connect v3 transport; no retries or redirects on writes."""

    base_url = "https://api.kite.trade"

    def __init__(self, credentials: KiteCredentials, session: Session | None = None) -> None:
        self._credentials = credentials
        self._session = session or requests.Session()
        self._headers = {
            "X-Kite-Version": "3",
            "Authorization": f"token {credentials.api_key}:{credentials.access_token}",
            "Accept": "application/json",
        }

    def _request(self, method: str, path: str, *, params=None, data=None) -> Any:
        method = method.upper()
        if method != "GET":
            raise KiteOpsError("broker trading writes are hard-disabled in this build; no request was sent")
        url = f"{self.base_url}{path}"
        try:
            response = self._session.request(
                method,
                url,
                headers=self._headers,
                params=params,
                data=data,
                timeout=(3.05, 10.0),
                allow_redirects=False,
            )
        except requests.RequestException as error:
            if method.upper() == "POST":
                raise KiteUnknownOutcome("Kite submission transport failed; do not retry; reconcile the order book") from error
            raise KiteOpsError(f"Kite {method} transport failed: {type(error).__name__}") from error

        if response.status_code < 200 or response.status_code >= 300:
            if method.upper() == "POST" and response.status_code >= 500:
                raise KiteUnknownOutcome(
                    f"Kite returned HTTP {response.status_code} after submission; do not retry; reconcile"
                )
            detail = ""
            try:
                body = response.json()
                detail = str(body.get("message", body.get("error_type", ""))) if isinstance(body, dict) else ""
            except Exception:
                detail = ""
            raise KiteRejected(f"Kite HTTP {response.status_code}: {detail or 'request rejected'}")

        try:
            body = response.json()
        except Exception as error:
            if method.upper() == "POST":
                raise KiteUnknownOutcome("Kite submission response was not valid JSON; do not retry; reconcile") from error
            raise KiteOpsError("Kite returned invalid JSON") from error

        if not isinstance(body, dict) or body.get("status") != "success" or "data" not in body:
            detail = body.get("message", "invalid Kite response envelope") if isinstance(body, dict) else "invalid Kite response envelope"
            if method.upper() == "POST":
                raise KiteUnknownOutcome(f"Kite submission response is ambiguous: {detail}; reconcile")
            raise KiteOpsError(f"Kite API response error: {detail}")
        return body["data"]

    def profile(self) -> dict[str, Any]:
        result = self._request("GET", "/user/profile")
        if not isinstance(result, dict):
            raise KiteOpsError("Kite profile response is malformed")
        return result

    def margins(self) -> dict[str, Any]:
        result = self._request("GET", "/user/margins/equity")
        if not isinstance(result, dict):
            raise KiteOpsError("Kite margins response is malformed")
        return result

    def orders(self) -> list[dict[str, Any]]:
        result = self._request("GET", "/orders")
        if not isinstance(result, list) or any(not isinstance(item, dict) for item in result):
            raise KiteOpsError("Kite orders response is malformed")
        return result

    def positions(self) -> dict[str, list[dict[str, Any]]]:
        result = self._request("GET", "/portfolio/positions")
        if not isinstance(result, dict) or not isinstance(result.get("net"), list):
            raise KiteOpsError("Kite positions response is malformed")
        return result

    def order_history(self, order_id: str) -> list[dict[str, Any]]:
        if not order_id or any(char not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_" for char in order_id):
            raise KiteOpsError("order_id contains unsupported characters")
        result = self._request("GET", f"/orders/{order_id}")
        if not isinstance(result, list) or any(not isinstance(item, dict) for item in result):
            raise KiteOpsError("Kite order history response is malformed")
        return result

    def order_trades(self, order_id: str) -> list[dict[str, Any]]:
        if not order_id or any(char not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_" for char in order_id):
            raise KiteOpsError("order_id contains unsupported characters")
        result = self._request("GET", f"/orders/{order_id}/trades")
        if not isinstance(result, list) or any(not isinstance(item, dict) for item in result):
            raise KiteOpsError("Kite order trades response is malformed")
        return result

    def instruments_nse_csv(self) -> str:
        url = f"{self.base_url}/instruments/NSE"
        try:
            response = self._session.request(
                "GET", url, headers={**self._headers, "Accept": "text/csv"},
                timeout=(3.05, 20.0), allow_redirects=False,
            )
        except requests.RequestException as error:
            raise KiteOpsError(f"Kite instrument download failed: {type(error).__name__}") from error
        if response.status_code != 200:
            raise KiteOpsError(f"Kite instrument download returned HTTP {response.status_code}")
        if not response.text.startswith("instrument_token,exchange_token,tradingsymbol"):
            raise KiteOpsError("Kite instrument master does not match the expected CSV header")
        return response.text

    def place_cnc_buy_limit(self, order: dict[str, str]) -> str:
        del order
        raise KiteOpsError("live order placement is hard-disabled; no HTTP request was sent")
