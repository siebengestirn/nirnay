"""CLI for deterministic research, point-in-time data and verification tasks."""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import sys
import tempfile
from datetime import date
from pathlib import Path
from typing import Any

from .alpha_search import AlphaCosts, Bar, search_alpha
from .demo import run_demo
from .market import market_event_from_dict, canonical_bytes
from .providers import FyersV3Normalizer, UpstoxV3Normalizer
from .reconcile import reconcile_latest
from .point_in_time import (CorporateAction, PITDatasetBuilder, PITListing,
                            PITReferenceData, PITUniverseMembership,
                            verify_published_dataset, write_dataset)


def _iter_jsonl(path: Path):
    """Yield immutable JSONL rows one at a time, without retaining the file."""
    with path.open(encoding="utf-8") as stream:
        for line_number, raw in enumerate(stream, 1):
            if not raw.strip():
                raise ValueError(f"{path}:{line_number}: blank lines are not valid in an immutable JSONL source")
            try:
                value = json.loads(raw)
            except json.JSONDecodeError as exc:
                raise ValueError(f"{path}:{line_number}: invalid JSON: {exc.msg}") from exc
            if not isinstance(value, dict):
                raise ValueError(f"{path}:{line_number}: each JSONL row must be an object")
            yield value


def _read_jsonl(path: Path) -> list[dict[str, Any]]:
    return list(_iter_jsonl(path))


def _load_reference_data(path: Path) -> PITReferenceData:
    raw = json.loads(path.read_text(encoding="utf-8"))
    allowed = {"listings", "corporate_actions", "universe_memberships"}
    if not isinstance(raw, dict) or not {"listings", "corporate_actions"}.issubset(raw) or set(raw) - allowed:
        raise ValueError("reference data must contain listings and corporate_actions arrays; universe_memberships is optional")
    for key in allowed:
        if key in raw and not isinstance(raw[key], list):
            raise ValueError(f"reference data {key} must be an array")
    listings = []
    for item in raw["listings"]:
        if not isinstance(item, dict):
            raise ValueError("each listing must be an object")
        values = dict(item)
        values["valid_from"] = date.fromisoformat(values["valid_from"])
        values["valid_to"] = None if values.get("valid_to") is None else date.fromisoformat(values["valid_to"])
        listings.append(PITListing(**values))
    actions = []
    for item in raw["corporate_actions"]:
        if not isinstance(item, dict):
            raise ValueError("each corporate action must be an object")
        values = dict(item)
        values["effective_date"] = date.fromisoformat(values["effective_date"])
        values["record_date"] = None if values.get("record_date") is None else date.fromisoformat(values["record_date"])
        values["payable_date"] = None if values.get("payable_date") is None else date.fromisoformat(values["payable_date"])
        actions.append(CorporateAction(**values))
    memberships = []
    for item in raw.get("universe_memberships", []):
        if not isinstance(item, dict):
            raise ValueError("each universe membership must be an object")
        values = dict(item)
        values["valid_from"] = date.fromisoformat(values["valid_from"])
        values["valid_to"] = None if values.get("valid_to") is None else date.fromisoformat(values["valid_to"])
        memberships.append(PITUniverseMembership(**values))
    return PITReferenceData(listings, actions, memberships)


