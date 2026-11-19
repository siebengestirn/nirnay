#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.1 exit condition: a captured session replayed in a fresh process yields
//! byte-identical event identities and continuity decisions.

use std::{
    fs::File,
    io::{
        BufRead, //
        BufReader,
        Write,
    },
    process::{
        Command, //
        Output,
    },
};

use market_event::{
    ContinuityMachine, //
    ObservationOutcome,
    capture_frame_id,
    observation_id,
};

const PAYLOADS: [&[u8]; 4] = [b"frame-a", b"frame-b", b"frame-c", b"frame-d"];
const OUTPUT_ENV: &str = "NIRNAY_M2_REPLAY_OUTPUT";

fn expected_lines() -> String {
    let mut machine = ContinuityMachine::default();
    let mut out = String::new();
    for (index, payload) in PAYLOADS.iter().enumerate() {
        let frame = capture_frame_id(payload).expect("frame id");
        let obs = observation_id("upstox", "NSE", "NSE_EQ|TEST", payload).expect("obs id");
        let sequence = (index as u64) + 1;
        let outcome = machine.observe(0, sequence).expect("continuity");
        let name = match outcome {
            ObservationOutcome::Initial => "initial",
            ObservationOutcome::InOrder => "in_order",
            ObservationOutcome::Duplicate => "duplicate",
            ObservationOutcome::OutOfOrder => "out_of_order",
            ObservationOutcome::Gap => "gap",
            ObservationOutcome::ConnectionReset => "connection_reset",
            ObservationOutcome::Recovered => "recovered",
        };
        out.push_str(&format!(
            "{frame}|{obs}|{sequence}|{name}|{}\n",
            machine.state() as u8
        ));
    }
    out
}

#[test]
#[ignore = "child-only replay worker invoked by parent with --ignored"]
fn child_replay_worker() {
    let path = std::env::var_os(OUTPUT_ENV).expect("child requires output path");
    let mut file = File::create(path).expect("create replay output");
    file.write_all(expected_lines().as_bytes())
        .expect("write replay output");
    file.sync_all().expect("sync replay output");
}

#[test]
fn fresh_process_replay_is_identical() {
    let directory = tempfile::tempdir().expect("tempdir");
    let output = directory.path().join("replay.txt");
    let child: Output = Command::new(std::env::current_exe().expect("test binary"))
        .arg("--ignored")
        .arg("child_replay_worker")
        .env(OUTPUT_ENV, &output)
        .output()
        .expect("spawn fresh replay process");
    assert!(
        child.status.success(),
        "fresh replay worker completed successfully"
    );
    let replayed = std::fs::read_to_string(&output).expect("read replay output");
    assert_eq!(
        replayed,
        expected_lines(),
        "identities and continuity match in a fresh process"
    );

    let lines: Vec<String> = BufReader::new(File::open(&output).expect("open"))
        .lines()
        .map(|line| line.expect("line"))
        .collect();
    assert_eq!(lines.len(), PAYLOADS.len(), "one line per captured frame");

    let mut machine = ContinuityMachine::default();
    for index in 0..PAYLOADS.len() {
        machine.observe(0, (index as u64) + 1).expect("in order");
    }
    assert!(machine.state().is_clean(), "replayed session ends clean");
}
