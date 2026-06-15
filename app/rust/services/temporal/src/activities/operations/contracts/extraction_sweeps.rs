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

