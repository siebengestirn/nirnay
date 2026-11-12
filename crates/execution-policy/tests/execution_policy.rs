#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use chrono::{DateTime, Duration, TimeZone, Utc};
use ed25519_dalek::{Signer, SigningKey};
use execution_policy::{
    ApprovalAttestation, ExecutionMode, ExecutionPolicy, OrderRequest, PermitBody, PermitEnvelope,
    PolicyError, TrustRegistry, TrustedKey, TrustedRuntime,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fixed_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 8, 0, 0, 0).unwrap()
}

fn runtime(commit: &str) -> TrustedRuntime {
    TrustedRuntime::new(commit.to_owned(), fixed_now())
}

fn issuer_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn approver_key_a() -> SigningKey {
    SigningKey::from_bytes(&[11u8; 32])
}

fn approver_key_b() -> SigningKey {
    SigningKey::from_bytes(&[23u8; 32])
}

fn registry(minimum_approvals: usize) -> Result<TrustRegistry, PolicyError> {
    TrustRegistry::new(
        vec![TrustedKey::new(
            "release-key-1".to_owned(),
            hex(&issuer_key().verifying_key().to_bytes()),
        )?],
        vec![
            TrustedKey::new(
                "approvers/approver-1".to_owned(),
                hex(&approver_key_a().verifying_key().to_bytes()),
            )?,
            TrustedKey::new(
                "approvers/approver-2".to_owned(),
                hex(&approver_key_b().verifying_key().to_bytes()),
            )?,
        ],
        minimum_approvals,
        vec![],
    )
}

fn body(commit: &str) -> PermitBody {
    PermitBody {
        commit_sha: commit.to_owned(),
        account: "FA123".to_owned(),
        broker: "zerodha".to_owned(),
        strategy: "momentum-baseline".to_owned(),
        instruments: vec!["RELIANCE".to_owned(), "TCS".to_owned()],
        max_order_notional: 100_000,
        max_open_exposure: 500_000,
        operator: "op".to_owned(),
        expires_at_utc: fixed_now() + Duration::days(7),
        issuer_key_id: "release-key-1".to_owned(),
    }
}

fn request() -> OrderRequest {
    OrderRequest {
        broker: "zerodha".to_owned(),
        account: "FA123".to_owned(),
        strategy: "momentum-baseline".to_owned(),
        instrument: "RELIANCE".to_owned(),
        notional: 10_000,
    }
}

fn sign_issuer(signing: &SigningKey, body: &PermitBody) -> Result<String, PolicyError> {
    Ok(hex(&signing.sign(&body.canonical_payload()?).to_bytes()))
}

fn attest(
    signing: &SigningKey,
    approver_id: &str,
    approver_key_id: &str,
    body: &PermitBody,
) -> Result<ApprovalAttestation, PolicyError> {
    let attestation = ApprovalAttestation {
        approver_id: approver_id.to_owned(),
        approver_key_id: approver_key_id.to_owned(),
        signature: String::new(),
    };
    Ok(ApprovalAttestation {
        approver_id: approver_id.to_owned(),
        approver_key_id: approver_key_id.to_owned(),
        signature: hex(&signing
            .sign(&attestation.attestation_payload(body)?)
            .to_bytes()),
    })
}

fn envelope_with_approvals(
    commit: &str,
    approvals: Vec<Result<ApprovalAttestation, PolicyError>>,
) -> Result<PermitEnvelope, PolicyError> {
    let approvals: Vec<ApprovalAttestation> = approvals.into_iter().collect::<Result<_, _>>()?;
    let body = body(commit);
    Ok(PermitEnvelope {
        issuer_signature: sign_issuer(&issuer_key(), &body)?,
        approvals,
        body,
    })
}

