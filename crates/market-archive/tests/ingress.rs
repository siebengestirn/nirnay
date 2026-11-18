#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Shared ingress port tests: Upstox raw frames through the durable journal.

use market_archive::{
    DurableIngestor, //
    IngestOutcome,
    IngressError,
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
        provider_sequence: Some(sequence),
        payload: payload.to_vec(),
    }
}

#[test]
fn upstox_raw_frames_keep_native_payload_and_capture_metadata() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("upstox.jsonl");
    let first: Vec<u8> = vec![0x00, 0x01, 0x02, 0xff];
    let second: Vec<u8> = vec![0x7f, 0x80, 0x00];
    let mut source = UpstoxV3Adapter::new(vec![frame(1, &first), frame(2, &second)]);
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
    let summary = ingestor.run(&mut source, 10).expect("run");
    assert_eq!(summary.committed, 2, "both native frames are committed");
    assert_eq!(
        ingestor.checkpoint().last_capture_sequence,
        Some(2),
        "the checkpoint tracks the last provider sequence"
    );

    let frames = scan_journal(&path).expect("scan").frames;
    assert_eq!(frames.len(), 2, "the journal has exactly two records");
    assert!(
        frames.iter().all(|item| item.source == UPSTOX_SOURCE_ID),
        "every record carries the Upstox source identity"
    );
    assert_eq!(
        frames[0].payload_base64,
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &first),
        "original bytes survive without reinterpretation"
    );
    assert_eq!(
        frames[0].capture_sequence,
        Some(1),
        "capture sequence is preserved"
    );
    assert_eq!(
        frames[0].entitlement_ref, "entitlement:operator:upstox",
        "entitlement reference is preserved without secrets"
    );
}

#[test]
fn duplicate_delivery_is_skipped_without_a_second_journal_record() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("duplicate.jsonl");
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
    let outcome = ingestor.ingest_one(frame(1, b"same-bytes")).expect("first");
    assert!(
        matches!(
            outcome,
            IngestOutcome::Committed {
                capture_sequence: 1
            }
        ),
        "the first delivery is committed"
    );
    let duplicate = ingestor.ingest_one(frame(1, b"same-bytes")).expect("retry");
    assert_eq!(
        duplicate,
        IngestOutcome::DuplicateSkipped,
        "an identical retry is skipped"
    );
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        1,
        "the duplicate adds no journal record"
    );
    assert_eq!(
        ingestor.checkpoint().duplicates_skipped,
        1,
        "the duplicate is accounted for"
    );
}

#[test]
fn malformed_frames_are_rejected_without_advancing_the_checkpoint() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("malformed.jsonl");
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
    assert_eq!(
        ingestor.ingest_one(frame(1, b"")).expect("empty payload"),
        IngestOutcome::MalformedRejected,
        "an empty payload is rejected"
    );
    let oversized = vec![0_u8; market_archive::MAX_PAYLOAD_BYTES + 1];
    assert_eq!(
        ingestor
            .ingest_one(frame(2, &oversized))
            .expect("oversized"),
        IngestOutcome::MalformedRejected,
        "an oversized payload is rejected"
    );
    assert_eq!(
        ingestor.checkpoint().committed_frames,
        0,
        "no malformed frame advanced the checkpoint"
    );
    assert_eq!(
        ingestor.checkpoint().malformed_rejected,
        2,
        "both rejections are accounted for"
    );
    assert!(
        scan_journal(&path).expect("scan").frames.is_empty(),
        "the journal has no records"
    );
}

#[test]
fn sequence_gap_is_recorded_and_every_received_frame_is_preserved() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("gap.jsonl");
    let mut source = UpstoxV3Adapter::new(vec![
        frame(1, b"first"),
        frame(4, b"after-gap"),
        frame(2, b"late"),
    ]);
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
    let summary = ingestor.run(&mut source, 10).expect("run");
    assert_eq!(summary.committed, 3, "all received frames are preserved");
    assert_eq!(
        ingestor.checkpoint().gaps_observed,
        1,
        "the missing sequence is reported"
    );
    assert_eq!(
        ingestor.checkpoint().out_of_order,
        1,
        "the late frame is reported without being dropped"
    );
}

#[test]
fn disconnect_stops_the_run_at_the_last_durable_frame() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("disconnect.jsonl");
    let mut source = UpstoxV3Adapter::new(vec![frame(1, b"before")]);
    source.disconnect();
    source.push(frame(2, b"after-disconnect"));
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
    let summary = ingestor.run(&mut source, 10).expect("run");
    assert!(summary.disconnected, "the disconnect is observed");
    assert_eq!(
        summary.committed, 1,
        "no frame after the disconnect is committed"
    );
    assert_eq!(
        ingestor.checkpoint().last_capture_sequence,
        Some(1),
        "the checkpoint stays at the last durable frame"
    );
}