def _write_many_new(items: list[tuple[Path, bytes]]) -> None:
    """Publish one or more immutable files without exposing partial file contents.

    Each file is staged and fsynced in its destination directory, then linked
    into place with no-overwrite semantics. If any publication fails, files
    created by this invocation are removed. Multiple destinations are not a
    filesystem-wide atomic transaction; consumers should treat the report
    sidecar as the commit marker for a multi-file output set.
    """
    if not items:
        raise ValueError("at least one immutable output is required")
    destinations = [Path(path) for path, _ in items]
    if len(set(destinations)) != len(destinations):
        raise ValueError("immutable output destinations must be unique")
    staged: list[tuple[Path, Path]] = []
    published: list[Path] = []
    try:
        for destination, content in items:
            destination.parent.mkdir(parents=True, exist_ok=True)
            fd, temporary_name = tempfile.mkstemp(
                prefix=f".{destination.name}.staging-", dir=destination.parent
            )
            temporary = Path(temporary_name)
            staged.append((temporary, destination))
            with os.fdopen(fd, "wb") as stream:
                stream.write(content)
                stream.flush()
                os.fsync(stream.fileno())
            os.chmod(temporary, 0o600)
        # No overwrite: link() fails if a target already exists, including a symlink.
        for temporary, destination in staged:
            os.link(temporary, destination)
            published.append(destination)
        for temporary, _ in staged:
            temporary.unlink(missing_ok=True)
        for parent in sorted({p.parent for p in destinations}, key=str):
            parent_fd = os.open(parent, os.O_RDONLY)
            try:
                os.fsync(parent_fd)
            finally:
                os.close(parent_fd)
    except Exception:
        # Roll back only names this invocation has successfully created.
        for destination in reversed(published):
            destination.unlink(missing_ok=True)
        raise
    finally:
        for temporary, _ in staged:
            temporary.unlink(missing_ok=True)


def _write_new(path: Path, content: bytes) -> None:
    _write_many_new([(path, content)])


def _normalize_upstox_bounded(args: argparse.Namespace, normalizer: UpstoxV3Normalizer) -> int:
    """Stream decoded Upstox rows to canonical events with bounded memory.

    Ordering is defined by `capture_sequence`: the decoder emits journal records
    in append order, so events are written in that order without a global sort.
    A non-increasing sequence is an error; gaps are reported and prevent a
    research-grade classification.
    """
    output = Path(args.output)
    report_path = output.with_suffix(output.suffix + ".quality.json")
    if output.exists() or report_path.exists():
        raise ValueError("immutable output or quality report already exists")
    output.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary_name = tempfile.mkstemp(
        prefix=f".{output.name}.normalize-", dir=output.parent
    )
    temporary = Path(temporary_name)
    hasher = hashlib.sha256()
    warnings: list[str] = []
    input_rows = 0
    event_count = 0
    previous_sequence = 0
    try:
        with os.fdopen(fd, "wb") as stream:
            for row in _iter_jsonl(Path(args.input)):
                if row.get("event_type") == "gap":
                    raise ValueError("Upstox decoded input must not contain gap records")
                if row.get("schema_version") != 1 or row.get("provider") != "upstox":
                    raise ValueError("decoded row must be schema_version 1 from provider upstox")
                sequence = row.get("capture_sequence")
                if type(sequence) is not int or sequence <= 0:
                    raise ValueError("decoded row lacks a positive capture_sequence")
                if previous_sequence and sequence <= previous_sequence:
                    raise ValueError(
                        f"capture_sequence {sequence} is not strictly increasing "
                        f"after {previous_sequence}"
                    )
                if previous_sequence == 0 and sequence != 1:
                    warnings.append(f"capture_stream_does_not_start_at_one:{sequence}")
                elif previous_sequence and sequence != previous_sequence + 1:
                    warnings.append(f"capture_sequence_gap:{previous_sequence}->{sequence}")
                previous_sequence = sequence
                frame_hash = row.get("payload_sha256")
                if (not isinstance(frame_hash, str) or len(frame_hash) != 64
                        or any(char not in "0123456789abcdef" for char in frame_hash)):
                    raise ValueError("decoded row lacks a valid payload_sha256")
                receive = row.get("received_at_utc_ns", row.get("received_utc_ns"))
                if type(receive) is not int or receive <= 0:
                    raise ValueError("decoded row lacks a valid received timestamp")
                payload = row.get("message")
                if not isinstance(payload, dict):
                    raise ValueError("decoded row lacks a message object")
                events = normalizer.normalize(
                    payload, received_utc_ns=receive,
                    received_monotonic_ns=row.get("received_monotonic_ns"),
                    local_sequence=sequence, source_frame_sha256=frame_hash,
                )
                for event in events:
                    encoded = canonical_bytes(event.as_dict()) + b"\n"
                    stream.write(encoded)
                    hasher.update(encoded)
                    event_count += 1
                input_rows += 1
            if event_count == 0:
                raise ValueError("no market observations were normalized")
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, 0o600)
        os.link(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)
    parent_fd = os.open(output.parent, os.O_RDONLY)
    try:
        os.fsync(parent_fd)
    finally:
        os.close(parent_fd)
    report = {
        "schema_version": 1,
        "provider": "upstox",
        "input_rows": input_rows,
        "canonical_events": event_count,
        "ordering_contract": (
            "capture_sequence strictly increasing; decoder append order; "
            "no global sort; bounded memory"
        ),
        "explicit_capture_gaps": len(warnings),
        "warnings": warnings,
        "output_sha256": hasher.hexdigest(),
        "research_grade": not warnings,
        "quality_limitations": [
            "provider_callbacks_are_not_raw_exchange_packets",
            "no_exchange_sequence_or_fifo_queue_position_is_inferred",
            "last_trade_snapshot_changes_are_not_confirmed_trade_prints",
        ],
    }
    report_content = (json.dumps(report, sort_keys=True, indent=2) + "\n").encode()
    _write_new(report_path, report_content)
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0