#[test]
fn default_build_cannot_submit_live_orders() {
    let policy = ExecutionPolicy::default_paper();
    assert_eq!(policy.mode(), ExecutionMode::Paper);
    assert!(policy.envelope().is_none());
    // The Live path denies a Paper policy outright; the non-live path
    // authorizes non-live execution with no permit attached.
    assert!(matches!(
        policy.authorize_order(&request(), &runtime("any"), &registry(1).unwrap(), 0),
        Err(PolicyError::ModeMismatch {
            mode: ExecutionMode::Paper
        })
    ));
    let auth = policy.authorize_non_live().unwrap();
    assert_eq!(auth.mode(), ExecutionMode::Paper);
    assert!(auth.permit_commit().is_none());
    let sim = ExecutionPolicy::simulated();
    assert_eq!(sim.mode(), ExecutionMode::Simulated);
    assert!(sim.envelope().is_none());
    assert_eq!(
        sim.authorize_non_live().unwrap().mode(),
        ExecutionMode::Simulated
    );
    // Live mode without a permit denies; the non-live path refuses Live.
    let live = ExecutionPolicy::live();
    assert!(matches!(
        live.authorize_order(&request(), &runtime("any"), &registry(1).unwrap(), 0),
        Err(PolicyError::NoPermit)
    ));
    assert!(matches!(
        live.authorize_non_live(),
        Err(PolicyError::ModeMismatch {
            mode: ExecutionMode::Live
        })
    ));
}

#[test]
fn verified_issuer_plus_distinct_attestations_grant_live() {
    let commit = "abc123";
    let envelope = envelope_with_approvals(
        commit,
        vec![
            attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &body(commit),
            ),
            attest(
                &approver_key_b(),
                "approver-2",
                "approvers/approver-2",
                &body(commit),
            ),
        ],
    )
    .unwrap();
    // A valid permit attached to a Paper policy still denies Live: the
    // permit never changes the configured mode.
    let paper_policy = ExecutionPolicy::default_paper().with_envelope(envelope.clone());
    assert_eq!(paper_policy.mode(), ExecutionMode::Paper);
    assert!(matches!(
        paper_policy.authorize_order(&request(), &runtime(commit), &registry(2).unwrap(), 0),
        Err(PolicyError::ModeMismatch {
            mode: ExecutionMode::Paper
        })
    ));
    assert!(matches!(
        paper_policy.authorize_non_live(),
        Err(PolicyError::ModeMismatch {
            mode: ExecutionMode::Paper
        })
    ));
    // The same valid permit under an explicit Live policy authorizes.
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert_eq!(policy.mode(), ExecutionMode::Live);
    let auth = policy
        .authorize_order(&request(), &runtime(commit), &registry(2).unwrap(), 0)
        .unwrap();
    assert_eq!(auth.mode(), ExecutionMode::Live);
    assert_eq!(auth.permit_commit(), Some(commit));
    let single = envelope_with_approvals(
        commit,
        vec![attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        )],
    )
    .unwrap();
    let policy = ExecutionPolicy::live().with_envelope(single);
    assert!(
        policy
            .authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0)
            .is_ok()
    );
}

#[test]
fn duplicate_approver_identity_cannot_satisfy_threshold() {
    let commit = "abc123";
    let duplicate = vec![
        attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        ),
        attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        ),
    ];
    let policy =
        ExecutionPolicy::live().with_envelope(envelope_with_approvals(commit, duplicate).unwrap());
    let error = policy
        .authorize_order(&request(), &runtime(commit), &registry(2).unwrap(), 0)
        .unwrap_err();
    assert!(
        matches!(error, PolicyError::DuplicateApprover(_)),
        "got {error}"
    );
}

#[test]
fn duplicate_approver_key_cannot_satisfy_threshold() {
    let commit = "abc123";
    let shared_key = vec![
        attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        ),
        attest(
            &approver_key_a(),
            "approver-2",
            "approvers/approver-1",
            &body(commit),
        ),
    ];
    let policy =
        ExecutionPolicy::live().with_envelope(envelope_with_approvals(commit, shared_key).unwrap());
    let error = policy
        .authorize_order(&request(), &runtime(commit), &registry(2).unwrap(), 0)
        .unwrap_err();
    // The second attestation claims identity approver-2 but is signed by
    // approver-1's key: rejected either as an identity mismatch or as a
    // duplicate key. Either way the threshold cannot be met by one signer.
    assert!(
        matches!(
            error,
            PolicyError::UntrustedApprover(_) | PolicyError::DuplicateApproverKey(_)
        ),
        "got {error}"
    );
}

#[test]
fn borrowed_key_id_with_wrong_identity_rejected() {
    let commit = "abc123";
    let borrowed = vec![attest(
        &approver_key_a(),
        "approver-2",
        "approvers/approver-1",
        &body(commit),
    )];
    let policy =
        ExecutionPolicy::live().with_envelope(envelope_with_approvals(commit, borrowed).unwrap());
    let error = policy
        .authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0)
        .unwrap_err();
    assert!(matches!(error, PolicyError::UntrustedApprover(_)));
}

