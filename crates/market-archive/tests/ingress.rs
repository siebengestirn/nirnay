#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Shared ingress contract tests, including delivery identity, sequence scope,
//! continuity state and bounded streaming replay.

use std::{
    path::PathBuf,
    process::{
        Command, //
        Output,
    },
};

use base64::Engine as _;
use market_archive::{
    DurableIngestor, //
    FYERS_SOURCE_ID,
    FyersCallback,
    FyersCallbackAdapter,
    IngestOutcome,
    IngressError,
    IngressSource,
    JournalReplaySource,
    Sequence,
    SequencePolicy,
    SourceFrame,
    UPSTOX_SOURCE_ID,
    UpstoxV3Adapter,
    scan_journal,
};

fn frame(sequence: u64, payload: &[u8]) -> SourceFrame {
    SourceFrame {
        source_id: UPSTOX_SOURCE_ID.to_owned(),
        provider_instrument_id: "NSE_EQ|INE848E01016".to_owned(),
        entitlement_ref: "entitlement:operator:upstox".to_owned(),
        received_utc_nanos: 1_760_000_000_000_000_000
            + i64::try_from(sequence).expect("small sequence"),
        received_monotonic_nanos: sequence,
        exchange_utc_nanos: Some(1_760_000_000_000_000_000),
        sequence: Some(Sequence::capture_session(sequence)),
        payload: payload.to_vec(),
    }
}

fn provider_frame(sequence: u64, instrument: &str, payload: &[u8]) -> SourceFrame {
    let mut item = frame(sequence, payload);
    item.provider_instrument_id = instrument.to_owned();
    item.sequence = Some(Sequence::provider(sequence));
    item
}

fn fyers_callback(sequence: u64, symbol: &str, json: &[u8]) -> FyersCallback {
    FyersCallback {
        provider_instrument_id: symbol.to_owned(),
        entitlement_ref: "entitlement:operator:fyers".to_owned(),
        received_utc_nanos: 1_760_000_000_000_000_000
            + i64::try_from(sequence).expect("small sequence"),
        received_monotonic_nanos: sequence,
        exchange_utc_nanos: Some(1_760_000_000_000_000_000),
        local_sequence: Some(sequence),
        callback_json: json.to_vec(),
    }
}

fn temp_path(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "nirnay-ingress-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("main")
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp root");
    path.join("frames.jsonl")
}

#[test]
fn upstox_raw_frames_keep_native_payload_and_capture_metadata() {
    let path = temp_path("native");
    let first: Vec<u8> = vec![0x00, 0x01, 0x02, 0xff];
    let mut source = UpstoxV3Adapter::new(vec![frame(1, &first), frame(2, b"second")]);
    let mut ingestor =
        DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("ingestor");
    let summary = ingestor.run(&mut source, 10).expect("run");
    assert_eq!(summary.committed, 2, "both frames are committed");
    let frames = scan_journal(&path).expect("scan").frames;
    assert_eq!(frames.len(), 2, "contract assertion");
    assert!(
        frames.iter().all(|item| item.source == UPSTOX_SOURCE_ID),
        "contract assertion"
    );
    assert_eq!(
        frames[0].payload_base64,
        base64::engine::general_purpose::STANDARD.encode(&first),
        "payload bytes are preserved exactly"
    );
    assert_eq!(frames[0].capture_sequence, Some(1), "contract assertion");
    assert_eq!(
        frames[0].entitlement_ref, "entitlement:operator:upstox",
        "contract assertion"
    );
}

#[test]
fn same_payload_with_different_capture_sequences_is_preserved() {
    let path = temp_path("same-payload");
    let mut ingestor =
        DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("ingestor");
    assert_eq!(
        ingestor
            .ingest_one(frame(1, b"identical-payload"))
            .expect("first"),
        IngestOutcome::Committed { sequence: Some(1) }
    );
    assert_eq!(
        ingestor
            .ingest_one(frame(2, b"identical-payload"))
            .expect("second"),
        IngestOutcome::Committed { sequence: Some(2) },
        "identical bytes from different deliveries are not a duplicate"
    );
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        2,
        "contract assertion"
    );
}

#[test]
fn same_delivery_and_payload_is_an_idempotent_duplicate() {
    let path = temp_path("same-delivery");
    let mut ingestor =
        DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("ingestor");
    ingestor.ingest_one(frame(3, b"payload")).expect("first");
    assert_eq!(
        ingestor.ingest_one(frame(3, b"payload")).expect("retry"),
        IngestOutcome::DuplicateSkipped
    );
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        1,
        "contract assertion"
    );
}