def _cmd_normalize(args: argparse.Namespace) -> int:
    instrument_map = json.loads(args.instrument_map.read_text(encoding="utf-8"))
    if not isinstance(instrument_map, dict) or any(not isinstance(k, str) or not isinstance(v, str) or not v for k, v in instrument_map.items()):
        raise ValueError("instrument map must be a JSON object of provider instrument ID to canonical instrument ID")
    venue_map = {}
    if args.venue_map:
        venue_map = json.loads(args.venue_map.read_text(encoding="utf-8"))
        if not isinstance(venue_map, dict) or any(not isinstance(k, str) or not isinstance(v, str) or not v for k, v in venue_map.items()):
            raise ValueError("venue map must be a JSON object of provider instrument ID to canonical venue")
    normalizer = (UpstoxV3Normalizer(instrument_map, venue_map) if args.provider == "upstox"
                  else FyersV3Normalizer(instrument_map))
    if args.provider == "upstox":
        return _normalize_upstox_bounded(args, normalizer)
    rows = _read_jsonl(args.input)
    events = []
    gaps = []
    warnings = []
    expected_chain = "0" * 64
    chain_seen = False
    chain_verified = False
    chain_presence: set[bool] = set()
    for idx, row in enumerate(rows, 1):
        if args.provider == "fyers":
            chain_presence.add("record_hash" in row)
        if args.provider == "fyers" and "record_hash" in row:
            record = dict(row)
            record_hash = record.pop("record_hash")
            previous = record.get("previous_hash")
            if previous != expected_chain:
                raise ValueError(f"FYERS journal hash-chain predecessor mismatch at row {idx}")
            expected = hashlib.sha256(bytes.fromhex(expected_chain) + canonical_bytes(record)).hexdigest()
            if record_hash != expected:
                raise ValueError(f"FYERS journal hash mismatch at row {idx}")
            expected_chain = expected
            chain_seen = True
        event_type = row.get("event_type")
        if event_type == "gap":
            gaps.append(row)
            continue
        if args.provider == "fyers" and event_type not in (None, "market_data"):
            continue
        # The Upstox recorder's diagnostic observations do not
        # contain decoded prices/depth. Raw binary Protobuf frames must first
        # pass through the Rust decoder and be exported as full decoded JSON.
        receive = row.get("received_utc_ns", row.get("received_at_utc_ns"))
        if type(receive) is not int or receive <= 0:
            raise ValueError(f"row {idx} lacks a valid received UTC nanosecond timestamp")
        monotonic = row.get("received_monotonic_ns")
        sequence = row.get("local_sequence", row.get("capture_sequence"))
        payload = row.get("payload") if args.provider == "fyers" else row.get("message", row)
        if args.provider == "upstox" and isinstance(payload, dict) and "feed_keys" in payload and "feeds" not in payload:
            raise ValueError("Upstox decoded-observations.jsonl is a diagnostic index, not a full quote export; decode raw protobuf frames and supply full V3 JSON message envelopes")
        if not isinstance(payload, dict):
            raise ValueError(f"row {idx} payload is not a decoded provider object")
        frame_hash = row.get("payload_sha256") if args.provider == "upstox" else row.get("record_hash")
        if frame_hash is None:
            frame_hash = row.get("source_frame_sha256")
        kwargs = {"received_utc_ns": receive, "received_monotonic_ns": monotonic,
                  "local_sequence": sequence, "source_frame_sha256": frame_hash}
        normalized = (normalizer.normalize(payload, **kwargs) if args.provider == "upstox"
                      else normalizer.normalize_observations(payload, **kwargs))
        events.extend(normalized)
    if args.provider == "fyers" and len(chain_presence) > 1:
        raise ValueError("FYERS input mixes hash-chained and unchained records")
    if args.provider == "fyers" and rows and not chain_seen:
        warnings.append("input_journal_hash_chain_not_present; source integrity was not verified by this command")
    if args.provider == "fyers" and chain_seen:
        chain_verified = True
    if gaps and not args.allow_gaps:
        raise ValueError(f"capture contains {len(gaps)} explicit FYERS callback gap records; refusing to publish normalized stream (use --allow-gaps only for explicitly quarantined research)")
    if not events:
        raise ValueError("no market observations were normalized")
    events.sort(key=lambda e: (e.received_utc_ns, e.provider, e.local_sequence if e.local_sequence is not None else -1, e.event_id))
    content = b"".join(canonical_bytes(e.as_dict()) + b"\n" for e in events)
    report_path = args.output.with_suffix(args.output.suffix + ".quality.json")
    report = {"schema_version": 1, "provider": args.provider, "input_rows": len(rows),
              "canonical_events": len(events), "explicit_capture_gaps": len(gaps),
              "source_hash_chain_present": chain_seen, "source_hash_chain_verified": chain_verified,
              "warnings": warnings, "output_sha256": hashlib.sha256(content).hexdigest(),
              "research_grade": not bool(gaps) and (args.provider != "fyers" or chain_verified),
              "quality_limitations": [
                  "provider_callbacks_are_not_raw_exchange_packets",
                  "no_exchange_sequence_or_fifo_queue_position_is_inferred",
                  "last_trade_snapshot_changes_are_not_confirmed_trade_prints",
              ]}
    report_content = (json.dumps(report, sort_keys=True, indent=2) + "\n").encode()
    _write_many_new([(args.output, content), (report_path, report_content)])
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0