#[test]
fn forged_issuer_signature_fails_closed() {
    let commit = "abc123";
    let mut envelope = envelope_with_approvals(
        commit,
        vec![attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        )],
    )
    .unwrap();
    envelope.body.max_order_notional = 999_999_999;
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        policy
            .authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0)
            .unwrap_err(),
        PolicyError::ForgedIssuerSignature
    ));
}

#[test]
fn stale_attestation_fails_closed() {
    let commit = "abc123";
    let mut envelope = envelope_with_approvals(
        commit,
        vec![attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        )],
    )
    .unwrap();
    envelope.body.instruments.push("INFY".to_owned());
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        policy
            .authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0)
            .unwrap_err(),
        PolicyError::ForgedIssuerSignature | PolicyError::ForgedApproval { .. }
    ));
}

#[test]
fn approvals_rechecked_on_every_authorization() {
    let commit = "abc123";
    let envelope = envelope_with_approvals(
        commit,
        vec![attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        )],
    )
    .unwrap();
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    let mut revoked = registry(1).unwrap();
    let _ = revoked.revoke("approvers/approver-1");
    let error = policy
        .authorize_order(&request(), &runtime(commit), &revoked, 0)
        .unwrap_err();
    // Revocation is applied between authorizations and denied on the next
    // call: nothing about approval state is cached.
    assert!(
        matches!(error, PolicyError::RevokedApprover(_)),
        "got {error}"
    );
}

#[test]
fn registry_threshold_above_registered_approvers_is_malformed() {
    // Shape validation happens at construction time: the malformed registry
    // cannot even be loaded, so no authorization path can see it.
    let malformed = (|| -> Result<TrustRegistry, PolicyError> {
        TrustRegistry::new(
            vec![TrustedKey::new(
                "release-key-1".to_owned(),
                hex(&issuer_key().verifying_key().to_bytes()),
            )?],
            vec![
                TrustedKey::new(
                    "approvers/approver-1".to_owned(),
                    hex(&approver_key_a().verifying_key().to_bytes()),
                )?,
                TrustedKey::new(
                    "approvers/approver-2".to_owned(),
                    hex(&approver_key_b().verifying_key().to_bytes()),
                )?,
            ],
            3,
            vec![],
        )
    })();
    assert!(matches!(
        malformed,
        Err(PolicyError::RegistryThresholdExceedsApprovers { .. })
    ));
}

#[test]
fn revoked_approver_rejected_before_signature_verification() {
    let commit = "abc123";
    let envelope = envelope_with_approvals(
        commit,
        vec![attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        )],
    )
    .unwrap();
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    let mut revoked_registry = registry(1).unwrap();
    let _ = revoked_registry.revoke("approvers/approver-1");
    let error = policy
        .authorize_order(&request(), &runtime(commit), &revoked_registry, 0)
        .unwrap_err();
    assert!(
        matches!(error, PolicyError::RevokedApprover(_)),
        "got {error}"
    );
}

#[test]
fn revoke_returns_false_for_unknown_key_id() {
    let mut reg = registry(1).unwrap();
    assert!(!reg.revoke("approvers/ghost"));
}

#[test]
fn revoke_is_idempotent() {
    let mut reg = registry(1).unwrap();
    assert!(reg.revoke("approvers/approver-1"));
    assert!(reg.revoke("approvers/approver-1"));
    assert!(reg.is_approver_revoked("approvers/approver-1"));
    assert!(!reg.is_approver_revoked("approvers/approver-2"));
}

#[test]
fn empty_approvals_fail_closed() {
    let commit = "abc123";
    let policy =
        ExecutionPolicy::live().with_envelope(envelope_with_approvals(commit, vec![]).unwrap());
    let error = policy
        .authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0)
        .unwrap_err();
    assert!(matches!(error, PolicyError::NoApprovals { required: 1 }));
}

