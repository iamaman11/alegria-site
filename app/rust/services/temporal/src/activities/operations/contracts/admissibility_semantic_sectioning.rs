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

