//! Short-lived integrity proof from the OAuth-verified Cloudflare resource
//! Worker to a separately credentialed native admission operator.
//!
//! HMAC verifies *which configured ingress wrote these exact bytes*, not that
//! a real ChatGPT user signed in. The Worker must first validate the OAuth
//! Bearer token, scope and props, and the native operator must separately
//! compare the provisioned user/tenant/SQL grant and inspect private intent.
//! Neither a valid HMAC nor a UUID is a Temporal Start capability alone.

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

pub const SIGNED_MCP_ADMISSION_REVISION_V1: u16 = 1;
pub const SIGNED_MCP_ADMISSION_TTL_SECONDS_V1: i64 = 30;
pub const SIGNED_MCP_ADMISSION_CLOCK_SKEW_SECONDS_V1: i64 = 5;
const SIGNED_MCP_ADMISSION_MAX_BYTES_V1: usize = 2048;
const SCOPE: &str = "site_architect.run_one";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedOneStepAdmissionV1 {
    pub schema_revision: u16,
    pub request_id: String,
    pub issuer: String,
    pub audience: String,
    pub subject: String,
    pub scope: String,
    pub issued_at_unix_seconds: i64,
    pub expires_at_unix_seconds: i64,
    pub nonce: String,
    pub signature_hex: String,
}

/// Only exact verified values are visible to a native caller; no tenant,
/// principal, role, policy or grant can be self-asserted by the envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSignedOneStepAdmissionV1 {
    pub request_id: String,
    pub issuer: String,
    pub audience: String,
    pub subject: String,
    pub nonce: String,
}

#[derive(Debug, Error)]
pub enum SignedMcpAdmissionErrorV1 {
    #[error("signed MCP admission malformed or not canonical")]
    InvalidWire,
    #[error("signed MCP admission differs from configured issuer, audience, subject or request")]
    Mismatch,
    #[error("signed MCP admission expired or not yet valid")]
    NotActive,
    #[error("signed MCP admission signature not verified")]
    InvalidSignature,
}

fn hex_bytes(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|part| {
            let hex = std::str::from_utf8(part).ok()?;
            u8::from_str_radix(hex, 16).ok()
        })
        .collect()
}

/// For private operator-owned key files only. Never take key material from MCP.
pub fn decode_admission_signing_key_v1(
    value: &[u8],
) -> Result<[u8; 32], SignedMcpAdmissionErrorV1> {
    let raw = std::str::from_utf8(value).map_err(|_| SignedMcpAdmissionErrorV1::InvalidWire)?;
    let bytes = hex_bytes(raw).ok_or(SignedMcpAdmissionErrorV1::InvalidWire)?;
    bytes
        .try_into()
        .map_err(|_| SignedMcpAdmissionErrorV1::InvalidWire)
}

fn canonical_uuid(value: &str, v4_only: bool) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|uuid| {
        !uuid.is_nil() && uuid.to_string() == value && (!v4_only || uuid.get_version_num() == 4)
    })
}

fn bounded_https(value: &str, expect_mcp: bool) -> bool {
    value.len() <= 512
        && value.is_ascii()
        && value.starts_with("https://")
        && value.bytes().all(|b| b.is_ascii_graphic())
        && !value.contains('@')
        && !value.contains('?')
        && !value.contains('#')
        && (!expect_mcp || value.ends_with("/mcp"))
}