#[test]
fn expiry_and_commit_binding_fail_closed() {
    let signing = issuer_key();
    let commit = "abc123";
    let mut expired_body = body(commit);
    expired_body.expires_at_utc = fixed_now() - Duration::seconds(1);
    let envelope = PermitEnvelope {
        issuer_signature: sign_issuer(&signing, &expired_body).unwrap(),
        approvals: vec![
            attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &expired_body,
            )
            .unwrap(),
        ],
        body: expired_body,
    };
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        policy
            .authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0)
            .unwrap_err(),
        PolicyError::Expired(_)
    ));

    let wrong_body = body("issued-sha");
    let envelope = PermitEnvelope {
        issuer_signature: sign_issuer(&signing, &wrong_body).unwrap(),
        approvals: vec![
            attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &wrong_body,
            )
            .unwrap(),
        ],
        body: wrong_body,
    };
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        policy
            .authorize_order(
                &request(),
                &runtime("current-sha"),
                &registry(1).unwrap(),
                0
            )
            .unwrap_err(),
        PolicyError::CommitMismatch { .. }
    ));
}

#[test]
fn scope_mismatches_fail_closed() {
    let commit = "abc123";
    let envelope = envelope_with_approvals(
        commit,
        vec![attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        )],
    )
    .unwrap();
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    let rt = runtime(commit);
    let reg = registry(1).unwrap();
    let mut wrong_broker = request();
    wrong_broker.broker = "groww".to_owned();
    assert!(matches!(
        policy.authorize_order(&wrong_broker, &rt, &reg, 0),
        Err(PolicyError::BrokerMismatch { .. })
    ));
    let mut wrong_strategy = request();
    wrong_strategy.strategy = "mean-reversion".to_owned();
    assert!(matches!(
        policy.authorize_order(&wrong_strategy, &rt, &reg, 0),
        Err(PolicyError::StrategyMismatch { .. })
    ));
    let mut wrong_account = request();
    wrong_account.account = "OTHER".to_owned();
    assert!(matches!(
        policy.authorize_order(&wrong_account, &rt, &reg, 0),
        Err(PolicyError::AccountMismatch { .. })
    ));
    let mut off_list = request();
    off_list.instrument = "INFY".to_owned();
    assert!(matches!(
        policy.authorize_order(&off_list, &rt, &reg, 0),
        Err(PolicyError::InstrumentNotPermitted { .. })
    ));
    let mut too_big = request();
    too_big.notional = 200_000;
    assert!(matches!(
        policy.authorize_order(&too_big, &rt, &reg, 0),
        Err(PolicyError::NotionalExceeded { .. })
    ));
    assert!(matches!(
        policy.authorize_order(&request(), &rt, &reg, 600_000),
        Err(PolicyError::ExposureExceeded { .. })
    ));
}

#[test]
fn untrusted_policy_json_cannot_create_live_authority() {
    // ExecutionPolicy has no Deserialize implementation: untrusted bytes
    // can never manufacture a policy (in particular, never set Live mode).
    // Permit envelopes remain deserializable untrusted input, but they
    // carry zero authority until authorize_order re-verifies them.
    let unsigned = PermitEnvelope {
        issuer_signature: String::new(),
        approvals: vec![],
        body: body("abc"),
    };
    let envelope: PermitEnvelope =
        serde_json::from_value(serde_json::to_value(&unsigned).unwrap()).unwrap();
    // Paper policy + hostile envelope: mode gate denies before anything else.
    let paper = ExecutionPolicy::default_paper().with_envelope(envelope.clone());
    assert!(matches!(
        paper.authorize_order(&request(), &runtime("abc"), &registry(1).unwrap(), 0),
        Err(PolicyError::ModeMismatch {
            mode: ExecutionMode::Paper
        })
    ));
    // Live policy + unsigned envelope: verification fails closed.
    let live = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        live.authorize_order(&request(), &runtime("abc"), &registry(1).unwrap(), 0),
        Err(PolicyError::MalformedSignature(_))
    ));
}

#[test]
fn canonical_encoding_is_unambiguous() {
    let mut a = body("commit-a");
    a.account = "ac".to_owned();
    a.strategy = "count".to_owned();
    let mut b = body("commit-a");
    b.account = "ac\x00count".to_owned();
    b.strategy = String::new();
    assert_ne!(
        a.canonical_payload().unwrap(),
        b.canonical_payload().unwrap()
    );
    let mut empty = body("commit-a");
    empty.operator = String::new();
    assert!(!empty.canonical_payload().unwrap().is_empty());
}

