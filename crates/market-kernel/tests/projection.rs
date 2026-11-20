#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.2 kernel projection tests. Uses a synthetic verified session; no real
//! provider capture is asserted.

use std::process::{
    Command, //
    Output,
};

use market_event::{
    ContentId, //
    OriginClass,
    SequenceScope,
    StreamState,
};
use market_kernel::{
    KernelError, //
    KernelSession,
    SourceObservation,
    project_observation,
    project_session,
};

fn hash(text: &str) -> ContentId {
    // Reuse the kernel identity contract rather than a second hash path.
    market_event::capture_frame_id(text.as_bytes()).expect("content id")
}

fn observation(frame: &str, instrument: &str, payload: &str, sequence: u64) -> SourceObservation {
    SourceObservation {
        raw_frame_hash: hash(frame),
        source_id: "upstox".to_owned(),
        source_session_id: "synthetic-session".to_owned(),
        provider_instrument_id: instrument.to_owned(),
        canonical_payload: payload.as_bytes().to_vec(),
        origin_class: OriginClass::SdkCallback,
        receive_utc_ns: 1_700_000_000_000_000_000 + sequence as i64,
        receive_monotonic_ns: sequence,
        source_time_ns: Some(1_699_999_999_000_000_000 + sequence as i64),
        source_clock_domain: Some("provider_feed_clock".to_owned()),
        channel_id: Some("test-channel".to_owned()),
        connection_epoch: 1,
        receive_sequence: sequence,
        sequence: Some(sequence),
        sequence_scope: SequenceScope::Connection,
        reference_revision: Some("revision-a".to_owned()),
        instrument_id: Some(format!("IN:{instrument}")),
        evidence_token: None,
        payload: None,
    }
}

fn session(observations: Vec<SourceObservation>) -> KernelSession {
    KernelSession {
        schema_version: market_kernel::KERNEL_SESSION_SCHEMA_VERSION,
        source_scope: "NSE".to_owned(),
        session_reference_revision: Some("revision-a".to_owned()),
        observations,
    }
}

#[test]
fn one_multi_instrument_frame_produces_distinct_observations() {
    let frame = "one-upstox-frame";
    let s = session(vec![
        observation(frame, "NSE_EQ|A", "payload-a", 1),
        observation(frame, "NSE_EQ|B", "payload-b", 2),
    ]);
    let events = project_session(&s).expect("project");
    assert_eq!(
        events.len(),
        2,
        "one observation per instrument-local payload"
    );
    assert_eq!(
        events[0].observation_id,
        observation_id_for("NSE_EQ|A", "payload-a"),
        "observation identity is derived from instrument-local content"
    );
    assert_ne!(
        events[0].observation_id, events[1].observation_id,
        "instruments differ"
    );
    assert_eq!(
        events[0].raw_frame_hash, events[1].raw_frame_hash,
        "one raw frame commitment"
    );
}

fn observation_id_for(instrument: &str, payload: &str) -> ContentId {
    market_event::observation_id("upstox", "NSE", instrument, payload.as_bytes()).expect("id")
}

#[test]
fn unresolved_instrument_is_not_actionable() {
    let mut obs = observation("frame", "NSE_EQ|X", "payload", 1);
    obs.instrument_id = None;
    obs.reference_revision = None;
    let s = session(vec![obs]);
    let events = project_session(&s).expect("project");
    assert_eq!(
        events[0].instrument_id, None,
        "unknown instrument stays unresolved"
    );
    assert!(!events[0].is_actionable(), "no invented identity or action");
}

#[test]
fn gap_degrades_to_non_actionable() {
    let s = session(vec![
        observation("f1", "NSE_EQ|A", "p1", 1),
        observation("f2", "NSE_EQ|A", "p2", 4),
    ]);
    let events = project_session(&s).expect("project");
    assert_eq!(
        events[0].quality,
        StreamState::Clean,
        "first observation is clean"
    );
    assert_eq!(
        events[1].quality,
        StreamState::Gap,
        "sequence gap is explicit"
    );
    assert!(
        !events[1].is_actionable(),
        "a gapped event cannot drive exposure"
    );
}

#[test]
fn reference_revision_is_bound() {
    let s = session(vec![observation("f", "NSE_EQ|A", "p", 1)]);
    let events = project_session(&s).expect("project");
    assert_eq!(
        events[0].reference_revision.as_deref(),
        Some("revision-a"),
        "the exact revision is bound"
    );
}

#[test]
fn duplicate_observation_in_session_is_refused() {
    let s = session(vec![
        observation("f", "NSE_EQ|A", "same", 1),
        observation("f", "NSE_EQ|A", "same", 2),
    ]);
    assert_eq!(
        project_session(&s),
        Err(KernelError::DuplicateObservation),
        "a replayed delivery is not admitted twice as new"
    );
}