#[test]
fn restart_rebuilds_checkpoint_and_consumer_can_resume() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("resume.jsonl");
    {
        let mut source = UpstoxV3Adapter::new(vec![frame(1, b"one"), frame(2, b"two")]);
        let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
        ingestor.run(&mut source, 10).expect("first run");
    }
    let resumed = DurableIngestor::resume(&path, UPSTOX_SOURCE_ID).expect("resume");
    let checkpoint = resumed.checkpoint().clone();
    assert_eq!(checkpoint.committed_frames, 2, "both frames are recovered");
    assert_eq!(
        checkpoint.last_capture_sequence,
        Some(2),
        "the last sequence is recovered"
    );
    assert!(
        checkpoint.last_payload_sha256.is_some(),
        "the last payload hash is recovered for consumer fencing"
    );

    let mut resumed = resumed;
    let mut source = UpstoxV3Adapter::new(vec![frame(3, b"three")]);
    let summary = resumed.run(&mut source, 10).expect("resumed run");
    assert_eq!(summary.committed, 1, "a new frame commits after restart");
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        3,
        "no committed frame was replayed into a duplicate"
    );
}

#[test]
fn foreign_source_frames_are_rejected_before_any_write() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("foreign.jsonl");
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
    let mut foreign = frame(1, b"fyers-payload");
    foreign.source_id = "fyers.data_socket".to_owned();
    assert!(
        matches!(
            ingestor.ingest_one(foreign),
            Err(IngressError::SourceMismatch { .. })
        ),
        "a foreign-source envelope is refused"
    );
    assert!(
        scan_journal(&path).expect("scan").frames.is_empty(),
        "the rejected frame left no journal record"
    );
}

#[test]
fn replaying_a_committed_frame_after_restart_is_skipped() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("replay.jsonl");
    let original = frame(7, b"committed-once");
    {
        let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
        ingestor.ingest_one(original.clone()).expect("first commit");
    }
    let mut resumed = DurableIngestor::resume(&path, UPSTOX_SOURCE_ID).expect("resume");
    assert_eq!(
        resumed.ingest_one(original).expect("replayed delivery"),
        IngestOutcome::DuplicateSkipped,
        "an exact replay after restart is recognised"
    );
    assert_eq!(
        scan_journal(&path).expect("scan").frames.len(),
        1,
        "the replay adds no second journal record"
    );
    assert_eq!(
        resumed.checkpoint().committed_frames,
        1,
        "the checkpoint does not advance on replay"
    );
}

#[test]
fn metadata_failure_does_not_advance_continuity() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("metadata.jsonl");
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
    let mut invalid = frame(5, b"valid-payload");
    invalid.provider_instrument_id = "x".repeat(1024 * 1024);
    assert_eq!(
        ingestor.ingest_one(invalid).expect("invalid metadata"),
        IngestOutcome::MalformedRejected,
        "an unrepresentable metadata field is rejected"
    );
    assert_eq!(
        ingestor.checkpoint().last_capture_sequence,
        None,
        "the failed frame left no phantom sequence"
    );
    assert_eq!(
        ingestor.checkpoint().gaps_observed,
        0,
        "the failed frame recorded no phantom gap"
    );
    assert!(
        scan_journal(&path).expect("scan").frames.is_empty(),
        "the rejected frame wrote nothing"
    );
}

#[test]
fn continuity_stream_map_is_bounded_by_source_supplied_ids() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("streams.jsonl");
    let mut ingestor = DurableIngestor::create_new(&path, UPSTOX_SOURCE_ID).expect("ingestor");
    for index in 0..1200_u64 {
        let mut item = frame(index, format!("payload-{index}").as_bytes());
        item.provider_instrument_id = format!("provider.instrument.{index}");
        ingestor.ingest_one(item).expect("commit stream frame");
    }
    assert!(
        ingestor.tracked_streams() <= 1024,
        "source-supplied instrument ids cannot grow the continuity map without bound"
    );
    assert_eq!(
        ingestor.checkpoint().committed_frames,
        1200,
        "every valid frame is still committed"
    );
}

// ---------------------------------------------------------------------------
// FYERS: second independent source implementation over the same ingress port.
// ---------------------------------------------------------------------------

use market_archive::{
    FYERS_SOURCE_ID, //
    FyersCallback,
    FyersCallbackAdapter,
};