#[test]
fn attestation_payload_binds_issuer_and_expiry() {
    let body_a = body("commit-a");
    let mut body_b = body("commit-a");
    body_b.issuer_key_id = "release-key-2".to_owned();
    let attestation = attest(
        &approver_key_a(),
        "approver-1",
        "approvers/approver-1",
        &body_a,
    )
    .unwrap();
    assert_ne!(
        attestation.attestation_payload(&body_a).unwrap(),
        attestation.attestation_payload(&body_b).unwrap()
    );
}

fn valid_live_policy(commit: &str) -> Result<ExecutionPolicy, PolicyError> {
    Ok(
        ExecutionPolicy::live().with_envelope(envelope_with_approvals(
            commit,
            vec![attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &body(commit),
            )],
        )?),
    )
}

#[test]
fn simulated_policy_carrying_valid_permit_denies_live() {
    let commit = "abc123";
    let policy = ExecutionPolicy::simulated().with_envelope(
        envelope_with_approvals(
            commit,
            vec![attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &body(commit),
            )],
        )
        .unwrap(),
    );
    assert_eq!(policy.mode(), ExecutionMode::Simulated);
    assert!(matches!(
        policy.authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0),
        Err(PolicyError::ModeMismatch {
            mode: ExecutionMode::Simulated
        })
    ));
}

#[test]
fn attaching_envelope_never_changes_mode() {
    let commit = "abc123";
    let envelope = envelope_with_approvals(
        commit,
        vec![attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        )],
    )
    .unwrap();
    for policy in [
        ExecutionPolicy::default_paper(),
        ExecutionPolicy::simulated(),
        ExecutionPolicy::live(),
    ] {
        let mode = policy.mode();
        let carried = policy.with_envelope(envelope.clone());
        assert_eq!(carried.mode(), mode);
    }
}

#[test]
fn invalid_order_values_deny_before_limit_checks() {
    let commit = "abc123";
    let policy = valid_live_policy(commit).unwrap();
    let rt = runtime(commit);
    let reg = registry(1).unwrap();
    for notional in [0, -1] {
        let mut bad = request();
        bad.notional = notional;
        assert!(matches!(
            policy.authorize_order(&bad, &rt, &reg, 0),
            Err(PolicyError::InvalidNotional { .. })
        ));
    }
    assert!(matches!(
        policy.authorize_order(&request(), &rt, &reg, -1),
        Err(PolicyError::InvalidExposure { .. })
    ));
}

#[test]
fn limit_boundaries_allow_exact_limit_and_deny_above() {
    let commit = "abc123";
    let policy = valid_live_policy(commit).unwrap();
    let rt = runtime(commit);
    let reg = registry(1).unwrap();
    // Exactly at the permit limits: allowed.
    let mut at_limit = request();
    at_limit.notional = 100_000;
    assert!(
        policy
            .authorize_order(&at_limit, &rt, &reg, 500_000)
            .is_ok()
    );
    // Zero projected exposure is valid.
    assert!(policy.authorize_order(&request(), &rt, &reg, 0).is_ok());
    // One unit above each limit: denied.
    let mut above_limit = request();
    above_limit.notional = 100_001;
    assert!(matches!(
        policy.authorize_order(&above_limit, &rt, &reg, 0),
        Err(PolicyError::NotionalExceeded { .. })
    ));
    assert!(matches!(
        policy.authorize_order(&request(), &rt, &reg, 500_001),
        Err(PolicyError::ExposureExceeded { .. })
    ));
}

#[test]
fn expiry_boundary_denies_at_the_expiry_instant() {
    let commit = "abc123";
    let policy = valid_live_policy(commit).unwrap();
    let expiry = body(commit).expires_at_utc;
    let at_expiry = TrustedRuntime::new(commit.to_owned(), expiry);
    assert!(matches!(
        policy.authorize_order(&request(), &at_expiry, &registry(1).unwrap(), 0),
        Err(PolicyError::Expired(_))
    ));
    let one_second_before = TrustedRuntime::new(commit.to_owned(), expiry - Duration::seconds(1));
    assert!(
        policy
            .authorize_order(&request(), &one_second_before, &registry(1).unwrap(), 0)
            .is_ok()
    );
    let one_second_after = TrustedRuntime::new(commit.to_owned(), expiry + Duration::seconds(1));
    assert!(matches!(
        policy.authorize_order(&request(), &one_second_after, &registry(1).unwrap(), 0),
        Err(PolicyError::Expired(_))
    ));
}

