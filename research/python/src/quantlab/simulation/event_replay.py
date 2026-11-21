"""Namespace shim for the typed event-replay simulator.

Every name below is a direct re-export of ``quantlab.sim.replay``. No replay
behavior is duplicated or reimplemented here.
"""
from __future__ import annotations

from ..sim.replay import (
    ComparisonResult,
    RunRecord,
    build_record,
    compare_records,
    read_record,
    record_from_dict,
    record_to_dict,
    replay_in_fresh_process,
    state_hash_from_config,
    verifier_recompute,
    write_record,
)

__all__ = [
    "ComparisonResult",
    "RunRecord",
    "build_record",
    "compare_records",
    "read_record",
    "record_from_dict",
    "record_to_dict",
    "replay_in_fresh_process",
    "state_hash_from_config",
    "verifier_recompute",
    "write_record",
]
