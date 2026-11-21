#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M3.1-A source capability registry tests. No entitlement is claimed by
//! loading the checked-in declaration.

use market_archive::{
    CapabilityError, //
    DataOperation,
    DepthModel,
    EvidenceSource,
    MarketDataRequirement,
    PermissionState,
    ProductKey,
    SourceProductRegistry,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const REGISTRY: &str = include_str!("../../../configs/providers/source-products.v1.json");

fn registry() -> Result<SourceProductRegistry, Box<dyn std::error::Error>> {
    Ok(SourceProductRegistry::from_bytes(REGISTRY.as_bytes())?)
}

fn product_key() -> ProductKey {
    ProductKey::new("upstox", "full", "NSE", "Cash")
}

fn registry_with(
    operation: DataOperation,
    state: &str,
    verified: bool,
) -> Result<SourceProductRegistry, Box<dyn std::error::Error>> {
    let mut value: serde_json::Value = serde_json::from_str(REGISTRY)?;
    value["products"][0]["rights"][operation.as_str()]["state"] =
        serde_json::Value::String(state.to_owned());
    if verified {
        value["products"][0]["entitlement"]["verified"] = serde_json::Value::Bool(true);
    }
    Ok(SourceProductRegistry::from_bytes(&serde_json::to_vec(
        &value,
    )?)?)
}

fn requirement_for(operation: DataOperation) -> MarketDataRequirement {
    let mut need = requirement();
    match operation {
        DataOperation::Capture => need.requires_capture = true,
        DataOperation::Store => need.requires_store = true,
        DataOperation::Replay => need.requires_replay = true,
        DataOperation::Backup => need.requires_backup = true,
        DataOperation::InternalAnalytics => need.requires_internal_analytics = true,
        DataOperation::ExternalRedistribution => need.requires_external_redistribution = true,
        DataOperation::Retention => need.requires_retention = true,
    }
    need
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
        requires_capture: false,
        requires_store: false,
        requires_replay: false,
        requires_backup: false,
        requires_internal_analytics: false,
        requires_external_redistribution: false,
        requires_retention: false,
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
fn kite_quote_supplies_a_snapshot_but_no_source_continuity() -> TestResult {
    let registry = registry()?;
    let kite = registry.product("zerodha_kite", "quote", "NSE", "Cash")?;
    assert!(kite.snapshot, "Kite quote returns a market-depth snapshot");
    let mut need = requirement();
    need.provider = "zerodha_kite".to_owned();
    need.product = "quote".to_owned();
    need.requires_clean_snapshot = true;
    registry.evaluate(&need)?;
    need.requires_source_continuity = true;
    assert!(
        matches!(
            registry.evaluate(&need),
            Err(CapabilityError::NoSourceContinuity)
        ),
        "a snapshot-only quote has no continuous source sequence"
    );
    Ok(())
}

#[test]
fn malformed_and_wrong_version_registries_are_refused() -> TestResult {
    assert!(
        matches!(
            SourceProductRegistry::from_bytes(
                b"{\"schema_version\":2,\"registry_id\":\"x\",\"rights_review\":{\"status\":\"pending\",\"review_date\":\"2026-11-20\",\"required_evidence\":[]},\"products\":[]}"
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

#[test]
fn unknown_and_prohibited_rights_do_not_grant_store_or_replay() -> TestResult {
    let registry = registry()?;
    let upstox = registry.product("upstox", "full", "NSE", "Cash")?;
    assert_eq!(
        upstox.rights.store.state,
        market_archive::PermissionState::Unknown,
        "no written evidence means UNKNOWN, not PERMITTED"
    );
    assert_eq!(
        upstox.rights.replay.state,
        market_archive::PermissionState::Unknown,
        "replay remains UNKNOWN until written evidence is reviewed"
    );
    let mut need = requirement();
    need.requires_store = true;
    assert!(
        matches!(
            registry.evaluate(&need),
            Err(CapabilityError::MissingRight(_))
        ),
        "UNKNOWN store right must fail closed"
    );
    Ok(())
}

#[test]
fn rights_review_requires_written_evidence() -> TestResult {
    let registry = registry()?;
    let review = &registry.rights_review;
    assert_eq!(
        review.status, "pending_written_evidence",
        "written evidence is pending"
    );
    for required in [
        "personal_capture",
        "local_retention",
        "historical_replay",
        "backup_retention",
        "internal_analytics",
        "external_redistribution",
        "retention_period",
    ] {
        assert!(
            review.required_evidence.iter().any(|item| item == required),
            "{required} must be in the written-evidence checklist"
        );
    }
    Ok(())
}

#[test]
fn checked_in_registry_grants_no_operation_to_any_product() -> TestResult {
    let registry = registry()?;
    for product in &registry.products {
        let name = &product.product;
        assert!(
            !product.entitlement.verified,
            "{name} has no verified entitlement"
        );
        for operation in DataOperation::ALL {
            assert_eq!(
                product.rights.right_for(operation).state,
                PermissionState::Unknown,
                "{name} is UNKNOWN for {operation:?}"
            );
            assert!(
                registry.authorize(&product.key(), operation).is_err(),
                "{name} is denied for {operation:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn all_seven_operations_fail_closed_for_unknown_and_prohibited() -> TestResult {
    let registry = registry()?;
    let key = product_key();
    for operation in DataOperation::ALL {
        assert!(
            matches!(
                registry.authorize(&key, operation),
                Err(CapabilityError::MissingRight(_))
            ),
            "{operation:?} is denied when UNKNOWN"
        );
    }
    for state in ["UNKNOWN", "PROHIBITED"] {
        for operation in DataOperation::ALL {
            let modified = registry_with(operation, state, true)?;
            assert!(
                matches!(
                    modified.authorize(&key, operation),
                    Err(CapabilityError::MissingRight(_))
                ),
                "{operation:?} is denied when {state}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_permitted_operation_authorizes_only_that_operation() -> TestResult {
    let key = product_key();
    for operation in DataOperation::ALL {
        let allowed = registry_with(operation, "PERMITTED", true)?;
        assert!(
            allowed.authorize(&key, operation).is_ok(),
            "{operation:?} is authorized by its own PERMITTED right"
        );
        for other in DataOperation::ALL {
            if other == operation {
                continue;
            }
            assert!(
                allowed.authorize(&key, other).is_err(),
                "{other:?} stays denied when only {operation:?} is PERMITTED"
            );
        }
    }
    Ok(())
}

#[test]
fn persistence_operations_require_a_verified_entitlement() -> TestResult {
    let key = product_key();
    for operation in DataOperation::ALL {
        let unverified = registry_with(operation, "PERMITTED", false)?;
        if operation.is_persistence() {
            assert!(
                matches!(
                    unverified.authorize(&key, operation),
                    Err(CapabilityError::EntitlementUnverified(_))
                ),
                "{operation:?} needs a verified entitlement"
            );
        } else {
            assert!(
                unverified.authorize(&key, operation).is_ok(),
                "{operation:?} does not persist data and needs no entitlement check"
            );
        }
    }
    Ok(())
}

#[test]
fn every_requirement_flag_is_checked_in_evaluate() -> TestResult {
    for operation in DataOperation::ALL {
        let denied = registry()?.evaluate(&requirement_for(operation));
        assert!(
            matches!(denied, Err(CapabilityError::MissingRight(_))),
            "requires_{operation:?} is enforced by evaluate"
        );
        let allowed = registry_with(operation, "PERMITTED", true)?;
        assert!(
            allowed.evaluate(&requirement_for(operation)).is_ok(),
            "requires_{operation:?} passes only with its right"
        );
    }
    Ok(())
}

#[test]
fn schema_coherence_is_enforced_by_evaluate() -> TestResult {
    let mut value: serde_json::Value = serde_json::from_str(REGISTRY)?;
    value["products"][0]["snapshot"] = serde_json::Value::Bool(false);
    let incoherent = SourceProductRegistry::from_bytes(&serde_json::to_vec(&value)?)?;
    assert!(
        matches!(
            incoherent.evaluate(&requirement()),
            Err(CapabilityError::InvalidRegistry(_))
        ),
        "snapshot_only with snapshot=false is refused at runtime"
    );
    Ok(())
}

#[test]
fn retention_period_has_no_fabricated_duration() -> TestResult {
    for product in &registry()?.products {
        let name = &product.product;
        assert_eq!(
            product.rights.retention_period.duration_days, None,
            "{name} declares no written retention duration"
        );
        assert_eq!(
            product.rights.retention_period.basis,
            EvidenceSource::None,
            "{name} records retention-period evidence as absent"
        );
    }
    Ok(())
}
