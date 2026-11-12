from __future__ import annotations

import argparse
import json
import os
import sys
from decimal import Decimal
from pathlib import Path

from .client import KiteClient, KiteCredentials
from .errors import KiteOpsError, RiskRejected
from .service import KiteOperations, KitePolicy


def _operations(config_path: Path, data_dir: Path) -> KiteOperations:
    policy = KitePolicy.load(config_path)
    client = KiteClient(KiteCredentials.from_environment_or_keychain())
    return KiteOperations(client, policy, data_dir)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Nirnay read-only Kite operations and guarded manual order tool")
    parser.add_argument("--config", type=Path, default=Path("data/zerodha-live.json"))
    parser.add_argument("--data-dir", type=Path, default=Path("data"))
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("login", help="authenticate locally and save the daily access token to macOS Keychain")
    commands.add_parser("doctor")
    commands.add_parser("instruments-refresh")
    commands.add_parser("orders")
    commands.add_parser("positions")
    plan = commands.add_parser("plan")
    plan.add_argument("symbol")
    plan.add_argument("quantity", type=int)
    plan.add_argument("price")
    reconcile = commands.add_parser("reconcile")

    args = parser.parse_args(argv)
    try:
        if args.command == "login":
            if not args.config.exists():
                raise KiteOpsError("private Kite policy is missing; copy configs/providers/zerodha-live.example.json to data/zerodha-live.json and set expected_user_id first")
            policy = KitePolicy.load(args.config)
            from .oauth import login
            result = login(expected_user_id=policy.expected_user_id)
            print(json.dumps({"status": "authenticated", "token_saved": result.token_saved,
                              "user_id_match": True, "access_token_printed": False}, sort_keys=True))
            return 0
        if not args.config.exists():
            raise KiteOpsError("private Kite policy is missing; copy configs/providers/zerodha-live.example.json to data/zerodha-live.json and review it")
        ops = _operations(args.config, args.data_dir)
        if args.command == "doctor":
            result = ops.doctor()
        elif args.command == "instruments-refresh":
            result = {"instrument_master": str(ops.refresh_instruments())}
        elif args.command == "orders":
            result = ops.client.orders()
        elif args.command == "positions":
            result = ops.client.positions()
        elif args.command == "plan":
            order_plan = ops.plan(args.symbol, args.quantity, args.price)
            result = {
                "symbol": order_plan.symbol,
                "quantity": order_plan.quantity,
                "limit_price_inr": format(order_plan.price, "f"),
                "notional_inr": format(order_plan.notional, "f"),
                "tick_size": format(order_plan.tick_size, "f"),
                "lot_size": order_plan.lot_size,
                "user_id": order_plan.user_id,
                "live_order_enabled": ops.policy.live_order_enabled,
                "submission": "not sent; live routing is hard-disabled",
            }
        elif args.command == "reconcile":
            result = ops.reconcile()
        else:
            raise KiteOpsError("unknown command")
        print(json.dumps(result, indent=2, sort_keys=True, default=str))
        return 0
    except (KiteOpsError, RiskRejected, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"kite operation failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
