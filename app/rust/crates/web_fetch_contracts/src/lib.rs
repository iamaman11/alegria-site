//! An independently owned, versioned public-page fetch contract.
//!
//! This crate has no HTTP, DNS, robots, storage or Temporal client. Its pure
//! static validation is deliberately NOT owner ValidateOnly, network
//! permission, source authentication or evidence of a successful fetch.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{
    ArtifactSchema, ArtifactTypeId, EffectId, IdempotencyPolicy, Operation, OperationContract,
    OperationId, OperationOwnerId, OperationRef, PortId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::{Host, Url};

pub const OPERATION_ID_V1: &str = "web.public_page.fetch";
pub const OPERATION_REVISION_V1: u16 = 1;
pub const OWNER_ID_V1: &str = "web.public_fetch.owner";
pub const REQUEST_PORT_V1: &str = "request";
pub const SNAPSHOT_PORT_V1: &str = "snapshot";
pub const EFFECT_ID_V1: &str = "web.public_page.fetch_and_persist";
pub const MAX_RESPONSE_BYTES_V1: u32 = 2 * 1024 * 1024;
pub const MAX_REDIRECTS_V1: u8 = 5;
pub const MAX_TIMEOUT_SECONDS_V1: u8 = 30;
pub const MAX_ALLOWED_HOSTS_V1: usize = 16;

pub fn request_schema_v1() -> ArtifactSchema {
    ArtifactSchema {
        id: ArtifactTypeId("web.public_fetch_request".into()),
        revision: 1,
    }
}

pub fn snapshot_schema_v1() -> ArtifactSchema {
    ArtifactSchema {
        id: ArtifactTypeId("web.page_snapshot".into()),
        revision: 1,
    }
}

/// An immutable command request. Only a separately admitted owner may
/// resolve DNS, fetch robots, perform HTTPS, persist a snapshot, or issue a
/// Temporal receipt. Caller-listed hosts are untrusted desired constraints,
/// NOT the operator's independent allowlist; the owner must intersect both.
/// Hosts are exact DNS names, never wildcard patterns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebFetchRequestV1 {
    pub tenant_id: String,
    pub source_url: String,
    pub allowed_hosts: BTreeSet<String>,
    pub max_response_bytes: u32,
    pub max_redirects: u8,
    pub timeout_seconds: u8,
}

/// Small owner output: HTML itself remains in an independently admitted
/// immutable source store; raw content must not flood Temporal History.
/// These fields are declarations until corroborated by owner/History proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebPageSnapshotV1 {
    pub tenant_id: String,
    pub snapshot_key: String,
    pub source_url: String,
    pub final_url: String,
    pub content_hash: String,
    pub captured_at_utc: String,
    pub body_bytes: u32,
    pub robots_evidence_hash: String,
    pub redirect_chain_hash: String,
}

pub struct WebPublicPageFetchV1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebFetchPolicyError {
    #[error("tenant identity must be present and bounded")]
    InvalidTenant,
    #[error("URL must be a well-formed absolute HTTPS URL without credentials or fragment")]
    InvalidUrl,
    #[error("host must be a public DNS name, not an IP literal or local/internal name")]
    InvalidHost,
    #[error("allowed_hosts must contain only exact public DNS names")]
    InvalidAllowlist,
    #[error("URL host is not in the request's exact host list")]
    HostNotAllowed,
    #[error("fetch byte/redirect/timeout budgets must be bounded")]
    InvalidBudget,
    #[error("redirect exceeds the command's maximum hop count")]
    RedirectLimit,
}

impl Operation for WebPublicPageFetchV1 {
    type Input = WebFetchRequestV1;
    type Output = WebPageSnapshotV1;
    type Failure = WebFetchPolicyError;

    fn contract() -> OperationContract {
        OperationContract {
            reference: OperationRef {
                id: OperationId(OPERATION_ID_V1.into()),
                revision: u32::from(OPERATION_REVISION_V1),
            },
            owner: OperationOwnerId(OWNER_ID_V1.into()),
            inputs: BTreeMap::from([(PortId(REQUEST_PORT_V1.into()), request_schema_v1())]),
            outputs: BTreeMap::from([(PortId(SNAPSHOT_PORT_V1.into()), snapshot_schema_v1())]),
            invalidates: BTreeSet::new(),
            effects: BTreeSet::from([EffectId(EFFECT_ID_V1.into())]),
            // Until a durable owner receipt/transaction fence is physically
            // proven, ambiguous network/storage outcomes cannot be retried.
            idempotency: IdempotencyPolicy::AtMostOnce,
        }
    }
}

