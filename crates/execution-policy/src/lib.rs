//! Central execution-mode policy (task P0-06, approval-authority revision).
//!
//! Live authority is cryptographically enforced through **independent
//! attestations**, not an issuer-signed list of names:
//!
//! - The issuer signs the permit body (commit, account, broker, strategy,
//!   instruments, limits, expiry) with the release key.
//! - Every approver separately signs the issuer's permit digest with their
//!   own Ed25519 key. Attestations bind to the issuer, the permit digest,
//!   and the expiry.
//! - Verification requires one distinct valid attestation per counted
//!   approval, from a registered approver key. Duplicate identities and
//!   duplicate keys are rejected; revocation is checked on every call.
//! - Registry configurations with a threshold above the registered approver
//!   count are rejected as malformed.
//!
//! Construction security: a carried envelope never grants authority. Every
//! authorization re-verifies the issuer signature and every attestation.
//! The runtime clock, commit identity and projected exposure come from the
//! trusted runtime and risk authorities, never from the strategy.
//!
//! Enforcement boundary and its limits:
//!
//! - Mode: only an explicitly Live-configured policy can reach permit
//!   verification. Paper and Simulated policies deny Live authorization even
//!   when they carry a valid permit; a permit never changes the mode.
//! - Scope: a permit authorizes only its account, broker, strategy,
//!   instrument allowlist, and financial limits. The permit schema has no
//!   venue/action/side fields, so those dimensions are NOT enforced —
//!   permissive assumptions about them are forbidden.
//! - Time: a permit is expired at its expiry instant. Unrepresentable
//!   timestamps fail closed; they are never replaced with a fallback.
//! - Limits: a request exactly at a limit is allowed; above denies.
//! - Revocation: the registry supplied by the authority-owning application
//!   carries revocations. If that snapshot is stale or unavailable, the
//!   deployment — not this crate — owns re-fetching it; this crate only
//!   evaluates the registry it is given. Production use requires a fresh
//!   snapshot (a separate risk authority decision, not yet built).
//! - Authorization is not reservation: this crate evaluates policy on the
//!   state it is given. Atomic exposure reservation, idempotency, and
//!   reconciliation belong to the independent risk authority, which does
//!   not exist yet. Do not describe this crate as an independent RMS.

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Policy encoding version. Any change to the canonical encoding bumps this.
pub const ENCODING_VERSION: u8 = 2;

/// Execution mode: distinct types enforced at the type level. `Live`
/// authority exists only while a cryptographically verified permit and its
/// approval attestations are valid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Broker simulator or internal paper broker. No real orders.
    #[default]
    Paper,
    /// Replay/simulated venue against recorded data. No broker connection.
    Simulated,
    /// Real order placement. Gated by a verified live permit.
    Live,
}

/// Order context for one authorization request. Supplied by the OMS; the
/// strategy can only propose these fields, the policy validates them
/// against the permit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub broker: String,
    pub account: String,
    pub strategy: String,
    pub instrument: String,
    pub notional: i64,
}

/// The permit body: everything the issuer signs. Excludes signatures and
/// attestations (those sign this digest).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermitBody {
    pub commit_sha: String,
    pub account: String,
    pub broker: String,
    pub strategy: String,
    pub instruments: Vec<String>,
    pub max_order_notional: i64,
    pub max_open_exposure: i64,
    pub operator: String,
    pub expires_at_utc: DateTime<Utc>,
    pub issuer_key_id: String,
}

/// One approver's independent attestation: the approver signs the issuer's
/// permit digest, binding the approval to the exact permit body, the
/// issuer, and the expiry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalAttestation {
    /// Approver identity (registry lookup key).
    pub approver_id: String,
    /// Key identity of the approver's registered key.
    pub approver_key_id: String,
    /// Hex Ed25519 signature over [`ApprovalAttestation::attestation_payload`].
    pub signature: String,
}

/// A live permit envelope: issuer signature over the body, plus one
/// independent attestation per distinct approver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermitEnvelope {
    pub body: PermitBody,
    /// Hex Ed25519 signature by the issuer over the canonical body payload.
    pub issuer_signature: String,
    pub approvals: Vec<ApprovalAttestation>,
}

