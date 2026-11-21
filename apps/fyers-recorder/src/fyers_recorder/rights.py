"""Explicit, fail-closed data-rights gate for the FYERS recorder.

The registry path is supplied by the operator. There is no default path, no
inference from an entitlement string and no right granted by this module.
Only ``PERMITTED`` in a declared product authorizes an operation; persistence
operations additionally require a verified entitlement.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

CAPTURE = "capture"
STORE = "store"
REPLAY = "replay"
BACKUP = "backup"
INTERNAL_ANALYTICS = "internal_analytics"
EXTERNAL_REDISTRIBUTION = "external_redistribution"
RETENTION = "retention"

ALL_OPERATIONS = (
    CAPTURE,
    STORE,
    REPLAY,
    BACKUP,
    INTERNAL_ANALYTICS,
    EXTERNAL_REDISTRIBUTION,
    RETENTION,
)
PERSISTENCE_OPERATIONS = frozenset({STORE, REPLAY, BACKUP, RETENTION})


@dataclass(frozen=True)
class ProductKey:
    provider: str
    product: str
    venue: str
    segment: str


class RightsDenied(ValueError):
    """The registry did not grant the requested operation."""


def load_registry(path: Path) -> dict[str, object]:
    if not str(path).strip():
        raise RightsDenied("an explicit rights registry path is required")
    try:
        registry = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RightsDenied(f"rights registry could not be read: {path}") from error
    if not isinstance(registry, dict) or registry.get("schema_version") != 1:
        raise RightsDenied("rights registry has an unsupported schema version")
    if not isinstance(registry.get("products"), list) or not registry["products"]:
        raise RightsDenied("rights registry declares no products")
    return registry


def authorize(path: Path, product_key: ProductKey, operation: str) -> None:
    """Return only when the exact product grants ``operation``.

    UNKNOWN, PROHIBITED, missing rights, missing products and unverified
    persistence entitlements all raise ``RightsDenied``.
    """
    if operation not in ALL_OPERATIONS:
        raise RightsDenied(f"unknown data operation: {operation}")
    registry = load_registry(path)
    identity = (
        ("provider", product_key.provider),
        ("product", product_key.product),
        ("venue", product_key.venue),
        ("segment", product_key.segment),
    )
    for product in registry["products"]:
        if not isinstance(product, dict) or any(product.get(k) != v for k, v in identity):
            continue
        subject = f"{product_key.provider}/{product_key.product} in {path}"
        rights = product.get("rights")
        if not isinstance(rights, dict):
            raise RightsDenied(f"no rights declared for {subject}")
        evidence = rights.get(operation)
        if not isinstance(evidence, dict) or evidence.get("state") != "PERMITTED":
            raise RightsDenied(f"{operation} is not permitted for {subject}")
        if operation in PERSISTENCE_OPERATIONS:
            entitlement = product.get("entitlement")
            if not isinstance(entitlement, dict) or entitlement.get("verified") is not True:
                raise RightsDenied(f"{operation} needs a verified entitlement for {subject}")
        return
    raise RightsDenied(f"no declared product for {product_key} in {path}")
