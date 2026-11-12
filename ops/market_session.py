#!/usr/bin/env python3
"""Run concurrent Upstox and FYERS read-only market-data capture sessions."""
from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def write_json(path: Path, value: dict) -> None:
    temp = path.with_name(path.name + f".{uuid.uuid4().hex}.tmp")
    with temp.open("x", encoding="utf-8") as stream:
        if os.name == "posix":
            os.chmod(temp, 0o600)
        json.dump(value, stream, ensure_ascii=False, sort_keys=True, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--providers", default="upstox,fyers", help="comma-separated subset of upstox,fyers")
    parser.add_argument("--upstox-mode", choices=("ltpc", "full"), default="full")
    parser.add_argument("--upstox-watchlist", type=Path, default=ROOT / "configs/providers/upstox-watchlist.txt")
    parser.add_argument("--fyers-watchlist", type=Path, default=ROOT / "configs/providers/fyers-watchlist.txt")
    parser.add_argument("--output", type=Path, default=ROOT / "data/market-sessions")
    parser.add_argument("--max-reconnects", type=int, default=10)
    args = parser.parse_args()

    providers = tuple(item.strip().lower() for item in args.providers.split(",") if item.strip())
    if not providers or len(set(providers)) != len(providers) or set(providers) - {"upstox", "fyers"}:
        parser.error("--providers must contain upstox and/or fyers, without duplicates")
    if args.max_reconnects < 0 or args.max_reconnects > 50:
        parser.error("--max-reconnects must be between 0 and 50")

    required = {
        "upstox": ("UPSTOX_ACCESS_TOKEN", "NIRNAY_UPSTOX_ENTITLEMENT_REF"),
        "fyers": ("FYERS_ACCESS_TOKEN", "NIRNAY_FYERS_ENTITLEMENT_REF"),
    }
    missing = [name for provider in providers for name in required[provider] if not os.environ.get(name)]
    if missing:
        print("Missing process environment values: " + ", ".join(missing), file=sys.stderr)
        return 2

    session_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S") + "-" + uuid.uuid4().hex[:8]
    session_dir = args.output / f"combined-{session_id}"
    session_dir.mkdir(mode=0o700, parents=True, exist_ok=False)
    if os.name == "posix":
        os.chmod(session_dir, 0o700)
    manifest = {
        "schema_version": 1,
        "session_id": session_id,
        "providers": list(providers),
        "started_at_utc": utc_now(),
        "ended_at_utc": None,
        "provider_exit_codes": {},
        "status": "running",
        "secrets_written": False,
        "notes": [
            "Each provider writes a separate raw archive and manifest.",
            "Upstox sequence is local receive order; FYERS Python SDK callbacks do not expose a verified provider sequence.",
            "This process starts market-data capture only; Kite order access is a separate operator workflow.",
        ],
    }
    write_json(session_dir / "session.json", manifest)

    env_base = os.environ.copy()
    commands: dict[str, tuple[list[str], dict[str, str]]] = {}
    if "upstox" in providers:
        env = env_base.copy()
        for secret_name in (
            "FYERS_ACCESS_TOKEN", "NIRNAY_FYERS_ENTITLEMENT_REF",
            "KITE_API_KEY", "KITE_ACCESS_TOKEN", "UPSTOX_ANALYTICS_TOKEN",
        ):
            env.pop(secret_name, None)
        commands["upstox"] = (
            [
                "cargo", "run", "--release", "-p", "upstox-tools", "--bin", "upstox-recorder", "--",
                "--instrument-file", str(args.upstox_watchlist), "--mode", args.upstox_mode,
                "--output-root", str(session_dir / "upstox"), "--max-reconnects", str(args.max_reconnects),
            ],
            env,
        )
    if "fyers" in providers:
        env = env_base.copy()
        for secret_name in (
            "UPSTOX_ACCESS_TOKEN", "NIRNAY_UPSTOX_ENTITLEMENT_REF",
            "UPSTOX_ANALYTICS_TOKEN", "KITE_API_KEY", "KITE_ACCESS_TOKEN",
        ):
            env.pop(secret_name, None)
        env["PYTHONPATH"] = str(ROOT / "apps/fyers-recorder/src") + os.pathsep + env.get("PYTHONPATH", "")
        commands["fyers"] = (
            [
                sys.executable, "-m", "fyers_recorder.cli", "--watchlist", str(args.fyers_watchlist),
                "--output", str(session_dir / "fyers"),
            ],
            env,
        )

    processes: dict[str, subprocess.Popen] = {}
    stop_requested = False

    def request_stop(signum, _frame):
        nonlocal stop_requested
        stop_requested = True
        print(f"Received signal {signum}; stopping provider processes", file=sys.stderr)

    signal.signal(signal.SIGINT, request_stop)
    signal.signal(signal.SIGTERM, request_stop)

    try:
        for provider, (command, env) in commands.items():
            log_path = session_dir / f"{provider}.stdout.log"
            error_path = session_dir / f"{provider}.stderr.log"
            stdout = log_path.open("x", encoding="utf-8")
            stderr = error_path.open("x", encoding="utf-8")
            if os.name == "posix":
                os.chmod(log_path, 0o600)
                os.chmod(error_path, 0o600)
            try:
                process = subprocess.Popen(
                    command,
                    cwd=ROOT,
                    env=env,
                    stdin=subprocess.DEVNULL,
                    stdout=stdout,
                    stderr=stderr,
                    start_new_session=True,
                    text=True,
                )
            finally:
                stdout.close()
                stderr.close()
            processes[provider] = process
            manifest["provider_processes"] = {name: proc.pid for name, proc in processes.items()}
            write_json(session_dir / "session.json", manifest)
            print(f"Started {provider} capture (pid={process.pid})", file=sys.stderr)

        while processes and not stop_requested:
            failed = [(name, proc.poll()) for name, proc in processes.items() if proc.poll() not in (None, 0)]
            finished = [(name, proc.poll()) for name, proc in processes.items() if proc.poll() == 0]
            if failed:
                for name, code in failed:
                    manifest["provider_exit_codes"][name] = code
                stop_requested = True
                manifest["status"] = "provider_failed"
                break
            if finished:
                for name, code in finished:
                    manifest["provider_exit_codes"][name] = code
                stop_requested = True
                manifest["status"] = "provider_ended"
                break
            time.sleep(0.5)
    except OSError as error:
        print(f"Could not start provider process: {error}", file=sys.stderr)
        manifest["status"] = "start_failed"
        stop_requested = True
    finally:
        for provider, process in processes.items():
            if process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGINT)
                except ProcessLookupError:
                    pass
        deadline = time.monotonic() + 20
        for provider, process in processes.items():
            if process.poll() is None:
                try:
                    process.wait(timeout=max(0.1, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    try:
                        os.killpg(process.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            manifest["provider_exit_codes"][provider] = process.returncode
        if manifest["status"] == "running":
            manifest["status"] = "stopped_by_operator" if stop_requested else "completed"
        manifest["ended_at_utc"] = utc_now()
        write_json(session_dir / "session.json", manifest)

    print(session_dir)
    return 0 if all(code == 0 for code in manifest["provider_exit_codes"].values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