/// Error for every fail-closed path. Authorization fails with a typed
/// category so callers can distinguish mode, permit, scope, input, and
/// timestamp denials; no variant leaks secret material.
#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("configured execution mode {mode:?} does not authorize live orders")]
    ModeMismatch { mode: ExecutionMode },
    #[error("live execution requires a permit; none is present")]
    NoPermit,
    #[error("permit expired at {0}")]
    Expired(DateTime<Utc>),
    #[error("permit was issued for commit {issued}, current commit is {current}")]
    CommitMismatch { issued: String, current: String },
    #[error("permit does not cover broker {broker}")]
    BrokerMismatch { broker: String },
    #[error("permit does not cover strategy {strategy}")]
    StrategyMismatch { strategy: String },
    #[error("permit does not cover account {account}")]
    AccountMismatch { account: String },
    #[error("instrument {instrument} is not in the permit allowlist")]
    InstrumentNotPermitted { instrument: String },
    #[error("order notional {requested} exceeds permit limit {allowed}")]
    NotionalExceeded { requested: i64, allowed: i64 },
    #[error("exposure {requested} exceeds permit limit {allowed}")]
    ExposureExceeded { requested: i64, allowed: i64 },
    #[error("order notional must be positive, got {value}")]
    InvalidNotional { value: i64 },
    #[error("projected exposure must be non-negative, got {value}")]
    InvalidExposure { value: i64 },
    #[error("permit financial limits are invalid: {detail}")]
    InvalidPermitLimits { detail: String },
    #[error("timestamp outside the supported canonical range: {detail}")]
    InvalidTimestamp { detail: String },
    #[error("invalid trust registry configuration: {detail}")]
    InvalidRegistry { detail: String },
    #[error("unknown issuer key ID {0}")]
    UnknownIssuer(String),
    #[error("approver key id {0} is not in the trusted registry")]
    UnknownApproverKey(String),
    #[error("approver identity {0} is not registered")]
    UntrustedApprover(String),
    #[error("signature is malformed: {0}")]
    MalformedSignature(String),
    #[error("issuer signature failed Ed25519 verification: forged or corrupted")]
    ForgedIssuerSignature,
    #[error("approval attestation for {approver} failed Ed25519 verification")]
    ForgedApproval { approver: String },
    #[error("approver key {0} has been revoked")]
    RevokedApprover(String),
    #[error("duplicate approver identity in permit: {0}")]
    DuplicateApprover(String),
    #[error("duplicate approver key in permit: {0}")]
    DuplicateApproverKey(String),
    #[error("permit requires {required} distinct approvals, found {found}")]
    InsufficientApprovals { required: usize, found: usize },
    #[error("approvals list is empty; at least {required} distinct attestations required")]
    NoApprovals { required: usize },
    #[error(
        "trust registry requires {required} approvals but has only {registered} registered approver keys"
    )]
    RegistryThresholdExceedsApprovers { required: usize, registered: usize },
}

fn push_field(payload: &mut Vec<u8>, value: &[u8]) {
    payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
    payload.extend_from_slice(value);
}

fn push_u64(payload: &mut Vec<u8>, value: u64) {
    payload.extend_from_slice(&value.to_le_bytes());
}

/// Canonical nanosecond representation of a permit expiry. Fallible: an
/// out-of-range timestamp is a denial, never a fallback timestamp or a
/// saturated value. Representation matches the historically signed bytes
/// for every in-range timestamp; only prior garbage-producing timestamps
/// now fail closed.
fn expiry_nanos(expiry: DateTime<Utc>) -> Result<u64, PolicyError> {
    expiry
        .timestamp_nanos_opt()
        .map(|nanos| nanos as u64)
        .ok_or_else(|| PolicyError::InvalidTimestamp {
            detail: format!("expiry {expiry} is outside the nanosecond range"),
        })
}