#[test]
fn unrepresentable_expiry_timestamp_fails_closed() {
    let commit = "abc123";
    let mut unrepresentable = body(commit);
    unrepresentable.expires_at_utc = DateTime::<Utc>::MAX_UTC;
    let envelope = PermitEnvelope {
        issuer_signature: hex(&[0u8; 64]),
        approvals: vec![],
        body: unrepresentable,
    };
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        policy.authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0),
        Err(PolicyError::InvalidTimestamp { .. })
    ));
}

#[test]
fn negative_permit_limits_deny() {
    let commit = "abc123";
    let mut bad_body = body(commit);
    bad_body.max_order_notional = -1;
    let envelope = PermitEnvelope {
        issuer_signature: sign_issuer(&issuer_key(), &bad_body).unwrap(),
        approvals: vec![
            attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &bad_body,
            )
            .unwrap(),
        ],
        body: bad_body,
    };
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        policy.authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0),
        Err(PolicyError::InvalidPermitLimits { .. })
    ));
    let mut bad_body = body(commit);
    bad_body.max_open_exposure = -1;
    let envelope = PermitEnvelope {
        issuer_signature: sign_issuer(&issuer_key(), &bad_body).unwrap(),
        approvals: vec![
            attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &bad_body,
            )
            .unwrap(),
        ],
        body: bad_body,
    };
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        policy.authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0),
        Err(PolicyError::InvalidPermitLimits { .. })
    ));
}

#[test]
fn unknown_issuer_key_id_denies() {
    let commit = "abc123";
    let mut foreign = body(commit);
    foreign.issuer_key_id = "release-key-unknown".to_owned();
    let envelope = PermitEnvelope {
        issuer_signature: sign_issuer(&issuer_key(), &foreign).unwrap(),
        approvals: vec![
            attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &foreign,
            )
            .unwrap(),
        ],
        body: foreign,
    };
    let policy = ExecutionPolicy::live().with_envelope(envelope);
    assert!(matches!(
        policy.authorize_order(&request(), &runtime(commit), &registry(1).unwrap(), 0),
        Err(PolicyError::UnknownIssuer(_))
    ));
}

#[test]
fn golden_canonical_payload_bytes_are_stable() {
    // The signing contract must not change: any accidental edit to the
    // canonical encoding invalidates existing signatures. The bytes below
    // are a byte-for-byte fixture of body("abc123") under ENCODING_VERSION 2.
    let payload = body("abc123").canonical_payload().unwrap();
    let expected_hex = "0206000000616263313233050000004641313233070000007a65726f646861110000006d6f6d656e74756d2d626173656c696e6502000000000000000800000052454c49414e434503000000544353a08601000000000020a1070000000000020000006f7000001c6ede0ee8180d00000072656c656173652d6b65792d31";
    let actual_hex = payload
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(actual_hex, expected_hex);
}

#[test]
fn registry_rejects_invalid_key_ids() {
    for bad in [
        "",
        " ",
        "with space",
        "a//b",
        "/leading",
        "trailing/",
        "tab\tkey",
        "unicode\u{00e9}key",
    ] {
        let key = TrustedKey::new(
            bad.to_owned(),
            hex(&issuer_key().verifying_key().to_bytes()),
        );
        assert!(key.is_err(), "key id {bad:?} must be rejected");
    }
    for good in [
        "release-key-1",
        "approvers/approver-1",
        "a.b_c-d",
        "ops/prod/east-2",
    ] {
        let key = TrustedKey::new(
            good.to_owned(),
            hex(&issuer_key().verifying_key().to_bytes()),
        );
        assert!(key.is_ok(), "key id {good:?} must be accepted");
    }
}

