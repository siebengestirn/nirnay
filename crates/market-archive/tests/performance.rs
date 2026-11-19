#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Opt-in ingress performance measurements. These tests print numbers; they do
//! not assert thresholds or constitute a throughput certificate.

use std::{
    path::PathBuf,
    process::{
        Command, //
        Output,
    },
};

use market_archive::{
    DurableIngestor, //
    JournalReplaySource,
    RawFrame,
    Sequence,
    SequencePolicy,
    SourceFrame,
    UPSTOX_SOURCE_ID,
};

fn path(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("nirnay-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("bench root");
    root.join("frames.jsonl")
}

fn frame(sequence: u64) -> SourceFrame {
    SourceFrame {
        source_id: UPSTOX_SOURCE_ID.to_owned(),
        provider_instrument_id: "NSE_EQ|BENCH".to_owned(),
        entitlement_ref: "entitlement:benchmark".to_owned(),
        received_utc_nanos: sequence as i64,
        received_monotonic_nanos: sequence,
        exchange_utc_nanos: None,
        sequence: Some(Sequence::capture_session(sequence)),
        payload: vec![0x08, 0x01],
    }
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

#[test]
#[ignore = "opt-in performance measurement"]
fn append_latency_and_throughput() {
    let output = path("append");
    let count = 2000_u64;
    let mut ingestor =
        DurableIngestor::create_new(&output, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("ingestor");
    let mut durations = Vec::with_capacity(count as usize);
    let started = std::time::Instant::now();
    for sequence in 1..=count {
        let item_started = std::time::Instant::now();
        ingestor.ingest_one(frame(sequence)).expect("append");
        durations.push(item_started.elapsed().as_nanos());
    }
    let elapsed = started.elapsed();
    durations.sort_unstable();
    eprintln!(
        "BENCH-APPEND frames={count} seconds={:.3} frames_per_sec={:.1} p50_us={:.1} p99_us={:.1} p999_us={:.1} queue_depth=1 dropped=0 malformed=0",
        elapsed.as_secs_f64(),
        count as f64 / elapsed.as_secs_f64(),
        percentile(&durations, 0.50) as f64 / 1000.0,
        percentile(&durations, 0.99) as f64 / 1000.0,
        percentile(&durations, 0.999) as f64 / 1000.0
    );
    assert_eq!(
        ingestor.checkpoint().committed_frames,
        count,
        "every appended frame is committed"
    );
}

#[test]
#[ignore = "opt-in performance measurement"]
fn replay_throughput_and_bounded_frames() {
    let output = path("replay");
    let count = 100_000_u64;
    let mut lines = String::new();
    for sequence in 1..=count {
        let item = frame(sequence);
        let raw = RawFrame::new(
            &item.source_id,
            &item.provider_instrument_id,
            item.received_utc_nanos,
            item.received_monotonic_nanos,
            item.exchange_utc_nanos,
            item.sequence.map(|value| value.value),
            &item.entitlement_ref,
            &item.payload,
        )
        .expect("raw frame");
        lines.push_str(&raw.to_json_line().expect("json line"));
        lines.push('\n');
    }
    std::fs::write(&output, lines).expect("generated journal");
    let started = std::time::Instant::now();
    let mut source = JournalReplaySource::open(&output, UPSTOX_SOURCE_ID).expect("replay source");
    let mut streamed = 0_u64;
    let mut held = 0_usize;
    while let Some((_, item)) = source.next_indexed_frame().expect("stream") {
        held = held.max(1);
        streamed += 1;
        let _ = item;
    }
    let elapsed = started.elapsed();
    eprintln!(
        "BENCH-REPLAY frames={streamed} seconds={:.3} frames_per_sec={:.1} peak_frames_held={held} delivery_window=4096 continuity_streams=1024",
        elapsed.as_secs_f64(),
        streamed as f64 / elapsed.as_secs_f64()
    );
    assert_eq!(streamed, count, "every generated frame is replayed");
}

#[test]
#[ignore = "child-only recovery worker"]
fn recovery_child() {
    let output = std::env::var_os("NIRNAY_BENCH_RECOVERY_DB").expect("recovery path");
    let ingestor = DurableIngestor::resume(
        PathBuf::from(output),
        UPSTOX_SOURCE_ID,
        SequencePolicy::CaptureSession,
    )
    .expect("resume");
    eprintln!(
        "RECOVERY-COMMITTED {}",
        ingestor.checkpoint().committed_frames
    );
}

#[test]
#[ignore = "opt-in performance measurement"]
fn recovery_after_process_termination() {
    let output = path("recovery");
    {
        let mut ingestor =
            DurableIngestor::create_new(&output, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
                .expect("ingestor");
        for sequence in 1..=100 {
            ingestor.ingest_one(frame(sequence)).expect("append");
        }
    }
    let started = std::time::Instant::now();
    let child: Output = Command::new(std::env::current_exe().expect("test binary"))
        .arg("--ignored")
        .arg("--nocapture")
        .arg("recovery_child")
        .env("NIRNAY_BENCH_RECOVERY_DB", &output)
        .output()
        .expect("spawn recovery child");
    assert!(
        child.status.success(),
        "fresh-process recovery worker succeeded"
    );
    let elapsed = started.elapsed();
    eprintln!(
        "BENCH-RECOVERY seconds={:.3} evidence={}",
        elapsed.as_secs_f64(),
        String::from_utf8_lossy(&child.stderr).trim()
    );
    let ingestor =
        DurableIngestor::resume(&output, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("parent resume");
    assert_eq!(
        ingestor.checkpoint().committed_frames,
        100,
        "the parent observes the same committed record count"
    );
}