impl PermitBody {
    /// Versioned, unambiguous canonical encoding: ENCODING_VERSION byte,
    /// then length-prefixed fields. Empty strings and embedded NULs are
    /// representable without ambiguity. The byte layout is unchanged from
    /// v2 for every representable value; only the fallible timestamp
    /// conversion differs (previously saturated, now denied).
    pub fn canonical_payload(&self) -> Result<Vec<u8>, PolicyError> {
        let mut payload = vec![ENCODING_VERSION];
        push_field(&mut payload, self.commit_sha.as_bytes());
        push_field(&mut payload, self.account.as_bytes());
        push_field(&mut payload, self.broker.as_bytes());
        push_field(&mut payload, self.strategy.as_bytes());
        push_u64(&mut payload, self.instruments.len() as u64);
        for instrument in &self.instruments {
            push_field(&mut payload, instrument.as_bytes());
        }
        push_u64(&mut payload, self.max_order_notional as u64);
        push_u64(&mut payload, self.max_open_exposure as u64);
        push_field(&mut payload, self.operator.as_bytes());
        push_u64(&mut payload, expiry_nanos(self.expires_at_utc)?);
        push_field(&mut payload, self.issuer_key_id.as_bytes());
        Ok(payload)
    }

    /// SHA-256 digest of the canonical payload: what approvers attest.
    pub fn digest(&self) -> Result<[u8; 32], PolicyError> {
        Ok(Sha256::digest(self.canonical_payload()?).into())
    }
}

impl ApprovalAttestation {
    /// Attestation payload: encoding version + approval domain tag, the
    /// permit digest, issuer key id, expiry, and the approver identity/key.
    /// Binds each approval to the exact permit body, the issuer, and the
    /// lifetime so attestations cannot be replayed elsewhere.
    pub fn attestation_payload(&self, body: &PermitBody) -> Result<Vec<u8>, PolicyError> {
        let mut payload = vec![ENCODING_VERSION, 0xAA];
        payload.extend_from_slice(&body.digest()?);
        push_field(&mut payload, body.issuer_key_id.as_bytes());
        push_u64(&mut payload, expiry_nanos(body.expires_at_utc)?);
        push_field(&mut payload, self.approver_id.as_bytes());
        push_field(&mut payload, self.approver_key_id.as_bytes());
        Ok(payload)
    }
}

/// Trust registry: issuer keys, approver keys, minimum approvals, and
/// revocation list. Loaded from protected configuration; never constructed
/// from strategy input.
///
/// Authority boundary: fields are private and construction goes through
/// [`TrustRegistry::new`], which enforces shape at load time. This type is
/// deliberately not deserializable: untrusted bytes must never become a
/// registry. Authorization re-verifies every signature and revocation
/// check against the registry instance the authority-owning application
/// supplied, not against strategy-provided state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRegistry {
    /// Release keys: issuer_key_id -> Ed25519 verifying key (hex, 32 bytes).
    issuers: Vec<TrustedKey>,
    /// Approver keys: approver_key_id -> Ed25519 verifying key. The approver
    /// identity is the key_id suffix after the final '/' (or the whole id).
    approvers: Vec<TrustedKey>,
    /// Minimum distinct valid approval attestations required.
    minimum_approvals: usize,
    /// List of approver key IDs that have been revoked.
    revoked_approvers: Vec<String>,
    /// Canonical fingerprint computed at construction (and on revocation).
    fingerprint: String,
}

impl TrustRegistry {
    /// Build a registry from protected configuration. This is the only
    /// construction path: duplicate IDs, duplicate or cross-role key
    /// material, unknown or duplicate revocation references, and invalid
    /// thresholds are all rejected here, and the configuration is
    /// canonicalized (sorted) so order never affects meaning.
    pub fn new(
        issuers: Vec<TrustedKey>,
        approvers: Vec<TrustedKey>,
        minimum_approvals: usize,
        revoked_approvers: Vec<String>,
    ) -> Result<Self, PolicyError> {
        let mut registry = Self {
            issuers,
            approvers,
            minimum_approvals,
            revoked_approvers,
            fingerprint: String::new(),
        };
        registry.validate()?;
        registry.canonicalize();
        registry.fingerprint = registry.compute_fingerprint();
        Ok(registry)
    }

