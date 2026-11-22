use std::collections::{BTreeMap, BTreeSet};

use contracts::generated::alegria::temporal::v1::{
    FreshnessReport, SeoScopePayload, SeoVerifiedFactSupportState, StepContractMeta,
};
use infrastructure::adapters::graph_capability_adapter;
use infrastructure::adapters::neo4rs_adapter;
use infrastructure::adapters::projection_materialize_adapter;
use infrastructure::adapters::qdrant_client_adapter;
use infrastructure::adapters::raw_crawl_adapter;
use infrastructure::adapters::semantic_search_adapter;
use infrastructure::adapters::sqlx_freshness_adapter::load_freshness_snapshot;
use infrastructure::adapters::sqlx_outbox_adapter;
use infrastructure::adapters::sqlx_pipeline_runtime_adapter::RuntimeProtoPayload;
use infrastructure::adapters::sqlx_reconcile_adapter;
use infrastructure::adapters::sqlx_runtime_outbox_adapter;
use infrastructure::adapters::sqlx_seo_adapter;
use infrastructure::adapters::sqlx_source_projection_adapter::load_source_registry_entries;
use infrastructure::adapters::voyage_api_adapter::{
    VoyageClient, VoyageEmbeddingOptions, VoyageInputType, VoyageOutputDtype, VoyageRerankOptions,
};
use infrastructure::adapters::whole_page_advisory_adapter;
use truth_domain::truth_governance::{
    adjudicate_truth_candidates_with_governance, SourceGovernanceRecord,
};
use truth_domain::review_decisions::{
    apply_exact_review_rejection_veto, held_semantic_identities, semantic_identity_is_held,
    non_structured_candidate_reason, validated_candidate_assertions, CandidateValidationSectionState,
    TruthAdjudicationCandidateDecision, ValidatedTruthCandidateRecord,
};
use truth_domain::evidence::{
    classify_candidate_completeness_local, classify_candidate_freshness_local,
    contains_exact_evidence_token, deterministic_evidence_has_role_context, find_evidence_span,
    section_source_tier, section_uncertainty_flags,
};
use truth_domain::procedural_candidates::{
    active_heuristic_truth_rules, params_for_rule, role_and_concept_for_rule,
};
use truth_domain::semantic_truth_write::{
    persisted_candidate_status, semantic_truth_write_outcome, truth_candidate_write_fields,
    validate_verified_truth_write_candidate, SemanticTruthWriteOutcome,
};
use primitives::errors::DomainError;
use primitives::hash::{blake3_hex, content_hash_v1};
use primitives::qdrant_point_id::qdrant_point_id_v1;
use truth_domain::truth_candidates::{
    validate_truth_candidate, TruthCandidateRuntime, TruthCandidateValidationResult,
    TruthParamValue, TruthStructuredCandidate,
};
use runtime_models::ReconcileTargetReportRecord;
use serde::{Deserialize, Serialize};
use serde_json::value::Value;

use super::AlegriaActivities;

macro_rules! impl_json_runtime_payload_local {
    ($ty:ty, $payload_type:expr) => {
        impl RuntimeProtoPayload for $ty {
            fn payload_type() -> &'static str {
                $payload_type
            }

            fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
                serde_json::to_vec(self).map_err(|e| DomainError::ContractViolation {
                    message: e.to_string(),
                })
            }

            fn decode_payload_bytes(
                payload_bytes: &[u8],
            ) -> std::result::Result<Self, DomainError> {
                serde_json::from_slice(payload_bytes).map_err(|e| DomainError::ContractViolation {
                    message: e.to_string(),
                })
            }
        }
    };
}