fn fyers_callback(sequence: u64, json: &[u8]) -> FyersCallback {
    FyersCallback {
        provider_instrument_id: "NSE_EQ|INE002A01018".to_owned(),
        entitlement_ref: "entitlement:operator:fyers".to_owned(),
        received_utc_nanos: 1_760_000_000_000_000_000 + i64::try_from(sequence).expect("sequence"),
        received_monotonic_nanos: sequence,
        exchange_utc_nanos: Some(1_760_000_000_000_000_000),
        local_sequence: Some(sequence),
        callback_json: json.to_vec(),
    }
}

#[test]
fn fyers_callbacks_keep_exact_json_bytes_and_local_sequence() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("fyers.jsonl");
    let first = br#"{"type":"sf","symbol":"SBIN-EQ","ltp":812.5}"#;
    let second = br#"{"type":"dp","symbol":"SBIN-EQ","bid_price1":812.0}"#;
    let mut source =
        FyersCallbackAdapter::new(vec![fyers_callback(1, first), fyers_callback(2, second)]);
    let mut ingestor = DurableIngestor::create_new(&path, FYERS_SOURCE_ID).expect("ingestor");
    let summary = ingestor.run(&mut source, 10).expect("run");
    assert_eq!(summary.committed, 2, "both callbacks are committed");

    let frames = scan_journal(&path).expect("scan").frames;
    assert_eq!(frames.len(), 2, "the journal has exactly two records");
    assert!(
        frames.iter().all(|item| item.source == FYERS_SOURCE_ID),
        "records carry the FYERS source identity"
    );
    assert_eq!(
        frames[0].payload_base64,
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, first),
        "the original callback bytes survive unchanged"
    );
    assert_eq!(
        frames[0].capture_sequence,
        Some(1),
        "the recorder's local sequence is preserved as metadata"
    );
}

#[test]
fn fyers_gap_duplicate_disconnect_and_restart_are_accounted() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("fyers-recovery.jsonl");
    {
        let mut source = FyersCallbackAdapter::new(vec![
            fyers_callback(1, b"{\"type\":\"sf\",\"ltp\":1}"),
            fyers_callback(4, b"{\"type\":\"sf\",\"ltp\":4}"),
        ]);
        source.push(fyers_callback(4, b"{\"type\":\"sf\",\"ltp\":4}"));
        source.disconnect();
        source.push(fyers_callback(5, b"{\"type\":\"sf\",\"ltp\":5}"));
        let mut ingestor = DurableIngestor::create_new(&path, FYERS_SOURCE_ID).expect("ingestor");
        let summary = ingestor.run(&mut source, 10).expect("run");
        assert!(summary.disconnected, "disconnect stops the FYERS run");
        assert_eq!(
            summary.committed, 2,
            "no frame after disconnect is committed"
        );
        assert_eq!(summary.duplicates, 1, "the repeated callback is skipped");
        assert_eq!(
            ingestor.checkpoint().gaps_observed,
            1,
            "the missing local sequence is reported"
        );
    }
    let resumed = DurableIngestor::resume(&path, FYERS_SOURCE_ID).expect("resume");
    assert_eq!(
        resumed.checkpoint().committed_frames,
        2,
        "the FYERS checkpoint survives restart"
    );
    assert_eq!(
        resumed.checkpoint().last_capture_sequence,
        Some(4),
        "the last local sequence survives restart"
    );
}

#[test]
fn independent_sources_share_the_port_without_cross_contamination() {
    let directory = tempfile::tempdir().expect("tempdir");
    let upstox_path = directory.path().join("upstox.jsonl");
    let fyers_path = directory.path().join("fyers.jsonl");
    let mut upstox = DurableIngestor::create_new(&upstox_path, UPSTOX_SOURCE_ID).expect("upstox");
    let mut fyers = DurableIngestor::create_new(&fyers_path, FYERS_SOURCE_ID).expect("fyers");

    upstox
        .ingest_one(frame(1, b"upstox-native"))
        .expect("upstox commit");
    fyers
        .ingest_one(fyers_callback(1, b"{\"type\":\"sf\"}").into_source_frame())
        .expect("fyers commit");

    let mut foreign = frame(2, b"fyers-sent-to-upstox");
    foreign.source_id = FYERS_SOURCE_ID.to_owned();
    assert!(
        matches!(
            upstox.ingest_one(foreign),
            Err(IngressError::SourceMismatch { .. })
        ),
        "a source cannot write through another source's ingestor"
    );
    assert_eq!(
        scan_journal(&upstox_path).expect("scan").frames.len(),
        1,
        "the rejected frame left the first journal untouched"
    );
    assert_eq!(
        scan_journal(&fyers_path).expect("scan").frames.len(),
        1,
        "the second source has one independent journal"
    );
}