    /// Canonical SHA-256 fingerprint of this registry (domain-separated,
    /// order-independent hex). Recorded with authorization evidence so the
    /// trust root in force for each decision is auditable.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    fn canonicalize(&mut self) {
        self.issuers.sort_by(|a, b| a.key_id.cmp(&b.key_id));
        self.approvers.sort_by(|a, b| a.key_id.cmp(&b.key_id));
        self.revoked_approvers.sort();
    }

    fn validate(&self) -> Result<(), PolicyError> {
        let invalid = |detail: String| PolicyError::InvalidRegistry { detail };

        // ID uniqueness.
        let mut issuer_ids = std::collections::BTreeSet::new();
        for key in &self.issuers {
            if !issuer_ids.insert(key.key_id.clone()) {
                return Err(invalid(format!("duplicate issuer key ID {}", key.key_id)));
            }
        }
        let mut approver_ids = std::collections::BTreeSet::new();
        for key in &self.approvers {
            if !approver_ids.insert(key.key_id.clone()) {
                return Err(invalid(format!("duplicate approver key ID {}", key.key_id)));
            }
        }

        // Material uniqueness (decoded bytes, format-independent).
        let mut issuer_materials = std::collections::BTreeSet::new();
        for key in &self.issuers {
            if !issuer_materials.insert(key.material()) {
                return Err(invalid(format!(
                    "duplicate issuer key material {}",
                    key.key_id
                )));
            }
        }
        let mut approver_materials = std::collections::BTreeSet::new();
        for key in &self.approvers {
            if !approver_materials.insert(key.material()) {
                return Err(invalid(format!(
                    "duplicate approver key material {}",
                    key.key_id
                )));
            }
        }

        // One key may not silently serve as both issuer and approver.
        if !issuer_materials.is_disjoint(&approver_materials) {
            return Err(invalid(
                "a key may not serve as both issuer and approver".to_owned(),
            ));
        }
        if !issuer_ids.is_disjoint(&approver_ids) {
            return Err(invalid(
                "a key ID may not appear as both issuer and approver".to_owned(),
            ));
        }

        // Revocation references must name registered approvers, once each.
        let mut seen_revoked = std::collections::BTreeSet::new();
        for key_id in &self.revoked_approvers {
            if !approver_ids.contains(key_id) {
                return Err(invalid(format!(
                    "revocation reference {key_id} is not a registered approver key"
                )));
            }
            if !seen_revoked.insert(key_id.clone()) {
                return Err(invalid(format!("duplicate revocation reference {key_id}")));
            }
        }

        // Shape: issuer/approver presence and threshold.
        if self.issuers.is_empty() {
            return Err(invalid("trust registry has no issuers".to_owned()));
        }
        if self.approvers.len() < self.minimum_approvals {
            return Err(PolicyError::RegistryThresholdExceedsApprovers {
                required: self.minimum_approvals,
                registered: self.approvers.len(),
            });
        }
        Ok(())
    }

    fn compute_fingerprint(&self) -> String {
        fn push_bytes(hasher: &mut Sha256, bytes: &[u8]) {
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        let mut hasher = Sha256::new();
        hasher.update(b"registry-v1\0");
        hasher.update((self.minimum_approvals as u64).to_le_bytes());
        for key in &self.issuers {
            push_bytes(&mut hasher, key.key_id.as_bytes());
            hasher.update(key.material());
        }
        for key in &self.approvers {
            push_bytes(&mut hasher, key.key_id.as_bytes());
            hasher.update(key.material());
        }
        for key_id in &self.revoked_approvers {
            push_bytes(&mut hasher, key_id.as_bytes());
        }
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Revoke an approver by key ID. Returns true if the approver was found and revoked.
    #[must_use]
    pub fn revoke(&mut self, key_id: &str) -> bool {
        if self.approvers.iter().any(|k| k.key_id == key_id) {
            if !self.revoked_approvers.iter().any(|r| r == key_id) {
                self.revoked_approvers.push(key_id.to_owned());
                self.revoked_approvers.sort();
                self.fingerprint = self.compute_fingerprint();
            }
            true
        } else {
            false
        }
    }

    /// Whether an approver key ID has been revoked.
    #[must_use]
    pub fn is_approver_revoked(&self, key_id: &str) -> bool {
        self.revoked_approvers.iter().any(|r| r == key_id)
    }

    fn issuer_key(&self, key_id: &str) -> Result<VerifyingKey, PolicyError> {
        let trusted = self
            .issuers
            .iter()
            .find(|key| key.key_id == key_id)
            .ok_or_else(|| PolicyError::UnknownIssuer(key_id.to_owned()))?;
        let bytes = decode_hex::<32>(&trusted.public_key_hex, "issuer key")?;
        VerifyingKey::from_bytes(&bytes).map_err(|error| {
            PolicyError::MalformedSignature(format!("issuer key rejected: {error}"))
        })
    }

    fn approver_key(&self, key_id: &str) -> Result<VerifyingKey, PolicyError> {
        let trusted = self
            .approvers
            .iter()
            .find(|key| key.key_id == key_id)
            .ok_or_else(|| PolicyError::UnknownApproverKey(key_id.to_owned()))?;
        let bytes = decode_hex::<32>(&trusted.public_key_hex, "approver key")?;
        VerifyingKey::from_bytes(&bytes).map_err(|error| {
            PolicyError::MalformedSignature(format!("approver key rejected: {error}"))
        })
    }

    #[must_use]
    pub fn approver_identity_registered(&self, approver_id: &str) -> bool {
        self.approvers
            .iter()
            .any(|key| approver_identity(&key.key_id) == approver_id)
    }

    pub fn validate_shape(&self) -> Result<(), PolicyError> {
        if self.issuers.is_empty() {
            return Err(PolicyError::MalformedSignature(
                "trust registry has no issuers".to_owned(),
            ));
        }
        if self.approvers.len() < self.minimum_approvals {
            return Err(PolicyError::RegistryThresholdExceedsApprovers {
                required: self.minimum_approvals,
                registered: self.approvers.len(),
            });
        }
        Ok(())
    }
}

/// A registered public key. Fields are private; construction is the
/// authority's job, so untrusted input cannot pose as a trusted key.
/// Every key is grammar-checked, hex-decoded, canonicalized, and Ed25519
/// validated at construction: invalid key material fails here, never at
/// verification time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedKey {
    key_id: String,
    /// Canonical (lowercase) hex of the 32-byte Ed25519 verifying key.
    public_key_hex: String,
    /// Decoded key material, parsed and validated at construction.
    material: [u8; 32],
}

