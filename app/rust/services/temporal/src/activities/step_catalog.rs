use primitives::errors::DomainError;

use super::AlegriaActivities;

pub(crate) fn run_layer_router(
    _acts: &AlegriaActivities,
    input: &seo_steps::layer_router_step::LayerRouterInput,
) -> seo_steps::layer_router_step::LayerRouterOutput {
    seo_steps::layer_router_step::execute(input)
}

pub(crate) fn run_operational_extraction(
    _acts: &AlegriaActivities,
    input: &seo_steps::operational_extraction_step::OperationalExtractionInput,
) -> seo_steps::operational_extraction_step::OperationalExtractionOutput {
    seo_steps::operational_extraction_step::execute(input)
}

pub(crate) fn run_editorial_extraction(
    _acts: &AlegriaActivities,
    input: &seo_steps::editorial_extraction_step::EditorialExtractionInput,
) -> seo_steps::editorial_extraction_step::EditorialExtractionOutput {
    seo_steps::editorial_extraction_step::execute(input)
}

pub(crate) fn run_triple_builder(
    _acts: &AlegriaActivities,
    input: &seo_steps::triple_builder_step::TripleBuilderInput,
) -> seo_steps::triple_builder_step::TripleBuilderOutput {
    seo_steps::triple_builder_step::execute(input)
}

pub(crate) fn run_contradiction_gate(
    _acts: &AlegriaActivities,
    input: &seo_steps::contradiction_gate_step::ContradictionGateInput,
) -> seo_steps::contradiction_gate_step::ContradictionGateOutput {
    seo_steps::contradiction_gate_step::execute(input)
}

pub(crate) fn run_hitl_decision(
    _acts: &AlegriaActivities,
    input: &seo_steps::hitl_decision_step::HitlDecisionInput,
) -> seo_steps::hitl_decision_step::HitlDecisionOutput {
    seo_steps::hitl_decision_step::execute(input)
}

pub(crate) async fn run_neo4j_backwrite(
    _acts: &AlegriaActivities,
    input: &super::operations::Neo4jBackwriteInput,
) -> Result<super::operations::Neo4jBackwriteOutput, DomainError> {
    super::operations::neo4j_backwrite_impl(input).await
}