#[test]
fn invalid_observation_is_refused_before_publication() {
    let mut obs = observation("f", "NSE_EQ|A", "p", 1);
    obs.canonical_payload = Vec::new();
    assert_eq!(
        project_observation(&session(vec![obs.clone()]), &obs),
        Err(KernelError::EmptyFrame),
        "an empty payload is refused before identity derivation"
    );
}

#[test]
fn fresh_process_replay_is_identical() {
    let directory = tempfile::tempdir().expect("tempdir");
    let input = directory.path().join("session.json");
    let s = session(vec![
        observation("f1", "NSE_EQ|A", "p1", 1),
        observation("f2", "NSE_EQ|B", "p2", 2),
    ]);
    std::fs::write(&input, serde_json::to_vec(&s).expect("serialize session"))
        .expect("write session");

    let binary = env!("CARGO_BIN_EXE_kernel_session");
    let first = directory.path().join("first.jsonl");
    let second = directory.path().join("second.jsonl");
    let run = |out: &std::path::Path| -> Output {
        Command::new(binary)
            .args([
                "--input",
                input.to_str().expect("path"),
                "--output",
                out.to_str().expect("path"),
            ])
            .output()
            .expect("run kernel projector")
    };
    let one = run(&first);
    assert!(one.status.success(), "first projection succeeded");
    let two = run(&second);
    assert!(two.status.success(), "second projection succeeded");
    assert_eq!(
        std::fs::read(&first).expect("first output"),
        std::fs::read(&second).expect("second output"),
        "fresh-process replay is byte-identical"
    );
    assert_eq!(
        String::from_utf8_lossy(&one.stderr),
        String::from_utf8_lossy(&two.stderr),
        "digest is identical across processes"
    );
    assert!(
        String::from_utf8_lossy(&one.stderr).contains("sha256="),
        "the binary reports a non-empty digest on stderr"
    );
}

/// A session-level reference revision is used when an observation omits one.
#[test]
fn session_revision_fallback_is_live() {
    let mut obs = observation("f", "NSE_EQ|A", "p", 1);
    obs.reference_revision = None;
    let s = session(vec![obs]);
    let events = project_session(&s).expect("project with session revision");
    assert_eq!(
        events[0].reference_revision.as_deref(),
        Some("revision-a"),
        "the session revision resolves the observation"
    );
    assert!(
        events[0].instrument_id.is_some(),
        "instrument stays resolved"
    );
}

/// A resolved observation with no revision anywhere is refused.
#[test]
fn missing_reference_revision_is_refused() {
    let mut obs = observation("f", "NSE_EQ|A", "p", 1);
    obs.reference_revision = None;
    let mut s = session(vec![obs]);
    s.session_reference_revision = None;
    assert_eq!(
        project_session(&s),
        Err(market_kernel::KernelError::MissingReferenceRevision),
        "no revision means no resolution"
    );
}

fn scoped(
    frame: &str,
    instrument: &str,
    sequence: Option<u64>,
    scope: SequenceScope,
    channel: Option<&str>,
    epoch: u64,
) -> SourceObservation {
    let mut observation = observation(frame, instrument, frame, sequence.unwrap_or(1));
    observation.sequence = sequence;
    observation.sequence_scope = scope;
    observation.channel_id = channel.map(str::to_owned);
    observation.connection_epoch = epoch;
    observation
}

fn session_of(observations: Vec<SourceObservation>) -> KernelSession {
    KernelSession {
        schema_version: market_kernel::KERNEL_SESSION_SCHEMA_VERSION,
        source_scope: "NSE".to_owned(),
        session_reference_revision: Some("revision-a".to_owned()),
        observations,
    }
}

/// No declared source sequence means no exchange-level completeness claim.
#[test]
fn unscoped_sequence_is_unverified() -> Result<(), Box<dyn std::error::Error>> {
    let events = project_session(&session_of(vec![scoped(
        "f1",
        "NSE_EQ|A",
        None,
        SequenceScope::None,
        Some("ch"),
        1,
    )]))?;
    assert_eq!(
        events[0].quality,
        StreamState::Unverified,
        "no scope yields Unverified"
    );
    assert_eq!(
        events[0].instrument_id, None,
        "an unverified event is not actionable"
    );
    assert!(
        !events[0].is_actionable(),
        "unverified events cannot drive exposure"
    );
    Ok(())
}

/// A local receive counter is ordered locally; it is not a source sequence.
#[test]
fn local_receive_order_is_not_a_source_sequence() -> Result<(), Box<dyn std::error::Error>> {
    let mut first = scoped("f1", "NSE_EQ|A", None, SequenceScope::None, Some("ch"), 1);
    let mut second = scoped("f2", "NSE_EQ|A", None, SequenceScope::None, Some("ch"), 1);
    first.receive_sequence = 1;
    second.receive_sequence = 2;
    let events = project_session(&session_of(vec![first, second]))?;
    assert!(
        events
            .iter()
            .all(|event| event.quality == StreamState::Unverified),
        "increasing local receive counters do not establish continuity"
    );
    Ok(())
}

