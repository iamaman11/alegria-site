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
    pub semantic_neighbor_refs: Vec<String>,
    pub semantic_neighbor_reason_codes: Vec<String>,
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
