#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! End-to-end adapter test: immutable dataset to account journal and replay.

use std::path::{
    Path, //
    PathBuf,
};
use std::process::{
    Command, //
    Output,
};

use serde_json::{
    Value, //
    json,
};

const RUN_ID: &str = "0123456789abcdef0123456789abcdef";
const INSTRUMENT: &str = "nse.cash.synthetic";

fn run_spec() -> Value {
    json!({
        "schema_version": 1,
        "stream_id": "dataset-paper-test",
        "run_id": RUN_ID,
        "account_id": 1,
        "strategy_id": 2,
        "algo_id": 3,
        "strategy_version": "dataset-paper-v1",
        "starting_cash_paise": 1_000_000,
        "trade_date": "2026-11-18",
        "max_quote_age_ns": 1_000_000_000_000_000_000_u64,
        "instrument": {"instrument_id": INSTRUMENT, "isin": "INE848E01016", "venue": "NSE",
                       "venue_symbol": "SYNTH", "market_lot": 1, "tick_paise": 5},
        "risk": {"max_order_quantity": 10, "max_order_notional_paise": 1_000_000,
                 "max_position_quantity": 10, "max_gross_exposure_paise": 1_000_000,
                 "max_daily_loss_paise": 100_000, "max_open_orders": 5,
                 "max_orders_per_second": 5, "max_price_deviation_ppm": 1_000_000,
                 "allow_short": false},
        "strategy": {"max_entry_price_paise": 10_100, "order_quantity": 1}
    })
}

fn dataset_event(sequence: u64, bid: &str, ask: &str) -> Value {
    json!({
        "schema_version": 1,
        "event_id": format!("upstox:{sequence}:NSE_EQ|SYNTH:book"),
        "provider": "upstox",
        "provider_instrument_id": "NSE_EQ|SYNTH",
        "instrument_id": INSTRUMENT,
        "venue": "NSE",
        "event_kind": "quote",
        "received_utc_ns": 1_000_000_000 + sequence,
        "received_monotonic_ns": sequence,
        "local_sequence": sequence,
        "source_timestamp_ns": 1_000_000_000 + sequence,
        "source_timestamp_semantics": "upstox_currentTs_feed_envelope_epoch_ms",
        "last_trade_time_ns": null,
        "last_trade_time_semantics": null,
        "last_price": ask,
        "last_quantity": 1,
        "cumulative_volume": 1,
        "aggressor_side": null,
        "bids": [{"price": bid, "quantity": 10, "order_count": null}],
        "asks": [{"price": ask, "quantity": 10, "order_count": null}],
        "raw_sha256": "a".repeat(64),
        "quality_flags": [],
        "source_frame_sha256": "b".repeat(64)
    })
}

fn write_fixture(root: &Path, rows: &[Value]) -> (PathBuf, PathBuf) {
    let dataset = root.join("dataset");
    std::fs::create_dir_all(&dataset).unwrap();
    let events = dataset.join("events.jsonl");
    let content = rows
        .iter()
        .map(|row| serde_json::to_string(row).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&events, content).unwrap();
    std::fs::write(
        dataset.join("manifest.json"),
        json!({"dataset_id": "dataset-paper-test", "schema_version": 1}).to_string(),
    )
    .unwrap();
    let spec = root.join("spec.json");
    std::fs::write(&spec, serde_json::to_string(&run_spec()).unwrap()).unwrap();
    (dataset, spec)
}

fn invoke(dataset: &Path, spec: &Path, output: &Path, replay: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_paper-run"));
    command
        .arg("--dataset")
        .arg(dataset)
        .arg("--spec")
        .arg(spec)
        .arg("--output")
        .arg(output);
    if replay {
        command.arg("--replay");
    }
    command.output().unwrap()
}

#[test]
fn dataset_produces_journal_and_replays_without_duplicate_orders() {
    let directory = tempfile::tempdir().unwrap();
    let rows = vec![
        dataset_event(1, "99.00", "101.00"),
        dataset_event(2, "99.50", "100.50"),
        dataset_event(3, "99.75", "100.25"),
    ];
    let (dataset, spec) = write_fixture(directory.path(), &rows);
    let output = directory.path().join("run");

    let first = invoke(&dataset, &spec, &output, false);
    assert!(
        first.status.success(),
        "first run failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        output.join("journal.jsonl").is_dir(),
        "journal was published"
    );
    assert!(
        output.join("run-report.json").is_file(),
        "report was published"
    );
    let first_report = std::fs::read(output.join("run-report.json")).unwrap();
    let first_value: Value = serde_json::from_slice(&first_report).unwrap();
    assert_eq!(first_value["input_rows"], 3, "all rows were consumed");
    assert_eq!(
        first_value["intent_submitted"], true,
        "one strategy intent was emitted"
    );
    let first_records = first_value["journal_records"].as_u64().unwrap();

    let second = invoke(&dataset, &spec, &output, true);
    assert!(
        second.status.success(),
        "replay failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_report = std::fs::read(output.join("run-report.json")).unwrap();
    let second_value: Value = serde_json::from_slice(&second_report).unwrap();
    for field in [
        "schema_version",
        "stream_id",
        "dataset_sha256",
        "input_rows",
        "quotes_accepted",
        "journal_sha256",
        "verification",
    ] {
        assert_eq!(
            second_value[field], first_value[field],
            "replay changed deterministic field {field}"
        );
    }
    assert_eq!(
        second_value["journal_records"].as_u64().unwrap(),
        first_records,
        "replay did not duplicate journal records"
    );
    assert_eq!(
        second_value["decision_deduplicated"], true,
        "the deterministic intent was recognized on replay"
    );
}

#[test]
fn rejects_a_price_that_is_not_an_exact_paise_decimal() {
    let directory = tempfile::tempdir().unwrap();
    let rows = vec![dataset_event(1, "99.001", "101.00")];
    let (dataset, spec) = write_fixture(directory.path(), &rows);
    let output = directory.path().join("rejected");
    let result = invoke(&dataset, &spec, &output, false);
    assert!(!result.status.success(), "sub-paise price must fail closed");
    assert!(
        !output.join("run-report.json").exists(),
        "no report is published"
    );
}

#[test]
fn rejects_an_instrument_outside_the_run_spec() {
    let directory = tempfile::tempdir().unwrap();
    let mut row = dataset_event(1, "99.00", "101.00");
    row["instrument_id"] = json!("nse.cash.other");
    let (dataset, spec) = write_fixture(directory.path(), &[row]);
    let output = directory.path().join("rejected-instrument");
    let result = invoke(&dataset, &spec, &output, false);
    assert!(
        !result.status.success(),
        "foreign instrument must fail closed"
    );
    assert!(
        !output.join("run-report.json").exists(),
        "no report is published"
    );
}
