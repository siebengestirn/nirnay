#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.1-G deterministic cross-source replay. Synthetic session only.

use std::process::Command;

use market_event::{
    MarketByPriceSnapshot,
    MarketDataEvent,
    MbpLevel, //
    OriginClass,
    SequenceScope,
    capture_frame_id,
};
use market_kernel::{
    KernelSession, //
    SourceObservation,
};

fn observation(source: &str, bid: i64, ask: i64, sequence: u64) -> SourceObservation {
    let bytes = format!("{source}:{bid}:{ask}:{sequence}").into_bytes();
    let frame = capture_frame_id(&bytes).expect("frame");
    let book = MarketByPriceSnapshot {
        instrument_id: "nse.cash.replayidx".to_owned(),
        venue: "NSE".to_owned(),
        source_id: source.to_owned(),
        product: "full".to_owned(),
        depth_levels: 1,
        bids: vec![MbpLevel {
            price_paise: bid,
            quantity: 100,
            order_count: Some(1),
        }],
        asks: vec![MbpLevel {
            price_paise: ask,
            quantity: 100,
            order_count: Some(1),
        }],
        source_time_ns: None,
        last_trade_time_ns: None,
        received_utc_ns: 1_700_000_000_000_000_000 + sequence as i64,
    };
    SourceObservation {
        raw_frame_hash: frame.clone(),
        source_id: source.to_owned(),
        source_session_id: "replay-session".to_owned(),
        provider_instrument_id: "nse.cash.replayidx".to_owned(),
        canonical_payload: bytes,
        origin_class: OriginClass::SdkCallback,
        receive_utc_ns: 1_700_000_000_000_000_000 + sequence as i64,
        receive_monotonic_ns: sequence,
        source_time_ns: None,
        source_clock_domain: None,
        channel_id: Some("replay-channel".to_owned()),
        connection_epoch: 1,
        receive_sequence: sequence,
        sequence: Some(sequence),
        sequence_scope: SequenceScope::Connection,
        reference_revision: Some("replay-revision".to_owned()),
        instrument_id: Some("nse.cash.replayidx".to_owned()),
        evidence_token: None,
        payload: Some(MarketDataEvent::MarketByPriceSnapshot(book)),
    }
}

fn session() -> KernelSession {
    KernelSession {
        schema_version: market_kernel::KERNEL_SESSION_SCHEMA_VERSION,
        source_scope: "NSE".to_owned(),
        session_reference_revision: Some("replay-revision".to_owned()),
        observations: vec![
            observation("upstox", 9_995, 10_005, 1),
            observation("fyers.data_socket", 9_994, 10_006, 1),
        ],
    }
}

#[test]
fn two_source_replay_is_byte_identical_in_fresh_processes() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let input = directory.path().join("session.json");
    std::fs::write(&input, serde_json::to_vec(&session())?)?;
    let binary = env!("CARGO_BIN_EXE_session_replay");
    let mut reports = Vec::new();
    for name in ["one.json", "two.json"] {
        let output = directory.path().join(name);
        let status = Command::new(binary)
            .args([
                "--input",
                input.to_string_lossy().as_ref(),
                "--output",
                output.to_string_lossy().as_ref(),
            ])
            .output()?;
        let detail = String::from_utf8_lossy(&status.stderr);
        assert!(status.status.success(), "replay succeeded: {detail}");
        reports.push(std::fs::read(output)?);
    }
    assert_eq!(
        reports[0], reports[1],
        "fresh-process reports are byte-identical"
    );
    let report: serde_json::Value = serde_json::from_slice(&reports[0])?;
    assert_eq!(
        report["sources"].as_array().map(Vec::len),
        Some(2),
        "two sources are reported"
    );
    assert_eq!(
        report["books"].as_array().map(Vec::len),
        Some(2),
        "two separate books are built"
    );
    assert_eq!(
        report["comparison"]["matched"], false,
        "different quotes are an explicit mismatch"
    );
    Ok(())
}