/// Honest, non-authorizing result of *pure static* policy validation.
#[derive(Debug, Clone, Serialize)]
pub struct WebFetchStaticPreviewV1 {
    pub status: &'static str,
    pub normalized_host: String,
    pub max_response_bytes: u32,
    pub max_redirects: u8,
    pub dns_and_robots_check: &'static str,
    pub source_authority: &'static str,
    pub owner_admission: &'static str,
    pub execution: &'static str,
}

fn valid_dns_host(host: &str) -> bool {
    if !host.is_ascii() || host.len() > 253 || host.ends_with('.') {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    if labels.iter().any(|label| {
        label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) {
        return false;
    }
    // Static protection for obvious local and non-public namespaces. This
    // does not prove the DNS answer is public: owner must re-check each hop,
    // including resolved addresses and DNS rebinding, at runtime.
    !matches!(
        labels.last().copied(),
        Some(
            "localhost"
                | "local"
                | "internal"
                | "lan"
                | "home"
                | "onion"
                | "test"
                | "invalid"
                | "example"
                | "arpa"
                | "corp"
        )
    )
}

fn checked_url_host(raw: &str) -> Result<(Url, String), WebFetchPolicyError> {
    if raw != raw.trim() || raw.contains('\\') || raw.chars().any(char::is_control) {
        return Err(WebFetchPolicyError::InvalidUrl);
    }
    let url = Url::parse(raw).map_err(|_| WebFetchPolicyError::InvalidUrl)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err(WebFetchPolicyError::InvalidUrl);
    }
    let host = match url.host() {
        Some(Host::Domain(host)) => host.to_ascii_lowercase(),
        _ => return Err(WebFetchPolicyError::InvalidHost),
    };
    if !valid_dns_host(&host) {
        return Err(WebFetchPolicyError::InvalidHost);
    }
    Ok((url, host))
}

fn checked_allowed_hosts(
    hosts: &BTreeSet<String>,
) -> Result<BTreeSet<String>, WebFetchPolicyError> {
    if hosts.is_empty() || hosts.len() > MAX_ALLOWED_HOSTS_V1 {
        return Err(WebFetchPolicyError::InvalidAllowlist);
    }
    let mut checked = BTreeSet::new();
    for host in hosts {
        let lower = host.to_ascii_lowercase();
        if !valid_dns_host(&lower) {
            return Err(WebFetchPolicyError::InvalidAllowlist);
        }
        checked.insert(lower);
    }
    Ok(checked)
}

pub fn validate_static_web_fetch_request_v1(
    request: &WebFetchRequestV1,
) -> Result<WebFetchStaticPreviewV1, WebFetchPolicyError> {
    if request.tenant_id.is_empty()
        || request.tenant_id.len() > 128
        || request.tenant_id.trim() != request.tenant_id
        || request.tenant_id.chars().any(char::is_control)
    {
        return Err(WebFetchPolicyError::InvalidTenant);
    }
    if request.max_response_bytes == 0
        || request.max_response_bytes > MAX_RESPONSE_BYTES_V1
        || request.max_redirects > MAX_REDIRECTS_V1
        || request.timeout_seconds == 0
        || request.timeout_seconds > MAX_TIMEOUT_SECONDS_V1
    {
        return Err(WebFetchPolicyError::InvalidBudget);
    }
    let allowed = checked_allowed_hosts(&request.allowed_hosts)?;
    let (_, host) = checked_url_host(&request.source_url)?;
    if !allowed.contains(&host) {
        return Err(WebFetchPolicyError::HostNotAllowed);
    }
    Ok(WebFetchStaticPreviewV1 {
        status: "static_policy_only_not_admitted",
        normalized_host: host,
        max_response_bytes: request.max_response_bytes,
        max_redirects: request.max_redirects,
        dns_and_robots_check: "pending_owner_runtime",
        source_authority: "not_authenticated",
        owner_admission: "not_issued",
        execution: "not_started",
    })
}

/// Apply the same exact-host policy to *every* discovered redirect target.
/// The future owner must independently check fresh public DNS answers, TLS,
/// robots on each origin/path and bytes *before* following any redirect.
pub fn validate_static_redirect_hop_v1(
    request: &WebFetchRequestV1,
    target_url: &str,
    completed_hops: u8,
) -> Result<(), WebFetchPolicyError> {
    validate_static_web_fetch_request_v1(request)?;
    if completed_hops >= request.max_redirects {
        return Err(WebFetchPolicyError::RedirectLimit);
    }
    let (_, target_host) = checked_url_host(target_url)?;
    let allowed = checked_allowed_hosts(&request.allowed_hosts)?;
    if !allowed.contains(&target_host) {
        return Err(WebFetchPolicyError::HostNotAllowed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> WebFetchRequestV1 {
        WebFetchRequestV1 {
            tenant_id: "tenant-a".into(),
            source_url: "https://www.example.com/reviews?q=kitchen".into(),
            allowed_hosts: BTreeSet::from(["www.example.com".into()]),
            max_response_bytes: 256 * 1024,
            max_redirects: 2,
            timeout_seconds: 10,
        }
    }

    #[test]
    fn contract_is_versioned_and_never_declares_pure_or_automatic_retry() {
        let contract = WebPublicPageFetchV1::contract();
        assert_eq!(contract.reference.id.as_str(), OPERATION_ID_V1);
        assert_eq!(contract.reference.revision, 1);
        assert_eq!(contract.owner.as_str(), OWNER_ID_V1);
        assert_eq!(contract.idempotency, IdempotencyPolicy::AtMostOnce);
        assert_eq!(
            contract.inputs[&PortId::from(REQUEST_PORT_V1)],
            request_schema_v1()
        );
        assert_eq!(
            contract.outputs[&PortId::from(SNAPSHOT_PORT_V1)],
            snapshot_schema_v1()
        );
        assert!(contract.effects.contains(&EffectId::from(EFFECT_ID_V1)));
    }

    #[test]
    fn static_preview_cannot_claim_network_or_robots_attestation() {
        let accepted = validate_static_web_fetch_request_v1(&request()).unwrap();
        assert_eq!(accepted.normalized_host, "www.example.com");
        assert_eq!(accepted.status, "static_policy_only_not_admitted");
        assert_eq!(accepted.dns_and_robots_check, "pending_owner_runtime");
        assert_eq!(accepted.source_authority, "not_authenticated");
        assert_eq!(accepted.owner_admission, "not_issued");
        assert_eq!(accepted.execution, "not_started");
    }

    #[test]
    fn deny_local_private_literal_userinfo_and_unsupported_urls() {
        for url in [
            "http://www.example.com/",
            "ftp://www.example.com/",
            "file:///etc/passwd",
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://192.168.1.10/",
            "https://169.254.169.254/latest/meta-data/",
            "https://localhost/",
            "https://service.internal/",
            "https://printer.local/",
            "https://name:token@www.example.com/",
            "https://www.example.com:8443/",
            "https://www.example.com/#section",
            "https://127.1/",
            " https://www.example.com/",
            "https://www.example.com/\\@elsewhere.net",
        ] {
            let mut candidate = request();
            candidate.source_url = url.into();
            assert!(
                validate_static_web_fetch_request_v1(&candidate).is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn exact_allowlist_rejects_wildcards_suffix_confusion_and_missing_hosts() {
        for hosts in [
            BTreeSet::new(),
            BTreeSet::from(["*.example.com".into()]),
            BTreeSet::from(["example.com".into()]),
            BTreeSet::from(["www.example.com.evil.net".into()]),
            BTreeSet::from(["www.example.com:443".into()]),
            BTreeSet::from(["localhost".into()]),
        ] {
            let mut candidate = request();
            candidate.allowed_hosts = hosts;
            assert!(validate_static_web_fetch_request_v1(&candidate).is_err());
        }
    }

    #[test]
    fn bounded_budgets_and_redirect_targets_fail_closed() {
        for (bytes, hops, timeout) in [
            (0, 1, 5),
            (MAX_RESPONSE_BYTES_V1 + 1, 1, 5),
            (1024, MAX_REDIRECTS_V1 + 1, 5),
            (1024, 1, 0),
            (1024, 1, MAX_TIMEOUT_SECONDS_V1 + 1),
        ] {
            let mut candidate = request();
            candidate.max_response_bytes = bytes;
            candidate.max_redirects = hops;
            candidate.timeout_seconds = timeout;
            assert_eq!(
                validate_static_web_fetch_request_v1(&candidate).unwrap_err(),
                WebFetchPolicyError::InvalidBudget
            );
        }
        let req = request();
        assert!(validate_static_redirect_hop_v1(&req, "https://www.example.com/next", 0).is_ok());
        assert_eq!(
            validate_static_redirect_hop_v1(&req, "https://www.example.com/next", 2).unwrap_err(),
            WebFetchPolicyError::RedirectLimit
        );
        for target in [
            "https://private.example.net/",
            "https://localhost/",
            "https://[::1]/",
            "http://www.example.com/redirect",
            "https://www.example.com.evil.net/",
        ] {
            assert!(validate_static_redirect_hop_v1(&req, target, 0).is_err());
        }
    }
}
