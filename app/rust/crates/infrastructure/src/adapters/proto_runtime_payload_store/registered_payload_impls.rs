macro_rules! impl_prost_runtime_payload {
    ($ty:ty, $name:literal) => {
        impl RuntimeProtoPayload for $ty {
            fn payload_type() -> &'static str {
                $name
            }
            fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
                Ok(self.encode_to_vec())
            }
            fn decode_payload_bytes(
                payload_bytes: &[u8],
            ) -> std::result::Result<Self, DomainError> {
                decode_prost(payload_bytes, stringify!($ty))
            }
        }
    };
}

macro_rules! impl_json_runtime_payload {
    ($ty:ty, $name:literal) => {
        impl RuntimeProtoPayload for $ty {
            fn payload_type() -> &'static str {
                $name
            }

            fn encode_payload_bytes(&self) -> std::result::Result<Vec<u8>, DomainError> {
                serde_json::to_vec(self)
                    .map_err(|e| contract_violation(format!("failed to encode {}: {e}", $name)))
            }

            fn decode_payload_bytes(
                payload_bytes: &[u8],
            ) -> std::result::Result<Self, DomainError> {
                serde_json::from_slice(payload_bytes)
                    .map_err(|e| contract_violation(format!("failed to decode {}: {e}", $name)))
            }
        }
    };
}