impl TrustedKey {
    /// Register a key from protected configuration. The key ID and the key
    /// material are validated now, so a registry built from constructed
    /// keys cannot contain malformed entries.
    pub fn new(key_id: String, public_key_hex: String) -> Result<Self, PolicyError> {
        validate_key_id(&key_id)?;
        let material = decode_hex::<32>(&public_key_hex, "public key")?;
        let verified =
            VerifyingKey::from_bytes(&material).map_err(|error| PolicyError::InvalidRegistry {
                detail: format!("public key for {key_id} failed Ed25519 validation: {error}"),
            })?;
        let encoded = verified
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join("");
        Ok(Self {
            key_id,
            public_key_hex: encoded,
            material,
        })
    }

    /// The decoded 32-byte Ed25519 verifying key material.
    #[must_use]
    pub fn material(&self) -> [u8; 32] {
        self.material
    }
}

/// Registry configuration is validated at construction. The canonical
/// fingerprint is the SHA-256 of a domain-separated, order-independent
/// byte form, so equivalent configurations in any input order produce the
/// same fingerprint and identical verification behavior.
/// Key-ID grammar for trust-registry registrations: 1..=128 chars of ASCII
/// alphanumerics and `./_-`, arranged in slash-separated non-empty segments.
fn validate_key_id(key_id: &str) -> Result<(), PolicyError> {
    let invalid = || PolicyError::InvalidRegistry {
        detail: format!("invalid key ID {key_id:?}"),
    };
    if key_id.is_empty() || key_id.len() > 128 {
        return Err(invalid());
    }
    if key_id.starts_with('/') || key_id.ends_with('/') || key_id.contains("//") {
        return Err(invalid());
    }
    if !key_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
    {
        return Err(invalid());
    }
    Ok(())
}

