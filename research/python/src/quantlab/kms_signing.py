"""AWS KMS Ed25519 signing backend for future independent risk services.

Private key material never leaves AWS KMS. This adapter validates key metadata,
exports only the public key, signs raw domain-separated payloads and locally
verifies every returned signature before returning it. IAM and key policies
must still be independently reviewed in deployment.
"""
from __future__ import annotations

import base64
from dataclasses import dataclass
from typing import Any, Protocol


class KmsSigningError(RuntimeError):
    pass


class KmsClient(Protocol):
    def describe_key(self, *, KeyId: str) -> dict[str, Any]: ...
    def get_public_key(self, *, KeyId: str) -> dict[str, Any]: ...
    def sign(self, **kwargs: Any) -> dict[str, Any]: ...


@dataclass(frozen=True, slots=True)
class KmsKeyIdentity:
    key_id: str
    arn: str
    key_spec: str
    key_usage: str
    key_state: str
    signing_algorithm: str


class AwsKmsEd25519Signer:
    KEY_SPEC = "ECC_NIST_EDWARDS25519"
    ALGORITHM = "ED25519_SHA_512"

    def __init__(self, client: KmsClient, key_id: str):
        if not isinstance(key_id, str) or not key_id.strip():
            raise ValueError("AWS KMS key id/ARN is required")
        self._client = client
        self.key_id = key_id
        self.identity = self._validate_metadata()
        self._public_key = self._load_public_key()

    @classmethod
    def from_default_aws_credentials(cls, key_id: str, *, region_name: str | None = None) -> "AwsKmsEd25519Signer":
        try:
            import boto3
        except ImportError as exc:
            raise KmsSigningError("install research extra 'kms' to use AWS KMS signing") from exc
        return cls(boto3.client("kms", region_name=region_name), key_id)

    def _validate_metadata(self) -> KmsKeyIdentity:
        try:
            metadata = self._client.describe_key(KeyId=self.key_id)["KeyMetadata"]
        except Exception as exc:
            raise KmsSigningError(f"AWS KMS DescribeKey failed: {type(exc).__name__}") from exc
        if metadata.get("KeySpec") != self.KEY_SPEC:
            raise KmsSigningError(f"KMS key must use {self.KEY_SPEC}")
        if metadata.get("KeyUsage") != "SIGN_VERIFY":
            raise KmsSigningError("KMS key usage must be SIGN_VERIFY")
        if metadata.get("KeyState") != "Enabled":
            raise KmsSigningError("KMS signing key is not Enabled")
        if metadata.get("KeyManager") not in {"CUSTOMER", None}:
            raise KmsSigningError("only customer-managed KMS keys are accepted")
        arn = metadata.get("Arn")
        if not isinstance(arn, str) or not arn:
            raise KmsSigningError("KMS DescribeKey omitted key ARN")
        return KmsKeyIdentity(self.key_id, arn, metadata["KeySpec"], metadata["KeyUsage"],
                              metadata["KeyState"], self.ALGORITHM)

    def _load_public_key(self) -> bytes:
        try:
            from cryptography.hazmat.primitives.serialization import load_der_public_key, Encoding, PublicFormat
            der = self._client.get_public_key(KeyId=self.key_id)["PublicKey"]
            key = load_der_public_key(der)
            raw = key.public_bytes(Encoding.Raw, PublicFormat.Raw)
        except Exception as exc:
            raise KmsSigningError(f"KMS public key is not a valid Ed25519 key: {type(exc).__name__}") from exc
        if len(raw) != 32:
            raise KmsSigningError("KMS Ed25519 public key must be 32 bytes")
        return raw

    @property
    def public_key_b64(self) -> str:
        return base64.b64encode(self._public_key).decode("ascii")

    def sign(self, message: bytes) -> bytes:
        if not isinstance(message, bytes) or not message or len(message) > 4096:
            raise ValueError("KMS raw Ed25519 message must be 1..4096 bytes")
        try:
            result = self._client.sign(
                KeyId=self.key_id, Message=message, MessageType="RAW",
                SigningAlgorithm=self.ALGORITHM,
            )
            signature = result["Signature"]
        except Exception as exc:
            raise KmsSigningError(f"AWS KMS Sign failed: {type(exc).__name__}") from exc
        if not isinstance(signature, bytes) or len(signature) != 64:
            raise KmsSigningError("AWS KMS returned an invalid Ed25519 signature")
        self.verify(message, signature)
        return signature

    def sign_b64(self, message: bytes) -> str:
        return base64.b64encode(self.sign(message)).decode("ascii")

    def verify(self, message: bytes, signature: bytes) -> None:
        try:
            from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
            Ed25519PublicKey.from_public_bytes(self._public_key).verify(signature, message)
        except Exception as exc:
            raise KmsSigningError("KMS signature failed local Ed25519 verification") from exc
