"""Fail-closed resolver for hash-pinned calibration artifacts.

Format validation is not fidelity evidence: an ``artifact_sha256`` only counts
when it resolves to a committed fixture whose dated body hashes back to the
declared digest.  No real calibration artifact exists, so summaries without a
dated artifact stay explicitly uncalibrated.
"""
from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import Any, Mapping

from .events import digest, is_sha256, to_jsonable
from .execution.costs import CostModel


class ArtifactError(ValueError):
    """Raised when a declared artifact cannot be resolved to a dated fixture."""


@dataclass(frozen=True, slots=True)
class CalibrationArtifact:
    """One resolved, dated, content-addressed fixture."""

    digest: str
    dated_on: str
    payload: dict[str, Any]
    path: str


@dataclass(frozen=True, slots=True)
class ArtifactResolver:
    """Resolve declared digests against a directory of committed fixtures."""

    root: Path

    def __post_init__(self) -> None:
        if not isinstance(self.root, Path):
            raise ValueError("artifact fixture root must be a Path")

    def resolve(self, artifact_sha256: str) -> CalibrationArtifact:
        """Resolve a digest to a dated fixture or fail closed."""
        if not is_sha256(artifact_sha256):
            raise ArtifactError("artifact_sha256 must be a lowercase SHA-256 digest")
        path = self.root / f"{artifact_sha256}.json"
        if not path.is_file():
            raise ArtifactError(f"artifact {artifact_sha256} is not a committed fixture")
        try:
            data: Any = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as exc:
            raise ArtifactError(f"artifact {artifact_sha256} is not readable JSON") from exc
        if not isinstance(data, dict):
            raise ArtifactError("artifact fixture must be a JSON object")
        if data.get("schema_version") != 1:
            raise ArtifactError("unsupported artifact fixture schema")
        if data.get("artifact_sha256") != artifact_sha256:
            raise ArtifactError("artifact fixture declares a different digest")
        dated_on = data.get("dated_on")
        if not isinstance(dated_on, str):
            raise ArtifactError("artifact fixture requires a dated_on field")
        try:
            date.fromisoformat(dated_on)
        except ValueError as exc:
            raise ArtifactError("artifact dated_on must be an ISO date") from exc
        payload = data.get("payload")
        if not isinstance(payload, dict):
            raise ArtifactError("artifact fixture requires a payload object")
        if digest(to_jsonable(payload)) != artifact_sha256:
            raise ArtifactError("artifact payload does not hash to the declared digest")
        return CalibrationArtifact(
            digest=artifact_sha256,
            dated_on=dated_on,
            payload=payload,
            path=str(path),
        )


def cost_provenance(cost_model: CostModel) -> tuple[tuple[str, ...], tuple[str, ...]]:
    """Collect declared artifact digests and assumption labels from a cost model."""
    if not isinstance(cost_model, CostModel):
        raise ValueError("cost_provenance requires a CostModel")
    rows: list[Any] = [
        *cost_model.fee_schedule.entries,
        *cost_model.slippage_schedule.entries,
        cost_model.impact,
        *cost_model.borrow,
        *cost_model.funding,
    ]
    if cost_model.margin is not None:
        rows.append(cost_model.margin)
    artifacts = tuple(str(row.artifact_sha256) for row in rows if row.artifact_sha256 is not None)
    assumptions = tuple(str(row.assumption) for row in rows if row.assumption is not None)
    return artifacts, assumptions


def calibration_statement(resolver: ArtifactResolver, cost_model: CostModel) -> str:
    """Return a claim-limited calibration statement; unresolved artifacts raise."""
    artifacts, assumptions = cost_provenance(cost_model)
    resolved = tuple(resolver.resolve(item) for item in artifacts)
    if not resolved:
        return "not calibrated; assumption-labelled research model only"
    if len(resolved) != len(artifacts):
        raise ArtifactError("not every declared artifact resolved")
    if any(item.payload.get("kind") != "calibration" for item in resolved):
        return "not calibrated; resolved fixture is not a calibration result"
    dated = sorted({item.dated_on for item in resolved})
    label = "calibrated; dated artifact" if len(dated) == 1 else "calibrated; dated artifacts"
    return f"{label} {', '.join(dated)}; assumptions retained: {len(assumptions)}"
