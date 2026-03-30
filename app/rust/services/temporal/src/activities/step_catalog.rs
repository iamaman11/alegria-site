use primitives::errors::DomainError;

use super::AlegriaActivities;

pub(crate) fn run_layer_router(
    _acts: &AlegriaActivities,
    input: &use_cases::layer_router_step::LayerRouterInput,
) -> use_cases::layer_router_step::LayerRouterOutput {
    use_cases::layer_router_step::execute(input)
}

pub(crate) fn run_entity_span_detection(
    _acts: &AlegriaActivities,
    input: &use_cases::entity_span_detection_step::EntitySpanInput,
) -> use_cases::entity_span_detection_step::EntitySpanOutput {
    use_cases::entity_span_detection_step::execute(input)
}

pub(crate) fn run_canonical_mapping(
    _acts: &AlegriaActivities,
    input: &use_cases::canonical_mapping_step::CanonicalMappingInput,
) -> use_cases::canonical_mapping_step::CanonicalMappingOutput {
    use_cases::canonical_mapping_step::execute(input)
}

pub(crate) fn run_procedural_extraction(
    _acts: &AlegriaActivities,
    input: &use_cases::procedural_extraction_step::ProceduralExtractionInput,
) -> use_cases::procedural_extraction_step::ProceduralExtractionOutput {
    use_cases::procedural_extraction_step::execute(input)
}

pub(crate) fn run_operational_extraction(
    _acts: &AlegriaActivities,
    input: &use_cases::operational_extraction_step::OperationalExtractionInput,
) -> use_cases::operational_extraction_step::OperationalExtractionOutput {
    use_cases::operational_extraction_step::execute(input)
}

pub(crate) fn run_editorial_extraction(
    _acts: &AlegriaActivities,
    input: &use_cases::editorial_extraction_step::EditorialExtractionInput,
) -> use_cases::editorial_extraction_step::EditorialExtractionOutput {
    use_cases::editorial_extraction_step::execute(input)
}

pub(crate) fn run_triple_builder(
    _acts: &AlegriaActivities,
    input: &use_cases::triple_builder_step::TripleBuilderInput,
) -> use_cases::triple_builder_step::TripleBuilderOutput {
    use_cases::triple_builder_step::execute(input)
}

pub(crate) fn run_completeness_judge(
    _acts: &AlegriaActivities,
    input: &use_cases::completeness_judge_step::CompletenessJudgeInput,
) -> use_cases::completeness_judge_step::CompletenessJudgeOutput {
    use_cases::completeness_judge_step::execute(input)
}

pub(crate) fn run_contradiction_gate(
    _acts: &AlegriaActivities,
    input: &use_cases::contradiction_gate_step::ContradictionGateInput,
) -> use_cases::contradiction_gate_step::ContradictionGateOutput {
    use_cases::contradiction_gate_step::execute(input)
}

pub(crate) fn run_hitl_decision(
    _acts: &AlegriaActivities,
    input: &use_cases::hitl_decision_step::HitlDecisionInput,
) -> use_cases::hitl_decision_step::HitlDecisionOutput {
    use_cases::hitl_decision_step::execute(input)
}

pub(crate) async fn run_neo4j_backwrite(
    _acts: &AlegriaActivities,
    input: &use_cases::neo4j_backwrite_step::Neo4jBackwriteInput,
) -> Result<use_cases::neo4j_backwrite_step::Neo4jBackwriteOutput, DomainError> {
    use_cases::neo4j_backwrite_step::execute(input).await
}
