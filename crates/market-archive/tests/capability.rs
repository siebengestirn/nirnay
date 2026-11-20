#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.1-A source capability registry tests. No entitlement is claimed by
//! loading the checked-in declaration.

use market_archive::{
    CapabilityError, //
    DepthModel,
    MarketDataRequirement,
    SourceProductRegistry,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const REGISTRY: &str = include_str!("../../../configs/providers/source-products.v1.json");

fn registry() -> Result<SourceProductRegistry, Box<dyn std::error::Error>> {
    Ok(SourceProductRegistry::from_bytes(REGISTRY.as_bytes())?)
}

fn requirement() -> MarketDataRequirement {
    MarketDataRequirement {
        provider: "upstox".to_owned(),
        product: "full".to_owned(),
        venue: "NSE".to_owned(),
        segment: "Cash".to_owned(),
        depth_model: DepthModel::Mbp,
        min_depth: 5,
        requires_trade_prints: false,
        requires_clean_snapshot: true,
        requires_source_continuity: false,
        requires_store: false,
        requires_replay: false,
    }
}

#[test]
fn checked_in_registry_parses_and_declares_three_products() -> TestResult {
    let registry = registry()?;
    assert_eq!(
        registry.products.len(),
        3,
        "three source products are declared"
    );
    assert!(
        registry.registry_id.len() > 1,
        "the registry has an identity"
    );
    Ok(())
}

#[test]
fn five_level_upstox_full_satisfies_matching_requirement() -> TestResult {
    registry()?.evaluate(&requirement())?;
    Ok(())
}

#[test]
fn ten_level_requirement_is_rejected_not_silently_satisfied() -> TestResult {
    let mut need = requirement();
    need.min_depth = 10;
    assert!(
        matches!(
            registry()?.evaluate(&need),
            Err(CapabilityError::InsufficientDepth {
                declared: 5,
                required: 10
            })
        ),
        "a five-level product cannot satisfy a ten-level requirement"
    );
    Ok(())
}

#[test]
fn unknown_provider_and_venue_are_rejected() -> TestResult {
    let mut need = requirement();
    need.provider = "unknown".to_owned();
    assert!(
        matches!(
            registry()?.evaluate(&need),
            Err(CapabilityError::UnknownProduct(..))
        ),
        "an undeclared provider is refused"
    );
    let mut need = requirement();
    need.venue = "BSE".to_owned();
    assert!(
        matches!(
            registry()?.evaluate(&need),
            Err(CapabilityError::UnknownProduct(..))
        ),
        "NSE coverage does not imply BSE coverage"
    );
    Ok(())
}

#[test]
fn mbo_requirement_is_not_satisfied_by_mbp_depth() -> TestResult {
    let mut need = requirement();
    need.depth_model = DepthModel::Mbo;
    assert!(
        matches!(
            registry()?.evaluate(&need),
            Err(CapabilityError::DepthModelMismatch { .. })
        ),
        "aggregated depth is not market-by-order"
    );
    Ok(())
}

#[test]
fn source_continuity_is_not_claimed_without_a_source_sequence() -> TestResult {
    let mut need = requirement();
    need.requires_source_continuity = true;
    assert!(
        matches!(
            registry()?.evaluate(&need),
            Err(CapabilityError::NoSourceContinuity)
        ),
        "upstox declares no source sequence in this registry"
    );
    Ok(())
}

#[test]
fn store_and_replay_require_verified_entitlement_and_rights() -> TestResult {
    let mut need = requirement();
    need.requires_store = true;
    assert!(
        matches!(
            registry()?.evaluate(&need),
            Err(CapabilityError::MissingRight(_))
        ),
        "store rights are not granted by this declaration"
    );
    let mut need = requirement();
    need.requires_replay = true;
    assert!(
        matches!(
            registry()?.evaluate(&need),
            Err(CapabilityError::MissingRight(_))
        ),
        "replay rights are not granted by this declaration"
    );
    Ok(())
}

#[test]
fn kite_quote_is_execution_support_not_an_mbp_source() -> TestResult {
    let mut need = requirement();
    need.provider = "zerodha_kite".to_owned();
    need.product = "quote".to_owned();
    need.requires_clean_snapshot = true;
    assert!(
        matches!(
            registry()?.evaluate(&need),
            Err(CapabilityError::NoSnapshot)
        ),
        "a touchline quote cannot supply a clean book snapshot"
    );
    Ok(())
}

#[test]
fn malformed_and_wrong_version_registries_are_refused() -> TestResult {
    assert!(
        matches!(
            SourceProductRegistry::from_bytes(
                b"{\"schema_version\":2,\"registry_id\":\"x\",\"products\":[]}"
            ),
            Err(CapabilityError::UnsupportedSchema(2))
        ),
        "an unknown registry version is refused before other checks"
    );
    let wrong_version = REGISTRY.replacen("\"schema_version\": 1", "\"schema_version\": 2", 1);
    assert!(
        matches!(
            SourceProductRegistry::from_bytes(wrong_version.as_bytes()),
            Err(CapabilityError::UnsupportedSchema(2))
        ),
        "a complete registry with an unknown version is refused"
    );
    assert!(
        SourceProductRegistry::from_bytes(b"{}").is_err(),
        "a registry missing required fields is refused"
    );
    Ok(())
}

#[test]
fn protocol_digest_is_real_or_explicitly_unavailable() -> TestResult {
    let registry = registry()?;
    let upstox = registry.product("upstox", "full", "NSE", "Cash")?;
    let digest = upstox
        .protocol
        .schema_digest
        .as_deref()
        .ok_or("upstox digest missing")?;
    assert_eq!(
        digest.len(),
        64,
        "the protobuf schema digest is a SHA-256 value"
    );
    let fyers = registry.product("fyers", "data_socket", "NSE", "Cash")?;
    assert!(
        fyers.protocol.schema_digest.is_none() && fyers.protocol.schema_reference.is_some(),
        "the SDK callback product names its protocol reference without inventing a digest"
    );
    Ok(())
}