def _cmd_reconcile(args: argparse.Namespace) -> int:
    events = [market_event_from_dict(row) for row in _read_jsonl(args.events)]
    results = reconcile_latest(events, now_ns=args.now_utc_ns,
        max_age_ns=args.max_age_ns, max_receive_skew_ns=args.max_receive_skew_ns,
        max_source_skew_ns=args.max_source_skew_ns,
        max_price_difference_bps=__import__("decimal").Decimal(str(args.max_price_difference_bps)))
    payload = {"schema_version": 1, "now_utc_ns": args.now_utc_ns,
               "results": [asdict_result(x) for x in results]}
    text = json.dumps(payload, sort_keys=True, indent=2, allow_nan=False) + "\n"
    if args.output:
        _write_new(args.output, text.encode())
    else:
        print(text, end="")
    return 0


def asdict_result(result):
    from dataclasses import asdict
    value = asdict(result)
    for key, item in list(value.items()):
        if hasattr(item, "as_tuple"):
            value[key] = str(item)
        elif item.__class__.__name__ == "Decimal":
            value[key] = str(item)
    return value


def _cmd_demo(_: argparse.Namespace) -> int:
    print(json.dumps(run_demo(), sort_keys=True, indent=2))
    return 0


def _cmd_build_dataset(args: argparse.Namespace) -> int:
    event_rows = _read_jsonl(args.events)
    events = [market_event_from_dict(row) for row in event_rows]
    refs = _load_reference_data(args.reference_data)
    builder = PITDatasetBuilder(refs)
    rows, manifest = builder.build(events, dataset_id=args.dataset_id,
                                   created_at_utc_ns=args.created_at_utc_ns,
                                   max_event_age_ns=args.max_event_age_ns,
                                   universe_id=args.universe_id)
    if not rows:
        raise ValueError("no eligible rows remain after point-in-time mapping and time filters")
    signer = None
    if args.kms_key_id:
        from .kms_signing import AwsKmsEd25519Signer
        signer = AwsKmsEd25519Signer.from_default_aws_credentials(args.kms_key_id, region_name=args.kms_region)
    destination = write_dataset(rows, manifest, args.output, parquet=args.parquet,
                                signer=signer, signing_key_id=args.kms_key_id)
    print(json.dumps({"status": "published", "path": str(destination),
                      "dataset_id": manifest.dataset_id, "row_count": manifest.row_count,
                      "data_sha256": manifest.data_sha256, "parquet": args.parquet,
                      "signed": signer is not None,
                      "signing_key_id": args.kms_key_id,
                      "signing_public_key_b64": None if signer is None else signer.public_key_b64,
                      "warnings": list(manifest.warnings)}, sort_keys=True, indent=2))
    return 0


