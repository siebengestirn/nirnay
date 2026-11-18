#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Real exchange source conformance gate.
//!
//! This test is deliberately ignored in the default suite. It certifies parser
//! compatibility only when an operator supplies dated source files with
//! provenance in `NIRNAY_SOURCE_FIXTURES_DIR`. Synthetic parser tests prove CSV
//! grammar and rule behavior; they are not evidence of exchange compatibility.

use std::path::{
    Path, //
    PathBuf,
};

use india_reference::{
    Limits, //
    SnapshotScope,
    SourceBatch,
    source,
};
use serde::Deserialize;
use sha2::Digest;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    file: String,
    source_url: String,
    retrieved_at_utc: String,
    sha256: String,
    size_bytes: u64,
    format_version: String,
    declared_coverage: String,
    expected_accepted: Option<usize>,
    expected_excluded: Option<usize>,
    expected_rejected: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureSet {
    nse: Fixture,
    bse: Fixture,
}

fn verify_fixture(root: &Path, fixture: &Fixture) -> Vec<u8> {
    let path = root.join(&fixture.file);
    assert!(path.is_file(), "fixture file must exist");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        bytes.len() as u64,
        fixture.size_bytes,
        "fixture size must match provenance"
    );
    let digest = format!("{:x}", sha2::Sha256::digest(&bytes));
    assert_eq!(
        digest, fixture.sha256,
        "fixture bytes must match provenance hash"
    );
    assert!(
        !fixture.source_url.is_empty()
            && !fixture.retrieved_at_utc.is_empty()
            && !fixture.declared_coverage.is_empty(),
        "fixture must record source URL, retrieval time and coverage"
    );
    bytes
}

#[test]
#[ignore = "requires operator-supplied dated NSE and BSE source files with provenance"]
fn dated_exchange_sources_conform() {
    let root = PathBuf::from(
        std::env::var_os("NIRNAY_SOURCE_FIXTURES_DIR")
            .expect("real source conformance requires NIRNAY_SOURCE_FIXTURES_DIR"),
    );
    let manifest: FixtureSet = serde_json::from_slice(
        &std::fs::read(root.join("provenance.json")).expect("provenance.json is required"),
    )
    .expect("fixture provenance must match the active schema");

    assert!(
        manifest.nse.format_version == "nse-cash-equity-v1",
        "unsupported NSE format version must fail closed"
    );
    assert!(
        manifest.bse.format_version == "bse-security-master-v1",
        "unsupported BSE format version must fail closed"
    );
    let nse_bytes = verify_fixture(&root, &manifest.nse);
    let bse_bytes = verify_fixture(&root, &manifest.bse);

    let scope = SnapshotScope::Full {
        covered_venues: [trading_core::Venue::Nse, trading_core::Venue::Bse]
            .into_iter()
            .collect(),
    };
    let nse = source::nse::parse_nse_cash_master(
        &nse_bytes,
        &manifest.nse.format_version,
        scope.clone(),
        Limits::default(),
    )
    .expect("NSE source must parse under its declared version");
    let bse = source::bse::parse_bse_master(
        &bse_bytes,
        &manifest.bse.format_version,
        scope,
        Limits::default(),
    )
    .expect("BSE source must parse under its declared version");

    assert!(
        nse.is_complete(),
        "NSE: every physical row has exactly one outcome"
    );
    assert!(
        bse.is_complete(),
        "BSE: every physical row has exactly one outcome"
    );
    assert!(
        nse.physical_rows > 0 && bse.physical_rows > 0,
        "dated source files must contain data rows"
    );
    assert!(
        !nse.accepted.is_empty() && !bse.accepted.is_empty(),
        "dated sources must yield accepted rows; otherwise the format is unsupported"
    );

    let repeat = source::nse::parse_nse_cash_master(
        &nse_bytes,
        &manifest.nse.format_version,
        SnapshotScope::Full {
            covered_venues: [trading_core::Venue::Nse].into_iter().collect(),
        },
        Limits::default(),
    )
    .expect("repeat NSE parse");
    assert_eq!(
        nse.accepted, repeat.accepted,
        "repeated ingestion of the same bytes is deterministic"
    );
    assert_eq!(
        nse.rejected, repeat.rejected,
        "rejections are deterministic across repeated ingestion"
    );

    check_counts("NSE", &manifest.nse, &nse);
    check_counts("BSE", &manifest.bse, &bse);
}

fn check_counts<T>(name: &str, fixture: &Fixture, batch: &SourceBatch<T>) {
    if let Some(expected) = fixture.expected_accepted {
        assert_eq!(
            batch.accepted.len(),
            expected,
            "{name} accepted count must match provenance"
        );
    }
    if let Some(expected) = fixture.expected_excluded {
        assert_eq!(
            batch.excluded.len(),
            expected,
            "{name} excluded count must match provenance"
        );
    }
    if let Some(expected) = fixture.expected_rejected {
        assert_eq!(
            batch.rejected.len(),
            expected,
            "{name} rejected count must match provenance"
        );
    }
}
