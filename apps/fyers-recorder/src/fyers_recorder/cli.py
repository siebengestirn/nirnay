from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

from .recorder import FyersRecorder, RecorderConfig


def read_symbols(path: Path) -> tuple[str, ...]:
    symbols = tuple(
        line.strip()
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    )
    return symbols


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Capture FYERS Data Socket SDK callbacks")
    parser.add_argument(
        "--registry", type=Path, required=True,
        help="explicit path to the source-product rights registry (no default)",
    )
    parser.add_argument("--watchlist", type=Path, default=Path("configs/providers/fyers-watchlist.txt"))
    parser.add_argument("--output", type=Path, default=Path("data/market-sessions/fyers"))
    parser.add_argument("--queue-capacity", type=int, default=16_384)
    parser.add_argument(
        "--max-symbols", type=int, default=100,
        help="operator-declared FYERS Data Socket entitlement bound (100 Standard, up to 5000 Prime); not remotely verified",
    )
    parser.add_argument("--no-depth", action="store_true")
    args = parser.parse_args(argv)

    token = os.environ.get("FYERS_ACCESS_TOKEN", "")
    entitlement = os.environ.get("NIRNAY_FYERS_ENTITLEMENT_REF", "")
    try:
        config = RecorderConfig(
            symbols=read_symbols(args.watchlist),
            entitlement_ref=entitlement,
            output_root=args.output,
            registry_path=args.registry,
            queue_capacity=args.queue_capacity,
            include_depth=not args.no_depth,
            max_symbols=args.max_symbols,
        )
        recorder = FyersRecorder(config, token)
        run_dir = recorder.run()
        print(run_dir)
        return 0
    except KeyboardInterrupt:
        print("FYERS capture interrupted", file=sys.stderr)
        return 130
    except Exception as error:
        print(f"FYERS capture failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
