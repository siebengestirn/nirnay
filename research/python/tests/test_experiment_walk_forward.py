"""Walk-forward declarations must obey the registered holdout geometry."""
from __future__ import annotations

import pytest

from quantlab.experiments import ExperimentError, validate_period_geometry
from quantlab.experiments.walk_forward import (build_walk_forward_declarations,
                                               search_space_for_declaration,
                                               validate_split_geometry)
from quantlab.walk_forward import WalkForwardSplit


def _declarations():
    return build_walk_forward_declarations(200, n_splits=3, embargo=2,
                                           holdout_fraction=0.2)


def test_declarations_are_deterministic_and_holdout_safe():
    first = _declarations()
    second = _declarations()
    assert first == second
    assert first["uses_final_holdout"] is False
    assert first["split_ids_sha256"] == second["split_ids_sha256"]
    assert len(first["declarations"]) == 3
    holdout = first["holdout_period"]
    assert holdout == {"start_index": 160, "end_index": 200}
    for declaration in first["declarations"]:
        assert declaration["validation_period"]["end_index"] <= (
            holdout["start_index"] - first["embargo_observations"]
        )
        assert declaration["validation_period"]["start_index"] >= (
            declaration["train_period"]["end_index"] + first["embargo_observations"]
        )


def test_split_touching_holdout_is_refused():
    bad = WalkForwardSplit(split_id=0, train_start=0, train_end=100,
                           test_start=102, test_end=165)
    with pytest.raises(ExperimentError, match="overlaps"):
        validate_split_geometry(bad, {"start_index": 160, "end_index": 200}, 2)


def test_split_without_embargo_is_refused():
    bad = WalkForwardSplit(split_id=0, train_start=0, train_end=100,
                           test_start=101, test_end=120)
    with pytest.raises(ExperimentError, match="embargo"):
        validate_split_geometry(bad, {"start_index": 160, "end_index": 200}, 2)


def test_search_space_declaration_passes_registered_geometry():
    declaration = _declarations()["declarations"][0]
    from quantlab.experiments.walk_forward import WalkForwardDeclaration

    value = WalkForwardDeclaration(
        split_id=declaration["split_id"],
        train_period=declaration["train_period"],
        validation_period=declaration["validation_period"],
        holdout_period=declaration["holdout_period"],
        embargo_observations=declaration["embargo_observations"],
    )
    space = search_space_for_declaration(
        value, "momentum", [{"family": "momentum", "lookback": 5}]
    )
    assert space["train_period"] == value.train_period
    assert space["validation_period"] == value.validation_period
    assert space["holdout_period"] == value.holdout_period
    record = {"validation_period": value.validation_period,
              "holdout_period": value.holdout_period, "search_space": space}
    validate_period_geometry(record)
    assert "split_ids_sha256" in space


def test_invalid_geometry_inputs_are_refused():
    with pytest.raises(ExperimentError, match="observations"):
        build_walk_forward_declarations(0, n_splits=2)
    with pytest.raises(ExperimentError, match="holdout_fraction"):
        build_walk_forward_declarations(200, n_splits=2, holdout_fraction=1.0)
    with pytest.raises(ExperimentError, match="start_index"):
        validate_split_geometry(
            WalkForwardSplit(0, 0, 10, 12, 20), {"start_ns": 1, "end_ns": 2}, 1
        )