fn bounded_subject(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

fn canonical_message(value: &SignedOneStepAdmissionV1) -> String {
    format!(
        "site-architect-admission-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        value.request_id,
        value.issuer,
        value.audience,
        value.subject,
        value.scope,
        value.issued_at_unix_seconds,
        value.expires_at_unix_seconds,
        value.nonce,
    )
}

fn sign(value: &SignedOneStepAdmissionV1, key: &[u8; 32]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("fixed 32-byte HMAC key");
    mac.update(canonical_message(value).as_bytes());
    let digest = mac.finalize().into_bytes();
    let mut result = [0u8; 32];
    result.copy_from_slice(&digest);
    result
}

/// Fixture helper: possession of a test key does not authenticate a user.
/// Production admissions must be signed only by the separately verified OAuth
/// resource Worker after its exact-scope check.
pub fn seal_one_step_admission_for_private_fixture_v1(
    request_id: &str,
    issuer: &str,
    audience: &str,
    subject: &str,
    issued_at_unix_seconds: i64,
    nonce: &str,
    key: &[u8; 32],
) -> SignedOneStepAdmissionV1 {
    let mut record = SignedOneStepAdmissionV1 {
        schema_revision: SIGNED_MCP_ADMISSION_REVISION_V1,
        request_id: request_id.into(),
        issuer: issuer.into(),
        audience: audience.into(),
        subject: subject.into(),
        scope: SCOPE.into(),
        issued_at_unix_seconds,
        expires_at_unix_seconds: issued_at_unix_seconds + SIGNED_MCP_ADMISSION_TTL_SECONDS_V1,
        nonce: nonce.into(),
        signature_hex: String::new(),
    };
    record.signature_hex = sign(&record, key)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    record
}

pub fn verify_signed_one_step_admission_v1(
    bytes: &[u8],
    key: &[u8; 32],
    expected_issuer: &str,
    expected_audience: &str,
    installed_subject: &str,
    expected_request_id: &str,
    now_unix_seconds: i64,
) -> Result<VerifiedSignedOneStepAdmissionV1, SignedMcpAdmissionErrorV1> {
    if bytes.is_empty() || bytes.len() > SIGNED_MCP_ADMISSION_MAX_BYTES_V1 {
        return Err(SignedMcpAdmissionErrorV1::InvalidWire);
    }
    let record: SignedOneStepAdmissionV1 =
        serde_json::from_slice(bytes).map_err(|_| SignedMcpAdmissionErrorV1::InvalidWire)?;
    let canonical =
        serde_json::to_vec(&record).map_err(|_| SignedMcpAdmissionErrorV1::InvalidWire)?;
    if canonical.as_slice() != bytes
        || record.schema_revision != SIGNED_MCP_ADMISSION_REVISION_V1
        || !canonical_uuid(&record.request_id, false)
        || !canonical_uuid(&record.nonce, true)
        || !bounded_https(&record.issuer, false)
        || !bounded_https(&record.audience, true)
        || !bounded_subject(&record.subject)
        || record.scope != SCOPE
    {
        return Err(SignedMcpAdmissionErrorV1::InvalidWire);
    }
    if record.issuer != expected_issuer
        || record.audience != expected_audience
        || record.subject != installed_subject
        || record.request_id != expected_request_id
    {
        return Err(SignedMcpAdmissionErrorV1::Mismatch);
    }
    if record.issued_at_unix_seconds <= 0
        || record
            .expires_at_unix_seconds
            .checked_sub(record.issued_at_unix_seconds)
            != Some(SIGNED_MCP_ADMISSION_TTL_SECONDS_V1)
        || now_unix_seconds
            < record
                .issued_at_unix_seconds
                .saturating_sub(SIGNED_MCP_ADMISSION_CLOCK_SKEW_SECONDS_V1)
        || now_unix_seconds >= record.expires_at_unix_seconds
    {
        return Err(SignedMcpAdmissionErrorV1::NotActive);
    }
    let bytes =
        hex_bytes(&record.signature_hex).ok_or(SignedMcpAdmissionErrorV1::InvalidSignature)?;
    if bytes.len() != 32 {
        return Err(SignedMcpAdmissionErrorV1::InvalidSignature);
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("fixed 32-byte HMAC key");
    mac.update(canonical_message(&record).as_bytes());
    mac.verify_slice(&bytes)
        .map_err(|_| SignedMcpAdmissionErrorV1::InvalidSignature)?;
    Ok(VerifiedSignedOneStepAdmissionV1 {
        request_id: record.request_id,
        issuer: record.issuer,
        audience: record.audience,
        subject: record.subject,
        nonce: record.nonce,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (SignedOneStepAdmissionV1, [u8; 32]) {
        let key = [0xab; 32];
        let record = seal_one_step_admission_for_private_fixture_v1(
            "d54eb078-c5e9-49ba-bc6e-c3a91fa778ca",
            "https://auth.example.org",
            "https://mcp.example.org/mcp",
            "verified-human-123",
            1_900_000_000,
            "c62d0fe6-49a4-4b15-9ced-1dc3e0c6300d",
            &key,
        );
        (record, key)
    }

    fn verify(
        record: &SignedOneStepAdmissionV1,
        key: &[u8; 32],
        now: i64,
    ) -> Result<VerifiedSignedOneStepAdmissionV1, SignedMcpAdmissionErrorV1> {
        verify_signed_one_step_admission_v1(
            &serde_json::to_vec(record).unwrap(),
            key,
            "https://auth.example.org",
            "https://mcp.example.org/mcp",
            "verified-human-123",
            "d54eb078-c5e9-49ba-bc6e-c3a91fa778ca",
            now,
        )
    }

    #[test]
    fn valid_signed_exact_scope_and_timing() {
        let (record, key) = sample();
        // Independent Node Web Crypto HMAC-SHA256 fixture: proves a shared
        // Worker/native byte contract, not merely self-consistent Rust crypto.
        assert_eq!(
            record.signature_hex,
            "c02cd955c775580df128b99f73873c1443b19575b8f4e552f5b53e4d1d70eaea"
        );
        assert_eq!(
            verify(&record, &key, 1_900_000_015).unwrap().request_id,
            record.request_id
        );
        assert!(matches!(
            verify(&record, &key, 1_900_000_030),
            Err(SignedMcpAdmissionErrorV1::NotActive)
        ));
        assert!(matches!(
            verify(&record, &key, 1_899_999_994),
            Err(SignedMcpAdmissionErrorV1::NotActive)
        ));
    }

    #[test]
    fn tampered_subject_scope_uuid_signature_and_key_refused() {
        let (record, key) = sample();
        let mut changed = record.clone();
        changed.subject = "forged-user".into();
        assert!(verify(&changed, &key, 1_900_000_015).is_err());
        let mut changed = record.clone();
        changed.scope = "site_architect.run_one other".into();
        assert!(verify(&changed, &key, 1_900_000_015).is_err());
        let mut changed = record.clone();
        changed.nonce = "00000000-0000-0000-0000-000000000000".into();
        assert!(verify(&changed, &key, 1_900_000_015).is_err());
        let mut changed = record.clone();
        changed.signature_hex = "00".repeat(32);
        assert!(verify(&changed, &key, 1_900_000_015).is_err());
        assert!(verify(&record, &[0xcd; 32], 1_900_000_015).is_err());
        assert!(decode_admission_signing_key_v1(b"abc").is_err());
        assert_eq!(
            decode_admission_signing_key_v1("ab".repeat(32).as_bytes()).unwrap(),
            key
        );
    }

    #[test]
    fn noncanonical_json_and_extra_fields_refused() {
        let (record, key) = sample();
        let json = serde_json::to_vec(&record).unwrap();
        let mut spaced = json.clone();
        spaced.push(b'\n');
        assert!(verify_signed_one_step_admission_v1(
            &spaced,
            &key,
            &record.issuer,
            &record.audience,
            &record.subject,
            &record.request_id,
            1_900_000_015
        )
        .is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        value["tenant_id"] = serde_json::json!("forged");
        assert!(verify_signed_one_step_admission_v1(
            &serde_json::to_vec(&value).unwrap(),
            &key,
            &record.issuer,
            &record.audience,
            &record.subject,
            &record.request_id,
            1_900_000_015
        )
        .is_err());
    }
}