fn decode_hex<const N: usize>(value: &str, what: &str) -> Result<[u8; N], PolicyError> {
    if !value.len().is_multiple_of(2) {
        return Err(PolicyError::MalformedSignature(format!(
            "{what} is not valid hex"
        )));
    }
    let raw = (0..value.len() / 2)
        .map(|index| u8::from_str_radix(&value[index * 2..index * 2 + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| PolicyError::MalformedSignature(format!("{what} is not valid hex")))?;
    raw.try_into()
        .map_err(|_| PolicyError::MalformedSignature(format!("{what} is not {N} bytes")))
}

/// The approver identity for a registered key: the suffix after the final
/// '/' in the key id, or the whole key id when it has no separator.
#[must_use]
pub fn approver_identity(key_id: &str) -> &str {
    key_id.rsplit('/').next().unwrap_or(key_id)
}

/// The central policy object. Only paper/simulated authority can exist
/// without a permit. A carried envelope has zero authority: every
/// authorization re-derives authority from cryptographic verification.
///
/// Authority boundary: this type is deliberately not deserializable. A
/// deserialized policy could otherwise set `mode = "live"` from untrusted
/// bytes; mode is established only through the explicit constructors
/// below, and Live additionally requires a verified permit on every call.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ExecutionPolicy {
    mode: ExecutionMode,
    /// Carried envelope; authority is re-derived on every use.
    envelope: Option<PermitEnvelope>,
}

/// Runtime facts supplied by trusted authorities, not by the strategy.
/// Fields are private so untrusted bytes cannot pose as trusted state.
/// Construction is the authority-owning application's responsibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedRuntime {
    /// Exact commit of the running binary, from the build/release system.
    commit_sha: String,
    /// Trusted current time (RTC or time authority).
    now_utc: DateTime<Utc>,
}

impl TrustedRuntime {
    /// Build trusted runtime facts from authority-owned sources only
    /// (build/release system for the commit, time authority for the clock).
    #[must_use]
    pub fn new(commit_sha: String, now_utc: DateTime<Utc>) -> Self {
        Self {
            commit_sha,
            now_utc,
        }
    }
}

impl ExecutionPolicy {
    /// Default: paper execution, no permit, no live capability.
    #[must_use]
    pub fn default_paper() -> Self {
        Self {
            mode: ExecutionMode::Paper,
            envelope: None,
        }
    }

    /// Simulated mode against recorded data; still no live capability.
    #[must_use]
    pub fn simulated() -> Self {
        Self {
            mode: ExecutionMode::Simulated,
            envelope: None,
        }
    }

    /// Explicit Live mode. This alone authorizes nothing: every order
    /// additionally requires a verified permit under a trusted registry.
    #[must_use]
    pub fn live() -> Self {
        Self {
            mode: ExecutionMode::Live,
            envelope: None,
        }
    }

    /// Carry a permit envelope for later verification. The mode is
    /// preserved exactly: attaching a permit never changes execution mode
    /// and never grants authority (every authorization re-derives it from
    /// cryptographic verification). A permit attached to a Paper or
    /// Simulated policy still denies Live orders.
    #[must_use]
    pub fn with_envelope(mut self, envelope: PermitEnvelope) -> Self {
        self.envelope = Some(envelope);
        self
    }

    #[must_use]
    pub const fn mode(&self) -> ExecutionMode {
        self.mode
    }

    #[must_use]
    pub const fn envelope(&self) -> Option<&PermitEnvelope> {
        self.envelope.as_ref()
    }

