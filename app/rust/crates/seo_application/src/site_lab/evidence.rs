//! Pure, non-authorizing grouping of consumer-need observations.
//!
//! This module does not fetch pages, infer needs, trust caller-supplied URLs,
//! or verify owner receipts. Source IDs here are *claims* awaiting separate
//! owner/source attestation. In particular, a SERP snippet and the page it
//! describes are two representations of one primary document, not two sources.

use std::collections::{BTreeMap, BTreeSet};

use operation_protocol::{owner_content_hash, RunScope};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const NEED_EVIDENCE_SCHEMA_REVISION_V1: u16 = 1;
pub const MAX_NEED_OBSERVATIONS: usize = 128;
pub const MAX_NEED_HYPOTHESES: usize = 32;

/// Provider-neutral representation. Only a direct primary document can
/// contribute to the structural two-origin candidate threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedEvidenceRepresentationV1 {
    PrimaryDocument,
    SearchSnippet,
    DerivedSummary,
    MirroredDocument,
}

/// The original publisher and document, NOT the aggregator, search engine,
/// crawler host, or vector-store endpoint. A trusted owner must authenticate
/// these claims and the document snapshot separately before any Truth claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeedEvidenceSourceV1 {
    pub source_id: String,
    pub tenant_id: String,
    pub source_run_id: String,
    pub retrieval_channel: String,
    pub representation: NeedEvidenceRepresentationV1,
    pub primary_origin_id: String,
    pub primary_document_id: String,
    pub provenance_locator: String,
    /// Claimed BLAKE3 of the original fetched document bytes, not the snippet.
    pub document_payload_hash: String,
}

/// A bounded assertion extracted from source material by a future owner.
/// Its excerpt digest is recomputed locally, but this does not establish that
/// the excerpt actually came from the claimed document or is representative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerNeedObservationV1 {
    pub observation_id: String,
    pub need_key: String,
    pub asserted_need: String,
    pub context: String,
    pub excerpt: String,
    pub excerpt_hash: String,
    pub source: NeedEvidenceSourceV1,
}

/// A proposed site change is deliberately separate from observations.
/// Nothing in this DTO asserts that the site change will improve outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerNeedHypothesisV1 {
    pub need_key: String,
    pub proposed_site_change: String,
    pub expected_benefit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumerNeedEvidenceBatchV1 {
    pub schema_revision: u16,
    pub scope: RunScope,
    pub observations: Vec<ConsumerNeedObservationV1>,
    pub hypotheses: Vec<ConsumerNeedHypothesisV1>,
}

/// A result can never be "confirmed" by pure structural grouping alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerNeedEvidenceStatusV1 {
    InsufficientEvidence,
    MultiSourceCandidateUnattested,
}

/// Safe reference to the original source without copying its page or excerpt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsumerNeedEvidencePointerV1 {
    pub observation_id: String,
    pub source_id: String,
    pub primary_origin_id: String,
    pub primary_document_id: String,
    pub document_payload_hash: String,
    pub excerpt_hash: String,
    pub provenance_locator: String,
    pub representation: NeedEvidenceRepresentationV1,
}

