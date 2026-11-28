//! A pure bridge from typed SERP *discovery* to proposed public page fetches,
//! then from claimed snapshot bytes to bounded candidate need observations.
//!
//! This does not authenticate DataForSEO, grant network access, authenticate a
//! website, attest a Temporal receipt, or validate a model's interpretation.
//! Provider output and snapshots remain untrusted until independently checked
//! by admitted source owners and exact Temporal History.

use std::collections::BTreeSet;

use dataforseo_contracts::{
    validate_discovery_shape_only, validate_parameters_only, DataForSeoGoogleOrganicSearchInputV1,
    DataForSeoGoogleOrganicSearchOutputV1, DataForSeoSourceStatusV1,
};
use operation_protocol::{owner_content_hash, RunScope};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use web_fetch_contracts::{
    validate_static_web_fetch_request_v1, WebFetchRequestV1, WebPageSnapshotV1,
    MAX_RESPONSE_BYTES_V1,
};

use super::evidence::{
    preview_consumer_need_evidence, ConsumerNeedEvidenceBatchV1, ConsumerNeedEvidenceError,
    ConsumerNeedEvidencePreviewV1, ConsumerNeedHypothesisV1, ConsumerNeedObservationV1,
    NeedEvidenceRepresentationV1, NeedEvidenceSourceV1, MAX_NEED_OBSERVATIONS,
    NEED_EVIDENCE_SCHEMA_REVISION_V1,
};

pub const MAX_BRIDGED_PAGES_V1: usize = 16;

/// The caller can request stricter limits; the web owner must separately
/// intersect the requested hosts with an operator-issued authoritative policy.
#[derive(Debug, Clone)]
pub struct SerpFetchPolicyV1 {
    pub requested_allowed_hosts: BTreeSet<String>,
    pub max_response_bytes: u32,
    pub max_redirects: u8,
    pub timeout_seconds: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SerpFetchCandidateStatusV1 {
    /// Suggested immutable request, NOT permission to fetch.
    RequiresIndependentOwnerAdmission,
    /// The result is not a safe, exact-host HTTPS candidate under this policy.
    RejectedStaticPolicy,
    /// Two SERP hits refer to the same exact URL, not independent evidence.
    DuplicateUrl,
}

#[derive(Debug, Clone, Serialize)]
pub struct SerpFetchCandidateV1 {
    pub rank: u32,
    pub source_url_hash: String,
    pub status: SerpFetchCandidateStatusV1,
    /// Never feed this into the runtime without independent owner admission.
    pub proposed_request: Option<WebFetchRequestV1>,
}

#[derive(Debug, Error)]
pub enum ResearchSourceBridgeErrorV1 {
    #[error("DataForSEO discovery result did not match the typed request")]
    InvalidDiscovery,
    #[error("page evidence batch exceeds 16 documents or 128 extracted observations")]
    TooManyPagesOrFindings,
    #[error("page snapshot tenant or source request does not match the research run")]
    ScopeMismatch,
    #[error("page source request or claimed snapshot URLs fail the static HTTPS host policy")]
    InvalidSourceUrl,
    #[error("page snapshot content length or bytes do not match the supplied document")]
    SnapshotMismatch,
    #[error("a claimed excerpt does not occur byte-for-byte in the supplied snapshot")]
    ExcerptNotInSnapshot,
    #[error("page source run/snapshot identity is missing or malformed")]
    InvalidSourceIdentity,
    #[error(transparent)]
    Evidence(#[from] ConsumerNeedEvidenceError),
}

/// A discovery hit is merely a proposed URL. Invalid/disallowed hits are
/// reported, not converted to fetch authorization. No DNS or network call.
pub fn preview_serp_fetch_candidates_v1(
    request: &DataForSeoGoogleOrganicSearchInputV1,
    discovery: &DataForSeoGoogleOrganicSearchOutputV1,
    policy: &SerpFetchPolicyV1,
) -> Result<Vec<SerpFetchCandidateV1>, ResearchSourceBridgeErrorV1> {
    if discovery.source_status != DataForSeoSourceStatusV1::SerpDiscoveryOnly {
        return Err(ResearchSourceBridgeErrorV1::InvalidDiscovery);
    }
    validate_parameters_only(request, &request.scope)
        .map_err(|_| ResearchSourceBridgeErrorV1::InvalidDiscovery)?;
    validate_discovery_shape_only(discovery, request)
        .map_err(|_| ResearchSourceBridgeErrorV1::InvalidDiscovery)?;
    let mut seen_urls = BTreeSet::new();
    let mut candidates = Vec::with_capacity(discovery.organic_results.len());
    for hit in &discovery.organic_results {
        let url_hash = owner_content_hash(hit.source_url.as_bytes());
        let repeated = !seen_urls.insert(url_hash.clone());
        let proposed_request = WebFetchRequestV1 {
            tenant_id: request.scope.tenant_id.clone(),
            source_url: hit.source_url.clone(),
            allowed_hosts: policy.requested_allowed_hosts.clone(),
            max_response_bytes: policy.max_response_bytes,
            max_redirects: policy.max_redirects,
            timeout_seconds: policy.timeout_seconds,
        };
        let matching_host = Url::parse(&hit.source_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| host.eq_ignore_ascii_case(&hit.domain_norm));
        let accepted = !repeated
            && matching_host
            && validate_static_web_fetch_request_v1(&proposed_request).is_ok();
        candidates.push(SerpFetchCandidateV1 {
            rank: hit.rank,
            source_url_hash: url_hash,
            status: if repeated {
                SerpFetchCandidateStatusV1::DuplicateUrl
            } else if accepted {
                SerpFetchCandidateStatusV1::RequiresIndependentOwnerAdmission
            } else {
                SerpFetchCandidateStatusV1::RejectedStaticPolicy
            },
            proposed_request: accepted.then_some(proposed_request),
        });
    }
    Ok(candidates)
}

/// A human/model-extracted assertion; exact excerpt bytes are checked against
/// the supplied raw document. This says nothing about excerpt relevance or
/// whether the speaker is a real consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicPageFindingV1 {
    pub observation_id: String,
    pub need_key: String,
    pub asserted_need: String,
    pub context: String,
    pub excerpt: String,
}

/// Raw source bytes deliberately live only in a borrowed, non-serializable
/// input; result and diagnostics never copy HTML or excerpt strings.
pub struct PublicPageEvidenceInputV1<'a> {
    pub request: &'a WebFetchRequestV1,
    pub snapshot: &'a WebPageSnapshotV1,
    pub document_bytes: &'a [u8],
    pub source_run_id: &'a str,
    pub findings: &'a [PublicPageFindingV1],
}