    /// The single gate every order must pass before reaching an adapter.
    ///
    /// `trusted_runtime` supplies the commit identity and clock from the
    /// runtime authority; `projected_exposure` comes from the independent
    /// risk service. A strategy can propose the [`OrderRequest`]; it cannot
    /// vouch for the environment, the registry, or its own approval.
    pub fn authorize_order(
        &self,
        request: &OrderRequest,
        trusted_runtime: &TrustedRuntime,
        registry: &TrustRegistry,
        projected_exposure: i64,
    ) -> Result<Authorization, PolicyError> {
        // 1. Execution mode is authoritative. A carried permit NEVER upgrades
        //    the configured mode: Paper and Simulated policies deny Live
        //    authorization regardless of what envelope they carry.
        if self.mode != ExecutionMode::Live {
            return Err(PolicyError::ModeMismatch { mode: self.mode });
        }

        // 2. Financial inputs are validated before any limit comparison.
        //    Zero/negative notional and negative projected exposure deny.
        //    Exact numeric types only; no float, no saturating arithmetic.
        if request.notional <= 0 {
            return Err(PolicyError::InvalidNotional {
                value: request.notional,
            });
        }
        if projected_exposure < 0 {
            return Err(PolicyError::InvalidExposure {
                value: projected_exposure,
            });
        }

        // 3. Live mode with no carried permit denies.
        let Some(envelope) = self.envelope.as_ref() else {
            return Err(PolicyError::NoPermit);
        };

        // 4. Re-verify the permit and its scope on every order. A carried
        //    envelope grants nothing by itself.
        verify_envelope(envelope, registry, trusted_runtime)?;
        let body = &envelope.body;
        if body.broker != request.broker {
            return Err(PolicyError::BrokerMismatch {
                broker: request.broker.clone(),
            });
        }
        if body.strategy != request.strategy {
            return Err(PolicyError::StrategyMismatch {
                strategy: request.strategy.clone(),
            });
        }
        if body.account != request.account {
            return Err(PolicyError::AccountMismatch {
                account: request.account.clone(),
            });
        }
        if !body.instruments.iter().any(|i| i == &request.instrument) {
            return Err(PolicyError::InstrumentNotPermitted {
                instrument: request.instrument.clone(),
            });
        }
        // Boundary (documented policy choice): a request exactly at a permit
        // limit is allowed; anything above denies.
        if request.notional > body.max_order_notional {
            return Err(PolicyError::NotionalExceeded {
                requested: request.notional,
                allowed: body.max_order_notional,
            });
        }
        if projected_exposure > body.max_open_exposure {
            return Err(PolicyError::ExposureExceeded {
                requested: projected_exposure,
                allowed: body.max_open_exposure,
            });
        }
        Ok(Authorization {
            mode: ExecutionMode::Live,
            permit_commit: Some(body.commit_sha.clone()),
            reasons: vec![format!(
                "verified Ed25519 permit {} with {} distinct approvals for account {}",
                body.issuer_key_id,
                envelope.approvals.len(),
                body.account
            )],
            registry_fingerprint: Some(registry.fingerprint().to_owned()),
        })
    }

    /// Authorize an order under the configured non-Live mode (Paper or
    /// Simulated). A carried envelope is rejected here: it cannot change
    /// the execution mode. Live orders must go through [`Self::authorize_order`].
    pub fn authorize_non_live(&self) -> Result<Authorization, PolicyError> {
        if self.mode == ExecutionMode::Live {
            return Err(PolicyError::ModeMismatch {
                mode: ExecutionMode::Live,
            });
        }
        if self.envelope.is_some() {
            return Err(PolicyError::ModeMismatch { mode: self.mode });
        }
        Ok(Authorization {
            mode: self.mode,
            permit_commit: None,
            reasons: vec![format!(
                "non-live execution under {:?} policy; no live authority",
                self.mode
            )],
            registry_fingerprint: None,
        })
    }
}