#[derive(Debug, Serialize)]
pub struct ConsumerNeedAssessmentV1 {
    pub hypothesis: ConsumerNeedHypothesisV1,
    pub status: ConsumerNeedEvidenceStatusV1,
    pub observations: Vec<ConsumerNeedEvidencePointerV1>,
    /// Two direct primary documents with different claimed original publisher,
    /// primary document and original document hash. Empty when insufficient.
    pub structurally_independent_observation_ids: Vec<String>,
    pub source_authentication: &'static str,
    pub owner_receipt_attestation: &'static str,
    pub execution: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ConsumerNeedEvidencePreviewV1 {
    pub schema_revision: u16,
    pub status: &'static str,
    pub assessments: Vec<ConsumerNeedAssessmentV1>,
    pub note: &'static str,
}

#[derive(Debug, Error)]
pub enum ConsumerNeedEvidenceError {
    #[error("unsupported consumer evidence schema revision")]
    UnsupportedRevision,
    #[error("consumer evidence batch is empty or exceeds size limits")]
    InvalidBatchSize,
    #[error("consumer evidence contains missing or oversized identifiers/text")]
    InvalidField,
    #[error("consumer evidence has an invalid document or excerpt digest")]
    InvalidHash,
    #[error("consumer observation IDs must be unique")]
    DuplicateObservation,
    #[error("a primary document is claimed under inconsistent source identities")]
    ConflictingPrimaryDocument,
    #[error("consumer evidence crosses tenant boundaries")]
    ScopeMismatch,
    #[error("hypotheses must have distinct need keys")]
    DuplicateHypothesis,
}

fn bounded(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum
}

fn blake3_shape(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn pointer(observation: &ConsumerNeedObservationV1) -> ConsumerNeedEvidencePointerV1 {
    ConsumerNeedEvidencePointerV1 {
        observation_id: observation.observation_id.clone(),
        source_id: observation.source.source_id.clone(),
        primary_origin_id: observation.source.primary_origin_id.clone(),
        primary_document_id: observation.source.primary_document_id.clone(),
        document_payload_hash: observation.source.document_payload_hash.clone(),
        excerpt_hash: observation.excerpt_hash.clone(),
        provenance_locator: observation.source.provenance_locator.clone(),
        representation: observation.source.representation,
    }
}

/// Deterministic structural grouping. The two-source threshold never treats
/// a search index, mirrored page or derived model summary as a primary source.
/// Caller-supplied source identities are still untrusted until owner validation.
pub fn preview_consumer_need_evidence(
    batch: &ConsumerNeedEvidenceBatchV1,
) -> Result<ConsumerNeedEvidencePreviewV1, ConsumerNeedEvidenceError> {
    if batch.schema_revision != NEED_EVIDENCE_SCHEMA_REVISION_V1 {
        return Err(ConsumerNeedEvidenceError::UnsupportedRevision);
    }
    if batch.observations.len() > MAX_NEED_OBSERVATIONS
        || batch.hypotheses.is_empty()
        || batch.hypotheses.len() > MAX_NEED_HYPOTHESES
        || !bounded(&batch.scope.tenant_id, 256)
        || !bounded(&batch.scope.run_id, 256)
    {
        return Err(ConsumerNeedEvidenceError::InvalidBatchSize);
    }

    let mut observation_ids = BTreeSet::new();
    let mut document_origins: BTreeMap<&str, &str> = BTreeMap::new();
    let mut source_identities: BTreeMap<&str, (&str, &str, &str)> = BTreeMap::new();
    let mut by_need: BTreeMap<&str, Vec<&ConsumerNeedObservationV1>> = BTreeMap::new();
    for observation in &batch.observations {
        let source = &observation.source;
        if source.tenant_id != batch.scope.tenant_id {
            return Err(ConsumerNeedEvidenceError::ScopeMismatch);
        }
        if !bounded(&observation.observation_id, 256)
            || !bounded(&observation.need_key, 256)
            || !bounded(&observation.asserted_need, 2048)
            || !bounded(&observation.context, 2048)
            || !bounded(&observation.excerpt, 4096)
            || !bounded(&source.source_id, 256)
            || !bounded(&source.source_run_id, 256)
            || !bounded(&source.retrieval_channel, 256)
            || !bounded(&source.primary_origin_id, 256)
            || !bounded(&source.primary_document_id, 2048)
            || !bounded(&source.provenance_locator, 2048)
        {
            return Err(ConsumerNeedEvidenceError::InvalidField);
        }
        if !blake3_shape(&source.document_payload_hash)
            || !blake3_shape(&observation.excerpt_hash)
            || owner_content_hash(observation.excerpt.as_bytes()) != observation.excerpt_hash
        {
            return Err(ConsumerNeedEvidenceError::InvalidHash);
        }
        if !observation_ids.insert(&observation.observation_id) {
            return Err(ConsumerNeedEvidenceError::DuplicateObservation);
        }
        let source_identity = (
            source.primary_origin_id.as_str(),
            source.primary_document_id.as_str(),
            source.document_payload_hash.as_str(),
        );
        if let Some(previous_identity) =
            source_identities.insert(source.source_id.as_str(), source_identity)
        {
            if previous_identity != source_identity {
                return Err(ConsumerNeedEvidenceError::ConflictingPrimaryDocument);
            }
        }
        if let Some(previous_origin) =
            document_origins.insert(&source.primary_document_id, &source.primary_origin_id)
        {
            if previous_origin != &source.primary_origin_id {
                return Err(ConsumerNeedEvidenceError::ConflictingPrimaryDocument);
            }
        }
        by_need.entry(&observation.need_key).or_default().push(observation);
    }

    let mut hypothesis_keys = BTreeSet::new();
    let mut assessments = Vec::with_capacity(batch.hypotheses.len());
    for hypothesis in &batch.hypotheses {
        if !bounded(&hypothesis.need_key, 256)
            || !bounded(&hypothesis.proposed_site_change, 4096)
            || !bounded(&hypothesis.expected_benefit, 2048)
        {
            return Err(ConsumerNeedEvidenceError::InvalidField);
        }
        if !hypothesis_keys.insert(&hypothesis.need_key) {
            return Err(ConsumerNeedEvidenceError::DuplicateHypothesis);
        }
        let mut relevant = by_need.get(hypothesis.need_key.as_str()).cloned().unwrap_or_default();
        relevant.sort_by(|first, second| first.observation_id.cmp(&second.observation_id));
        let primary: Vec<_> = relevant
            .iter()
            .copied()
            .filter(|observation| {
                observation.source.representation == NeedEvidenceRepresentationV1::PrimaryDocument
            })
            .collect();
        let independent_pair = primary.iter().enumerate().find_map(|(index, first)| {
            primary[index + 1..].iter().find_map(|second| {
                let first_source = &first.source;
                let second_source = &second.source;
                (first_source.primary_origin_id != second_source.primary_origin_id
                    && first_source.primary_document_id != second_source.primary_document_id
                    && first_source.document_payload_hash != second_source.document_payload_hash)
                    .then(|| vec![first.observation_id.clone(), second.observation_id.clone()])
            })
        });
        assessments.push(ConsumerNeedAssessmentV1 {
            hypothesis: hypothesis.clone(),
            status: if independent_pair.is_some() {
                ConsumerNeedEvidenceStatusV1::MultiSourceCandidateUnattested
            } else {
                ConsumerNeedEvidenceStatusV1::InsufficientEvidence
            },
            observations: relevant.iter().map(|observation| pointer(observation)).collect(),
            structurally_independent_observation_ids: independent_pair.unwrap_or_default(),
            source_authentication: "not_checked",
            owner_receipt_attestation: "not_checked",
            execution: "not_started",
        });
    }
    assessments.sort_by(|first, second| first.hypothesis.need_key.cmp(&second.hypothesis.need_key));
    Ok(ConsumerNeedEvidencePreviewV1 {
        schema_revision: NEED_EVIDENCE_SCHEMA_REVISION_V1,
        status: "structural_only_unattested",
        assessments,
        note: "Two independent claimed primary origin/document/content identities are only a structural candidate. Verify publisher, fetched source bytes, owner receipt, tenant authority and excerpt provenance separately before calling any finding confirmed.",
    })
}

#[cfg(test)]
mod tests;
