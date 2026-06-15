use std::collections::{BTreeMap, BTreeSet};

use primitives::hash::{blake3_hex, content_hash_v1};
use serde::{Deserialize, Serialize};

use super::raw_crawl_adapter::RawSectionRecord;

const EXPERT_CORE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExpertStageStatus {
    Executed,
    Skipped,
    Blocked,
    NeedsHitl,
    Verified,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertStageRecord {
    pub stage_name: String,
    pub schema_version: u32,
    pub idempotency_key: String,
    pub input_hash: String,
    pub output_hash: String,
    pub status: ExpertStageStatus,
    pub error_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageContextProfile {
    pub country_hints: Vec<String>,
    pub visa_type_hints: Vec<String>,
    pub authority_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WholePageSemanticPassOutput {
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
pub struct CasGateOutput {
    pub section_key: String,
    pub snapshot_hash: String,
    pub is_replay_safe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectioningContractGateOutput {
    pub heading_present: bool,
    pub text_present: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubspanLayerRouterOutput {
    pub mixed_layers: Vec<String>,
    pub needs_split: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyIntakeGateOutput {
    pub accepted_keys: Vec<String>,
    pub unresolved_mentions: Vec<String>,
    pub needs_hitl: bool,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionSchemaValidateOutput {
    pub status: String,
    pub blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateValidationOutput {
    pub accepted_count: usize,
    pub needs_hitl_count: usize,
    pub rejected_count: usize,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionLoopOutput {
    pub decision: String,
    pub blockers: Vec<String>,
    pub needs_hitl: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthAdjudicationOutput {
    pub decision: String,
    pub status: ExpertStageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedTruthWriteOutput {
    pub write_allowed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertExtractionSectionOutcome {
    pub raw_section_id: i64,
    pub source_url: String,
    pub primary_layer: String,
    pub needs_hitl: bool,
    pub verified_ready: bool,
    pub final_status: ExpertStageStatus,
    pub stage_records: Vec<ExpertStageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExpertExtractionCoreReport {
    pub schema_version: u32,
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_hitl_section_count: usize,
    pub verified_ready_section_count: usize,
    pub procedural_rule_count: usize,
    pub operational_entity_count: usize,
    pub editorial_topic_count: usize,
    pub triple_count: usize,
    pub contradiction_conflict_count: usize,
    pub sections: Vec<ExpertExtractionSectionOutcome>,
}

