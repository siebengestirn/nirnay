#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.3 tests: artifact gating and golden-vector parity with the Python
//! `Fraction` reference. The fixture is a systems vector, not alpha evidence.

use std::path::Path;

use serde::Deserialize;
use strategy_runtime::{
    FrozenArtifact, //
    Rational,
    RationalEmaCross,
    SUPPORTED_RULE,
    Strategy,
    StrategyEvent,
};

#[derive(Debug, Deserialize)]
struct VectorEvent {
    sequence: u64,
    instrument: String,
    midpoint_num: i64,
    midpoint_den: i64,
    best_bid: i64,
    best_ask: i64,
    position: i64,
    has_open_order: bool,
    intent: Option<GoldenIntent>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct GoldenIntent {
    intent_id: String,
    candidate_id: String,
    instrument_id: String,
    side: String,
    quantity: i64,
    limit_price_paise: i64,
}

#[derive(Debug, Deserialize)]
struct VectorFile {
    params: Params,
    events: Vec<VectorEvent>,
}

#[derive(Debug, Deserialize)]
struct Params {
    fast: i64,
    slow: i64,
    quantity: i64,
    min_observations: u32,
    aggressive_limit_bps: i64,
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

use std::path::PathBuf;

fn artifact() -> FrozenArtifact {
    FrozenArtifact::load(&golden_path("artifact-v1.json"), SUPPORTED_RULE).expect("gated artifact")
}

#[test]
fn frozen_artifact_is_eligible() {
    let a = artifact();
    assert_eq!(
        a.status,
        strategy_runtime::ArtifactStatus::ApprovedForPaper,
        "fixture is paper-approved"
    );
    assert!(
        !a.artifact_sha256.is_empty(),
        "the exact artifact bytes are hashed"
    );
}

#[test]
fn candidate_artifact_is_refused() {
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(golden_path("artifact-v1.json")).expect("artifact"))
            .expect("json");
    value["status"] = serde_json::json!("candidate");
    let path = std::env::temp_dir().join(format!("nirnay-candidate-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(&value).expect("serialize")).expect("write");
    assert!(
        FrozenArtifact::load(&path, SUPPORTED_RULE).is_err(),
        "a research candidate cannot be executed by the runtime"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn artifact_without_experiment_lineage_is_refused() {
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(golden_path("artifact-v1.json")).expect("artifact"))
            .expect("json");
    value["experiment_ids"] = serde_json::json!([]);
    let path = std::env::temp_dir().join(format!("nirnay-no-lineage-{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(&value).expect("serialize")).expect("write");
    assert!(
        FrozenArtifact::load(&path, SUPPORTED_RULE).is_err(),
        "an artifact without experiment lineage is refused"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn golden_vectors_match_python_fraction_reference() {
    let artifact = artifact();
    let vector: VectorFile = serde_json::from_slice(
        &std::fs::read(golden_path("ema-rational-v1.json")).expect("vectors"),
    )
    .expect("vector json");
    let mut strategy = RationalEmaCross::from_artifact(
        &artifact,
        vector.params.fast,
        vector.params.slow,
        vector.params.quantity,
        vector.params.min_observations,
        vector.params.aggressive_limit_bps,
    )
    .expect("strategy");

    let mut emitted = 0;
    for event in vector.events {
        let midpoint =
            Rational::fraction(event.midpoint_num, event.midpoint_den).expect("rational");
        let input = StrategyEvent {
            observation_sequence: event.sequence,
            instrument_id: event.instrument.clone(),
            midpoint,
            best_bid: event.best_bid,
            best_ask: event.best_ask,
            position_quantity: event.position,
            has_open_order: event.has_open_order,
        };
        let intent = strategy.on_event(&input).expect("strategy step");
        match (intent, event.intent) {
            (Some(live), Some(golden)) => {
                assert_eq!(
                    live.intent_id, golden.intent_id,
                    "intent id matches parity vector"
                );
                assert_eq!(
                    live.candidate_id, golden.candidate_id,
                    "candidate id matches"
                );
                assert_eq!(
                    live.instrument_id, golden.instrument_id,
                    "instrument matches"
                );
                assert_eq!(live.side, golden.side, "side matches");
                assert_eq!(live.quantity, golden.quantity, "quantity matches");
                assert_eq!(
                    live.limit_price_paise, golden.limit_price_paise,
                    "exact limit price matches"
                );
                emitted += 1;
            }
            (None, None) => {}
            (live, golden) => panic!("intent mismatch: live={live:?} golden={golden:?}"),
        }
    }
    assert!(emitted > 0, "the vector exercises at least one intent");
}

#[test]
fn non_monotonic_observation_is_refused() {
    let artifact = artifact();
    let mut strategy =
        RationalEmaCross::from_artifact(&artifact, 2, 3, 1, 3, 100).expect("strategy");
    let input = StrategyEvent {
        observation_sequence: 5,
        instrument_id: "IN:TEST".to_owned(),
        midpoint: Rational::integer(100),
        best_bid: 9_995,
        best_ask: 10_005,
        position_quantity: 0,
        has_open_order: false,
    };
    strategy.on_event(&input).expect("first");
    assert!(
        strategy.on_event(&input).is_err(),
        "a repeated or regressed observation is refused"
    );
}

#[test]
fn strategy_event_surface_is_narrow() {
    // Structural check: serializing the input exposes exactly the observed
    // fields. No account, cash, order, broker or risk state can be supplied.
    let input = StrategyEvent {
        observation_sequence: 1,
        instrument_id: "IN:TEST".to_owned(),
        midpoint: Rational::integer(100),
        best_bid: 9_995,
        best_ask: 10_005,
        position_quantity: 0,
        has_open_order: false,
    };
    let json = serde_json::to_value(&input).expect("serialize event");
    let object = json.as_object().expect("event is an object");
    let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "best_ask",
            "best_bid",
            "has_open_order",
            "instrument_id",
            "midpoint",
            "observation_sequence",
            "position_quantity"
        ],
        "strategy input exposes only market observations and caller context"
    );
    for forbidden in ["account", "cash", "broker", "order", "permit", "risk"] {
        assert!(
            !object.contains_key(forbidden),
            "strategy input must not carry {forbidden} state"
        );
    }
}

/// A struct-literal artifact cannot bypass runtime gating through the
/// constructor. This reproduces the reviewer's forged-artifact probe.
#[test]
fn struct_literal_artifact_cannot_bypass_gating() {
    let forged = FrozenArtifact {
        candidate_id: "forged".to_owned(),
        hypothesis: String::new(),
        return_source: "momentum".to_owned(),
        universe: vec!["IN:TEST".to_owned()],
        entry_rule: "unsupported_rule".to_owned(),
        exit_rule: "none".to_owned(),
        rebalance_frequency: None,
        experiment_ids: Vec::new(),
        validation: strategy_runtime::ArtifactValidation {
            walk_forward_windows: 0,
            holdout_locked: false,
            multiple_testing_correction: String::new(),
            deflated_sharpe: String::new(),
            cost_model: String::new(),
            probability_of_backtest_overfit: None,
        },
        status: strategy_runtime::ArtifactStatus::ApprovedForPaper,
        artifact_sha256: String::new(),
    };
    let result = RationalEmaCross::from_artifact(&forged, 2, 3, 1, 3, 100);
    assert!(
        matches!(
            result,
            Err(strategy_runtime::StrategyError::ArtifactRejected(_))
        ),
        "the constructor re-gates the artifact before any strategy exists"
    );
}

/// The committed generator is executable and reproduces the checked-in golden
/// vectors byte for byte.
#[test]
fn golden_generator_reproduces_vectors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden_path = root.join("tests/golden/ema-rational-v1.json");
    let committed = std::fs::read(&golden_path).expect("read committed vector");
    let output = std::process::Command::new("python3")
        .arg(root.join("tests/generate_golden.py"))
        .output()
        .expect("run golden generator");
    let detail = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the generator is executable: {detail}"
    );
    let regenerated = std::fs::read(&golden_path).expect("read regenerated vector");
    assert_eq!(
        committed, regenerated,
        "the generator reproduces the golden vector"
    );
}
