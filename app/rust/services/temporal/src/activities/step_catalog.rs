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

pub(crate) fn run_serp_ingest(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::SerpIngestInputPayload,
) -> contracts::generated::alegria::temporal::v1::SerpIngestOutputPayload {
    use_cases::serp_ingest_step::execute(input)
}

pub(crate) fn run_serp_normalize(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::SerpNormalizeInputPayload,
) -> contracts::generated::alegria::temporal::v1::SerpNormalizeOutputPayload {
    use_cases::serp_normalize_step::execute(input)
}

pub(crate) fn run_opportunity_build(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::OpportunityBuildInputPayload,
) -> contracts::generated::alegria::temporal::v1::OpportunityBuildOutputPayload {
    use_cases::opportunity_build_step::execute(input)
}

pub(crate) fn run_ia_build(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::IaBuildInputPayload,
) -> contracts::generated::alegria::temporal::v1::IaBuildOutputPayload {
    use_cases::ia_build_step::execute(input)
}

pub(crate) fn run_link_recommend(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::LinkRecommendInputPayload,
) -> contracts::generated::alegria::temporal::v1::LinkRecommendOutputPayload {
    use_cases::link_recommend_step::execute(input)
}

pub(crate) fn run_global_site_reconcile(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::GlobalSiteReconcileInputPayload,
) -> contracts::generated::alegria::temporal::v1::GlobalSiteReconcileOutputPayload {
    use_cases::global_site_reconcile_step::execute(input)
}

pub(crate) fn run_draft_assemble(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::DraftAssembleInputPayload,
) -> contracts::generated::alegria::temporal::v1::DraftAssembleOutputPayload {
    use_cases::draft_assemble_step::execute(input)
}

pub(crate) fn run_draft_normalize(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::DraftNormalizeInputPayload,
) -> contracts::generated::alegria::temporal::v1::DraftNormalizeOutputPayload {
    use_cases::draft_normalize_step::execute(input)
}

pub(crate) fn run_content_contract_validate(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::ContentContractValidateInputPayload,
) -> contracts::generated::alegria::temporal::v1::ContentContractValidateOutputPayload {
    use_cases::content_contract_validate_step::execute(input)
}

pub(crate) fn run_draft_qa(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::DraftQaInputPayload,
) -> contracts::generated::alegria::temporal::v1::DraftQaOutputPayload {
    use_cases::draft_qa_step::execute(input)
}

pub(crate) fn run_cms_publish(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::CmsPublishInputPayload,
) -> contracts::generated::alegria::temporal::v1::CmsPublishOutputPayload {
    use_cases::cms_publish_step::execute(input)
}

pub(crate) fn run_rebuild_detect(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::RebuildDetectInputPayload,
) -> contracts::generated::alegria::temporal::v1::RebuildDetectOutputPayload {
    use_cases::rebuild_detect_step::execute(input)
}

pub(crate) fn run_publish_materialize(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::PublishMaterializeInputPayload,
) -> contracts::generated::alegria::temporal::v1::PublishMaterializeOutputPayload {
    use_cases::publish_materialize_step::execute(input)
}

pub(crate) fn run_render_preview_validate(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::RenderPreviewValidateInputPayload,
) -> contracts::generated::alegria::temporal::v1::RenderPreviewValidateOutputPayload {
    use_cases::render_preview_validate_step::execute(input)
}

pub(crate) fn run_finalize_publish(
    _acts: &AlegriaActivities,
    input: &contracts::generated::alegria::temporal::v1::FinalizePublishInputPayload,
) -> contracts::generated::alegria::temporal::v1::FinalizePublishOutputPayload {
    use_cases::finalize_publish_step::execute(input)
}