#[test]
fn same_delivery_with_different_payload_is_a_conflict() {
    let path = temp_path("conflict");
    let mut ingestor =
        DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("ingestor");
    ingestor.ingest_one(frame(4, b"original")).expect("first");
    assert_eq!(
        ingestor.ingest_one(frame(4, b"changed")).expect("conflict"),
        IngestOutcome::ConflictRejected
    );
    assert_eq!(
        ingestor.checkpoint().conflicts_rejected,
        1,
        "contract assertion"
    );
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        1,
        "contract assertion"
    );
}

#[test]
fn unknown_sequence_never_infers_duplicate_from_content() {
    let path = temp_path("unknown");
    let mut a = frame(0, b"same-bytes");
    let mut b = frame(0, b"same-bytes");
    a.sequence = None;
    b.sequence = None;
    b.received_monotonic_nanos += 1;
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::None)
        .expect("ingestor");
    ingestor.ingest_one(a).expect("first");
    assert_eq!(
        ingestor.ingest_one(b).expect("second"),
        IngestOutcome::Committed { sequence: None },
        "unknown-identity frames with equal bytes are preserved"
    );
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        2,
        "contract assertion"
    );
}

#[test]
fn fyers_session_sequence_across_symbols_has_no_false_per_symbol_gap() {
    let path = temp_path("fyers-session");
    let mut source = FyersCallbackAdapter::new(vec![
        fyers_callback(1, "SBIN-EQ", b"{\"type\":\"sf\"}"),
        fyers_callback(2, "TCS-EQ", b"{\"type\":\"sf\"}"),
        fyers_callback(3, "SBIN-EQ", b"{\"type\":\"dp\"}"),
    ]);
    let mut ingestor =
        DurableIngestor::create_new(&path, FYERS_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("ingestor");
    let summary = ingestor.run(&mut source, 10).expect("run");
    assert_eq!(summary.committed, 3, "all callbacks are preserved");
    assert_eq!(
        ingestor.checkpoint().gaps_observed,
        0,
        "a session-wide sequence cannot create per-symbol gaps"
    );
    assert_eq!(ingestor.checkpoint().out_of_order, 0, "contract assertion");
    assert_eq!(
        ingestor.checkpoint().last_capture_sequence,
        Some(3),
        "contract assertion"
    );
}

#[test]
fn provider_sequence_uses_per_instrument_continuity() {
    let path = temp_path("provider-sequence");
    let mut ingestor = DurableIngestor::create_new(
        &path,
        UPSTOX_SOURCE_ID,
        SequencePolicy::ProviderPerInstrument,
    )
    .expect("ingestor");
    ingestor
        .ingest_one(provider_frame(1, "A", b"a1"))
        .expect("A1");
    ingestor
        .ingest_one(provider_frame(2, "B", b"b1"))
        .expect("B1");
    ingestor
        .ingest_one(provider_frame(4, "A", b"a4"))
        .expect("A4");
    assert_eq!(
        ingestor.checkpoint().gaps_observed,
        1,
        "A's gap is reported"
    );
    assert_eq!(
        ingestor.policy(),
        SequencePolicy::ProviderPerInstrument,
        "provider sequence scope is explicit"
    );
}

#[test]
fn more_than_1024_instruments_have_no_silent_continuity_loss() {
    let path = temp_path("continuity-bound");
    let mut ingestor = DurableIngestor::create_new(
        &path,
        UPSTOX_SOURCE_ID,
        SequencePolicy::ProviderPerInstrument,
    )
    .expect("ingestor");
    for index in 0..1100_u64 {
        let instrument = format!("provider.instrument.{index}");
        ingestor
            .ingest_one(provider_frame(
                index,
                &instrument,
                format!("p{index}").as_bytes(),
            ))
            .expect("commit");
    }
    assert!(
        ingestor.tracked_streams() <= 1024,
        "the continuity map is bounded"
    );
    assert!(
        ingestor.checkpoint().unknown_continuity_streams > 0,
        "eviction is recorded as unknown continuity, never as a fresh clean stream"
    );
    assert_eq!(
        ingestor.checkpoint().committed_frames,
        1100,
        "contract assertion"
    );
}

#[test]
fn malformed_payload_and_foreign_source_fail_closed() {
    let path = temp_path("malformed");
    let mut ingestor =
        DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("ingestor");
    assert_eq!(
        ingestor.ingest_one(frame(1, b"")).expect("empty"),
        IngestOutcome::MalformedRejected
    );
    let oversized = vec![0_u8; market_archive::MAX_PAYLOAD_BYTES + 1];
    assert_eq!(
        ingestor
            .ingest_one(frame(2, &oversized))
            .expect("oversized"),
        IngestOutcome::MalformedRejected
    );
    let mut foreign = frame(3, b"foreign");
    foreign.source_id = FYERS_SOURCE_ID.to_owned();
    assert!(
        matches!(
            ingestor.ingest_one(foreign),
            Err(IngressError::SourceMismatch { .. })
        ),
        "a foreign source is refused before any write"
    );
    assert_eq!(
        ingestor.checkpoint().committed_frames,
        0,
        "contract assertion"
    );
    assert!(
        scan_journal(&path).expect("scan").frames.is_empty(),
        "contract assertion"
    );
}

#[test]
fn disconnect_stops_at_the_last_durable_frame() {
    let path = temp_path("disconnect");
    let mut source = UpstoxV3Adapter::new(vec![frame(1, b"before")]);
    source.disconnect();
    source.push(frame(2, b"after"));
    let mut ingestor =
        DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("ingestor");
    let summary = ingestor.run(&mut source, 10).expect("run");
    assert!(summary.disconnected, "contract assertion");
    assert_eq!(summary.committed, 1, "contract assertion");
    assert_eq!(
        ingestor.checkpoint().last_capture_sequence,
        Some(1),
        "contract assertion"
    );
}

#[test]
fn restart_and_replay_commit_nothing_twice() {
    let path = temp_path("restart");
    {
        let mut source = UpstoxV3Adapter::new(vec![frame(1, b"one"), frame(2, b"two")]);
        let mut ingestor =
            DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
                .expect("ingestor");
        ingestor.run(&mut source, 10).expect("capture");
    }
    let mut source = JournalReplaySource::open(&path, UPSTOX_SOURCE_ID).expect("replay source");
    let mut ingestor =
        DurableIngestor::resume(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("resume");
    let summary = ingestor.replay(&mut source).expect("replay");
    assert_eq!(summary.committed, 0, "contract assertion");
    assert_eq!(summary.duplicates, 2, "contract assertion");
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        2,
        "contract assertion"
    );
}

#[test]
fn replay_reproduces_frames_and_preserves_source_boundary() {
    let root = temp_path("replay-fidelity");
    let captured = root.with_file_name("captured.jsonl");
    let replayed = root.with_file_name("replayed.jsonl");
    {
        let mut source =
            FyersCallbackAdapter::new(vec![fyers_callback(9, "SBIN-EQ", b"{\"type\":\"sf\"}")]);
        let mut ingestor =
            DurableIngestor::create_new(&captured, FYERS_SOURCE_ID, SequencePolicy::CaptureSession)
                .expect("ingestor");
        ingestor.run(&mut source, 10).expect("capture");
    }
    let mut source = JournalReplaySource::open(&captured, FYERS_SOURCE_ID).expect("replay source");
    assert_eq!(source.source_id(), FYERS_SOURCE_ID, "contract assertion");
    let mut ingestor =
        DurableIngestor::create_new(&replayed, FYERS_SOURCE_ID, SequencePolicy::CaptureSession)
            .expect("replay ingestor");
    ingestor.run(&mut source, 10).expect("replay");
    assert_eq!(
        scan_journal(&replayed).expect("scan").frames,
        scan_journal(&captured).expect("scan").frames
    );

    let mut wrong = DurableIngestor::create_new(
        root.with_file_name("wrong.jsonl"),
        UPSTOX_SOURCE_ID,
        SequencePolicy::CaptureSession,
    )
    .expect("upstox ingestor");
    let mut replay = JournalReplaySource::open(&captured, FYERS_SOURCE_ID).expect("replay source");
    assert!(
        matches!(
            wrong.replay(&mut replay),
            Err(IngressError::SourceMismatch { .. })
        ),
        "a replay cannot cross into another source's ingestor"
    );
}

#[test]
fn torn_or_corrupt_journal_fails_closed() {
    let path = temp_path("corruption");
    {
        let mut source = UpstoxV3Adapter::new(vec![frame(1, b"valid")]);
        let mut ingestor =
            DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
                .expect("ingestor");
        ingestor.run(&mut source, 10).expect("capture");
    }
    let mut bytes = std::fs::read(&path).expect("journal");
    bytes.extend_from_slice(b"{\"partial\":");
    std::fs::write(&path, bytes).expect("append torn tail");
    assert!(
        DurableIngestor::resume(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession,).is_err(),
        "resume refuses a torn journal"
    );
    assert!(
        JournalReplaySource::open(&path, UPSTOX_SOURCE_ID).is_ok(),
        "open succeeds; the reader reports the tear on iteration"
    );
    let mut replay = JournalReplaySource::open(&path, UPSTOX_SOURCE_ID).expect("source");
    let mut failed = false;
    for _ in 0..4 {
        if replay.next_indexed_frame().is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "replay fails on the corrupt record");
}

#[test]
#[ignore = "child-only fresh-process recovery worker"]
fn child_recovery_worker() {
    let path = std::env::var_os("NIRNAY_INGRESS_RECOVERY_DB").expect("recovery path");
    let ingestor = DurableIngestor::resume(
        PathBuf::from(path),
        UPSTOX_SOURCE_ID,
        SequencePolicy::CaptureSession,
    )
    .expect("resume in fresh process");
    assert_eq!(
        ingestor.checkpoint().committed_frames,
        2,
        "contract assertion"
    );
    assert_eq!(
        ingestor.checkpoint().last_capture_sequence,
        Some(2),
        "contract assertion"
    );
}

#[test]
fn fresh_process_recovery_matches_committed_records_and_checkpoint() {
    let path = temp_path("fresh-process");
    {
        let mut source = UpstoxV3Adapter::new(vec![frame(1, b"one"), frame(2, b"two")]);
        let mut ingestor =
            DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID, SequencePolicy::CaptureSession)
                .expect("ingestor");
        ingestor.run(&mut source, 10).expect("capture");
    }
    let child: Output = Command::new(std::env::current_exe().expect("test binary"))
        .arg("--ignored")
        .arg("child_recovery_worker")
        .env("NIRNAY_INGRESS_RECOVERY_DB", &path)
        .output()
        .expect("spawn recovery worker");
    assert!(child.status.success(), "fresh-process recovery succeeded");
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        2,
        "contract assertion"
    );
}

