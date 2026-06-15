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

