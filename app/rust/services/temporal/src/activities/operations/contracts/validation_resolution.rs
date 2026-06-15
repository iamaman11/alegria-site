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
    pub retrieval_collection_used: Option<String>,
    pub retrieval_evidence_refs: Vec<String>,
    pub semantic_diagnostic_reason_codes: Vec<String>,
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
    pub retrieval_trace_refs: Vec<String>,
    pub retrieval_trace_reason_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionLoopOutput {
    pub section_count: usize,
    pub blocked_section_count: usize,
    pub needs_hitl_count: usize,
    pub rejected_count: usize,
    pub sections: Vec<ResolutionLoopSectionState>,
}