#[test]
fn large_journal_replay_uses_bounded_memory() {
    let path = temp_path("large-replay");
    let count = 20_000_u64;
    {
        let mut output = String::new();
        for index in 0..count {
            let item = frame(index, format!("payload-{index}").as_bytes());
            let raw = market_archive::RawFrame::new(
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
            output.push_str(&raw.to_json_line().expect("journal line"));
            output.push('\n');
        }
        std::fs::write(&path, output).expect("generated journal");
    }
    let mut source = JournalReplaySource::open(&path, UPSTOX_SOURCE_ID).expect("replay source");
    let mut seen = 0_u64;
    while let Some((ordinal, _)) = source.next_indexed_frame().expect("stream") {
        assert_eq!(ordinal, seen, "ordinals are assigned in journal order");
        seen += 1;
    }
    assert_eq!(seen, count, "all frames stream through a bounded reader");
}

#[test]
#[ignore = "benchmark only: generate and replay a large journal"]
fn replay_benchmark_large_generated_journal() {
    let path = temp_path("benchmark");
    let count = 200_000_u64;
    {
        let mut output = String::new();
        for index in 0..count {
            let item = frame(index, format!("payload-{index}").as_bytes());
            let raw = market_archive::RawFrame::new(
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
            output.push_str(&raw.to_json_line().expect("journal line"));
            output.push('\n');
        }
        std::fs::write(&path, output).expect("generated benchmark journal");
    }
    let started = std::time::Instant::now();
    let mut source = JournalReplaySource::open(&path, UPSTOX_SOURCE_ID).expect("replay source");
    let mut streamed = 0_u64;
    while source.next_indexed_frame().expect("stream").is_some() {
        streamed += 1;
    }
    let elapsed = started.elapsed();
    eprintln!(
        "INGRESS-REPLAY-BENCH frames={streamed} elapsed_ms={} frames_per_sec={:.1} memory_frames_held=1",
        elapsed.as_millis(),
        streamed as f64 / elapsed.as_secs_f64()
    );
    assert_eq!(streamed, count, "contract assertion");
}
