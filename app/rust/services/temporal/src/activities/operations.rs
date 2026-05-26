use std::collections::{BTreeMap, BTreeSet};

use contracts::generated::alegria::temporal::v1::{
    FreshnessReport, SeoScopePayload, SeoVerifiedFactSupportState, StepContractMeta,
};
use infrastructure::adapters::projection_materialize_adapter;
use infrastructure::adapters::raw_crawl_adapter;
use infrastructure::adapters::sqlx_freshness_adapter::load_freshness_snapshot;
use infrastructure::adapters::sqlx_outbox_adapter;
use infrastructure::adapters::sqlx_pipeline_runtime_adapter::RuntimeProtoPayload;
use infrastructure::adapters::sqlx_reconcile_adapter;
use infrastructure::adapters::sqlx_seo_adapter;
use infrastructure::adapters::sqlx_source_projection_adapter::load_source_registry_entries;
use infrastructure::adapters::whole_page_advisory_adapter;
use policies::truth_governance::{
    adjudicate_truth_candidates_with_governance, SourceGovernanceRecord,
};
use primitives::errors::DomainError;
use primitives::hash::{blake3_hex, content_hash_v1};
use primitives::truth_candidates::{
    validate_truth_candidate, TruthCandidateRuntime, TruthCandidateValidationResult,
    TruthParamValue, TruthStructuredCandidate,
};
use runtime_models::ReconcileTargetReportRecord;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{types::Json, Row};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jBackwriteInput {
    pub target_system: String,
    pub dry_run: bool,
    pub max_retry_count: Option<i32>,
    pub batch_limit: Option<i64>,
    pub requeue_base_delay_sec: Option<i64>,
    pub requeue_jitter_sec: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jBackwriteOutput {
    pub target_system: String,
    pub dry_run: bool,
    pub stale_candidates: i64,
    pub failed_candidates: i64,
    pub reset_stale_processing: i64,
    pub requeued_failed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionSyncInput {
    pub run_id: String,
    pub target_system: String,
    pub step_name: String,
    pub batch_limit: i64,
    pub lease_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionSyncOutput {
    pub run_id: String,
    pub target_system: String,
    pub processed_events: i64,
    pub retried_events: i64,
    pub failed_events: i64,
    pub remaining_pending: i64,
    pub remaining_failed: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSectionSampleInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSectionSampleOutput {
    pub section_id: String,
    pub page_id: i64,
    pub source_url: String,
    pub source_domain: String,
    pub heading_path: String,
    pub section_type: String,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoPreflightInput {
    pub run_id: String,
    pub context_key: String,
    pub scope: SeoScopePayload,
    pub projection_max_lag_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoPreflightOutput {
    pub context_key: String,
    pub normalized_profile: String,
    pub page_type_count: i64,
    pub page_node_count: i64,
    pub navigation_item_count: i64,
    pub verified_rule_count: i64,
    pub pending_rule_count: i64,
    pub qdrant_point_count: i64,
    pub projection_blocked: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthAdmissibilityGateInput {
    pub run_id: String,
    pub context_key: String,
    pub applicant_profile: String,
    pub verified_support: Vec<SeoVerifiedFactSupportState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthAdmissibilityGateOutput {
    pub context_key: String,
    pub applicant_profile: String,
    pub admissible_support_count: usize,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanApprovalWaitInput {
    pub run_id: String,
    pub page_node_key: String,
    pub revision_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageSemanticPassInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageContextProfile {
    pub country_hints: Vec<String>,
    pub visa_type_hints: Vec<String>,
    pub authority_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageAdvisorySignal {
    pub page_mode_candidates: Vec<String>,
    pub layer_candidates: Vec<String>,
    pub context_profile_candidates: WholePageContextProfile,
    pub mixed_section_pressure: bool,
    pub retrieval_confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageSemanticPageState {
    pub page_id: i64,
    pub section_count: usize,
    pub page_mode_hint: String,
    pub page_mode_confidence: f32,
    pub dominant_layers: Vec<String>,
    pub layer_scores: BTreeMap<String, f32>,
    pub page_summary: String,
    pub page_context_profile: WholePageContextProfile,
    pub mixed_section_ids: Vec<i64>,
    pub global_entities: Vec<String>,
    pub advisory_model_used: bool,
    pub advisory_consensus: String,
    pub advisory_prototype_families: Vec<String>,
    pub uncertainty_flags: Vec<String>,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageSemanticPassOutput {
    pub page_count: usize,
    pub pages: Vec<WholePageSemanticPageState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub source_url: String,
    pub heading_path: String,
    pub section_type: String,
    pub content_hash: String,
    pub text_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningOutput {
    pub page_count: usize,
    pub section_count: usize,
    pub sections: Vec<SectioningSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageUtilitySweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageUtilitySectionDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub allow_procedural_extraction: bool,
    pub allow_editorial_extraction: bool,
    pub allow_structural_extraction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageUtilitySweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub decisions: Vec<PageUtilitySectionDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockRelevanceSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockRelevanceSectionDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub block_role: String,
    pub allow_extraction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomBlockRelevanceSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub decisions: Vec<DomBlockRelevanceSectionDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningContractGateInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningContractDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub heading_present: bool,
    pub text_present: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningContractGateOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub decisions: Vec<SectioningContractDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasGateInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasGateDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub snapshot_hash: String,
    pub is_replay_safe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasGateOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub decisions: Vec<CasGateDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvidenceRegisterInput {
    pub run_id: String,
    pub context_key: String,
    pub raw_page_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvidenceRegisterOutput {
    pub context_key: String,
    pub page_count: usize,
    pub section_count: usize,
    pub unique_source_count: usize,
    pub evidence_refs: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionSemanticGateBundle {
    pub page_utility: PageUtilitySweepOutput,
    pub dom_relevance: DomBlockRelevanceSweepOutput,
    pub sectioning_contract: SectioningContractGateOutput,
    pub cas_gate: CasGateOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerRouterSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub gates: SectionSemanticGateBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerRouterSectionDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub primary_layer: String,
    pub secondary_layers: Vec<seo_steps::layer_router_step::SecondaryLayer>,
    pub confidence: f32,
    pub needs_hitl: bool,
    pub blocked_by_gate: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerRouterSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_hitl_count: usize,
    pub decisions: Vec<LayerRouterSectionDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubspanLayerRouterInput {
    pub run_id: String,
    pub layer_router: LayerRouterSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubspanLayerRouterDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub mixed_layers: Vec<String>,
    pub needs_split: bool,
    pub blocked_by_gate: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubspanLayerRouterOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_split_count: usize,
    pub decisions: Vec<SubspanLayerRouterDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySpanSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub gates: SectionSemanticGateBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySpanSectionMentions {
    pub section_id: i64,
    pub page_id: i64,
    pub mentions: Vec<seo_steps::entity_span_detection_step::EntityMention>,
    pub blocked_by_gate: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySpanSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub mention_count: usize,
    pub sections: Vec<EntitySpanSectionMentions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalMappingSweepInput {
    pub run_id: String,
    pub entity_spans: EntitySpanSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalMappingSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub mappings: Vec<seo_steps::canonical_mapping_step::MappingResult>,
    pub blocked_by_gate: bool,
    pub needs_hitl: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalMappingSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_hitl_count: usize,
    pub sections: Vec<CanonicalMappingSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyIntakeGateInput {
    pub run_id: String,
    pub canonical_mapping: CanonicalMappingSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyIntakeGateDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub accepted_keys: Vec<String>,
    pub unresolved_mentions: Vec<String>,
    pub blocked_by_gate: bool,
    pub needs_hitl: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyIntakeGateOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_hitl_count: usize,
    pub sections: Vec<OntologyIntakeGateDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralExtractionSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub gates: SectionSemanticGateBundle,
    pub entity_spans: EntitySpanSweepOutput,
    pub ontology: OntologyIntakeGateOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralExtractionSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub rules: Vec<seo_steps::procedural_extraction_step::ProceduralRule>,
    pub blocked_by_gate: bool,
    pub skipped: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralExtractionSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub skipped_section_count: usize,
    pub rule_count: usize,
    pub sections: Vec<ProceduralExtractionSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalExtractionSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub gates: SectionSemanticGateBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalExtractionSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub entities: Vec<seo_steps::operational_extraction_step::OperationalEntity>,
    pub blocked_by_gate: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalExtractionSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub entity_count: usize,
    pub sections: Vec<OperationalExtractionSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorialExtractionSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub gates: SectionSemanticGateBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorialExtractionSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub topics: Vec<seo_steps::editorial_extraction_step::EditorialTopic>,
    pub blocked_by_gate: bool,
    pub skipped: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorialExtractionSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub skipped_section_count: usize,
    pub topic_count: usize,
    pub sections: Vec<EditorialExtractionSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoSignalExtractionSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub gates: SectionSemanticGateBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoSignalRecord {
    pub signal_type: String,
    pub value: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoSignalExtractionSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub signals: Vec<SeoSignalRecord>,
    pub blocked_by_gate: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeoSignalExtractionSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub signal_count: usize,
    pub sections: Vec<SeoSignalExtractionSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommercialSignalExtractionSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub gates: SectionSemanticGateBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommercialSignalRecord {
    pub signal_type: String,
    pub value: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommercialSignalExtractionSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub signals: Vec<CommercialSignalRecord>,
    pub blocked_by_gate: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommercialSignalExtractionSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub signal_count: usize,
    pub sections: Vec<CommercialSignalExtractionSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionSchemaValidateInput {
    pub run_id: String,
    pub procedural: ProceduralExtractionSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionSchemaSectionDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub status: String,
    pub blocking_reasons: Vec<String>,
    pub blocked_by_gate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionSchemaValidateOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub invalid_section_count: usize,
    pub sections: Vec<ExtractionSchemaSectionDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateValidationInput {
    pub run_id: String,
    pub context_key: String,
    pub raw_page_ids: Vec<i64>,
    pub procedural: ProceduralExtractionSweepOutput,
    pub ontology: OntologyIntakeGateOutput,
    pub schema_validate: ExtractionSchemaValidateOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedTruthCandidateRecord {
    pub section_id: i64,
    pub page_id: i64,
    pub rule_candidate_id: String,
    pub role: String,
    pub concept_canonical_key: String,
    pub raw_mention: String,
    pub params: TruthParamValue,
    pub source_key: String,
    pub source_tier: String,
    pub confidence: f64,
    pub evidence_quote: String,
    pub span_start: usize,
    pub span_end: usize,
    pub source_snapshot_hash: String,
    pub freshness_class: String,
    pub completeness_class: String,
    pub epistemic_status: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateValidationSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub candidates: Vec<ValidatedTruthCandidateRecord>,
    pub accepted_count: usize,
    pub needs_hitl_count: usize,
    pub rejected_count: usize,
    pub blocked_by_gate: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateValidationOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub accepted_count: usize,
    pub needs_hitl_count: usize,
    pub rejected_count: usize,
    pub sections: Vec<CandidateValidationSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleBuilderSweepInput {
    pub run_id: String,
    pub procedural: ProceduralExtractionSweepOutput,
    pub operational: OperationalExtractionSweepOutput,
    pub editorial: EditorialExtractionSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleBuilderSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub triples: Vec<seo_steps::triple_builder_step::BuiltTriple>,
    pub blocked_by_gate: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleBuilderSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub triple_count: usize,
    pub sections: Vec<TripleBuilderSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletenessJudgeSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub entity_spans: EntitySpanSweepOutput,
    pub procedural: ProceduralExtractionSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletenessJudgeSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub output: seo_steps::completeness_judge_step::CompletenessJudgeOutput,
    pub blocked_by_gate: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletenessJudgeSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_hitl_count: usize,
    pub sections: Vec<CompletenessJudgeSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionLoopInput {
    pub run_id: String,
    pub ontology: OntologyIntakeGateOutput,
    pub schema_validate: ExtractionSchemaValidateOutput,
    pub completeness: CompletenessJudgeSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionLoopSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub decision: String,
    pub blockers: Vec<String>,
    pub needs_hitl: bool,
    pub blocked_by_gate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionLoopOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_hitl_count: usize,
    pub rejected_count: usize,
    pub sections: Vec<ResolutionLoopSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionGateSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub procedural: ProceduralExtractionSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionGateSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub output: seo_steps::contradiction_gate_step::ContradictionGateOutput,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionGateSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_hitl_count: usize,
    pub conflict_count: usize,
    pub sections: Vec<ContradictionGateSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthAdjudicationSweepInput {
    pub run_id: String,
    pub context_key: String,
    pub candidate_validation: CandidateValidationOutput,
    pub resolution: ResolutionLoopOutput,
    pub contradiction: ContradictionGateSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthAdjudicationCandidateDecision {
    pub section_id: i64,
    pub page_id: i64,
    pub rule_candidate_id: String,
    pub role: String,
    pub concept_canonical_key: String,
    pub params: TruthParamValue,
    pub source_key: String,
    pub source_tier: String,
    pub confidence: f64,
    pub freshness_class: String,
    pub completeness_class: String,
    pub evidence_quote: String,
    pub span_start: usize,
    pub span_end: usize,
    pub source_snapshot_hash: String,
    pub decision: String,
    pub publish_admissibility: String,
    pub verification_method: String,
    pub adjudication_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthAdjudicationSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub decision: String,
    pub status: String,
    pub verified_count: usize,
    pub needs_hitl_count: usize,
    pub rejected_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthAdjudicationSweepOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub verified_count: usize,
    pub needs_hitl_count: usize,
    pub rejected_count: usize,
    pub decisions: Vec<TruthAdjudicationCandidateDecision>,
    pub sections: Vec<TruthAdjudicationSectionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedTruthWriteInput {
    pub run_id: String,
    pub context_key: String,
    pub truth_adjudication: TruthAdjudicationSweepOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedTruthWriteOutput {
    pub context_key: String,
    pub verified_rule_count: usize,
    pub demoted_rule_count: usize,
    pub changed_truth_keys: Vec<String>,
    pub status: String,
}

impl_json_runtime_payload_local!(
    SemanticSectionSampleInput,
    "alegria.runtime.json.SemanticSectionSampleInput"
);
impl_json_runtime_payload_local!(
    SemanticSectionSampleOutput,
    "alegria.runtime.json.SemanticSectionSampleOutput"
);
impl_json_runtime_payload_local!(
    ProjectionSyncInput,
    "alegria.runtime.json.ProjectionSyncInput"
);
impl_json_runtime_payload_local!(
    ProjectionSyncOutput,
    "alegria.runtime.json.ProjectionSyncOutput"
);
impl_json_runtime_payload_local!(SeoPreflightInput, "alegria.runtime.json.SeoPreflightInput");
impl_json_runtime_payload_local!(SeoPreflightOutput, "alegria.runtime.json.SeoPreflightOutput");
impl_json_runtime_payload_local!(
    TruthAdmissibilityGateInput,
    "alegria.runtime.json.TruthAdmissibilityGateInput"
);
impl_json_runtime_payload_local!(
    TruthAdmissibilityGateOutput,
    "alegria.runtime.json.TruthAdmissibilityGateOutput"
);
impl_json_runtime_payload_local!(
    HumanApprovalWaitInput,
    "alegria.runtime.json.HumanApprovalWaitInput"
);
impl_json_runtime_payload_local!(
    WholePageSemanticPassInput,
    "alegria.runtime.json.WholePageSemanticPassInput"
);
impl_json_runtime_payload_local!(
    WholePageSemanticPassOutput,
    "alegria.runtime.json.WholePageSemanticPassOutput"
);
impl_json_runtime_payload_local!(SectioningInput, "alegria.runtime.json.SectioningInput");
impl_json_runtime_payload_local!(SectioningOutput, "alegria.runtime.json.SectioningOutput");
impl_json_runtime_payload_local!(
    PageUtilitySweepInput,
    "alegria.runtime.json.PageUtilitySweepInput"
);
impl_json_runtime_payload_local!(
    PageUtilitySweepOutput,
    "alegria.runtime.json.PageUtilitySweepOutput"
);
impl_json_runtime_payload_local!(
    DomBlockRelevanceSweepInput,
    "alegria.runtime.json.DomBlockRelevanceSweepInput"
);
impl_json_runtime_payload_local!(
    DomBlockRelevanceSweepOutput,
    "alegria.runtime.json.DomBlockRelevanceSweepOutput"
);
impl_json_runtime_payload_local!(
    SectioningContractGateInput,
    "alegria.runtime.json.SectioningContractGateInput"
);
impl_json_runtime_payload_local!(
    SectioningContractGateOutput,
    "alegria.runtime.json.SectioningContractGateOutput"
);
impl_json_runtime_payload_local!(CasGateInput, "alegria.runtime.json.CasGateInput");
impl_json_runtime_payload_local!(CasGateOutput, "alegria.runtime.json.CasGateOutput");
impl_json_runtime_payload_local!(
    RawEvidenceRegisterInput,
    "alegria.runtime.json.RawEvidenceRegisterInput"
);
impl_json_runtime_payload_local!(
    RawEvidenceRegisterOutput,
    "alegria.runtime.json.RawEvidenceRegisterOutput"
);
impl_json_runtime_payload_local!(
    SectionSemanticGateBundle,
    "alegria.runtime.json.SectionSemanticGateBundle"
);
impl_json_runtime_payload_local!(
    LayerRouterSweepInput,
    "alegria.runtime.json.LayerRouterSweepInput"
);
impl_json_runtime_payload_local!(
    LayerRouterSweepOutput,
    "alegria.runtime.json.LayerRouterSweepOutput"
);
impl_json_runtime_payload_local!(
    SubspanLayerRouterInput,
    "alegria.runtime.json.SubspanLayerRouterInput"
);
impl_json_runtime_payload_local!(
    SubspanLayerRouterOutput,
    "alegria.runtime.json.SubspanLayerRouterOutput"
);
impl_json_runtime_payload_local!(
    EntitySpanSweepInput,
    "alegria.runtime.json.EntitySpanSweepInput"
);
impl_json_runtime_payload_local!(
    EntitySpanSweepOutput,
    "alegria.runtime.json.EntitySpanSweepOutput"
);
impl_json_runtime_payload_local!(
    CanonicalMappingSweepInput,
    "alegria.runtime.json.CanonicalMappingSweepInput"
);
impl_json_runtime_payload_local!(
    CanonicalMappingSweepOutput,
    "alegria.runtime.json.CanonicalMappingSweepOutput"
);
impl_json_runtime_payload_local!(
    OntologyIntakeGateInput,
    "alegria.runtime.json.OntologyIntakeGateInput"
);
impl_json_runtime_payload_local!(
    OntologyIntakeGateOutput,
    "alegria.runtime.json.OntologyIntakeGateOutput"
);
impl_json_runtime_payload_local!(
    ProceduralExtractionSweepInput,
    "alegria.runtime.json.ProceduralExtractionSweepInput"
);
impl_json_runtime_payload_local!(
    ProceduralExtractionSweepOutput,
    "alegria.runtime.json.ProceduralExtractionSweepOutput"
);
impl_json_runtime_payload_local!(
    OperationalExtractionSweepInput,
    "alegria.runtime.json.OperationalExtractionSweepInput"
);
impl_json_runtime_payload_local!(
    OperationalExtractionSweepOutput,
    "alegria.runtime.json.OperationalExtractionSweepOutput"
);
impl_json_runtime_payload_local!(
    EditorialExtractionSweepInput,
    "alegria.runtime.json.EditorialExtractionSweepInput"
);
impl_json_runtime_payload_local!(
    EditorialExtractionSweepOutput,
    "alegria.runtime.json.EditorialExtractionSweepOutput"
);
impl_json_runtime_payload_local!(
    SeoSignalExtractionSweepInput,
    "alegria.runtime.json.SeoSignalExtractionSweepInput"
);
impl_json_runtime_payload_local!(
    SeoSignalExtractionSweepOutput,
    "alegria.runtime.json.SeoSignalExtractionSweepOutput"
);
impl_json_runtime_payload_local!(
    CommercialSignalExtractionSweepInput,
    "alegria.runtime.json.CommercialSignalExtractionSweepInput"
);
impl_json_runtime_payload_local!(
    CommercialSignalExtractionSweepOutput,
    "alegria.runtime.json.CommercialSignalExtractionSweepOutput"
);
impl_json_runtime_payload_local!(
    ExtractionSchemaValidateInput,
    "alegria.runtime.json.ExtractionSchemaValidateInput"
);
impl_json_runtime_payload_local!(
    ExtractionSchemaValidateOutput,
    "alegria.runtime.json.ExtractionSchemaValidateOutput"
);
impl_json_runtime_payload_local!(
    CandidateValidationInput,
    "alegria.runtime.json.CandidateValidationInput"
);
impl_json_runtime_payload_local!(
    CandidateValidationOutput,
    "alegria.runtime.json.CandidateValidationOutput"
);
impl_json_runtime_payload_local!(
    TripleBuilderSweepInput,
    "alegria.runtime.json.TripleBuilderSweepInput"
);
impl_json_runtime_payload_local!(
    TripleBuilderSweepOutput,
    "alegria.runtime.json.TripleBuilderSweepOutput"
);
impl_json_runtime_payload_local!(
    CompletenessJudgeSweepInput,
    "alegria.runtime.json.CompletenessJudgeSweepInput"
);
impl_json_runtime_payload_local!(
    CompletenessJudgeSweepOutput,
    "alegria.runtime.json.CompletenessJudgeSweepOutput"
);
impl_json_runtime_payload_local!(
    ResolutionLoopInput,
    "alegria.runtime.json.ResolutionLoopInput"
);
impl_json_runtime_payload_local!(
    ResolutionLoopOutput,
    "alegria.runtime.json.ResolutionLoopOutput"
);
impl_json_runtime_payload_local!(
    ContradictionGateSweepInput,
    "alegria.runtime.json.ContradictionGateSweepInput"
);
impl_json_runtime_payload_local!(
    ContradictionGateSweepOutput,
    "alegria.runtime.json.ContradictionGateSweepOutput"
);
impl_json_runtime_payload_local!(
    TruthAdjudicationSweepInput,
    "alegria.runtime.json.TruthAdjudicationSweepInput"
);
impl_json_runtime_payload_local!(
    TruthAdjudicationSweepOutput,
    "alegria.runtime.json.TruthAdjudicationSweepOutput"
);
impl_json_runtime_payload_local!(
    VerifiedTruthWriteInput,
    "alegria.runtime.json.VerifiedTruthWriteInput"
);
impl_json_runtime_payload_local!(
    VerifiedTruthWriteOutput,
    "alegria.runtime.json.VerifiedTruthWriteOutput"
);

fn dominant_layers(text: &str) -> Vec<String> {
    let scores = layer_scores_for_text(text);
    let mut layers = select_layers_from_scores(&scores);
    if layers.is_empty() {
        layers.push("procedural".to_string());
    }
    layers
}

#[derive(Debug, Clone)]
struct WholePageDeterministicSnapshot {
    page_mode_hint: String,
    page_mode_confidence: f32,
    dominant_layers: Vec<String>,
    layer_scores: BTreeMap<String, f32>,
    page_summary: String,
    page_context_profile: WholePageContextProfile,
    mixed_section_ids: Vec<i64>,
    global_entities: Vec<String>,
    uncertainty_flags: Vec<String>,
    reason_codes: Vec<String>,
}

fn normalized_marker_score(text: &str, markers: &[&str]) -> f32 {
    if markers.is_empty() {
        return 0.0;
    }
    let lowered = text.to_lowercase();
    let hits = markers
        .iter()
        .filter(|marker| lowered.contains(**marker))
        .count() as f32;
    hits / markers.len() as f32
}

fn layer_scores_for_text(text: &str) -> BTreeMap<String, f32> {
    let mut scores = BTreeMap::new();
    scores.insert(
        "procedural".to_string(),
        normalized_marker_score(
            text,
            &["паспорт", "страхов", "анкет", "fee", "eur", "сбор", "visa", "виза"],
        ),
    );
    scores.insert(
        "operational".to_string(),
        normalized_marker_score(
            text,
            &["schedule", "график", "holiday", "appointment", "запись", "время работы"],
        ),
    );
    scores.insert(
        "editorial".to_string(),
        normalized_marker_score(text, &["faq", "что делать", "почему", "ошибк", "отказ", "проблем"]),
    );
    scores.insert(
        "seo".to_string(),
        normalized_marker_score(text, &["seo", "serp", "ключев", "ranking", "organic traffic"]),
    );
    scores.insert(
        "commercial".to_string(),
        normalized_marker_score(
            text,
            &["consultation", "book now", "услуга", "под ключ", "заказать", "service package"],
        ),
    );
    scores
}

fn select_layers_from_scores(scores: &BTreeMap<String, f32>) -> Vec<String> {
    let max_score = scores.values().copied().fold(0.0_f32, f32::max);
    let threshold = if max_score >= 0.55 {
        max_score * 0.55
    } else {
        0.20
    };
    scores
        .iter()
        .filter(|(_, score)| **score >= threshold && **score > 0.0)
        .map(|(layer, _)| layer.clone())
        .collect()
}

fn page_mode_scores(
    text: &str,
    sections: &[raw_crawl_adapter::RawSectionRecord],
) -> BTreeMap<String, f32> {
    let mut scores = BTreeMap::new();
    let nav_sections = sections
        .iter()
        .filter(|section| {
            let kind = section.section_type.to_ascii_lowercase();
            kind.contains("nav") || kind.contains("toc")
        })
        .count() as f32;
    let footer_sections = sections
        .iter()
        .filter(|section| section.section_type.to_ascii_lowercase().contains("footer"))
        .count() as f32;
    let total_sections = sections.len().max(1) as f32;
    scores.insert(
        "directory_page".to_string(),
        normalized_marker_score(text, &["sitemap", "directory", "каталог", "index page"]) * 0.8,
    );
    scores.insert(
        "menu_page".to_string(),
        normalized_marker_score(text, &["breadcrumb", "menu", "навигац", "sidebar"])
            + (nav_sections / total_sections) * 0.5,
    );
    scores.insert(
        "utility_page".to_string(),
        normalized_marker_score(text, &["privacy", "cookie", "login", "terms", "policy"])
            + (footer_sections / total_sections) * 0.2,
    );
    scores.insert(
        "landing_page".to_string(),
        normalized_marker_score(text, &["consultation", "book now", "услуга", "под ключ", "cta"]),
    );
    let content_support = normalized_marker_score(
        text,
        &["visa", "виза", "requirements", "документ", "appointment", "faq", "guide"],
    );
    scores.insert(
        "content_page".to_string(),
        (0.45 + content_support).min(1.0),
    );
    scores
}

fn select_page_mode_and_confidence(scores: &BTreeMap<String, f32>) -> (String, f32) {
    let mut ranked = scores.iter().collect::<Vec<_>>();
    ranked.sort_by(|lhs, rhs| rhs.1.total_cmp(lhs.1).then_with(|| lhs.0.cmp(rhs.0)));
    if let Some((mode, score)) = ranked.first() {
        let runner_up = ranked.get(1).map(|(_, value)| **value).unwrap_or(0.0);
        let confidence = (*score - runner_up).max(0.15) + 0.5;
        (mode.to_string(), confidence.min(0.95))
    } else {
        ("content_page".to_string(), 0.5)
    }
}

fn section_mixed_layer_score(text: &str) -> usize {
    layer_scores_for_text(text)
        .values()
        .filter(|score| **score >= 0.20)
        .count()
}

fn build_page_sketch(
    page_id: i64,
    sections: &[raw_crawl_adapter::RawSectionRecord],
    snapshot: &WholePageDeterministicSnapshot,
) -> String {
    let source_url = sections
        .first()
        .map(|section| section.source_url.as_str())
        .unwrap_or_default();
    let headings = sections
        .iter()
        .map(|section| section.heading_path.trim())
        .filter(|heading| !heading.is_empty())
        .take(6)
        .collect::<Vec<_>>();
    let excerpts = sections
        .iter()
        .filter(|section| !section.content_md.trim().is_empty())
        .take(3)
        .map(|section| {
            let excerpt = section.content_md.chars().take(220).collect::<String>();
            format!("section:{} type:{} text:{}", section.id, section.section_type, excerpt)
        })
        .collect::<Vec<_>>();
    let mut section_histogram = BTreeMap::new();
    for section in sections {
        *section_histogram
            .entry(section.section_type.to_ascii_lowercase())
            .or_insert(0usize) += 1;
    }
    let section_histogram = section_histogram
        .into_iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "page_id:{page_id}\nsource_url:{source_url}\nheadings:{}\nsection_histogram:{}\npage_mode_hint:{}\ndominant_layers:{}\ncontext_country:{}\ncontext_visa:{}\ncontext_authority:{}\nglobal_entities:{}\nsummary:{}\nexcerpts:\n{}",
        headings.join(" | "),
        section_histogram,
        snapshot.page_mode_hint,
        snapshot.dominant_layers.join("|"),
        snapshot.page_context_profile.country_hints.join("|"),
        snapshot.page_context_profile.visa_type_hints.join("|"),
        snapshot.page_context_profile.authority_hints.join("|"),
        snapshot.global_entities.join("|"),
        snapshot.page_summary,
        excerpts.join("\n")
    )
}

fn deterministic_whole_page_snapshot(
    sections: &[raw_crawl_adapter::RawSectionRecord],
    combined: &str,
) -> WholePageDeterministicSnapshot {
    let layer_scores = layer_scores_for_text(combined);
    let dominant_layers = select_layers_from_scores(&layer_scores);
    let page_mode_score_map = page_mode_scores(combined, sections);
    let (page_mode_hint, page_mode_confidence) =
        select_page_mode_and_confidence(&page_mode_score_map);
    let mixed_section_ids = sections
        .iter()
        .filter_map(|section| {
            if section_mixed_layer_score(&section.content_md) >= 2 {
                Some(section.id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut reason_codes = Vec::new();
    reason_codes.push(format!("page_mode:{}", page_mode_hint));
    if !mixed_section_ids.is_empty() {
        reason_codes.push("mixed_section_detected".to_string());
    }
    let mut uncertainty_flags = Vec::new();
    if page_mode_confidence < 0.65 {
        uncertainty_flags.push("low_page_mode_confidence".to_string());
    }
    if dominant_layers.len() > 1 {
        uncertainty_flags.push("multi_layer_page".to_string());
    }
    WholePageDeterministicSnapshot {
        page_mode_hint,
        page_mode_confidence,
        dominant_layers,
        layer_scores,
        page_summary: summarize(combined),
        page_context_profile: page_context_profile(combined),
        mixed_section_ids,
        global_entities: global_entities(combined),
        uncertainty_flags,
        reason_codes,
    }
}

fn advisory_signal_from_hits(
    advisory_hits: &[whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit],
) -> WholePageAdvisorySignal {
    let mut page_mode_candidates = Vec::new();
    let mut layer_candidates = Vec::new();
    let mut country_hints = Vec::new();
    let mut visa_type_hints = Vec::new();
    let mut authority_hints = Vec::new();
    let mut mixed_section_pressure = false;
    let mut retrieval_confidence = 0.0_f32;

    for hit in advisory_hits {
        if !page_mode_candidates.contains(&hit.page_mode) {
            page_mode_candidates.push(hit.page_mode.clone());
        }
        for layer in &hit.dominant_layers {
            if !layer_candidates.contains(layer) {
                layer_candidates.push(layer.clone());
            }
        }
        merge_unique_strings(&mut country_hints, hit.country_hints.clone());
        merge_unique_strings(&mut visa_type_hints, hit.visa_type_hints.clone());
        merge_unique_strings(&mut authority_hints, hit.authority_hints.clone());
        mixed_section_pressure |= hit.mixed_section_pressure;
        retrieval_confidence = retrieval_confidence.max(hit.score);
    }

    WholePageAdvisorySignal {
        page_mode_candidates,
        layer_candidates,
        context_profile_candidates: WholePageContextProfile {
            country_hints,
            visa_type_hints,
            authority_hints,
        },
        mixed_section_pressure,
        retrieval_confidence,
    }
}

fn advisory_hit_limit() -> u64 {
    std::env::var("WHOLE_PAGE_ADVISORY_LIMIT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3)
}

fn summarize(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(180).collect())
        .unwrap_or_default()
}

fn global_entities(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut entities = Vec::new();
    for token in ["passport", "паспорт", "insurance", "страхов", "vfs", "посольств"] {
        if lowered.contains(token) {
            entities.push(token.to_string());
        }
    }
    entities.sort();
    entities.dedup();
    entities
}

fn page_context_profile(text: &str) -> WholePageContextProfile {
    let lowered = text.to_lowercase();

    let mut country_hints = Vec::new();
    for (hint, tokens) in [
        ("ES", &["spain", "spanish", "испан", "españa"][..]),
        ("PL", &["poland", "polish", "польш", "polska"][..]),
        ("FR", &["france", "french", "франц"][..]),
        ("DE", &["germany", "german", "герман", "deutschland"][..]),
    ] {
        if tokens.iter().any(|token| lowered.contains(token)) {
            country_hints.push(hint.to_string());
        }
    }

    let mut visa_type_hints = Vec::new();
    for (hint, tokens) in [
        ("tourist", &["tourist", "tourism", "турист", "шенген"][..]),
        ("work", &["work visa", "рабоч", "employment visa"][..]),
        ("student", &["student visa", "study visa", "учеб", "student"][..]),
    ] {
        if tokens.iter().any(|token| lowered.contains(token)) {
            visa_type_hints.push(hint.to_string());
        }
    }

    let mut authority_hints = Vec::new();
    for (hint, tokens) in [
        ("consulate", &["consulate", "consular", "консуль", "посольств"][..]),
        ("visa_center", &["vfs", "visa center", "визов"][..]),
        (
            "government",
            &["ministry", "gov.", ".gov", "government", "министер"][..],
        ),
    ] {
        if tokens.iter().any(|token| lowered.contains(token)) {
            authority_hints.push(hint.to_string());
        }
    }

    country_hints.sort();
    country_hints.dedup();
    visa_type_hints.sort();
    visa_type_hints.dedup();
    authority_hints.sort();
    authority_hints.dedup();

    WholePageContextProfile {
        country_hints,
        visa_type_hints,
        authority_hints,
    }
}

fn merge_unique_strings(left: &mut Vec<String>, right: impl IntoIterator<Item = String>) {
    for item in right {
        if !left.contains(&item) {
            left.push(item);
        }
    }
}

fn supportive_section_count_for_layer(
    sections: &[raw_crawl_adapter::RawSectionRecord],
    layer: &str,
) -> usize {
    sections
        .iter()
        .filter(|section| dominant_layers(&section.content_md).iter().any(|value| value == layer))
        .count()
}

fn fuse_with_advisory_retrieval(
    sections: &[raw_crawl_adapter::RawSectionRecord],
    snapshot: WholePageDeterministicSnapshot,
    advisory_hits: &[whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit],
) -> WholePageSemanticPageState {
    let mut dominant_layers = snapshot.dominant_layers.clone();
    let mut layer_scores = snapshot.layer_scores.clone();
    let mut page_context_profile = snapshot.page_context_profile.clone();
    let mut mixed_section_ids = snapshot.mixed_section_ids.clone();
    let global_entities = snapshot.global_entities.clone();
    let mut uncertainty_flags = snapshot.uncertainty_flags.clone();
    let mut reason_codes = snapshot.reason_codes.clone();
    let mut page_mode_confidence = snapshot.page_mode_confidence;
    let mut advisory_prototype_families = advisory_hits
        .iter()
        .map(|hit| hit.prototype_family.clone())
        .collect::<Vec<_>>();
    advisory_prototype_families.dedup();

    let advisory_model_used = !advisory_hits.is_empty();
    let advisory_consensus;
    if !advisory_model_used {
        advisory_consensus = "not_used".to_string();
    } else {
        let advisory_signal = advisory_signal_from_hits(advisory_hits);
        let mut mode_votes = BTreeMap::<String, f32>::new();
        let mut layer_votes = BTreeMap::<String, f32>::new();
        let mut country_votes = BTreeMap::<String, f32>::new();
        let mut visa_votes = BTreeMap::<String, f32>::new();
        let mut authority_votes = BTreeMap::<String, f32>::new();

        for hit in advisory_hits {
            *mode_votes.entry(hit.page_mode.clone()).or_insert(0.0) += hit.score;
            for layer in &hit.dominant_layers {
                *layer_votes.entry(layer.clone()).or_insert(0.0) += hit.score;
            }
            for hint in &hit.country_hints {
                *country_votes.entry(hint.clone()).or_insert(0.0) += hit.score;
            }
            for hint in &hit.visa_type_hints {
                *visa_votes.entry(hint.clone()).or_insert(0.0) += hit.score;
            }
            for hint in &hit.authority_hints {
                *authority_votes.entry(hint.clone()).or_insert(0.0) += hit.score;
            }
        }

        let top_mode = mode_votes
            .iter()
            .max_by(|lhs, rhs| lhs.1.total_cmp(rhs.1))
            .map(|(mode, score)| (mode.clone(), *score));
        advisory_consensus = if let Some((mode, score)) = top_mode {
            if mode == snapshot.page_mode_hint {
                page_mode_confidence = page_mode_confidence.max((0.70 + score / 4.0).min(0.92));
                reason_codes.push(format!("advisory_page_mode_confirmed:{mode}"));
                "agree".to_string()
            } else {
                page_mode_confidence = (page_mode_confidence * 0.85).max(0.45);
                uncertainty_flags.push(format!("advisory_page_mode_conflict:{mode}"));
                reason_codes.push(format!("advisory_page_mode_candidate:{mode}"));
                "conflict".to_string()
            }
        } else {
            "no_signal".to_string()
        };

        for (layer, vote) in layer_votes {
            let supported_sections = supportive_section_count_for_layer(sections, &layer);
            if !dominant_layers.contains(&layer) && vote >= 1.2 && supported_sections > 0 {
                dominant_layers.push(layer.clone());
                reason_codes.push(format!("advisory_layer_expansion:{layer}"));
            }
            let base = layer_scores.get(&layer).copied().unwrap_or(0.0);
            let fused = if supported_sections > 0 {
                base.max((vote / 3.0).min(0.85))
            } else {
                base
            };
            if fused > 0.0 {
                layer_scores.insert(layer, fused);
            }
        }

        if advisory_signal.mixed_section_pressure && mixed_section_ids.is_empty() {
            let advisory_mixed = sections
                .iter()
                .filter_map(|section| {
                    if section_mixed_layer_score(&section.content_md) >= 2 {
                        Some(section.id)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if !advisory_mixed.is_empty() {
                for section_id in advisory_mixed {
                    if !mixed_section_ids.contains(&section_id) {
                        mixed_section_ids.push(section_id);
                    }
                }
                reason_codes.push("advisory_mixed_section_support".to_string());
            }
        }
        if advisory_signal.retrieval_confidence >= 0.85 {
            reason_codes.push("high_confidence_advisory_retrieval".to_string());
        }

        for (hint, _) in country_votes {
            if page_context_profile.country_hints.contains(&hint) {
                reason_codes.push(format!("advisory_country_confirmed:{hint}"));
            } else {
                uncertainty_flags.push(format!("advisory_only_country_hint:{hint}"));
            }
        }
        for (hint, _) in visa_votes {
            if page_context_profile.visa_type_hints.contains(&hint) {
                reason_codes.push(format!("advisory_visa_confirmed:{hint}"));
            } else {
                uncertainty_flags.push(format!("advisory_only_visa_hint:{hint}"));
            }
        }
        for (hint, _) in authority_votes {
            if page_context_profile.authority_hints.contains(&hint) {
                reason_codes.push(format!("advisory_authority_confirmed:{hint}"));
            } else {
                uncertainty_flags.push(format!("advisory_only_authority_hint:{hint}"));
            }
        }
    }

    dominant_layers.sort();
    dominant_layers.dedup();
    page_context_profile.country_hints.sort();
    page_context_profile.country_hints.dedup();
    page_context_profile.visa_type_hints.sort();
    page_context_profile.visa_type_hints.dedup();
    page_context_profile.authority_hints.sort();
    page_context_profile.authority_hints.dedup();
    uncertainty_flags.sort();
    uncertainty_flags.dedup();
    reason_codes.sort();
    reason_codes.dedup();

    WholePageSemanticPageState {
        page_id: sections.first().map(|section| section.page_id).unwrap_or_default(),
        section_count: sections.len(),
        page_mode_hint: snapshot.page_mode_hint,
        page_mode_confidence,
        dominant_layers,
        layer_scores,
        page_summary: snapshot.page_summary,
        page_context_profile,
        mixed_section_ids,
        global_entities,
        advisory_model_used,
        advisory_consensus,
        advisory_prototype_families,
        uncertainty_flags,
        reason_codes,
    }
}

fn block_role_for_section(
    section: &raw_crawl_adapter::RawSectionRecord,
) -> seo_steps::dom_block_relevance_step::BlockRole {
    let section_type = section.section_type.to_lowercase();
    let heading = section.heading_path.to_lowercase();
    if section_type.contains("nav") || heading.contains("breadcrumb") {
        seo_steps::dom_block_relevance_step::BlockRole::Navigation
    } else if section_type.contains("toc") {
        seo_steps::dom_block_relevance_step::BlockRole::Toc
    } else if section_type.contains("footer") {
        seo_steps::dom_block_relevance_step::BlockRole::Footer
    } else {
        seo_steps::dom_block_relevance_step::BlockRole::ContentMain
    }
}

struct SectionSemanticGateIndexes {
    procedural_allowed: BTreeMap<i64, bool>,
    editorial_allowed: BTreeMap<i64, bool>,
    structural_allowed: BTreeMap<i64, bool>,
    dom_allowed: BTreeMap<i64, bool>,
    contract_pass: BTreeMap<i64, bool>,
    replay_safe: BTreeMap<i64, bool>,
}

impl SectionSemanticGateIndexes {
    fn from_bundle(bundle: &SectionSemanticGateBundle) -> Self {
        Self {
            procedural_allowed: bundle
                .page_utility
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.allow_procedural_extraction))
                .collect(),
            editorial_allowed: bundle
                .page_utility
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.allow_editorial_extraction))
                .collect(),
            structural_allowed: bundle
                .page_utility
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.allow_structural_extraction))
                .collect(),
            dom_allowed: bundle
                .dom_relevance
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.allow_extraction))
                .collect(),
            contract_pass: bundle
                .sectioning_contract
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.decision == "pass"))
                .collect(),
            replay_safe: bundle
                .cas_gate
                .decisions
                .iter()
                .map(|decision| (decision.section_id, decision.is_replay_safe))
                .collect(),
        }
    }

    fn blocked_by_gate(&self, section_id: i64) -> bool {
        !self.structural_allowed.get(&section_id).copied().unwrap_or(false)
            || !self.dom_allowed.get(&section_id).copied().unwrap_or(false)
            || !self.contract_pass.get(&section_id).copied().unwrap_or(false)
            || !self.replay_safe.get(&section_id).copied().unwrap_or(false)
    }

    fn allow_procedural_extraction(&self, section_id: i64) -> bool {
        self.procedural_allowed
            .get(&section_id)
            .copied()
            .unwrap_or(false)
    }

    fn allow_editorial_extraction(&self, section_id: i64) -> bool {
        self.editorial_allowed
            .get(&section_id)
            .copied()
            .unwrap_or(false)
    }
}

fn section_source_tier(section: &raw_crawl_adapter::RawSectionRecord) -> String {
    let dtype = section.source_dtype.to_ascii_lowercase();
    let domain = section.source_domain.to_ascii_lowercase();
    if dtype.contains("government")
        || dtype.contains("official")
        || domain.contains(".gov")
        || domain.contains("embassy")
        || domain.contains("consulate")
    {
        "government".to_string()
    } else if dtype.contains("vfs") || domain.contains("vfsglobal") {
        "vfs".to_string()
    } else if dtype.contains("editorial") || domain.contains("news") || domain.contains("blog") {
        "editorial".to_string()
    } else {
        "low_trust".to_string()
    }
}

fn find_evidence_span(raw_text: &str, candidates: &[String]) -> (usize, usize, String) {
    for candidate in candidates {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(start) = raw_text.find(trimmed) {
            let end = start + trimmed.len();
            return (start, end, trimmed.to_string());
        }
    }
    let fallback = raw_text.trim();
    if fallback.is_empty() {
        (0, 0, String::new())
    } else {
        let quote = fallback
            .split('.')
            .next()
            .unwrap_or(fallback)
            .trim()
            .to_string();
        let start = raw_text.find(&quote).unwrap_or(0);
        (start, start + quote.len(), quote)
    }
}

fn extract_first_number(token: &str) -> Option<f64> {
    let normalized = token.replace(',', ".");
    let digits = normalized
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    digits.parse::<f64>().ok()
}

fn extract_first_days(token: &str) -> Option<i64> {
    let digits = token
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse::<i64>().ok()
}

fn normalize_numeric_token_fragments(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .flat_map(|token| {
            let normalized = token.replace(',', ".");
            let fragments = normalized
                .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .filter(|fragment| !fragment.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if fragments.is_empty() {
                vec![normalized]
            } else {
                fragments
            }
        })
        .collect()
}

fn role_and_concept_for_rule(rule: &seo_steps::procedural_extraction_step::ProceduralRule) -> (&'static str, String) {
    match rule.rule_key.as_str() {
        "consular_fee" => ("FEE_ITEM", "consular_fee".to_string()),
        "processing_time" => ("TIMELINE_ITEM", "processing_time".to_string()),
        "passport_required" => ("DOCUMENT_REQUIRED", "passport".to_string()),
        "insurance_required" => ("DOCUMENT_REQUIRED", "medical_insurance".to_string()),
        _ => ("DOCUMENT_REQUIRED", rule.rule_key.clone()),
    }
}

fn params_for_rule(rule: &seo_steps::procedural_extraction_step::ProceduralRule) -> TruthParamValue {
    match rule.rule_key.as_str() {
        "consular_fee" => {
            let amount = rule
                .numeric_tokens
                .first()
                .and_then(|token| extract_first_number(token))
                .unwrap_or_default();
            TruthParamValue::object([
                ("amount", TruthParamValue::Decimal(amount)),
                ("currency", TruthParamValue::Text("EUR".to_string())),
            ])
        }
        "processing_time" => {
            let days = rule
                .numeric_tokens
                .first()
                .and_then(|token| extract_first_days(token))
                .unwrap_or_default();
            TruthParamValue::object([("days", TruthParamValue::Integer(days))])
        }
        "passport_required" => TruthParamValue::object([
            ("subtype", TruthParamValue::Text("passport".to_string())),
            ("severity", TruthParamValue::Text("mandatory".to_string())),
        ]),
        "insurance_required" => TruthParamValue::object([
            ("subtype", TruthParamValue::Text("medical_insurance".to_string())),
            ("severity", TruthParamValue::Text("mandatory".to_string())),
        ]),
        _ => TruthParamValue::object([(
            "subtype",
            TruthParamValue::Text(rule.rule_key.clone()),
        )]),
    }
}

fn truth_param_value_to_json_local(value: &TruthParamValue) -> Value {
    match value {
        TruthParamValue::Null => Value::Null,
        TruthParamValue::Bool(value) => Value::Bool(*value),
        TruthParamValue::Integer(value) => json!(value),
        TruthParamValue::Decimal(value) => json!(value),
        TruthParamValue::Text(value) => Value::String(value.clone()),
        TruthParamValue::List(values) => {
            Value::Array(values.iter().map(truth_param_value_to_json_local).collect())
        }
        TruthParamValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), truth_param_value_to_json_local(value)))
                .collect(),
        ),
    }
}

fn classify_candidate_completeness_local(
    validation: &TruthCandidateValidationResult,
) -> String {
    if validation.issues.iter().any(|issue| {
        matches!(
            issue.code.as_str(),
            "fee_item_incomplete"
                | "timeline_item_incomplete"
                | "where_to_apply_incomplete"
                | "role_specific_params_missing"
                | "missing_numeric_params"
                | "missing_range_bounds"
                | "candidate_marked_incomplete"
        )
    }) {
        "incomplete".to_string()
    } else if validation.epistemic_status == "needs_hitl" {
        "partial".to_string()
    } else {
        "complete".to_string()
    }
}

fn section_uncertainty_flags(raw_text: &str) -> Vec<String> {
    let lowered = raw_text.to_lowercase();
    let mut flags = Vec::new();
    if [
        "archived",
        "archive",
        "outdated",
        "obsolete",
        "retained for record-keeping",
        "retained for record keeping",
        "устар",
        "архив",
        "может быть устар",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        flags.push("stale_source".to_string());
    }
    flags
}

fn classify_candidate_freshness_local(uncertainty_flags: &[String]) -> String {
    if uncertainty_flags.iter().any(|flag| {
        flag == "stale_source" || flag == "validator:freshness_or_temporality_ambiguous"
    }) {
        "stale".to_string()
    } else if uncertainty_flags
        .iter()
        .any(|flag| flag == "freshness_ambiguous" || flag == "temporal_ambiguous")
    {
        "watch".to_string()
    } else {
        "fresh".to_string()
    }
}

fn non_structured_candidate_reason(candidate: &ValidatedTruthCandidateRecord) -> String {
    if candidate.freshness_class != "fresh" {
        format!(
            "freshness_block; freshness_class={}",
            candidate.freshness_class
        )
    } else if candidate.completeness_class == "incomplete" {
        format!(
            "completeness_block; completeness_class={}",
            candidate.completeness_class
        )
    } else if candidate.epistemic_status == "needs_hitl" {
        "candidate_validation_requires_hitl".to_string()
    } else {
        "non_structured_input".to_string()
    }
}

fn semantic_rule_instance_id_local(context_key: &str, role: &str, concept_canonical_key: &str) -> String {
    blake3_hex(format!("{context_key}|{role}|{concept_canonical_key}").as_bytes())
}

pub(crate) fn test_step_prepare_impl(workflow_id: &str) -> String {
    format!("prepared:{workflow_id}")
}

pub(crate) fn test_step_finalize_impl(prepared_token: &str) -> String {
    format!("completed:{prepared_token}")
}

pub(crate) async fn check_data_freshness_impl(
    acts: &AlegriaActivities,
    threshold_input: &str,
) -> Result<String, DomainError> {
    let threshold_hours: i64 = threshold_input
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(24);

    let snapshot = load_freshness_snapshot(&acts.pool, threshold_hours)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    let report = FreshnessReport {
        meta: Some(StepContractMeta {
            run_id: "operational:freshness".to_string(),
            step_name: "check_data_freshness".to_string(),
            schema_version: 1,
            input_hash: content_hash_v1(threshold_input),
            output_hash: String::new(),
            idempotency_key: content_hash_v1(&format!("operational:freshness|{}", threshold_input)),
            requires_hitl: false,
            prompt_version: String::new(),
            model_version: String::new(),
            registry_version: String::new(),
            error_class: String::new(),
            retry_class: "transient".to_string(),
            executor_version: AlegriaActivities::current_build_id(),
            derivation_version: "check_data_freshness@1".to_string(),
            scope_signature: String::new(),
            max_retries: 3,
        }),
        threshold_hours,
        stale_count: snapshot.stale_count,
        max_lag_hours: snapshot.max_lag_hours,
        status: if snapshot.stale_count > 0 {
            "stale"
        } else {
            "ok"
        }
        .to_string(),
    };

    serde_json::to_string(&report).map_err(AlegriaActivities::classify_error)
}

pub(crate) async fn neo4j_backwrite_impl(
    input: &Neo4jBackwriteInput,
) -> Result<Neo4jBackwriteOutput, DomainError> {
    let mut opts = sqlx_reconcile_adapter::load_default_reconcile_options();
    opts.dry_run = input.dry_run;
    if let Some(v) = input.max_retry_count {
        opts.max_retry_count = v;
    }
    if let Some(v) = input.batch_limit {
        opts.batch_limit = v;
    }
    if let Some(v) = input.requeue_base_delay_sec {
        opts.requeue_base_delay_sec = v;
    }
    if let Some(v) = input.requeue_jitter_sec {
        opts.requeue_jitter_sec = v;
    }

    let target = if input.target_system.trim().is_empty() {
        "neo4j"
    } else {
        input.target_system.as_str()
    };
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(target, &opts)
        .await
        .map_err(AlegriaActivities::classify_error)?;

    Ok(Neo4jBackwriteOutput {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub(crate) async fn projection_reconcile_impl(
    target_system: &str,
    dry_run: bool,
    max_retry_count: i32,
    batch_limit: i64,
    requeue_base_delay_sec: i64,
    requeue_jitter_sec: i64,
) -> Result<ReconcileTargetReportRecord, DomainError> {
    let report = sqlx_reconcile_adapter::reconcile_target_system_default(
        target_system,
        &sqlx_reconcile_adapter::ReconcileOptionsRecord {
            max_retry_count,
            batch_limit,
            dry_run,
            requeue_base_delay_sec,
            requeue_jitter_sec,
        },
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    Ok(ReconcileTargetReportRecord {
        target_system: report.target_system,
        dry_run: report.dry_run,
        stale_candidates: report.stale_candidates,
        failed_candidates: report.failed_candidates,
        reset_stale_processing: report.reset_stale_processing,
        requeued_failed: report.requeued_failed,
    })
}

pub(crate) async fn projection_sync_impl(
    acts: &AlegriaActivities,
    input: &ProjectionSyncInput,
) -> Result<ProjectionSyncOutput, DomainError> {
    let worker_id = format!("projection-sync:{}:{}", input.step_name, input.run_id);
    let batch = sqlx_outbox_adapter::claim_outbox_batch_for_run_target(
        &acts.pool,
        &worker_id,
        &input.run_id,
        &input.target_system,
        input.batch_limit.max(1),
        input.lease_seconds.max(30),
    )
    .await
    .map_err(AlegriaActivities::classify_error)?;

    let mut processed_events = 0_i64;
    let retried_events = 0_i64;
    let failed_events = 0_i64;

    for event in batch {
        match projection_materialize_adapter::dispatch_event(
            &event.target_system,
            &event.event_type,
            &event.aggregate_key,
            &event.payload_type,
            &event.payload_bytes,
        )
        .await
        {
            Ok(_) => {
                sqlx_outbox_adapter::mark_done(&acts.pool, event.event_id)
                    .await
                    .map_err(AlegriaActivities::classify_error)?;
                processed_events += 1;
            }
            Err(err) => {
                let message = err.to_string();
                if event.retry_count + 1 >= 10 {
                    sqlx_outbox_adapter::mark_failed(&acts.pool, event.event_id, &message)
                        .await
                        .map_err(AlegriaActivities::classify_error)?;
                } else {
                    sqlx_outbox_adapter::mark_retry(&acts.pool, event.event_id, &message, 30)
                        .await
                        .map_err(AlegriaActivities::classify_error)?;
                }
                return Err(DomainError::InfraUnavailable { message });
            }
        }
    }

    let backlog = sqlx::query(
        r#"
        SELECT
          COUNT(*) FILTER (WHERE status = 'pending') AS pending_events,
          COUNT(*) FILTER (WHERE status = 'failed') AS failed_events
        FROM system.sync_outbox
        WHERE run_id = $1
          AND target_system = $2
        "#,
    )
    .bind(&input.run_id)
    .bind(&input.target_system)
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;

    let remaining_pending = backlog.get::<i64, _>("pending_events");
    let remaining_failed = backlog.get::<i64, _>("failed_events");

    Ok(ProjectionSyncOutput {
        run_id: input.run_id.clone(),
        target_system: input.target_system.clone(),
        processed_events,
        retried_events,
        failed_events,
        remaining_pending,
        remaining_failed,
        status: if remaining_pending == 0 && remaining_failed == 0 && retried_events == 0 {
            "done".to_string()
        } else if failed_events > 0 || retried_events > 0 {
            "blocked".to_string()
        } else {
            "partial".to_string()
        },
    })
}

pub(crate) async fn load_semantic_section_sample_impl(
    acts: &AlegriaActivities,
    input: &SemanticSectionSampleInput,
) -> Result<SemanticSectionSampleOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids)
            .await
            .map_err(AlegriaActivities::classify_error)?;
    let section = sections
        .into_iter()
        .find(|section| !section.content_md.trim().is_empty())
        .ok_or_else(|| DomainError::ValidationFailure {
            message: "no non-empty raw section available for semantic slice".to_string(),
        })?;

    Ok(SemanticSectionSampleOutput {
        section_id: section.id.to_string(),
        page_id: section.page_id,
        source_url: section.source_url,
        source_domain: section.source_domain,
        heading_path: section.heading_path,
        section_type: section.section_type,
        raw_text: section.content_md,
    })
}

pub(crate) async fn seo_preflight_impl(
    acts: &AlegriaActivities,
    input: &SeoPreflightInput,
) -> Result<SeoPreflightOutput, DomainError> {
    sqlx_seo_adapter::ensure_seo_runtime_registries(&acts.pool).await?;
    let scope = seo_domain::identity::derive_scope_from_payload(&input.scope)?;
    let normalized_profile =
        sqlx_seo_adapter::validate_applicant_profile_reference(&acts.pool, &scope.applicant_profile)
            .await?;

    let context_row = sqlx::query(
        "SELECT count(*)::bigint AS count FROM kb.visa_contexts WHERE context_key = $1 AND status = 'active'",
    )
    .bind(&input.context_key)
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;
    let context_count: i64 = context_row.get("count");
    if context_count == 0 {
        return Err(DomainError::ValidationFailure {
            message: format!("active context is missing for `{}`", input.context_key),
        });
    }

    let page_type_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM site.registry_page_types WHERE status = 'active'",
    )
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;
    let page_node_count: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM site.page_nodes")
        .fetch_one(&*acts.pool)
        .await
        .map_err(AlegriaActivities::classify_error)?;
    let navigation_item_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM site.navigation_items")
            .fetch_one(&*acts.pool)
            .await
            .map_err(AlegriaActivities::classify_error)?;
    let verified_rule_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key = $1 AND status = 'verified'",
    )
    .bind(&input.context_key)
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;
    let pending_rule_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM verified.rule_instances WHERE context_key = $1 AND status = 'pending'",
    )
    .bind(&input.context_key)
    .fetch_one(&*acts.pool)
    .await
    .map_err(AlegriaActivities::classify_error)?;
    let qdrant_point_count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM kb.qdrant_points")
            .fetch_one(&*acts.pool)
            .await
            .map_err(AlegriaActivities::classify_error)?;

    let projection_statuses = sqlx_seo_adapter::read_projection_sync_status(&acts.pool).await?;
    let projection_blocked = projection_statuses.iter().any(|status| {
        status.failed_events > 0
            || (status.open_event_count() > 0
                && status.max_open_lag_ms > input.projection_max_lag_ms)
    });

    Ok(SeoPreflightOutput {
        context_key: input.context_key.clone(),
        normalized_profile,
        page_type_count,
        page_node_count,
        navigation_item_count,
        verified_rule_count,
        pending_rule_count,
        qdrant_point_count,
        projection_blocked,
        status: if projection_blocked { "warn" } else { "ok" }.to_string(),
    })
}

pub(crate) async fn whole_page_semantic_pass_impl(
    acts: &AlegriaActivities,
    input: &WholePageSemanticPassInput,
) -> Result<WholePageSemanticPassOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let mut by_page: BTreeMap<i64, Vec<raw_crawl_adapter::RawSectionRecord>> = BTreeMap::new();
    for section in sections {
        by_page.entry(section.page_id).or_default().push(section);
    }
    let pages = by_page
        .into_iter()
        .map(|(_page_id, sections)| async move {
            let combined = sections
                .iter()
                .map(|section| section.content_md.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let snapshot = deterministic_whole_page_snapshot(&sections, &combined);
            let page_sketch = build_page_sketch(sections[0].page_id, &sections, &snapshot);
            let advisory_hits =
                match whole_page_advisory_adapter::search_whole_page_prototypes(
                    &page_sketch,
                    advisory_hit_limit(),
                )
                    .await
                {
                    Ok(hits) => hits,
                    Err(err) => {
                        tracing::warn!(
                            page_id = sections[0].page_id,
                            error = %err,
                            "whole-page advisory retrieval unavailable; falling back to deterministic semantics"
                        );
                        Vec::new()
                    }
                };
            fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits)
        })
        .collect::<Vec<_>>();
    let mut resolved_pages = Vec::with_capacity(pages.len());
    for page in pages {
        resolved_pages.push(page.await);
    }
    Ok(WholePageSemanticPassOutput {
        page_count: resolved_pages.len(),
        pages: resolved_pages,
    })
}

pub(crate) async fn sectioning_impl(
    acts: &AlegriaActivities,
    input: &SectioningInput,
) -> Result<SectioningOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let page_count = sections
        .iter()
        .map(|section| section.page_id)
        .collect::<BTreeSet<_>>()
        .len();
    let mapped = sections
        .iter()
        .map(|section| SectioningSectionState {
            section_id: section.id,
            page_id: section.page_id,
            source_url: section.source_url.clone(),
            heading_path: section.heading_path.clone(),
            section_type: section.section_type.clone(),
            content_hash: section.content_hash.clone(),
            text_len: section.content_md.len(),
        })
        .collect::<Vec<_>>();
    Ok(SectioningOutput {
        page_count,
        section_count: mapped.len(),
        sections: mapped,
    })
}

pub(crate) async fn page_utility_sweep_impl(
    acts: &AlegriaActivities,
    input: &PageUtilitySweepInput,
) -> Result<PageUtilitySweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let output = seo_steps::page_utility_classifier_step::execute(
                &seo_steps::page_utility_classifier_step::PageUtilityClassifierInput {
                    url: section.source_url.clone(),
                    title: section.heading_path.clone(),
                    raw_text: section.content_md.clone(),
                },
            );
            PageUtilitySectionDecision {
                section_id: section.id,
                page_id: section.page_id,
                allow_procedural_extraction: output.allow_procedural_extraction,
                allow_editorial_extraction: output.allow_editorial_extraction,
                allow_structural_extraction: output.allow_structural_extraction,
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.allow_structural_extraction)
        .count();
    Ok(PageUtilitySweepOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn dom_block_relevance_sweep_impl(
    acts: &AlegriaActivities,
    input: &DomBlockRelevanceSweepInput,
) -> Result<DomBlockRelevanceSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let role = block_role_for_section(section);
            let output = seo_steps::dom_block_relevance_step::execute(&[
                seo_steps::dom_block_relevance_step::DomBlockInput {
                    dom_block_id: format!("raw-section:{}", section.id),
                    block_role: role.clone(),
                    text: section.content_md.clone(),
                },
            ]);
            let block = output.blocks.into_iter().next().expect("single block");
            DomBlockRelevanceSectionDecision {
                section_id: section.id,
                page_id: section.page_id,
                block_role: format!("{:?}", role).to_lowercase(),
                allow_extraction: block.allow_extraction,
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.allow_extraction)
        .count();
    Ok(DomBlockRelevanceSweepOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn sectioning_contract_gate_impl(
    acts: &AlegriaActivities,
    input: &SectioningContractGateInput,
) -> Result<SectioningContractGateOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| {
            let heading_present = !section.heading_path.trim().is_empty();
            let text_present = !section.content_md.trim().is_empty();
            SectioningContractDecision {
                section_id: section.id,
                page_id: section.page_id,
                heading_present,
                text_present,
                decision: if heading_present && text_present {
                    "pass".to_string()
                } else {
                    "blocked".to_string()
                },
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| decision.decision != "pass")
        .count();
    Ok(SectioningContractGateOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn cas_gate_impl(
    acts: &AlegriaActivities,
    input: &CasGateInput,
) -> Result<CasGateOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let decisions = sections
        .iter()
        .map(|section| CasGateDecision {
            section_id: section.id,
            page_id: section.page_id,
            snapshot_hash: if section.content_hash.trim().is_empty() {
                content_hash_v1(&section.content_md)
            } else {
                section.content_hash.clone()
            },
            is_replay_safe: !section.content_md.trim().is_empty(),
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| !decision.is_replay_safe)
        .count();
    Ok(CasGateOutput {
        section_count: decisions.len(),
        blocked_section_count,
        decisions,
    })
}

pub(crate) async fn raw_evidence_register_impl(
    acts: &AlegriaActivities,
    input: &RawEvidenceRegisterInput,
) -> Result<RawEvidenceRegisterOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let page_count = sections
        .iter()
        .map(|section| section.page_id)
        .collect::<BTreeSet<_>>()
        .len();
    let unique_source_count = sections
        .iter()
        .map(|section| section.source_url.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let evidence_refs = sections
        .iter()
        .map(|section| format!("raw.section:{}", section.id))
        .collect::<Vec<_>>();
    Ok(RawEvidenceRegisterOutput {
        context_key: input.context_key.clone(),
        page_count,
        section_count: evidence_refs.len(),
        unique_source_count,
        evidence_refs,
        status: "registered".to_string(),
    })
}

pub(crate) async fn layer_router_sweep_impl(
    acts: &AlegriaActivities,
    input: &LayerRouterSweepInput,
) -> Result<LayerRouterSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let decisions = sections
        .iter()
        .map(|section| {
            let output = seo_steps::layer_router_step::execute(
                &seo_steps::layer_router_step::LayerRouterInput {
                    section_id: section.id.to_string(),
                    heading_text: section.heading_path.clone(),
                    raw_text: section.content_md.clone(),
                    source_tier: section.source_dtype.clone(),
                    block_type: section.section_type.clone(),
                },
            );
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let decision = if blocked_by_gate {
                "blocked_by_gate"
            } else if output.needs_hitl {
                "needs_hitl"
            } else {
                "pass"
            };
            LayerRouterSectionDecision {
                section_id: section.id,
                page_id: section.page_id,
                primary_layer: output.primary_layer,
                secondary_layers: output.secondary_layers,
                confidence: output.confidence,
                needs_hitl: output.needs_hitl,
                blocked_by_gate,
                decision: decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| decision.blocked_by_gate)
        .count();
    let needs_hitl_count = decisions
        .iter()
        .filter(|decision| !decision.blocked_by_gate && decision.needs_hitl)
        .count();
    Ok(LayerRouterSweepOutput {
        section_count: decisions.len(),
        blocked_section_count,
        needs_hitl_count,
        decisions,
    })
}

pub(crate) async fn subspan_layer_router_impl(
    input: &SubspanLayerRouterInput,
) -> Result<SubspanLayerRouterOutput, DomainError> {
    let decisions = input
        .layer_router
        .decisions
        .iter()
        .map(|decision| {
            let mixed_layers = decision
                .secondary_layers
                .iter()
                .map(|layer| layer.layer.clone())
                .collect::<Vec<_>>();
            let needs_split = !mixed_layers.is_empty();
            let stage_decision = if decision.blocked_by_gate {
                "blocked_by_gate"
            } else if needs_split {
                "needs_hitl"
            } else {
                "pass"
            };
            SubspanLayerRouterDecision {
                section_id: decision.section_id,
                page_id: decision.page_id,
                mixed_layers,
                needs_split,
                blocked_by_gate: decision.blocked_by_gate,
                decision: stage_decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = decisions
        .iter()
        .filter(|decision| decision.blocked_by_gate)
        .count();
    let needs_split_count = decisions
        .iter()
        .filter(|decision| !decision.blocked_by_gate && decision.needs_split)
        .count();
    Ok(SubspanLayerRouterOutput {
        section_count: decisions.len(),
        blocked_section_count,
        needs_split_count,
        decisions,
    })
}

pub(crate) async fn entity_span_sweep_impl(
    acts: &AlegriaActivities,
    input: &EntitySpanSweepInput,
) -> Result<EntitySpanSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_outputs = sections
        .iter()
        .map(|section| {
            let output = seo_steps::entity_span_detection_step::execute(
                &seo_steps::entity_span_detection_step::EntitySpanInput {
                    section_id: section.id.to_string(),
                    raw_text: section.content_md.clone(),
                },
            );
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            EntitySpanSectionMentions {
                section_id: section.id,
                page_id: section.page_id,
                mentions: output.mentions,
                blocked_by_gate,
                decision: if blocked_by_gate {
                    "blocked_by_gate".to_string()
                } else {
                    "pass".to_string()
                },
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = section_outputs
        .iter()
        .filter(|section| section.blocked_by_gate)
        .count();
    let mention_count = section_outputs
        .iter()
        .map(|section| section.mentions.len())
        .sum();
    Ok(EntitySpanSweepOutput {
        section_count: section_outputs.len(),
        blocked_section_count,
        mention_count,
        sections: section_outputs,
    })
}

pub(crate) async fn canonical_mapping_sweep_impl(
    input: &CanonicalMappingSweepInput,
) -> Result<CanonicalMappingSweepOutput, DomainError> {
    let sections = input
        .entity_spans
        .sections
        .iter()
        .map(|section| {
            let output = seo_steps::canonical_mapping_step::execute(
                &seo_steps::canonical_mapping_step::CanonicalMappingInput {
                    section_id: section.section_id.to_string(),
                    mentions: section
                        .mentions
                        .iter()
                        .map(
                            |mention| seo_steps::canonical_mapping_step::MentionForMapping {
                                raw_text: mention.raw_text.clone(),
                                entity_type: mention.entity_type.clone(),
                            },
                        )
                        .collect(),
                },
            );
            let needs_hitl = output
                .mappings
                .iter()
                .any(|mapping| mapping.needs_hitl);
            let decision = if section.blocked_by_gate {
                "blocked_by_gate"
            } else if needs_hitl {
                "needs_hitl"
            } else {
                "pass"
            };
            CanonicalMappingSectionState {
                section_id: section.section_id,
                page_id: section.page_id,
                mappings: output.mappings,
                blocked_by_gate: section.blocked_by_gate,
                needs_hitl,
                decision: decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = sections
        .iter()
        .filter(|section| section.blocked_by_gate)
        .count();
    let needs_hitl_count = sections
        .iter()
        .filter(|section| !section.blocked_by_gate && section.needs_hitl)
        .count();
    Ok(CanonicalMappingSweepOutput {
        section_count: sections.len(),
        blocked_section_count,
        needs_hitl_count,
        sections,
    })
}

pub(crate) async fn ontology_intake_gate_impl(
    input: &OntologyIntakeGateInput,
) -> Result<OntologyIntakeGateOutput, DomainError> {
    let sections = input
        .canonical_mapping
        .sections
        .iter()
        .map(|section| {
            let unresolved_mentions = section
                .mappings
                .iter()
                .filter(|mapping| mapping.needs_hitl || mapping.canonical_key.is_none())
                .map(|mapping| mapping.raw_text.clone())
                .collect::<Vec<_>>();
            let accepted_keys = section
                .mappings
                .iter()
                .filter_map(|mapping| mapping.canonical_key.clone())
                .collect::<Vec<_>>();
            let needs_hitl = !unresolved_mentions.is_empty();
            let decision = if section.blocked_by_gate {
                "blocked_by_gate"
            } else if needs_hitl {
                "needs_hitl"
            } else {
                "pass"
            };
            OntologyIntakeGateDecision {
                section_id: section.section_id,
                page_id: section.page_id,
                accepted_keys,
                unresolved_mentions,
                blocked_by_gate: section.blocked_by_gate,
                needs_hitl,
                decision: decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let blocked_section_count = sections
        .iter()
        .filter(|section| section.blocked_by_gate)
        .count();
    let needs_hitl_count = sections
        .iter()
        .filter(|section| !section.blocked_by_gate && section.needs_hitl)
        .count();
    Ok(OntologyIntakeGateOutput {
        section_count: sections.len(),
        blocked_section_count,
        needs_hitl_count,
        sections,
    })
}

pub(crate) async fn procedural_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &ProceduralExtractionSweepInput,
) -> Result<ProceduralExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let entity_by_section: BTreeMap<i64, &EntitySpanSectionMentions> = input
        .entity_spans
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            // Procedural extraction remains allowed even when ontology intake marked
            // entity-span mentions as unresolved. Otherwise fee/timeline sections with
            // deterministic numeric patterns get dropped before the strict procedural
            // path can build candidates, which weakens expert extraction and turns
            // ontology ambiguity into a false hard skip.
            let skipped = !gates.allow_procedural_extraction(section.id);
            let rules = if blocked_by_gate || skipped {
                Vec::new()
            } else {
                let entity = entity_by_section.get(&section.id).copied().unwrap();
                seo_steps::procedural_extraction_step::execute(
                    &seo_steps::procedural_extraction_step::ProceduralExtractionInput {
                        section_id: section.id.to_string(),
                        raw_text: section.content_md.clone(),
                        mentions: entity.mentions.clone(),
                    },
                )
                .rules
            };
            let decision = if blocked_by_gate {
                "blocked_by_gate"
            } else if skipped {
                "skipped"
            } else {
                "pass"
            };
            ProceduralExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                rules,
                blocked_by_gate,
                skipped,
                decision: decision.to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(ProceduralExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        skipped_section_count: section_states.iter().filter(|section| section.skipped).count(),
        rule_count: section_states.iter().map(|section| section.rules.len()).sum(),
        sections: section_states,
    })
}

pub(crate) async fn operational_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &OperationalExtractionSweepInput,
) -> Result<OperationalExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let entities = if blocked_by_gate {
                Vec::new()
            } else {
                seo_steps::operational_extraction_step::execute(
                    &seo_steps::operational_extraction_step::OperationalExtractionInput {
                        section_id: section.id.to_string(),
                        raw_text: section.content_md.clone(),
                    },
                )
                .entities
            };
            OperationalExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                entities,
                blocked_by_gate,
                decision: if blocked_by_gate {
                    "blocked_by_gate"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(OperationalExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        entity_count: section_states.iter().map(|section| section.entities.len()).sum(),
        sections: section_states,
    })
}

pub(crate) async fn editorial_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &EditorialExtractionSweepInput,
) -> Result<EditorialExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let skipped = !gates.allow_editorial_extraction(section.id);
            let topics = if blocked_by_gate || skipped {
                Vec::new()
            } else {
                seo_steps::editorial_extraction_step::execute(
                    &seo_steps::editorial_extraction_step::EditorialExtractionInput {
                        section_id: section.id.to_string(),
                        raw_text: section.content_md.clone(),
                    },
                )
                .topics
            };
            EditorialExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                topics,
                blocked_by_gate,
                skipped,
                decision: if blocked_by_gate {
                    "blocked_by_gate"
                } else if skipped {
                    "skipped"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(EditorialExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        skipped_section_count: section_states.iter().filter(|section| section.skipped).count(),
        topic_count: section_states.iter().map(|section| section.topics.len()).sum(),
        sections: section_states,
    })
}

pub(crate) async fn seo_signal_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &SeoSignalExtractionSweepInput,
) -> Result<SeoSignalExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let lowered = section.content_md.to_lowercase();
            let mut signals = Vec::new();
            if !blocked_by_gate && (lowered.contains("seo") || lowered.contains("serp")) {
                signals.push(SeoSignalRecord {
                    signal_type: "ranking_signal".to_string(),
                    value: "seo_or_serp_mentioned".to_string(),
                    confidence: 0.82,
                });
            }
            if !blocked_by_gate && (lowered.contains("keyword") || lowered.contains("ключев")) {
                signals.push(SeoSignalRecord {
                    signal_type: "keyword_signal".to_string(),
                    value: "keyword_language_present".to_string(),
                    confidence: 0.79,
                });
            }
            SeoSignalExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                signals,
                blocked_by_gate,
                decision: if blocked_by_gate {
                    "blocked_by_gate"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(SeoSignalExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        signal_count: section_states.iter().map(|section| section.signals.len()).sum(),
        sections: section_states,
    })
}

pub(crate) async fn commercial_signal_extraction_sweep_impl(
    acts: &AlegriaActivities,
    input: &CommercialSignalExtractionSweepInput,
) -> Result<CommercialSignalExtractionSweepOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let gates = SectionSemanticGateIndexes::from_bundle(&input.gates);
    let section_states = sections
        .iter()
        .map(|section| {
            let blocked_by_gate = gates.blocked_by_gate(section.id);
            let lowered = section.content_md.to_lowercase();
            let mut signals = Vec::new();
            if !blocked_by_gate
                && (lowered.contains("консультац")
                    || lowered.contains("под ключ")
                    || lowered.contains("заказать")
                    || lowered.contains("service"))
            {
                signals.push(CommercialSignalRecord {
                    signal_type: "service_offer".to_string(),
                    value: "service_offer_detected".to_string(),
                    confidence: 0.84,
                });
            }
            if !blocked_by_gate
                && (lowered.contains("стоимость услуги")
                    || lowered.contains("our fee")
                    || lowered.contains("price"))
            {
                signals.push(CommercialSignalRecord {
                    signal_type: "commercial_price".to_string(),
                    value: "commercial_price_detected".to_string(),
                    confidence: 0.81,
                });
            }
            CommercialSignalExtractionSectionState {
                section_id: section.id,
                page_id: section.page_id,
                signals,
                blocked_by_gate,
                decision: if blocked_by_gate {
                    "blocked_by_gate"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(CommercialSignalExtractionSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        signal_count: section_states.iter().map(|section| section.signals.len()).sum(),
        sections: section_states,
    })
}

pub(crate) async fn extraction_schema_validate_impl(
    input: &ExtractionSchemaValidateInput,
) -> Result<ExtractionSchemaValidateOutput, DomainError> {
    let sections = input
        .procedural
        .sections
        .iter()
        .map(|section| {
            let mut reasons = Vec::new();
            if !section.blocked_by_gate && !section.skipped {
                if section
                    .rules
                    .iter()
                    .any(|rule| rule.rule_key.trim().is_empty() || rule.confidence <= 0.0)
                {
                    reasons.push("invalid_procedural_candidate_shape".to_string());
                }
            }
            let status = if section.blocked_by_gate {
                "blocked"
            } else if !reasons.is_empty() {
                "invalid"
            } else {
                "valid"
            };
            ExtractionSchemaSectionDecision {
                section_id: section.section_id,
                page_id: section.page_id,
                status: status.to_string(),
                blocking_reasons: reasons,
                blocked_by_gate: section.blocked_by_gate,
            }
        })
        .collect::<Vec<_>>();
    Ok(ExtractionSchemaValidateOutput {
        section_count: sections.len(),
        blocked_section_count: sections.iter().filter(|section| section.blocked_by_gate).count(),
        invalid_section_count: sections
            .iter()
            .filter(|section| section.status == "invalid")
            .count(),
        sections,
    })
}

pub(crate) async fn candidate_validation_impl(
    acts: &AlegriaActivities,
    input: &CandidateValidationInput,
) -> Result<CandidateValidationOutput, DomainError> {
    let sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let raw_by_section: BTreeMap<i64, &raw_crawl_adapter::RawSectionRecord> =
        sections.iter().map(|section| (section.id, section)).collect();
    let schema_by_section: BTreeMap<i64, &ExtractionSchemaSectionDecision> = input
        .schema_validate
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let ontology_by_section: BTreeMap<i64, &OntologyIntakeGateDecision> = input
        .ontology
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let mut section_states = Vec::new();
    for section in &input.procedural.sections {
        let raw = raw_by_section
            .get(&section.section_id)
            .ok_or_else(|| DomainError::ValidationFailure {
                message: format!(
                    "missing raw section for candidate validation: {}",
                    section.section_id
                ),
            })?;
        let schema = schema_by_section
            .get(&section.section_id)
            .ok_or_else(|| DomainError::ValidationFailure {
                message: format!(
                    "missing schema validation section for {}",
                    section.section_id
                ),
            })?;
        let ontology = ontology_by_section
            .get(&section.section_id)
            .ok_or_else(|| DomainError::ValidationFailure {
                message: format!("missing ontology section for {}", section.section_id),
            })?;
        let mut candidates = Vec::new();
        for (ordinal, rule) in section.rules.iter().enumerate() {
            let (role, concept_key) = role_and_concept_for_rule(rule);
            let evidence_targets = if !rule.numeric_tokens.is_empty() {
                rule.numeric_tokens.clone()
            } else {
                vec![match rule.rule_key.as_str() {
                    "passport_required" => "паспорт".to_string(),
                    "insurance_required" => "страхов".to_string(),
                    _ => rule.rule_key.clone(),
                }]
            };
            let (span_start, span_end, evidence_quote) =
                find_evidence_span(&raw.content_md, &evidence_targets);
            let uncertainty_flags = section_uncertainty_flags(&raw.content_md);
            let runtime = TruthCandidateRuntime {
                rule_candidate_id: blake3_hex(
                    format!(
                        "{}|{}|{}|{}|{}",
                        input.context_key, raw.id, ordinal, role, concept_key
                    )
                    .as_bytes(),
                ),
                context_key: input.context_key.clone(),
                role: role.to_string(),
                concept_canonical_key: concept_key.clone(),
                raw_mention: evidence_quote.clone(),
                params: params_for_rule(rule),
                scope: TruthParamValue::object(Vec::<(String, TruthParamValue)>::new()),
                severity: "mandatory".to_string(),
                derivation_type: "deterministic_procedural_extraction".to_string(),
                confidence: rule.confidence as f64,
                evidence_section_id: raw.id,
                evidence_quote: evidence_quote.clone(),
                span_start,
                span_end,
                source_key: raw.source_url.clone(),
                source_tier: section_source_tier(raw),
                source_snapshot_hash: if raw.content_hash.trim().is_empty() {
                    content_hash_v1(&raw.content_md)
                } else {
                    raw.content_hash.clone()
                },
                is_numeric: !rule.numeric_tokens.is_empty(),
                is_range: false,
                is_incomplete: false,
                uncertainty_flags: uncertainty_flags.clone(),
            };
            let validation = validate_truth_candidate(&runtime, &raw.content_md);
            let mut epistemic_status = validation.epistemic_status.clone();
            if ontology.needs_hitl && epistemic_status != "rejected" {
                epistemic_status = "needs_hitl".to_string();
            }
            if schema.status == "invalid" && epistemic_status != "rejected" {
                epistemic_status = "rejected".to_string();
            }
            candidates.push(ValidatedTruthCandidateRecord {
                section_id: section.section_id,
                page_id: section.page_id,
                rule_candidate_id: runtime.rule_candidate_id,
                role: runtime.role,
                concept_canonical_key: runtime.concept_canonical_key,
                raw_mention: runtime.raw_mention,
                params: runtime.params,
                source_key: runtime.source_key,
                source_tier: runtime.source_tier,
                confidence: runtime.confidence,
                evidence_quote: runtime.evidence_quote,
                span_start: runtime.span_start,
                span_end: runtime.span_end,
                source_snapshot_hash: runtime.source_snapshot_hash,
                freshness_class: classify_candidate_freshness_local(&uncertainty_flags),
                completeness_class: classify_candidate_completeness_local(&validation),
                epistemic_status,
                issues: validation
                    .issues
                    .iter()
                    .map(|issue| format!("{}:{}", issue.code, issue.message))
                    .collect(),
            });
        }
        let accepted_count = candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "structured")
            .count();
        let needs_hitl_count = candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "needs_hitl")
            .count();
        let rejected_count = candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "rejected")
            .count();
        let status = if section.blocked_by_gate {
            "blocked"
        } else if needs_hitl_count > 0 {
            "needs_hitl"
        } else if rejected_count > 0 && accepted_count == 0 {
            "rejected"
        } else {
            "accepted"
        };
        section_states.push(CandidateValidationSectionState {
            section_id: section.section_id,
            page_id: section.page_id,
            candidates,
            accepted_count,
            needs_hitl_count,
            rejected_count,
            blocked_by_gate: section.blocked_by_gate,
            status: status.to_string(),
        });
    }
    Ok(CandidateValidationOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.blocked_by_gate)
            .count(),
        accepted_count: section_states.iter().map(|section| section.accepted_count).sum(),
        needs_hitl_count: section_states
            .iter()
            .map(|section| section.needs_hitl_count)
            .sum(),
        rejected_count: section_states.iter().map(|section| section.rejected_count).sum(),
        sections: section_states,
    })
}

pub(crate) async fn triple_builder_sweep_impl(
    input: &TripleBuilderSweepInput,
) -> Result<TripleBuilderSweepOutput, DomainError> {
    let operational_by_section: BTreeMap<i64, &OperationalExtractionSectionState> = input
        .operational
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let editorial_by_section: BTreeMap<i64, &EditorialExtractionSectionState> = input
        .editorial
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let sections = input
        .procedural
        .sections
        .iter()
        .map(|section| {
            let operational = operational_by_section
                .get(&section.section_id)
                .copied()
                .unwrap();
            let editorial = editorial_by_section
                .get(&section.section_id)
                .copied()
                .unwrap();
            let output = seo_steps::triple_builder_step::execute(
                &seo_steps::triple_builder_step::TripleBuilderInput {
                    section_id: section.section_id.to_string(),
                    procedural_rules: section
                        .rules
                        .iter()
                        .map(|rule| seo_steps::triple_builder_step::ProceduralRuleForTriple {
                            rule_key: rule.rule_key.clone(),
                            role_type: rule.role_type,
                        })
                        .collect(),
                    operational_entities: operational
                        .entities
                        .iter()
                        .map(
                            |entity| seo_steps::triple_builder_step::OperationalEntityForTriple {
                                entity_kind: entity.entity_kind.clone(),
                                value: entity.value.clone(),
                            },
                        )
                        .collect(),
                    editorial_topics: editorial
                        .topics
                        .iter()
                        .map(
                            |topic| seo_steps::triple_builder_step::EditorialTopicForTriple {
                                topic_type: topic.topic_type.clone(),
                                topic_key_candidate: topic.topic_key_candidate.clone(),
                            },
                        )
                        .collect(),
                },
            );
            TripleBuilderSectionState {
                section_id: section.section_id,
                page_id: section.page_id,
                triples: output.triples,
                blocked_by_gate: section.blocked_by_gate,
                decision: if section.blocked_by_gate {
                    "blocked_by_gate"
                } else {
                    "pass"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    Ok(TripleBuilderSweepOutput {
        section_count: sections.len(),
        blocked_section_count: sections.iter().filter(|section| section.blocked_by_gate).count(),
        triple_count: sections.iter().map(|section| section.triples.len()).sum(),
        sections,
    })
}

pub(crate) async fn completeness_judge_sweep_impl(
    acts: &AlegriaActivities,
    input: &CompletenessJudgeSweepInput,
) -> Result<CompletenessJudgeSweepOutput, DomainError> {
    let raw_sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let raw_by_section: BTreeMap<i64, &raw_crawl_adapter::RawSectionRecord> =
        raw_sections.iter().map(|section| (section.id, section)).collect();
    let entity_by_section: BTreeMap<i64, &EntitySpanSectionMentions> = input
        .entity_spans
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let sections = input
        .procedural
        .sections
        .iter()
        .map(|section| {
            let raw = raw_by_section.get(&section.section_id).copied().unwrap();
            let entity = entity_by_section.get(&section.section_id).copied().unwrap();
            let numeric_tokens: BTreeSet<String> = section
                .rules
                .iter()
                .flat_map(|rule| normalize_numeric_token_fragments(&rule.numeric_tokens))
                .collect();
            let source_numeric_tokens: BTreeSet<String> = entity
                .mentions
                .iter()
                .filter(|mention| mention.has_numeric)
                .flat_map(|mention| normalize_numeric_token_fragments(std::slice::from_ref(&mention.raw_text)))
                .collect();
            let output = seo_steps::completeness_judge_step::execute(
                &seo_steps::completeness_judge_step::CompletenessJudgeInput {
                    section_id: section.section_id.to_string(),
                    raw_text: raw.content_md.clone(),
                    source_numeric_tokens: source_numeric_tokens.into_iter().collect(),
                    extracted_numeric_tokens: numeric_tokens.into_iter().collect(),
                    extracted_rule_keys: section
                        .rules
                        .iter()
                        .map(|rule| rule.rule_key.clone())
                        .collect(),
                },
            );
            CompletenessJudgeSectionState {
                section_id: section.section_id,
                page_id: section.page_id,
                blocked_by_gate: section.blocked_by_gate,
                status: if section.blocked_by_gate {
                    "blocked"
                } else if output.needs_hitl {
                    "needs_hitl"
                } else {
                    "pass"
                }
                .to_string(),
                output,
            }
        })
        .collect::<Vec<_>>();
    Ok(CompletenessJudgeSweepOutput {
        section_count: sections.len(),
        blocked_section_count: sections.iter().filter(|section| section.blocked_by_gate).count(),
        needs_hitl_count: sections
            .iter()
            .filter(|section| !section.blocked_by_gate && section.output.needs_hitl)
            .count(),
        sections,
    })
}

pub(crate) async fn resolution_loop_impl(
    input: &ResolutionLoopInput,
) -> Result<ResolutionLoopOutput, DomainError> {
    let ontology_by_section: BTreeMap<i64, &OntologyIntakeGateDecision> = input
        .ontology
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let schema_by_section: BTreeMap<i64, &ExtractionSchemaSectionDecision> = input
        .schema_validate
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let sections = input
        .completeness
        .sections
        .iter()
        .map(|section| {
            let ontology = ontology_by_section.get(&section.section_id).copied().unwrap();
            let schema = schema_by_section.get(&section.section_id).copied().unwrap();
            let decision = if section.blocked_by_gate {
                "drop_with_reason"
            } else if section.output.needs_hitl || ontology.needs_hitl {
                "pause_for_hitl"
            } else if schema.status == "invalid" {
                "drop_with_reason"
            } else {
                "accept"
            };
            let blockers = section
                .output
                .missing_elements
                .iter()
                .map(|missing| missing.action.clone())
                .chain(ontology.unresolved_mentions.iter().cloned())
                .collect::<Vec<_>>();
            ResolutionLoopSectionState {
                section_id: section.section_id,
                page_id: section.page_id,
                decision: decision.to_string(),
                blockers,
                needs_hitl: !section.blocked_by_gate
                    && (section.output.needs_hitl || ontology.needs_hitl),
                blocked_by_gate: section.blocked_by_gate,
            }
        })
        .collect::<Vec<_>>();
    Ok(ResolutionLoopOutput {
        section_count: sections.len(),
        blocked_section_count: sections.iter().filter(|section| section.blocked_by_gate).count(),
        needs_hitl_count: sections.iter().filter(|section| section.needs_hitl).count(),
        rejected_count: sections
            .iter()
            .filter(|section| !section.blocked_by_gate && section.decision == "drop_with_reason")
            .count(),
        sections,
    })
}

pub(crate) async fn contradiction_gate_sweep_impl(
    acts: &AlegriaActivities,
    input: &ContradictionGateSweepInput,
) -> Result<ContradictionGateSweepOutput, DomainError> {
    let raw_sections =
        raw_crawl_adapter::load_raw_sections_by_page_ids(&acts.pool, &input.raw_page_ids).await?;
    let raw_by_section: BTreeMap<i64, &raw_crawl_adapter::RawSectionRecord> =
        raw_sections.iter().map(|section| (section.id, section)).collect();
    let sections = input
        .procedural
        .sections
        .iter()
        .map(|section| {
            let raw = raw_by_section.get(&section.section_id).copied().unwrap();
            let facts = section
                .rules
                .iter()
                .map(|rule| {
                    let predicate = match rule.rule_key.as_str() {
                        "consular_fee" => "amount",
                        "processing_time" => "days",
                        _ => "required",
                    };
                    let value_normalized = if !rule.numeric_tokens.is_empty() {
                        rule.numeric_tokens.join("|")
                    } else {
                        rule.rule_key.clone()
                    };
                    seo_steps::contradiction_gate_step::FactAssertion {
                        subject_key: format!("section:{}:{}", section.section_id, rule.rule_key),
                        predicate_key: predicate.to_string(),
                        value_normalized,
                        source_key: Some(raw.source_url.clone()),
                        confidence: rule.confidence,
                    }
                })
                .collect::<Vec<_>>();
            let output = seo_steps::contradiction_gate_step::execute(
                &seo_steps::contradiction_gate_step::ContradictionGateInput {
                    run_id: input.run_id.clone(),
                    facts,
                },
            );
            ContradictionGateSectionState {
                section_id: section.section_id,
                page_id: section.page_id,
                status: if section.blocked_by_gate {
                    "blocked"
                } else if output.is_blocked {
                    "rejected"
                } else if output.needs_hitl {
                    "needs_hitl"
                } else {
                    "pass"
                }
                .to_string(),
                output,
            }
        })
        .collect::<Vec<_>>();
    Ok(ContradictionGateSweepOutput {
        section_count: sections.len(),
        blocked_section_count: sections.iter().filter(|section| section.status == "blocked").count(),
        needs_hitl_count: sections
            .iter()
            .filter(|section| section.status == "needs_hitl")
            .count(),
        conflict_count: sections.iter().map(|section| section.output.conflict_count).sum(),
        sections,
    })
}

pub(crate) async fn truth_adjudication_sweep_impl(
    acts: &AlegriaActivities,
    input: &TruthAdjudicationSweepInput,
) -> Result<TruthAdjudicationSweepOutput, DomainError> {
    let resolution_by_section: BTreeMap<i64, &ResolutionLoopSectionState> = input
        .resolution
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();
    let contradiction_by_section: BTreeMap<i64, &ContradictionGateSectionState> = input
        .contradiction
        .sections
        .iter()
        .map(|section| (section.section_id, section))
        .collect();

    let mut decisions = Vec::new();
    let mut candidate_bindings =
        BTreeMap::<String, (&ValidatedTruthCandidateRecord, i64, i64)>::new();
    let mut grouped_candidates =
        BTreeMap::<(String, String), Vec<&ValidatedTruthCandidateRecord>>::new();
    let mut fixed_states = BTreeMap::<i64, TruthAdjudicationSectionState>::new();
    let source_registry = load_source_registry_entries(&acts.pool)
        .await?
        .into_iter()
        .map(|(source_key, record)| {
            (
                source_key,
                SourceGovernanceRecord {
                    source_type: record.source_type,
                    trust_level: record.trust_level,
                    authority_class: record.authority_class,
                    independence_group_key: record.independence_group_key,
                    freshness_ttl_days: record.freshness_ttl_days,
                    override_eligible: record.override_eligible,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    for section in &input.candidate_validation.sections {
        let resolution = resolution_by_section.get(&section.section_id).copied().unwrap();
        let contradiction = contradiction_by_section.get(&section.section_id).copied().unwrap();
        for candidate in &section.candidates {
            candidate_bindings.insert(
                candidate.rule_candidate_id.clone(),
                (candidate, section.section_id, section.page_id),
            );
        }

        if section.blocked_by_gate {
            fixed_states.insert(
                section.section_id,
                TruthAdjudicationSectionState {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    decision: "blocked".to_string(),
                    status: "blocked".to_string(),
                    verified_count: 0,
                    needs_hitl_count: 0,
                    rejected_count: 0,
                },
            );
            continue;
        }

        let structured_count = section
            .candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "structured")
            .count();
        let needs_hitl_candidates = section
            .candidates
            .iter()
            .filter(|candidate| candidate.epistemic_status == "needs_hitl")
            .collect::<Vec<_>>();
        if structured_count == 0 && !needs_hitl_candidates.is_empty() {
            for candidate in &needs_hitl_candidates {
                decisions.push(TruthAdjudicationCandidateDecision {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    role: candidate.role.clone(),
                    concept_canonical_key: candidate.concept_canonical_key.clone(),
                    params: candidate.params.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: candidate.completeness_class.clone(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    decision: "needs_hitl".to_string(),
                    publish_admissibility: "needs_hitl".to_string(),
                    verification_method: "truth_adjudication@1".to_string(),
                    adjudication_reason: non_structured_candidate_reason(candidate),
                });
            }
            fixed_states.insert(
                section.section_id,
                TruthAdjudicationSectionState {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    decision: "needs_hitl".to_string(),
                    status: "needs_hitl".to_string(),
                    verified_count: 0,
                    needs_hitl_count: needs_hitl_candidates.len(),
                    rejected_count: 0,
                },
            );
            continue;
        }

        if contradiction.output.is_blocked {
            for candidate in &section.candidates {
                decisions.push(TruthAdjudicationCandidateDecision {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    role: candidate.role.clone(),
                    concept_canonical_key: candidate.concept_canonical_key.clone(),
                    params: candidate.params.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: candidate.completeness_class.clone(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    decision: "rejected".to_string(),
                    publish_admissibility: "not_admissible".to_string(),
                    verification_method: "truth_adjudication@1".to_string(),
                    adjudication_reason: "contradiction_block".to_string(),
                });
            }
            fixed_states.insert(
                section.section_id,
                TruthAdjudicationSectionState {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    decision: "rejected".to_string(),
                    status: "rejected".to_string(),
                    verified_count: 0,
                    needs_hitl_count: 0,
                    rejected_count: section.candidates.len(),
                },
            );
            continue;
        }

        if resolution.needs_hitl || resolution.decision == "pause_for_hitl" {
            for candidate in &section.candidates {
                decisions.push(TruthAdjudicationCandidateDecision {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    rule_candidate_id: candidate.rule_candidate_id.clone(),
                    role: candidate.role.clone(),
                    concept_canonical_key: candidate.concept_canonical_key.clone(),
                    params: candidate.params.clone(),
                    source_key: candidate.source_key.clone(),
                    source_tier: candidate.source_tier.clone(),
                    confidence: candidate.confidence,
                    freshness_class: candidate.freshness_class.clone(),
                    completeness_class: "partial".to_string(),
                    evidence_quote: candidate.evidence_quote.clone(),
                    span_start: candidate.span_start,
                    span_end: candidate.span_end,
                    source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                    decision: "needs_hitl".to_string(),
                    publish_admissibility: "needs_hitl".to_string(),
                    verification_method: "truth_adjudication@1".to_string(),
                    adjudication_reason: "resolution_loop_requires_hitl".to_string(),
                });
            }
            fixed_states.insert(
                section.section_id,
                TruthAdjudicationSectionState {
                    section_id: section.section_id,
                    page_id: section.page_id,
                    decision: "needs_hitl".to_string(),
                    status: "needs_hitl".to_string(),
                    verified_count: 0,
                    needs_hitl_count: section.candidates.len(),
                    rejected_count: 0,
                },
            );
            continue;
        }

        for candidate in &section.candidates {
            grouped_candidates
                .entry((candidate.role.clone(), candidate.concept_canonical_key.clone()))
                .or_default()
                .push(candidate);
        }
    }

    for ((_role, _concept), group) in grouped_candidates {
        let structured = group
            .iter()
            .map(|candidate| TruthStructuredCandidate {
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                context_key: input.context_key.clone(),
                role: candidate.role.clone(),
                concept_canonical_key: candidate.concept_canonical_key.clone(),
                params: candidate.params.clone(),
                source_key: candidate.source_key.clone(),
                source_tier: candidate.source_tier.clone(),
                confidence: candidate.confidence,
                freshness_class: candidate.freshness_class.clone(),
                completeness_class: candidate.completeness_class.clone(),
                evidence_quote: candidate.evidence_quote.clone(),
                epistemic_status: candidate.epistemic_status.clone(),
            })
            .collect::<Vec<_>>();
        let adjudication =
            adjudicate_truth_candidates_with_governance(&structured, &source_registry);
        for decision in adjudication.decisions {
            let (candidate, section_id, page_id) = candidate_bindings
                .get(&decision.rule_candidate_id)
                .copied()
                .ok_or_else(|| DomainError::ValidationFailure {
                    message: format!(
                        "candidate section binding is missing for `{}`",
                        decision.rule_candidate_id
                    ),
                })?;
            decisions.push(TruthAdjudicationCandidateDecision {
                section_id,
                page_id,
                rule_candidate_id: candidate.rule_candidate_id.clone(),
                role: candidate.role.clone(),
                concept_canonical_key: candidate.concept_canonical_key.clone(),
                params: candidate.params.clone(),
                source_key: candidate.source_key.clone(),
                source_tier: candidate.source_tier.clone(),
                confidence: candidate.confidence,
                freshness_class: candidate.freshness_class.clone(),
                completeness_class: candidate.completeness_class.clone(),
                evidence_quote: candidate.evidence_quote.clone(),
                span_start: candidate.span_start,
                span_end: candidate.span_end,
                source_snapshot_hash: candidate.source_snapshot_hash.clone(),
                decision: decision.decision,
                publish_admissibility: decision.publish_admissibility,
                verification_method: decision.verification_method,
                adjudication_reason: decision.adjudication_reason,
            });
        }
    }

    let mut section_states = input
        .candidate_validation
        .sections
        .iter()
        .map(|section| {
            if let Some(state) = fixed_states.remove(&section.section_id) {
                return state;
            }
            let mut verified_count = 0usize;
            let mut needs_hitl_count = 0usize;
            let mut rejected_count = 0usize;
            for decision in decisions
                .iter()
                .filter(|decision| decision.section_id == section.section_id)
            {
                match decision.decision.as_str() {
                    "verified" => verified_count += 1,
                    "needs_hitl" => needs_hitl_count += 1,
                    _ => rejected_count += 1,
                }
            }
            let section_decision = if verified_count > 0 {
                "verified"
            } else if needs_hitl_count > 0 {
                "needs_hitl"
            } else {
                "rejected"
            };
            TruthAdjudicationSectionState {
                section_id: section.section_id,
                page_id: section.page_id,
                decision: section_decision.to_string(),
                status: section_decision.to_string(),
                verified_count,
                needs_hitl_count,
                rejected_count,
            }
        })
        .collect::<Vec<_>>();
    section_states.sort_by_key(|section| section.section_id);

    Ok(TruthAdjudicationSweepOutput {
        section_count: section_states.len(),
        blocked_section_count: section_states
            .iter()
            .filter(|section| section.status == "blocked")
            .count(),
        verified_count: section_states.iter().map(|section| section.verified_count).sum(),
        needs_hitl_count: section_states.iter().map(|section| section.needs_hitl_count).sum(),
        rejected_count: section_states.iter().map(|section| section.rejected_count).sum(),
        decisions,
        sections: section_states,
    })
}

pub(crate) async fn verified_truth_write_impl(
    acts: &AlegriaActivities,
    input: &VerifiedTruthWriteInput,
) -> Result<VerifiedTruthWriteOutput, DomainError> {
    let mut changed_truth_keys = Vec::new();
    let mut verified_rule_count = 0usize;
    let mut demoted_rule_count = 0usize;
    let source_section_ids = input
        .truth_adjudication
        .decisions
        .iter()
        .map(|decision| decision.section_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let raw_sections = raw_crawl_adapter::load_raw_sections_by_ids(&acts.pool, &source_section_ids).await?;
    let raw_by_section: BTreeMap<i64, raw_crawl_adapter::RawSectionRecord> = raw_sections
        .into_iter()
        .map(|section| (section.id, section))
        .collect();

    for decision in &input.truth_adjudication.decisions {
        let raw_section = raw_by_section.get(&decision.section_id).ok_or_else(|| {
            DomainError::ValidationFailure {
                message: format!(
                    "verified truth write missing raw section {} for candidate/source registration",
                    decision.section_id
                ),
            }
        })?;
        raw_crawl_adapter::ensure_source(&acts.pool, raw_section).await?;
        let evidence_quote = if decision.evidence_quote.trim().is_empty() {
            decision.concept_canonical_key.clone()
        } else {
            decision.evidence_quote.clone()
        };
        let span_start = decision.span_start.max(0) as i32;
        let span_end = std::cmp::max(decision.span_end as i32, span_start + 1);
        let severity = decision
            .params
            .as_object()
            .and_then(|params| params.get("severity"))
            .and_then(|value| match value {
                TruthParamValue::Text(value) => Some(value.as_str()),
                _ => None,
            })
            .filter(|value| matches!(*value, "mandatory" | "recommended" | "optional" | "unknown"))
            .unwrap_or("unknown");
        let is_numeric = decision
            .params
            .as_object()
            .map(|params| {
                params.contains_key("amount")
                    || params.contains_key("min_amount")
                    || params.contains_key("max_amount")
                    || params.contains_key("days")
                    || params.contains_key("min_days")
                    || params.contains_key("max_days")
                    || params.contains_key("duration_days")
            })
            .unwrap_or(false);
        sqlx::query(
            "INSERT INTO extracted.rule_candidates (
                 rule_candidate_id,
                 context_key,
                 raw_section_id,
                 role,
                 concept_canonical_key,
                 raw_mention,
                 params,
                 scope,
                 severity,
                 applies_to_profiles,
                 exceptions_raw,
                 conditions_raw,
                 alternatives,
                 modality_raw,
                 derivation_type,
                 is_numeric,
                 is_range,
                 is_incomplete,
                 confidence,
                 evidence_section_id,
                 evidence_quote,
                 span_start,
                 span_end,
                 source_key,
                 source_snapshot_hash,
                 llm_provider,
                 llm_model,
                 prompt_version,
                 epistemic_status,
                 uncertainty_flags
             )
             VALUES (
                 $1, $2, $3, $4, $5, $6, $7, '{}'::jsonb, $8, '[]'::jsonb,
                 '', '', '[]'::jsonb, '', 'direct', $9, false, $10, $11, $12,
                 $13, $14, $15, $16, $17, 'deterministic', 'cutover-runtime', 'cutover@1', $18, '[]'::jsonb
             )
             ON CONFLICT (rule_candidate_id) DO UPDATE
             SET role = EXCLUDED.role,
                 concept_canonical_key = EXCLUDED.concept_canonical_key,
                 raw_mention = EXCLUDED.raw_mention,
                 params = EXCLUDED.params,
                 severity = EXCLUDED.severity,
                 is_numeric = EXCLUDED.is_numeric,
                 is_incomplete = EXCLUDED.is_incomplete,
                 confidence = EXCLUDED.confidence,
                 evidence_section_id = EXCLUDED.evidence_section_id,
                 evidence_quote = EXCLUDED.evidence_quote,
                 span_start = EXCLUDED.span_start,
                 span_end = EXCLUDED.span_end,
                 source_key = EXCLUDED.source_key,
                 source_snapshot_hash = EXCLUDED.source_snapshot_hash,
                 llm_provider = EXCLUDED.llm_provider,
                 llm_model = EXCLUDED.llm_model,
                 prompt_version = EXCLUDED.prompt_version,
                 epistemic_status = EXCLUDED.epistemic_status,
                 updated_at = now()"
        )
        .bind(&decision.rule_candidate_id)
        .bind(&input.context_key)
        .bind(decision.section_id)
        .bind(&decision.role)
        .bind(&decision.concept_canonical_key)
        .bind(&evidence_quote)
        .bind(Json::<Value>(truth_param_value_to_json_local(&decision.params)))
        .bind(severity)
        .bind(is_numeric)
        .bind(decision.completeness_class != "complete")
        .bind(decision.confidence)
        .bind(decision.section_id)
        .bind(&evidence_quote)
        .bind(span_start)
        .bind(span_end)
        .bind(&decision.source_key)
        .bind(&decision.source_snapshot_hash)
        .bind(&decision.decision)
        .execute(&*acts.pool)
        .await
        .map_err(AlegriaActivities::classify_error)?;
        let rule_instance_id = semantic_rule_instance_id_local(
            &input.context_key,
            &decision.role,
            &decision.concept_canonical_key,
        );
        let changed_key = format!("verified.rule_instance:{rule_instance_id}");
        match decision.decision.as_str() {
            "verified" => {
                sqlx::query(
                    "INSERT INTO verified.rule_instances (
                         rule_instance_id,
                         context_key,
                         rule_type_key,
                         concept_key,
                         role_type,
                         params,
                         status,
                         source_key,
                         confidence,
                         effective_from,
                         rule_candidate_id,
                         evidence_section_id,
                         evidence_quote,
                         span_start,
                         span_end,
                         source_snapshot_hash,
                         verification_method,
                         adjudication_reason,
                         publish_admissibility,
                         freshness_class,
                         completeness_class,
                         registry_version,
                         prompt_version,
                         model_version,
                         pipeline_version
                     )
                     VALUES (
                         $1, $2, $3, $4, $5, $6, 'verified', $7, $8, current_date,
                         $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23
                     )
                     ON CONFLICT (rule_instance_id) DO UPDATE
                     SET rule_type_key = EXCLUDED.rule_type_key,
                         concept_key = EXCLUDED.concept_key,
                         role_type = EXCLUDED.role_type,
                         params = EXCLUDED.params,
                         status = EXCLUDED.status,
                         source_key = EXCLUDED.source_key,
                         confidence = EXCLUDED.confidence,
                         effective_from = EXCLUDED.effective_from,
                         rule_candidate_id = EXCLUDED.rule_candidate_id,
                         evidence_section_id = EXCLUDED.evidence_section_id,
                         evidence_quote = EXCLUDED.evidence_quote,
                         span_start = EXCLUDED.span_start,
                         span_end = EXCLUDED.span_end,
                         source_snapshot_hash = EXCLUDED.source_snapshot_hash,
                         verification_method = EXCLUDED.verification_method,
                         adjudication_reason = EXCLUDED.adjudication_reason,
                         publish_admissibility = EXCLUDED.publish_admissibility,
                         freshness_class = EXCLUDED.freshness_class,
                         completeness_class = EXCLUDED.completeness_class,
                         registry_version = EXCLUDED.registry_version,
                         prompt_version = EXCLUDED.prompt_version,
                         model_version = EXCLUDED.model_version,
                         pipeline_version = EXCLUDED.pipeline_version,
                         updated_at = now()"
                )
                .bind(&rule_instance_id)
                .bind(&input.context_key)
                .bind(decision.role.to_ascii_lowercase())
                .bind(&decision.concept_canonical_key)
                .bind(decision.role.to_ascii_lowercase())
                .bind(Json::<Value>(truth_param_value_to_json_local(&decision.params)))
                .bind(&decision.source_key)
                .bind(decision.confidence)
                .bind(&decision.rule_candidate_id)
                .bind(decision.section_id)
                .bind(&evidence_quote)
                .bind(span_start)
                .bind(span_end)
                .bind(&decision.source_snapshot_hash)
                .bind(&decision.verification_method)
                .bind(&decision.adjudication_reason)
                .bind(&decision.publish_admissibility)
                .bind(&decision.freshness_class)
                .bind(&decision.completeness_class)
                .bind("registry@1")
                .bind("cutover@1")
                .bind("deterministic")
                .bind("truth_adjudication_runtime@1")
                .execute(&*acts.pool)
                .await
                .map_err(AlegriaActivities::classify_error)?;
                verified_rule_count += 1;
                changed_truth_keys.push(changed_key);
            }
            "needs_hitl" | "rejected" => {
                let status = if decision.decision == "needs_hitl" {
                    "disputed"
                } else {
                    "deprecated"
                };
                sqlx::query(
                    "UPDATE verified.rule_instances
                     SET status = $2,
                         publish_admissibility = $3,
                         verification_method = $4,
                         adjudication_reason = $5,
                         updated_at = now()
                     WHERE rule_instance_id = $1",
                )
                .bind(&rule_instance_id)
                .bind(status)
                .bind(&decision.publish_admissibility)
                .bind(&decision.verification_method)
                .bind(&decision.adjudication_reason)
                .execute(&*acts.pool)
                .await
                .map_err(AlegriaActivities::classify_error)?;
                demoted_rule_count += 1;
                changed_truth_keys.push(changed_key);
            }
            _ => {}
        }
    }

    changed_truth_keys.sort();
    changed_truth_keys.dedup();
    Ok(VerifiedTruthWriteOutput {
        context_key: input.context_key.clone(),
        verified_rule_count,
        demoted_rule_count,
        status: if changed_truth_keys.is_empty() {
            "no_change".to_string()
        } else {
            "written".to_string()
        },
        changed_truth_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(id: i64, content_md: &str) -> raw_crawl_adapter::RawSectionRecord {
        raw_crawl_adapter::RawSectionRecord {
            id,
            page_id: 42,
            source_url: "https://example.test/spain-tourist-visa".to_string(),
            source_domain: "example.test".to_string(),
            source_dtype: "html".to_string(),
            heading_path: "Spain tourist visa".to_string(),
            section_type: "content".to_string(),
            content_md: content_md.to_string(),
            content_hash: "hash".to_string(),
        }
    }

    #[test]
    fn advisory_conflict_does_not_replace_deterministic_page_mode() {
        let sections = vec![section(
            7,
            "Spain tourist visa requirements. Passport, insurance, fee and appointment details.",
        )];
        let snapshot = deterministic_whole_page_snapshot(&sections, &sections[0].content_md);
        let advisory_hits = vec![whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
            prototype_id: "utility".to_string(),
            prototype_family: "utility_page".to_string(),
            score: 0.91,
            page_mode: "utility_page".to_string(),
            dominant_layers: Vec::new(),
            country_hints: Vec::new(),
            visa_type_hints: Vec::new(),
            authority_hints: Vec::new(),
            mixed_section_pressure: false,
        }];

        let fused = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
        assert_eq!(fused.page_mode_hint, "content_page");
        assert!(fused
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "advisory_page_mode_conflict:utility_page"));
    }

    #[test]
    fn advisory_only_country_hint_does_not_promote_context_profile() {
        let sections = vec![section(
            9,
            "Student visa guidance with procedural steps and appointment notes.",
        )];
        let snapshot = deterministic_whole_page_snapshot(&sections, &sections[0].content_md);
        let advisory_hits = vec![whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
            prototype_id: "poland".to_string(),
            prototype_family: "country_poland_work_authority".to_string(),
            score: 0.88,
            page_mode: "content_page".to_string(),
            dominant_layers: vec!["procedural".to_string()],
            country_hints: vec!["PL".to_string()],
            visa_type_hints: vec!["work".to_string()],
            authority_hints: vec!["government".to_string()],
            mixed_section_pressure: false,
        }];

        let fused = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
        assert!(!fused.page_context_profile.country_hints.contains(&"PL".to_string()));
        assert!(fused
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "advisory_only_country_hint:PL"));
    }

    #[test]
    fn low_score_advisory_hit_does_not_change_page_mode_confidence() {
        let sections = vec![section(
            11,
            "Tourist visa document checklist, fee details and passport requirements.",
        )];
        let snapshot = deterministic_whole_page_snapshot(&sections, &sections[0].content_md);
        let baseline_confidence = snapshot.page_mode_confidence;
        let advisory_hits = vec![whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
            prototype_id: "weak".to_string(),
            prototype_family: "content_operational".to_string(),
            score: 0.40,
            page_mode: "content_page".to_string(),
            dominant_layers: vec!["operational".to_string()],
            country_hints: Vec::new(),
            visa_type_hints: Vec::new(),
            authority_hints: Vec::new(),
            mixed_section_pressure: false,
        }];

        let fused = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
        assert_eq!(fused.page_mode_hint, "content_page");
        assert!((fused.page_mode_confidence - baseline_confidence).abs() < f32::EPSILON);
        assert!(!fused
            .reason_codes
            .iter()
            .any(|code| code == "high_confidence_advisory_retrieval"));
    }

    #[test]
    fn mixed_pressure_without_section_evidence_does_not_promote_mixed_sections() {
        let sections = vec![section(
            13,
            "Passport and fee guidance only. Consular fee and passport copy.",
        )];
        let snapshot = deterministic_whole_page_snapshot(&sections, &sections[0].content_md);
        assert!(snapshot.mixed_section_ids.is_empty());
        let advisory_hits = vec![whole_page_advisory_adapter::WholePageAdvisoryRetrievalHit {
            prototype_id: "mixed".to_string(),
            prototype_family: "content_mixed_procedural_operational".to_string(),
            score: 0.89,
            page_mode: "content_page".to_string(),
            dominant_layers: vec!["procedural".to_string(), "operational".to_string()],
            country_hints: Vec::new(),
            visa_type_hints: vec!["tourist".to_string()],
            authority_hints: vec!["consulate".to_string()],
            mixed_section_pressure: true,
        }];

        let fused = fuse_with_advisory_retrieval(&sections, snapshot, &advisory_hits);
        assert!(fused.mixed_section_ids.is_empty());
        assert!(!fused
            .reason_codes
            .iter()
            .any(|code| code == "advisory_mixed_section_support"));
    }

    #[test]
    fn noisy_footer_heavy_page_stays_content_procedural() {
        let content = "Spain tourist visa requirements. Passport copy, insurance, application form, fee 80 EUR.\n\
Footer links contact privacy menu privacy cookie login terms.\n\
Footer navigation directory menu links help login cookie.";
        let sections = vec![
            section(
                21,
                "Spain tourist visa requirements. Passport copy, insurance, application form, fee 80 EUR.",
            ),
            raw_crawl_adapter::RawSectionRecord {
                id: 22,
                page_id: 42,
                source_url: "https://example.test/spain-tourist-visa".to_string(),
                source_domain: "example.test".to_string(),
                source_dtype: "html".to_string(),
                heading_path: "Footer".to_string(),
                section_type: "footer".to_string(),
                content_md:
                    "Footer links contact privacy menu privacy cookie login terms. Footer navigation directory menu links help login cookie."
                        .to_string(),
                content_hash: "footer".to_string(),
            },
        ];
        let snapshot = deterministic_whole_page_snapshot(&sections, content);
        assert_eq!(snapshot.page_mode_hint, "content_page");
        assert!(snapshot.dominant_layers.contains(&"procedural".to_string()));
        assert!(!snapshot.uncertainty_flags.is_empty() || snapshot.page_mode_confidence >= 0.45);
    }

    #[test]
    fn utility_cookie_login_page_is_classified_as_utility() {
        let content = "Privacy policy. Cookie settings. Login and account access. Terms and legal notice.";
        let sections = vec![section(31, content)];
        let snapshot = deterministic_whole_page_snapshot(&sections, content);
        assert_eq!(snapshot.page_mode_hint, "utility_page");
        assert!(snapshot.page_mode_confidence >= 0.50);
    }

    #[test]
    fn menu_directory_page_is_not_misclassified_as_content() {
        let content = "Breadcrumb menu. Directory of visa pages. Destination index. Category links. Sidebar navigation.";
        let sections = vec![section(41, content)];
        let snapshot = deterministic_whole_page_snapshot(&sections, content);
        assert!(
            snapshot.page_mode_hint == "menu_page" || snapshot.page_mode_hint == "directory_page",
            "expected menu/directory classification, got {}",
            snapshot.page_mode_hint
        );
    }

    #[test]
    fn mixed_procedural_editorial_page_sets_multi_layer_flag() {
        let content = "Tourist visa requirements. Passport, insurance and fee 80 EUR. FAQ: why refusals happen and what mistakes to avoid.";
        let sections = vec![section(51, content)];
        let snapshot = deterministic_whole_page_snapshot(&sections, content);
        assert!(snapshot.dominant_layers.contains(&"procedural".to_string()));
        assert!(snapshot.dominant_layers.contains(&"editorial".to_string()));
        assert!(snapshot
            .uncertainty_flags
            .iter()
            .any(|flag| flag == "multi_layer_page"));
        assert_eq!(snapshot.mixed_section_ids, vec![51]);
    }
}