impl_prost_runtime_payload!(
    SeoSiteBuildInputPayload,
    "alegria.temporal.v1.SeoSiteBuildInputPayload"
);
impl_prost_runtime_payload!(
    SerpIngestInputPayload,
    "alegria.temporal.v1.SerpIngestInputPayload"
);
impl_prost_runtime_payload!(
    SerpIngestOutputPayload,
    "alegria.temporal.v1.SerpIngestOutputPayload"
);
impl_prost_runtime_payload!(
    CrawlSourcesInputPayload,
    "alegria.temporal.v1.CrawlSourcesInputPayload"
);
impl_prost_runtime_payload!(
    CrawlSourcesOutputPayload,
    "alegria.temporal.v1.CrawlSourcesOutputPayload"
);
impl_prost_runtime_payload!(
    RawKnowledgeIngestionInputPayload,
    "alegria.temporal.v1.RawKnowledgeIngestionInputPayload"
);
impl_prost_runtime_payload!(
    RawKnowledgeIngestionOutputPayload,
    "alegria.temporal.v1.RawKnowledgeIngestionOutputPayload"
);
impl_prost_runtime_payload!(
    GlobalSiteReconcileInputPayload,
    "alegria.temporal.v1.GlobalSiteReconcileInputPayload"
);
impl_prost_runtime_payload!(
    GlobalSiteReconcileOutputPayload,
    "alegria.temporal.v1.GlobalSiteReconcileOutputPayload"
);
impl_prost_runtime_payload!(
    SerpNormalizeInputPayload,
    "alegria.temporal.v1.SerpNormalizeInputPayload"
);
impl_prost_runtime_payload!(
    SerpNormalizeOutputPayload,
    "alegria.temporal.v1.SerpNormalizeOutputPayload"
);
impl_prost_runtime_payload!(
    OpportunityBuildInputPayload,
    "alegria.temporal.v1.OpportunityBuildInputPayload"
);
impl_prost_runtime_payload!(
    OpportunityBuildOutputPayload,
    "alegria.temporal.v1.OpportunityBuildOutputPayload"
);
impl_prost_runtime_payload!(
    IaBuildInputPayload,
    "alegria.temporal.v1.IaBuildInputPayload"
);
impl_prost_runtime_payload!(
    IaBuildOutputPayload,
    "alegria.temporal.v1.IaBuildOutputPayload"
);
impl_prost_runtime_payload!(
    LinkRecommendInputPayload,
    "alegria.temporal.v1.LinkRecommendInputPayload"
);
impl_prost_runtime_payload!(
    LinkRecommendOutputPayload,
    "alegria.temporal.v1.LinkRecommendOutputPayload"
);
impl_prost_runtime_payload!(
    DraftAssembleInputPayload,
    "alegria.temporal.v1.DraftAssembleInputPayload"
);
impl_prost_runtime_payload!(
    DraftAssembleOutputPayload,
    "alegria.temporal.v1.DraftAssembleOutputPayload"
);
impl_prost_runtime_payload!(
    DraftNormalizeInputPayload,
    "alegria.temporal.v1.DraftNormalizeInputPayload"
);
impl_prost_runtime_payload!(
    DraftNormalizeOutputPayload,
    "alegria.temporal.v1.DraftNormalizeOutputPayload"
);
impl_prost_runtime_payload!(
    ContentBlockPlanState,
    "alegria.temporal.v1.ContentBlockPlanState"
);
impl_prost_runtime_payload!(LlmDraftRequest, "alegria.temporal.v1.LlmDraftRequest");
impl_prost_runtime_payload!(LlmDraftCandidate, "alegria.temporal.v1.LlmDraftCandidate");
impl_prost_runtime_payload!(
    EditorialDraftGenerateInputPayload,
    "alegria.temporal.v1.EditorialDraftGenerateInputPayload"
);
impl_prost_runtime_payload!(
    EditorialDraftGenerateOutputPayload,
    "alegria.temporal.v1.EditorialDraftGenerateOutputPayload"
);
impl_prost_runtime_payload!(
    DraftQaInputPayload,
    "alegria.temporal.v1.DraftQaInputPayload"
);
impl_prost_runtime_payload!(
    ContentContractValidateInputPayload,
    "alegria.temporal.v1.ContentContractValidateInputPayload"
);
impl_prost_runtime_payload!(
    ContentContractValidateOutputPayload,
    "alegria.temporal.v1.ContentContractValidateOutputPayload"
);
impl_prost_runtime_payload!(
    DraftQaOutputPayload,
    "alegria.temporal.v1.DraftQaOutputPayload"
);
impl_prost_runtime_payload!(
    CmsPublishInputPayload,
    "alegria.temporal.v1.CmsPublishInputPayload"
);
impl_prost_runtime_payload!(
    CmsPublishOutputPayload,
    "alegria.temporal.v1.CmsPublishOutputPayload"
);
impl_prost_runtime_payload!(CmsReviewPage, "alegria.temporal.v1.CmsReviewPage");
impl_prost_runtime_payload!(
    CmsApprovalDecision,
    "alegria.temporal.v1.CmsApprovalDecision"
);
impl_prost_runtime_payload!(PublishArtifact, "alegria.temporal.v1.PublishArtifact");
impl_prost_runtime_payload!(
    PublishMaterializeInputPayload,
    "alegria.temporal.v1.PublishMaterializeInputPayload"
);
impl_prost_runtime_payload!(
    PublishMaterializeOutputPayload,
    "alegria.temporal.v1.PublishMaterializeOutputPayload"
);
impl_prost_runtime_payload!(
    RenderPreviewValidateInputPayload,
    "alegria.temporal.v1.RenderPreviewValidateInputPayload"
);
impl_prost_runtime_payload!(
    RenderPreviewValidateOutputPayload,
    "alegria.temporal.v1.RenderPreviewValidateOutputPayload"
);
impl_prost_runtime_payload!(
    FinalizePublishInputPayload,
    "alegria.temporal.v1.FinalizePublishInputPayload"
);
impl_prost_runtime_payload!(
    FinalizePublishOutputPayload,
    "alegria.temporal.v1.FinalizePublishOutputPayload"
);
impl_prost_runtime_payload!(
    RebuildDetectInputPayload,
    "alegria.temporal.v1.RebuildDetectInputPayload"
);
impl_prost_runtime_payload!(
    RebuildDetectOutputPayload,
    "alegria.temporal.v1.RebuildDetectOutputPayload"
);