/// Assemble and group candidate evidence from actual bytes supplied by the
/// caller. Even a matching digest and excerpt are only local consistency:
/// ownership of the bytes and receipt still require a trusted fetch owner.
pub fn bind_public_page_findings_v1(
    scope: &RunScope,
    pages: &[PublicPageEvidenceInputV1<'_>],
    hypotheses: Vec<ConsumerNeedHypothesisV1>,
) -> Result<ConsumerNeedEvidencePreviewV1, ResearchSourceBridgeErrorV1> {
    if pages.is_empty()
        || pages.len() > MAX_BRIDGED_PAGES_V1
        || pages.iter().map(|page| page.findings.len()).sum::<usize>() > MAX_NEED_OBSERVATIONS
    {
        return Err(ResearchSourceBridgeErrorV1::TooManyPagesOrFindings);
    }
    let mut observations = Vec::new();
    for page in pages {
        let request = page.request;
        let snapshot = page.snapshot;
        if request.tenant_id != scope.tenant_id || snapshot.tenant_id != scope.tenant_id {
            return Err(ResearchSourceBridgeErrorV1::ScopeMismatch);
        }
        if request.source_url != snapshot.source_url
            || validate_static_web_fetch_request_v1(request).is_err()
        {
            return Err(ResearchSourceBridgeErrorV1::InvalidSourceUrl);
        }
        let final_request = WebFetchRequestV1 {
            source_url: snapshot.final_url.clone(),
            ..request.clone()
        };
        if validate_static_web_fetch_request_v1(&final_request).is_err()
            || (request.max_redirects == 0 && snapshot.source_url != snapshot.final_url)
        {
            return Err(ResearchSourceBridgeErrorV1::InvalidSourceUrl);
        }
        let final_host = Url::parse(&snapshot.final_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .ok_or(ResearchSourceBridgeErrorV1::InvalidSourceUrl)?;
        let source_run_id = page.source_run_id;
        if source_run_id.is_empty()
            || source_run_id.len() > 256
            || source_run_id.chars().any(char::is_control)
            || snapshot.snapshot_key.is_empty()
            || snapshot.snapshot_key.len() > 256
            || snapshot.snapshot_key.chars().any(char::is_control)
        {
            return Err(ResearchSourceBridgeErrorV1::InvalidSourceIdentity);
        }
        let bytes = page.document_bytes;
        let canonical_hash = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        if bytes.is_empty()
            || bytes.len() > usize::try_from(MAX_RESPONSE_BYTES_V1).unwrap_or(usize::MAX)
            || bytes.len() > usize::try_from(request.max_response_bytes).unwrap_or(usize::MAX)
            || u32::try_from(bytes.len()).ok() != Some(snapshot.body_bytes)
            || owner_content_hash(bytes) != snapshot.content_hash
            || !canonical_hash(&snapshot.robots_evidence_hash)
            || !canonical_hash(&snapshot.redirect_chain_hash)
        {
            return Err(ResearchSourceBridgeErrorV1::SnapshotMismatch);
        }
        let opaque_snapshot_id = owner_content_hash(snapshot.snapshot_key.as_bytes());
        let opaque_url_id = owner_content_hash(snapshot.final_url.as_bytes());
        for finding in page.findings {
            if finding.excerpt.is_empty()
                || !bytes
                    .windows(finding.excerpt.len())
                    .any(|candidate| candidate == finding.excerpt.as_bytes())
            {
                return Err(ResearchSourceBridgeErrorV1::ExcerptNotInSnapshot);
            }
            observations.push(ConsumerNeedObservationV1 {
                observation_id: finding.observation_id.clone(),
                need_key: finding.need_key.clone(),
                asserted_need: finding.asserted_need.clone(),
                context: finding.context.clone(),
                excerpt_hash: owner_content_hash(finding.excerpt.as_bytes()),
                excerpt: finding.excerpt.clone(),
                source: NeedEvidenceSourceV1 {
                    source_id: format!("web-snapshot:{opaque_snapshot_id}"),
                    tenant_id: scope.tenant_id.clone(),
                    source_run_id: source_run_id.to_owned(),
                    retrieval_channel: "web.public_page.fetch@1".into(),
                    representation: NeedEvidenceRepresentationV1::PrimaryDocument,
                    primary_origin_id: final_host.clone(),
                    primary_document_id: format!("url:{opaque_url_id}"),
                    provenance_locator: format!("web-snapshot:{opaque_snapshot_id}"),
                    document_payload_hash: snapshot.content_hash.clone(),
                },
            });
        }
    }
    preview_consumer_need_evidence(&ConsumerNeedEvidenceBatchV1 {
        schema_revision: NEED_EVIDENCE_SCHEMA_REVISION_V1,
        scope: scope.clone(),
        observations,
        hypotheses,
    })
    .map_err(Into::into)
}

#[cfg(test)]
mod tests;