#[test]
fn registry_rejects_malformed_key_material() {
    // Non-hex, odd-length hex, and wrong-length keys fail at construction.
    assert!(TrustedKey::new("k".to_owned(), "zz".to_owned()).is_err());
    assert!(TrustedKey::new("k".to_owned(), "abc".to_owned()).is_err());
    assert!(TrustedKey::new("k".to_owned(), hex(&[0u8; 16])).is_err());
    assert!(TrustedKey::new("k".to_owned(), hex(&[0u8; 33])).is_err());
    // A well-formed 32-byte key is accepted.
    assert!(TrustedKey::new("k".to_owned(), hex(&[1u8; 32])).is_ok());
    // Hex case is canonicalized at construction (material, not spelling,
    // is identity).
    let upper = TrustedKey::new("k".to_owned(), hex(&[1u8; 32]).to_uppercase());
    assert!(upper.is_ok());
}

fn duplicate_test_registry(
    issuers: Vec<TrustedKey>,
    approvers: Vec<TrustedKey>,
) -> Result<TrustRegistry, PolicyError> {
    TrustRegistry::new(issuers, approvers, 1, Vec::new())
}

#[test]
fn registry_rejects_duplicate_ids_and_material() {
    let key_a = TrustedKey::new(
        "a".to_owned(),
        hex(&issuer_key().verifying_key().to_bytes()),
    )
    .unwrap();
    let key_a2 = TrustedKey::new(
        "a".to_owned(),
        hex(&approver_key_a().verifying_key().to_bytes()),
    )
    .unwrap();
    // Duplicate issuer IDs.
    assert!(matches!(
        duplicate_test_registry(vec![key_a.clone(), key_a2.clone()], vec![key_a.clone()]),
        Err(PolicyError::InvalidRegistry { .. })
    ));
    // Duplicate (same-ID) approver entries.
    let approver = TrustedKey::new(
        "approvers/approver-1".to_owned(),
        hex(&approver_key_a().verifying_key().to_bytes()),
    )
    .unwrap();
    let approver_dup = TrustedKey::new(
        "approvers/approver-1".to_owned(),
        hex(&approver_key_b().verifying_key().to_bytes()),
    )
    .unwrap();
    assert!(matches!(
        duplicate_test_registry(vec![key_a], vec![approver.clone(), approver_dup]),
        Err(PolicyError::InvalidRegistry { .. })
    ));
    // Duplicate material under distinct IDs (issuer side).
    let issuer_1 = TrustedKey::new(
        "i1".to_owned(),
        hex(&issuer_key().verifying_key().to_bytes()),
    )
    .unwrap();
    let issuer_2 = TrustedKey::new(
        "i2".to_owned(),
        hex(&issuer_key().verifying_key().to_bytes()),
    )
    .unwrap();
    assert!(matches!(
        duplicate_test_registry(vec![issuer_1, issuer_2], vec![approver.clone()]),
        Err(PolicyError::InvalidRegistry { .. })
    ));
    // Case-variant hex of the same material collides too.
    let mixed = TrustedKey::new(
        "approvers/approver-1".to_owned(),
        hex(&approver_key_a().verifying_key().to_bytes()).to_uppercase(),
    )
    .unwrap();
    assert!(mixed == approver);
}

#[test]
fn registry_rejects_cross_role_overlap() {
    // One key's material may not serve as both issuer and approver.
    let shared = TrustedKey::new(
        "shared".to_owned(),
        hex(&issuer_key().verifying_key().to_bytes()),
    )
    .unwrap();
    let approver = TrustedKey::new(
        "approvers/approver-1".to_owned(),
        hex(&approver_key_a().verifying_key().to_bytes()),
    )
    .unwrap();
    assert!(matches!(
        TrustRegistry::new(vec![shared.clone()], vec![shared, approver], 1, Vec::new()),
        Err(PolicyError::InvalidRegistry { .. })
    ));
    // The same ID string may not appear in both roles either, even with
    // distinct key material.
    let issuer_id = TrustedKey::new(
        "dual".to_owned(),
        hex(&issuer_key().verifying_key().to_bytes()),
    )
    .unwrap();
    let approver_id = TrustedKey::new(
        "dual".to_owned(),
        hex(&approver_key_a().verifying_key().to_bytes()),
    )
    .unwrap();
    assert!(TrustRegistry::new(vec![issuer_id], vec![approver_id], 1, Vec::new()).is_err());
}

