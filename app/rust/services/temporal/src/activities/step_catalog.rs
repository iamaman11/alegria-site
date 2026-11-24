use primitives::errors::DomainError;

use super::AlegriaActivities;

pub(crate) fn run_layer_router(
    _acts: &AlegriaActivities,
    input: &seo_domain::extraction::layer_routing::LayerRouterInput,
) -> seo_domain::extraction::layer_routing::LayerRouterOutput {
    seo_domain::extraction::layer_routing::execute(input)
}

pub(crate) fn run_page_utility_classifier(
    _acts: &AlegriaActivities,
    input: &acquisition::source_relevance::PageUtilityClassifierInput,
) -> acquisition::source_relevance::PageUtilityClassifierOutput {
    acquisition::source_relevance::classify_page(input)
}

pub(crate) fn run_dom_block_relevance(
    _acts: &AlegriaActivities,
    input: &[acquisition::source_relevance::DomBlockInput],
) -> acquisition::source_relevance::DomBlockRelevanceOutput {
    acquisition::source_relevance::score_dom_blocks(input)
}

pub(crate) fn run_entity_span_detection(
    _acts: &AlegriaActivities,
    input: &seo_domain::extraction::entity_spans::EntitySpanInput,
) -> seo_domain::extraction::entity_spans::EntitySpanOutput {
    seo_domain::extraction::entity_spans::execute(input)
}

pub(crate) fn run_canonical_mapping(
    _acts: &AlegriaActivities,
    input: &seo_domain::canonical_mapping::CanonicalMappingInput,
) -> seo_domain::canonical_mapping::CanonicalMappingOutput {
    seo_domain::canonical_mapping::execute(input)
}

pub(crate) fn run_procedural_extraction(
    _acts: &AlegriaActivities,
    input: &seo_steps::procedural_extraction_step::ProceduralExtractionInput,
) -> seo_steps::procedural_extraction_step::ProceduralExtractionOutput {
    seo_steps::procedural_extraction_step::execute(input)
}

pub(crate) fn run_operational_extraction(
    _acts: &AlegriaActivities,
    input: &seo_domain::extraction::operational_entities::OperationalExtractionInput,
) -> seo_domain::extraction::operational_entities::OperationalExtractionOutput {
    seo_domain::extraction::operational_entities::execute(input)
}

pub(crate) fn run_editorial_extraction(
    _acts: &AlegriaActivities,
    input: &seo_domain::extraction::editorial_topics::EditorialExtractionInput,
) -> seo_domain::extraction::editorial_topics::EditorialExtractionOutput {
    seo_domain::extraction::editorial_topics::execute(input)
}

pub(crate) fn run_triple_builder(
    _acts: &AlegriaActivities,
    input: &seo_steps::triple_builder_step::TripleBuilderInput,
) -> seo_steps::triple_builder_step::TripleBuilderOutput {
    seo_steps::triple_builder_step::execute(input)
}

pub(crate) fn run_completeness_judge(
    _acts: &AlegriaActivities,
    input: &truth_domain::completeness::CompletenessJudgeInput,
) -> truth_domain::completeness::CompletenessJudgeOutput {
    truth_domain::completeness::execute(input)
}

pub(crate) fn run_contradiction_gate(
    _acts: &AlegriaActivities,
    input: &truth_domain::contradictions::ContradictionGateInput,
) -> truth_domain::contradictions::ContradictionGateOutput {
    truth_domain::contradictions::execute(input)
}

pub(crate) fn run_hitl_decision(
    _acts: &AlegriaActivities,
    input: &truth_domain::hitl_decision::HitlDecisionInput,
) -> truth_domain::hitl_decision::HitlDecisionOutput {
    truth_domain::hitl_decision::execute(input)
}

pub(crate) async fn run_neo4j_backwrite(
    _acts: &AlegriaActivities,
    input: &super::operations::Neo4jBackwriteInput,
) -> Result<super::operations::Neo4jBackwriteOutput, DomainError> {
    super::operations::neo4j_backwrite_impl(input).await
}