impl_json_runtime_payload!(
    seo_steps::page_utility_classifier_step::PageUtilityClassifierInput,
    "alegria.runtime.json.PageUtilityClassifierInput"
);
impl_json_runtime_payload!(
    seo_steps::page_utility_classifier_step::PageUtilityClassifierOutput,
    "alegria.runtime.json.PageUtilityClassifierOutput"
);
impl_json_runtime_payload!(
    Vec<seo_steps::dom_block_relevance_step::DomBlockInput>,
    "alegria.runtime.json.DomBlockInputList"
);
impl_json_runtime_payload!(
    seo_steps::dom_block_relevance_step::DomBlockRelevanceOutput,
    "alegria.runtime.json.DomBlockRelevanceOutput"
);
impl_json_runtime_payload!(
    seo_steps::layer_router_step::LayerRouterInput,
    "alegria.runtime.json.LayerRouterInput"
);
impl_json_runtime_payload!(
    seo_steps::layer_router_step::LayerRouterOutput,
    "alegria.runtime.json.LayerRouterOutput"
);
impl_json_runtime_payload!(
    seo_steps::entity_span_detection_step::EntitySpanInput,
    "alegria.runtime.json.EntitySpanInput"
);
impl_json_runtime_payload!(
    seo_steps::entity_span_detection_step::EntitySpanOutput,
    "alegria.runtime.json.EntitySpanOutput"
);
impl_json_runtime_payload!(
    seo_steps::canonical_mapping_step::CanonicalMappingInput,
    "alegria.runtime.json.CanonicalMappingInput"
);
impl_json_runtime_payload!(
    seo_steps::canonical_mapping_step::CanonicalMappingOutput,
    "alegria.runtime.json.CanonicalMappingOutput"
);
impl_json_runtime_payload!(
    seo_steps::procedural_extraction_step::ProceduralExtractionInput,
    "alegria.runtime.json.ProceduralExtractionInput"
);
impl_json_runtime_payload!(
    seo_steps::procedural_extraction_step::ProceduralExtractionOutput,
    "alegria.runtime.json.ProceduralExtractionOutput"
);
impl_json_runtime_payload!(
    seo_steps::operational_extraction_step::OperationalExtractionInput,
    "alegria.runtime.json.OperationalExtractionInput"
);
impl_json_runtime_payload!(
    seo_steps::operational_extraction_step::OperationalExtractionOutput,
    "alegria.runtime.json.OperationalExtractionOutput"
);
impl_json_runtime_payload!(
    seo_steps::editorial_extraction_step::EditorialExtractionInput,
    "alegria.runtime.json.EditorialExtractionInput"
);
impl_json_runtime_payload!(
    seo_steps::editorial_extraction_step::EditorialExtractionOutput,
    "alegria.runtime.json.EditorialExtractionOutput"
);
impl_json_runtime_payload!(
    seo_steps::completeness_judge_step::CompletenessJudgeInput,
    "alegria.runtime.json.CompletenessJudgeInput"
);
impl_json_runtime_payload!(
    seo_steps::completeness_judge_step::CompletenessJudgeOutput,
    "alegria.runtime.json.CompletenessJudgeOutput"
);
impl_json_runtime_payload!(
    seo_steps::triple_builder_step::TripleBuilderInput,
    "alegria.runtime.json.TripleBuilderInput"
);
impl_json_runtime_payload!(
    seo_steps::triple_builder_step::TripleBuilderOutput,
    "alegria.runtime.json.TripleBuilderOutput"
);
impl_json_runtime_payload!(
    seo_steps::contradiction_gate_step::ContradictionGateInput,
    "alegria.runtime.json.ContradictionGateInput"
);
impl_json_runtime_payload!(
    seo_steps::contradiction_gate_step::ContradictionGateOutput,
    "alegria.runtime.json.ContradictionGateOutput"
);
impl_json_runtime_payload!(
    seo_steps::hitl_decision_step::HitlDecisionInput,
    "alegria.runtime.json.HitlDecisionInput"
);
impl_json_runtime_payload!(
    seo_steps::hitl_decision_step::HitlDecisionOutput,
    "alegria.runtime.json.HitlDecisionOutput"
);