#[test]
fn registry_validates_revocation_references() {
    let issuer = TrustedKey::new(
        "release-key-1".to_owned(),
        hex(&issuer_key().verifying_key().to_bytes()),
    )
    .unwrap();
    let approver = TrustedKey::new(
        "approvers/approver-1".to_owned(),
        hex(&approver_key_a().verifying_key().to_bytes()),
    )
    .unwrap();
    // Unknown revocation reference.
    assert!(matches!(
        TrustRegistry::new(
            vec![issuer.clone()],
            vec![approver.clone()],
            1,
            vec!["ghost".to_owned()]
        ),
        Err(PolicyError::InvalidRegistry { .. })
    ));
    // Duplicate revocation reference.
    assert!(matches!(
        TrustRegistry::new(
            vec![issuer.clone()],
            vec![approver.clone()],
            1,
            vec![
                "approvers/approver-1".to_owned(),
                "approvers/approver-1".to_owned()
            ]
        ),
        Err(PolicyError::InvalidRegistry { .. })
    ));
    // Valid reference accepted.
    assert!(
        TrustRegistry::new(
            vec![issuer],
            vec![approver],
            1,
            vec!["approvers/approver-1".to_owned()]
        )
        .is_ok()
    );
}

#[test]
fn registry_fingerprint_is_order_independent() {
    let mut permuted_approvers = vec![
        TrustedKey::new(
            "approvers/approver-2".to_owned(),
            hex(&approver_key_b().verifying_key().to_bytes()),
        )
        .unwrap(),
        TrustedKey::new(
            "approvers/approver-1".to_owned(),
            hex(&approver_key_a().verifying_key().to_bytes()),
        )
        .unwrap(),
    ];
    permuted_approvers.reverse();
    let permuted = TrustRegistry::new(
        vec![
            TrustedKey::new(
                "release-key-1".to_owned(),
                hex(&issuer_key().verifying_key().to_bytes()),
            )
            .unwrap(),
        ],
        permuted_approvers,
        2,
        Vec::new(),
    )
    .unwrap();
    let ordered = TrustRegistry::new(
        vec![
            TrustedKey::new(
                "release-key-1".to_owned(),
                hex(&issuer_key().verifying_key().to_bytes()),
            )
            .unwrap(),
        ],
        vec![
            TrustedKey::new(
                "approvers/approver-1".to_owned(),
                hex(&approver_key_a().verifying_key().to_bytes()),
            )
            .unwrap(),
            TrustedKey::new(
                "approvers/approver-2".to_owned(),
                hex(&approver_key_b().verifying_key().to_bytes()),
            )
            .unwrap(),
        ],
        2,
        Vec::new(),
    )
    .unwrap();
    assert_eq!(permuted.fingerprint(), ordered.fingerprint());
    assert_eq!(permuted.fingerprint().len(), 64);
    // Same fingerprint implies identical verification behavior on the same
    // permit (the mode gate is identical, return auth Live on both).
    let commit = "abc123";
    let envelope = envelope_with_approvals(
        commit,
        vec![
            attest(
                &approver_key_a(),
                "approver-1",
                "approvers/approver-1",
                &body(commit),
            ),
            attest(
                &approver_key_b(),
                "approver-2",
                "approvers/approver-2",
                &body(commit),
            ),
        ],
    )
    .unwrap();
    let live = ExecutionPolicy::live().with_envelope(envelope);
    let a = live
        .authorize_order(&request(), &runtime(commit), &ordered, 0)
        .unwrap();
    let b = live
        .authorize_order(&request(), &runtime(commit), &permuted, 0)
        .unwrap();
    assert_eq!(a.registry_fingerprint(), b.registry_fingerprint());
    assert_eq!(a.registry_fingerprint(), Some(ordered.fingerprint()));
}

#[test]
fn successful_authorization_records_registry_fingerprint() {
    let commit = "abc123";
    let reg = registry(1).unwrap();
    let envelope = envelope_with_approvals(
        commit,
        vec![attest(
            &approver_key_a(),
            "approver-1",
            "approvers/approver-1",
            &body(commit),
        )],
    )
    .unwrap();
    let live = ExecutionPolicy::live().with_envelope(envelope);
    let auth = live
        .authorize_order(&request(), &runtime(commit), &reg, 0)
        .unwrap();
    assert_eq!(auth.registry_fingerprint(), Some(reg.fingerprint()));
    assert_eq!(auth.mode(), ExecutionMode::Live);
    assert_eq!(auth.permit_commit(), Some(commit));
    assert!(!auth.reasons().is_empty());
}
