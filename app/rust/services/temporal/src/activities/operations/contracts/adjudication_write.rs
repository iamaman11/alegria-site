#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionGateSweepInput {
    pub run_id: String,
    pub raw_page_ids: Vec<i64>,
    pub procedural: ProceduralExtractionSweepOutput,
    pub candidate_validation: CandidateValidationOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionGateSectionState {
    pub section_id: i64,
    pub page_id: i64,
    pub output: truth_domain::contradictions::ContradictionGateOutput,
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
