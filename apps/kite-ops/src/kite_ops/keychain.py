"""macOS Keychain access without placing secrets in process arguments.

The Swift helper uses Security.framework. Secrets are sent over stdin when
writing and captured from stdout only inside the caller process when reading.
This module refuses to fall back to shell commands that would
place secret values in argv or shell history.
"""
from __future__ import annotations

import os
import subprocess
from pathlib import Path

from .errors import KiteOpsError


KEYCHAIN_ACCOUNT = "ZERODHA"


def _helper_path() -> Path:
    return Path(__file__).resolve().parent / "native" / "keychain.swift"


def _invoke(operation: str, service: str, account: str, *, secret: bytes | None = None) -> bytes:
    if os.name != "posix" or os.uname().sysname != "Darwin":
        raise KiteOpsError("macOS Keychain integration is available only on macOS")
    if not service or not account:
        raise KiteOpsError("Keychain service and account are required")
    helper = _helper_path()
    if not helper.is_file():
        raise KiteOpsError("bundled Security.framework Keychain helper is missing")
    try:
        result = subprocess.run(
            ["swift", str(helper), operation, service, account],
            input=secret, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            check=False, timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise KiteOpsError(f"Keychain helper could not run: {type(exc).__name__}") from exc
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", "replace").strip()
        raise KiteOpsError(detail or f"Keychain helper failed with status {result.returncode}")
    return result.stdout


def get_secret(service: str, account: str = KEYCHAIN_ACCOUNT) -> str:
    value = _invoke("get", service, account)
    try:
        decoded = value.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise KiteOpsError(f"Keychain item {service} is not valid UTF-8") from exc
    if not decoded:
        raise KiteOpsError(f"Keychain item {service} is empty")
    return decoded


def set_secret(service: str, secret: str, account: str = KEYCHAIN_ACCOUNT) -> None:
    if not isinstance(secret, str) or not secret.strip():
        raise KiteOpsError("refusing to store an empty Keychain secret")
    _invoke("set", service, account, secret=secret.encode("utf-8"))


def delete_secret(service: str, account: str = KEYCHAIN_ACCOUNT) -> None:
    _invoke("delete", service, account)
