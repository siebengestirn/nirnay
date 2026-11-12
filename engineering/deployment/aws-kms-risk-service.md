# AWS KMS deployment profile for Nirnay risk and dataset signing

**Status:** implementation adapter exists; no AWS resources are deployed by this repository. This document is a deployment specification, not evidence of a configured or certified service.

## 1. Key roles: do not reuse keys

Provision separate asymmetric signing keys for separate purposes:

- **Risk runtime signer:** signs `RuntimeEvidence` returned by the isolated risk service.
- **Dataset release signer:** signs the digest of a dataset publication manifest.
- **Offline trust-registry root:** maintained outside the application runtime and used to sign registry snapshots. Do not make this key available to the OMS, strategy process, market-data process or ordinary CI.
- **Release issuer and human approvers:** separate principals and key material, independent of the risk service and control-source authorities.

The current trust verifier requires distinct public keys and principal IDs across root, issuers, approvers, risk service and the eight control source roles. A different key ID referencing the same public key is not independent.

## 2. KMS key profile

For Ed25519 signing, use an asymmetric customer-managed KMS key with:

- `KeySpec = ECC_NIST_EDWARDS25519`;
- `KeyUsage = SIGN_VERIFY`;
- `SigningAlgorithm = ED25519_SHA_512`;
- key state `Enabled`;
- a dedicated AWS account/region selected by security/operations.

The adapter verifies metadata at construction, exports only the public key, calls KMS `Sign` with `MessageType=RAW`, and locally verifies every returned signature before returning it. The KMS API has raw-message size constraints; dataset signing therefore signs a domain-separated 32-byte SHA-256 digest of the canonical manifest, not the entire variable-length manifest.

Official references: [AWS KMS asymmetric key specs](https://docs.aws.amazon.com/kms/latest/developerguide/asymmetric-key-specs.html), [AWS KMS Sign API](https://docs.aws.amazon.com/kms/latest/APIReference/API_Sign.html), [AWS KMS GetPublicKey API](https://docs.aws.amazon.com/kms/latest/APIReference/API_GetPublicKey.html).

## 3. IAM and key policy

Create separate workload roles for the risk service and dataset publisher. The risk-service role should have only the minimum required KMS actions on the one key ARN:

- `kms:DescribeKey`;
- `kms:GetPublicKey`;
- `kms:Sign`.

It must not have `kms:CreateKey`, `kms:PutKeyPolicy`, `kms:ScheduleKeyDeletion`, broad `kms:*`, or access to release-issuer/approver keys. Restrict the KMS key policy so only the intended workload role can use this key for signing and a separately administered key-management role can change its policy/rotation/state. Dataset publisher gets its own key. Do not put AWS access keys in a repository, CI variable file, shell profile or Keychain; use short-lived workload identity (for example, an AWS role) in deployed environments.

The exact IAM conditions and key-policy principals depend on the AWS account, role ARNs, region and deployment service. They must be reviewed against current AWS documentation and tested with an explicit deny-by-default policy before deployment. This repository does not ship a generic permissive key policy because a copy/paste policy with placeholder principals is not safe to deploy.

## 4. Independent control sources

The risk service expects eight source roles and consumes each source sequence once. A failed signing operation burns the already-consumed observation batch; sources must issue fresh attestations rather than replay. The risk service expects eight source roles: `account_risk`, `kill_switch`, `market_data`, `reconciliation`, `broker_entitlement`, `trading_session`, `clock_sync`, and `deployed_artifact`. Each source:

- runs under a separate workload identity and signing key;
- signs the canonical versioned observation body with a domain-separated Ed25519 signature; each observation carries a strictly increasing `source_sequence` persisted by that source across restarts;
- scopes the evidence to the account/broker/instrument/venue/strategy/commit as appropriate;
- uses a short, role-specific validity period;
- is checked against the risk service's atomic observation replay ledger; the repository provides an in-memory test ledger and a single-node SQLite WAL/FULL ledger. A multi-instance deployment must use a transactional shared compare-and-set store with equivalent durable semantics;
- emits a revocation/health signal and is monitored independently.

Use authenticated service-to-service transport (for example, mTLS plus cloud workload identity), strict network policy, separate deployment permissions and independent data stores/authorities where appropriate. Do not have the strategy process manufacture these observations. A caller-supplied `true` flag is not an attestation.

**Implementation gap:** this repository defines and verifies the signed observation contract, but does not include deployed collectors for broker reconciliation, exchange session calendars, clock monitoring, entitlement records, kill-switch state, or production artifact attestations. A fake/test source is not a production source.

## 5. Trust-registry lifecycle

The registry is an offline-root-signed, expiring snapshot containing issuers, risk services, approvers and control-source public keys/roles/principals/revocation state. Operational process must define:

1. key creation and ownership approval;
2. public-key registration and independent role/principal review;
3. root signing ceremony with recorded participants and artifacts;
4. bounded registry lifetime and controlled distribution;
5. revocation and emergency rollover procedure;
6. historical retention of old registry versions for audit;
7. verifier behavior when registry is missing, expired, revoked or not yet distributed (fail closed).

Do not fetch a new trust registry from the same service whose evidence is being verified. A registry update is a security-sensitive release.

## 6. Audit, monitoring and operations

Before enabling any live authorization service, require:

- CloudTrail and centralized immutable audit retention for KMS key and IAM policy operations;
- alarms for `Sign` errors, denied calls, unexpected principals, key disablement and rotation/revocation events;
- metrics for observation age, missing roles, scope mismatches, clock offset, reconciliation breaks, unresolved gaps and permit expiry;
- no secrets, raw access tokens, full account identifiers, or unlicensed market data in logs;
- backup/export of public keys, trust registry and verification artifacts (never private key material);
- service deployment and key-policy review by a person independent of the strategy author;
- outage/compromise/restore drills, including a test that risk-service or KMS unavailability blocks authorization.

The `AwsKmsEd25519Signer` is a signing adapter. It does not deploy this infrastructure, collect risk facts, operate CloudTrail, or certify key management.
