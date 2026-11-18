#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Real exchange source conformance gate.
//!
//! This test is deliberately ignored in the default suite. It certifies parser
//! compatibility only when an operator supplies dated source files with
//! provenance in `NIRNAY_SOURCE_FIXTURES_DIR`. Synthetic parser tests prove CSV
//! grammar and rule behavior; they are not evidence of exchange compatibility.

use std::collections::BTreeMap;
use std::path::{
    Path, //
    PathBuf,
};

use india_reference::{
    Limits, //
    SnapshotScope,
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureSet {
    nse: Fixture,
    bse: Fixture,
}

fn verify_fixture(root: &Path, fixture: &Fixture) -> Vec<u8> {
    let path = root.join(&fixture.file);
    assert!(path.is_file(), "fixture {} is missing", fixture.file);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        bytes.len() as u64,
        fixture.size_bytes,
        "fixture {} size matches provenance",
        fixture.file
    );
    let digest = format!("{:x}", sha2::Sha256::digest(&bytes));
    assert_eq!(
        digest, fixture.sha256,
        "fixture {} bytes match provenance hash",
        fixture.file
    );
    assert!(
        !fixture.source_url.is_empty()
            && !fixture.retrieved_at_utc.is_empty()
            && !fixture.declared_coverage.is_empty(),
        "fixture {} records source URL, retrieval time and coverage",
        fixture.file
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

    let mut seen = BTreeMap::new();
    for row in &nse.accepted {
        let key = row.isin.as_str().to_owned();
        assert!(
            seen.insert(key.clone(), row.line).is_none(),
            "duplicate ISIN {key} in accepted NSE rows"
        );
    }
}