/// One raw channel carrying several instruments shares one sequence tracker.
#[test]
fn channel_sequence_is_shared_across_instruments() -> Result<(), Box<dyn std::error::Error>> {
    let events = project_session(&session_of(vec![
        scoped(
            "f1",
            "NSE_EQ|A",
            Some(1),
            SequenceScope::Channel,
            Some("ch"),
            1,
        ),
        scoped(
            "f2",
            "NSE_EQ|B",
            Some(3),
            SequenceScope::Channel,
            Some("ch"),
            1,
        ),
    ]))?;
    assert_eq!(
        events[0].quality,
        StreamState::Clean,
        "first channel observation is clean"
    );
    assert_eq!(
        events[1].quality,
        StreamState::Gap,
        "a channel gap affects later instruments"
    );
    Ok(())
}

/// Provider-per-instrument sequences must not be conflated.
#[test]
fn provider_instrument_sequences_are_independent() -> Result<(), Box<dyn std::error::Error>> {
    let events = project_session(&session_of(vec![
        scoped(
            "f1",
            "NSE_EQ|A",
            Some(1),
            SequenceScope::ProviderInstrument,
            None,
            1,
        ),
        scoped(
            "f2",
            "NSE_EQ|B",
            Some(1),
            SequenceScope::ProviderInstrument,
            None,
            1,
        ),
    ]))?;
    assert!(
        events
            .iter()
            .all(|event| event.quality == StreamState::Clean),
        "two instruments at sequence 1 are not a gap under per-instrument scope"
    );
    Ok(())
}

/// Connection epochs and distinct channels are separate continuity scopes.
#[test]
fn epochs_and_channels_separate_trackers() -> Result<(), Box<dyn std::error::Error>> {
    let events = project_session(&session_of(vec![
        scoped(
            "f1",
            "NSE_EQ|A",
            Some(5),
            SequenceScope::Channel,
            Some("ch-a"),
            1,
        ),
        scoped(
            "f2",
            "NSE_EQ|A",
            Some(1),
            SequenceScope::Channel,
            Some("ch-a"),
            2,
        ),
        scoped(
            "f3",
            "NSE_EQ|A",
            Some(1),
            SequenceScope::Channel,
            Some("ch-b"),
            2,
        ),
    ]))?;
    assert!(
        events
            .iter()
            .all(|event| event.quality == StreamState::Clean),
        "a new epoch or a different channel starts a new tracker"
    );
    Ok(())
}

/// A reconnect must never clear an existing gap without recovery evidence.
#[test]
fn reconnect_after_gap_stays_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
    let events = project_session(&session_of(vec![
        scoped(
            "f1",
            "NSE_EQ|A",
            Some(1),
            SequenceScope::Channel,
            Some("ch"),
            1,
        ),
        scoped(
            "f2",
            "NSE_EQ|A",
            Some(4),
            SequenceScope::Channel,
            Some("ch"),
            1,
        ),
        scoped(
            "f3",
            "NSE_EQ|A",
            Some(1),
            SequenceScope::Channel,
            Some("ch"),
            2,
        ),
    ]))?;
    assert_eq!(
        events[1].quality,
        StreamState::Gap,
        "sequence jump opens a gap"
    );
    assert_eq!(
        events[2].quality,
        StreamState::Recovering,
        "a new epoch cannot clear a gap"
    );
    assert!(
        !events[2].is_actionable(),
        "reconnected order stays non-actionable"
    );
    Ok(())
}

/// Per-instrument scopes isolate a gap on one instrument from another.
#[test]
fn per_instrument_gap_does_not_leak_to_other_instruments() -> Result<(), Box<dyn std::error::Error>>
{
    let events = project_session(&session_of(vec![
        scoped(
            "f1",
            "NSE_EQ|A",
            Some(1),
            SequenceScope::ProviderInstrument,
            None,
            1,
        ),
        scoped(
            "f2",
            "NSE_EQ|A",
            Some(4),
            SequenceScope::ProviderInstrument,
            None,
            1,
        ),
        scoped(
            "f3",
            "NSE_EQ|B",
            Some(1),
            SequenceScope::ProviderInstrument,
            None,
            1,
        ),
    ]))?;
    assert_eq!(
        events[1].quality,
        StreamState::Gap,
        "instrument A has a gap"
    );
    assert_eq!(
        events[2].quality,
        StreamState::Clean,
        "instrument B is unaffected"
    );
    Ok(())
}