fn verify_envelope(
    envelope: &PermitEnvelope,
    registry: &TrustRegistry,
    runtime: &TrustedRuntime,
) -> Result<(), PolicyError> {
    registry.validate_shape()?;
    let body = &envelope.body;
    // Permit financial limits must themselves be in-range: a non-positive
    // order cap or a negative exposure cap cannot authorize anything.
    // (Scope note: the permit schema carries account, broker, strategy,
    // instruments, and limits. It has no venue, action, or side fields, so
    // those dimensions are not enforced — the crate does not assume
    // universal scope it cannot express.)
    if body.max_order_notional <= 0 {
        return Err(PolicyError::InvalidPermitLimits {
            detail: format!(
                "max_order_notional must be positive, got {}",
                body.max_order_notional
            ),
        });
    }
    if body.max_open_exposure < 0 {
        return Err(PolicyError::InvalidPermitLimits {
            detail: format!(
                "max_open_exposure must be non-negative, got {}",
                body.max_open_exposure
            ),
        });
    }
    // Boundary (documented policy choice): a permit is expired AT its expiry
    // instant; anything at or after expiry denies.
    if runtime.now_utc >= body.expires_at_utc {
        return Err(PolicyError::Expired(body.expires_at_utc));
    }
    if body.commit_sha != runtime.commit_sha {
        return Err(PolicyError::CommitMismatch {
            issued: body.commit_sha.clone(),
            current: runtime.commit_sha.clone(),
        });
    }

    // Issuer signature over the canonical body payload. Any payload-encoding
    // failure (e.g. unrepresentable expiry) fails closed before verification.
    let issuer_key = registry.issuer_key(&body.issuer_key_id)?;
    let issuer_signature_bytes = decode_hex::<64>(&envelope.issuer_signature, "issuer signature")?;
    let issuer_signature = Signature::from_bytes(&issuer_signature_bytes);
    let payload = body.canonical_payload()?;
    issuer_key
        .verify(&payload, &issuer_signature)
        .map_err(|_| PolicyError::ForgedIssuerSignature)?;

    // Approvals: distinct identities, distinct keys, individually signed.
    if envelope.approvals.is_empty() {
        return Err(PolicyError::NoApprovals {
            required: registry.minimum_approvals,
        });
    }
    for attestation in &envelope.approvals {
        if registry.is_approver_revoked(&attestation.approver_key_id) {
            return Err(PolicyError::RevokedApprover(
                attestation.approver_key_id.clone(),
            ));
        }
    }
    let mut seen_identities: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut seen_keys: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut valid_attestations = 0usize;
    for attestation in &envelope.approvals {
        let verifying_key = registry.approver_key(&attestation.approver_key_id)?;
        // The attestation identity must match the identity registered for
        // that key: an attacker cannot borrow another approver's key id.
        if attestation.approver_id != approver_identity(&attestation.approver_key_id) {
            return Err(PolicyError::UntrustedApprover(
                attestation.approver_id.clone(),
            ));
        }
        // Duplicate identities and duplicate keys are rejected outright:
        // counting entries would let one approver satisfy a multi-approval
        // threshold.
        if !seen_identities.insert(attestation.approver_id.as_str()) {
            return Err(PolicyError::DuplicateApprover(
                attestation.approver_id.clone(),
            ));
        }
        if !seen_keys.insert(attestation.approver_key_id.as_str()) {
            return Err(PolicyError::DuplicateApproverKey(
                attestation.approver_key_id.clone(),
            ));
        }
        let signature_bytes = decode_hex::<64>(&attestation.signature, "approval signature")?;
        let signature = Signature::from_bytes(&signature_bytes);
        let signed = attestation.attestation_payload(body)?;
        verifying_key
            .verify(&signed, &signature)
            .map_err(|_| PolicyError::ForgedApproval {
                approver: attestation.approver_id.clone(),
            })?;
        valid_attestations += 1;
    }
    if valid_attestations < registry.minimum_approvals {
        return Err(PolicyError::InsufficientApprovals {
            required: registry.minimum_approvals,
            found: valid_attestations,
        });
    }
    Ok(())
}

/// The decision record. Bound to the permit commit so a policy change can
/// never silently authorize outstanding orders.
///
/// Opaque by construction: fields are private, there is no public
/// constructor, and the type is not serializable. The only way to obtain
/// one is a successful [`ExecutionPolicy::authorize_order`] or
/// [`ExecutionPolicy::authorize_non_live`] call, so a downstream consumer
/// can never be handed a caller-fabricated proof. The authority-owning
/// runtime must own the policy, registry, runtime facts, and this decision;
/// request handlers must not accept a caller-supplied registry or
/// authorization as proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorization {
    mode: ExecutionMode,
    permit_commit: Option<String>,
    reasons: Vec<String>,
    registry_fingerprint: Option<String>,
}

impl Authorization {
    /// The execution mode granted by this decision.
    #[must_use]
    pub fn mode(&self) -> ExecutionMode {
        self.mode
    }

    /// The permit commit this Live decision is bound to (`None` for
    /// non-live decisions).
    #[must_use]
    pub fn permit_commit(&self) -> Option<&str> {
        self.permit_commit.as_deref()
    }

    /// Human-readable grant details recorded at decision time.
    #[must_use]
    pub fn reasons(&self) -> &[String] {
        &self.reasons
    }

    /// Canonical fingerprint of the trust registry behind this decision
    /// (`None` on non-live evaluations, which consult no registry).
    #[must_use]
    pub fn registry_fingerprint(&self) -> Option<&str> {
        self.registry_fingerprint.as_deref()
    }
}