def _cmd_verify_dataset(args: argparse.Namespace) -> int:
    trusted: dict[str, str] = {}
    for item in args.trusted_key:
        key_id, separator, public_key = item.partition("=")
        if not separator or not key_id or not public_key or key_id in trusted:
            raise ValueError("--trusted-key must be unique KEY_ID=BASE64 values")
        trusted[key_id] = public_key
    result = verify_published_dataset(args.dataset, trusted_public_keys=trusted or None)
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0


def _cmd_alpha_search(args: argparse.Namespace) -> int:
    bars = []
    with args.csv.open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        if not reader.fieldnames or not {"timestamp_ns", "close"}.issubset(reader.fieldnames):
            raise ValueError("CSV must contain timestamp_ns and close columns")
        for line, row in enumerate(reader, 2):
            try:
                bars.append(Bar(timestamp_ns=int(row["timestamp_ns"]), close=float(row["close"])))
            except (TypeError, ValueError) as exc:
                raise ValueError(f"{args.csv}:{line}: invalid timestamp_ns or close") from exc
    costs = AlphaCosts(brokerage_bps=args.brokerage_bps,
                       exchange_and_regulatory_bps=args.exchange_regulatory_bps,
                       taxes_bps=args.taxes_bps, spread_bps=args.spread_bps,
                       slippage_bps=args.slippage_bps, market_impact_bps=args.market_impact_bps)
    report = search_alpha(bars, costs=costs, periods_per_year=args.periods_per_year,
                          require_statistical_significance=args.require_statistical_significance)
    payload = json.dumps(report, sort_keys=True, indent=2, allow_nan=False) + "\n"
    if args.output:
        _write_new(args.output, payload.encode("utf-8"))
    else:
        print(payload, end="")
    # A rejected candidate is a valid research result, not a CLI execution error.
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="quantlab", description="Nirnay deterministic quantitative research tools")
    sub = parser.add_subparsers(dest="command", required=True)
    demo = sub.add_parser("demo", help="run deterministic synthetic EMA/backtest fixture")
    demo.set_defaults(handler=_cmd_demo)

    build = sub.add_parser("build-dataset", help="build immutable bitemporal point-in-time JSONL/Parquet dataset")
    build.add_argument("--events", type=Path, required=True, help="canonical MarketEvent JSONL")
    build.add_argument("--reference-data", type=Path, required=True, help="bitemporal listing/action JSON")
    build.add_argument("--dataset-id", required=True)
    build.add_argument("--created-at-utc-ns", type=int, required=True)
    build.add_argument("--max-event-age-ns", type=int)
    build.add_argument("--universe-id", help="apply a bitemporal point-in-time universe membership filter")
    build.add_argument("--output", type=Path, required=True)
    build.add_argument("--parquet", action="store_true", help="also emit Zstandard-compressed Parquet; requires pyarrow")
    build.add_argument("--kms-key-id", help="optional AWS KMS Ed25519 key ARN for detached dataset-manifest signing")
    build.add_argument("--kms-region", help="optional AWS region for the dataset-signing KMS key")
    build.set_defaults(handler=_cmd_build_dataset)

    verify = sub.add_parser("verify-dataset", help="verify manifest, artifact checksums and canonical dataset digest")
    verify.add_argument("dataset", type=Path)
    verify.add_argument("--trusted-key", action="append", default=[], metavar="KEY_ID=BASE64",
                        help="trusted Ed25519 public key for signed datasets; repeat per key")
    verify.set_defaults(handler=_cmd_verify_dataset)

    normalize = sub.add_parser("normalize", help="normalize decoded Upstox V3 JSON or FYERS SDK journal to canonical MarketEvent JSONL")
    normalize.add_argument("--provider", choices=("upstox", "fyers"), required=True)
    normalize.add_argument("--input", type=Path, required=True)
    normalize.add_argument("--instrument-map", type=Path, required=True)
    normalize.add_argument("--venue-map", type=Path)
    normalize.add_argument("--output", type=Path, required=True)
    normalize.add_argument("--allow-gaps", action="store_true", help="allow explicit FYERS gap records; output is marked non-research-grade")
    normalize.set_defaults(handler=_cmd_normalize)

    reconcile = sub.add_parser("reconcile", help="cross-check latest Upstox/FYERS canonical state observations")
    reconcile.add_argument("--events", type=Path, required=True)
    reconcile.add_argument("--now-utc-ns", type=int, required=True)
    reconcile.add_argument("--max-age-ns", type=int, default=5_000_000_000)
    reconcile.add_argument("--max-receive-skew-ns", type=int, default=2_000_000_000)
    reconcile.add_argument("--max-source-skew-ns", type=int, default=2_000_000_000)
    reconcile.add_argument("--max-price-difference-bps", type=float, default=5.0)
    reconcile.add_argument("--output", type=Path)
    reconcile.set_defaults(handler=_cmd_reconcile)

    alpha = sub.add_parser("alpha-search", help="bounded close-based candidate-family research screen")
    alpha.add_argument("csv", type=Path, help="chronological CSV with timestamp_ns,close")
    alpha.add_argument("--periods-per-year", type=float, default=252.0,
                       help="annualisation factor matching bar cadence; not inferred from timestamps")
    alpha.add_argument("--brokerage-bps", type=float, default=2.0)
    alpha.add_argument("--exchange-regulatory-bps", type=float, default=0.5)
    alpha.add_argument("--taxes-bps", type=float, default=0.5)
    alpha.add_argument("--spread-bps", type=float, default=2.0)
    alpha.add_argument("--slippage-bps", type=float, default=1.0)
    alpha.add_argument("--market-impact-bps", type=float, default=1.0)
    alpha.add_argument("--require-statistical-significance", action="store_true")
    alpha.add_argument("--output", type=Path)
    alpha.set_defaults(handler=_cmd_alpha_search)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return int(args.handler(args))
    except (OSError, ValueError, RuntimeError) as exc:
        print(f"quantlab: error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
