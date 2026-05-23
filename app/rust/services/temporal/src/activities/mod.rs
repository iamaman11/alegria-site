use std::sync::Arc;

use contracts::generated::alegria::temporal::v1::{
    CmsApprovalDecision, CmsPublishInputPayload, CmsPublishOutputPayload,
    ContentContractValidateInputPayload, ContentContractValidateOutputPayload,
    CrawlSourcesInputPayload, CrawlSourcesOutputPayload, DraftAssembleInputPayload,
    DraftAssembleOutputPayload, DraftNormalizeInputPayload, DraftNormalizeOutputPayload,
    DraftQaInputPayload, DraftQaOutputPayload, EditorialDraftGenerateInputPayload,
    EditorialDraftGenerateOutputPayload, FinalizePublishInputPayload, FinalizePublishOutputPayload,
    GlobalSiteReconcileInputPayload, GlobalSiteReconcileOutputPayload, IaBuildInputPayload,
    IaBuildOutputPayload, LinkRecommendInputPayload, LinkRecommendOutputPayload,
    OpportunityBuildInputPayload, OpportunityBuildOutputPayload,
    ProjectionBarrierAuditInputPayload, ProjectionBarrierAuditOutputPayload,
    PublishMaterializeInputPayload, PublishMaterializeOutputPayload,
    RawKnowledgeIngestionInputPayload, RawKnowledgeIngestionOutputPayload,
    RebuildDetectInputPayload, RebuildDetectOutputPayload, ReconcileTargetInputPayload,
    RenderPreviewValidateInputPayload, RenderPreviewValidateOutputPayload,
    SeoSiteBuildInputPayload, SeoVerifiedFactSupportState, SerpIngestInputPayload,
    SerpIngestOutputPayload, SerpNormalizeInputPayload, SerpNormalizeOutputPayload,
};
use infrastructure::adapters::temporalio_sdk_adapter::{
    activities, ActivityContext, ActivityError,
};
use infrastructure::adapters::{
    seo_ports_sqlx_adapter::SqlxSeoRuntimeRepository, sqlx_adapter::AlegriaPgPool, sqlx_seo_adapter,
};
use primitives::errors::DomainError;
use runtime_models::ReconcileTargetReportRecord;
use seo_ports::{ProjectionStatusRepository, VerifiedSupportBundleRequest};

mod content_generation;
pub(crate) mod operations;
mod runtime;
mod step_catalog;

pub struct AlegriaActivities {
    pub pool: Arc<AlegriaPgPool>,
}

fn strict_projection_barrier_enabled() -> bool {
    std::env::var("SEO_STRICT_PROJECTION_BARRIER")
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "strict")
        })
        .unwrap_or(false)
}

async fn observe_projection_barrier(
    pool: &AlegriaPgPool,
    checkpoint: &str,
    run_id: &str,
) -> Result<(), DomainError> {
    let statuses = sqlx_seo_adapter::read_projection_sync_status_for_run(pool, run_id).await?;
    let blocked_events: i64 = statuses
        .iter()
        .map(|status| status.blocking_event_count())
        .sum();
    let max_lag_ms = statuses
        .iter()
        .map(|status| status.max_open_lag_ms)
        .max()
        .unwrap_or(0);
    if blocked_events == 0 {
        tracing::info!(checkpoint, "projection barrier clear");
        return Ok(());
    }

    for status in &statuses {
        if status.blocking_event_count() == 0 {
            continue;
        }
        tracing::warn!(
            checkpoint,
            target_system = %status.target_system,
            pending_events = status.pending_events,
            processing_events = status.processing_events,
            failed_events = status.failed_events,
            max_open_lag_ms = status.max_open_lag_ms,
            oldest_open_aggregate_key = status.oldest_open_aggregate_key.as_deref().unwrap_or(""),
            latest_failed_aggregate_key = status.latest_failed_aggregate_key.as_deref().unwrap_or(""),
            "projection barrier has open or failed events"
        );
    }

    if strict_projection_barrier_enabled() {
        return Err(DomainError::ValidationFailure {
            message: format!(
                "projection barrier blocked at {checkpoint}: blocked_events={blocked_events}, max_lag_ms={max_lag_ms}"
            ),
        });
    }
    Ok(())
}

#[allow(dead_code)]
#[activities]
impl AlegriaActivities {
    #[activity]
    pub async fn generate_content(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "generate_content", 1, &run_id, || async {
            content_generation::generate_content_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn validate_blocks(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "validate_blocks", 1, &run_id, || async {
            content_generation::validate_blocks_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_page_utility_classifier_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::page_utility_classifier_step::PageUtilityClassifierInput,
    ) -> Result<seo_steps::page_utility_classifier_step::PageUtilityClassifierOutput, ActivityError>
    {
        let run_id = if input.url.trim().is_empty() {
            "page_utility_classifier".to_string()
        } else {
            input.url.clone()
        };
        self.execute_step(&run_id, "page_utility_classifier", 1, &input, || async {
            Ok(step_catalog::run_page_utility_classifier(
                self.as_ref(),
                &input,
            ))
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_dom_block_relevance_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: Vec<seo_steps::dom_block_relevance_step::DomBlockInput>,
    ) -> Result<seo_steps::dom_block_relevance_step::DomBlockRelevanceOutput, ActivityError> {
        let run_id = input
            .first()
            .map(|item| item.dom_block_id.clone())
            .unwrap_or_else(|| "dom_block_relevance".to_string());
        self.execute_step(&run_id, "dom_block_relevance_filter", 1, &input, || async {
            Ok(step_catalog::run_dom_block_relevance(self.as_ref(), &input))
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_layer_router_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::layer_router_step::LayerRouterInput,
    ) -> Result<seo_steps::layer_router_step::LayerRouterOutput, ActivityError> {
        self.execute_step(&input.section_id, "layer_router", 1, &input, || async {
            Ok(step_catalog::run_layer_router(self.as_ref(), &input))
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_entity_span_detection_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::entity_span_detection_step::EntitySpanInput,
    ) -> Result<seo_steps::entity_span_detection_step::EntitySpanOutput, ActivityError> {
        self.execute_step(
            &input.section_id,
            "entity_span_detection",
            1,
            &input,
            || async {
                Ok(step_catalog::run_entity_span_detection(
                    self.as_ref(),
                    &input,
                ))
            },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_canonical_mapping_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::canonical_mapping_step::CanonicalMappingInput,
    ) -> Result<seo_steps::canonical_mapping_step::CanonicalMappingOutput, ActivityError> {
        self.execute_step(
            &input.section_id,
            "canonical_mapping",
            1,
            &input,
            || async { Ok(step_catalog::run_canonical_mapping(self.as_ref(), &input)) },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_procedural_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::procedural_extraction_step::ProceduralExtractionInput,
    ) -> Result<seo_steps::procedural_extraction_step::ProceduralExtractionOutput, ActivityError>
    {
        self.execute_step(
            &input.section_id,
            "procedural_extraction",
            1,
            &input,
            || async {
                Ok(step_catalog::run_procedural_extraction(
                    self.as_ref(),
                    &input,
                ))
            },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_operational_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::operational_extraction_step::OperationalExtractionInput,
    ) -> Result<seo_steps::operational_extraction_step::OperationalExtractionOutput, ActivityError>
    {
        self.execute_step(
            &input.section_id,
            "operational_extraction",
            1,
            &input,
            || async {
                Ok(step_catalog::run_operational_extraction(
                    self.as_ref(),
                    &input,
                ))
            },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_editorial_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::editorial_extraction_step::EditorialExtractionInput,
    ) -> Result<seo_steps::editorial_extraction_step::EditorialExtractionOutput, ActivityError>
    {
        self.execute_step(
            &input.section_id,
            "editorial_extraction",
            1,
            &input,
            || async {
                Ok(step_catalog::run_editorial_extraction(
                    self.as_ref(),
                    &input,
                ))
            },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_completeness_judge_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::completeness_judge_step::CompletenessJudgeInput,
    ) -> Result<seo_steps::completeness_judge_step::CompletenessJudgeOutput, ActivityError> {
        self.execute_step(
            &input.section_id,
            "completeness_judge",
            1,
            &input,
            || async { Ok(step_catalog::run_completeness_judge(self.as_ref(), &input)) },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_triple_builder_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::triple_builder_step::TripleBuilderInput,
    ) -> Result<seo_steps::triple_builder_step::TripleBuilderOutput, ActivityError> {
        self.execute_step(&input.section_id, "triple_builder", 1, &input, || async {
            Ok(step_catalog::run_triple_builder(self.as_ref(), &input))
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_contradiction_gate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::contradiction_gate_step::ContradictionGateInput,
    ) -> Result<seo_steps::contradiction_gate_step::ContradictionGateOutput, ActivityError> {
        self.execute_step(&input.run_id, "contradiction_gate", 1, &input, || async {
            Ok(step_catalog::run_contradiction_gate(self.as_ref(), &input))
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_hitl_decision_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: seo_steps::hitl_decision_step::HitlDecisionInput,
    ) -> Result<seo_steps::hitl_decision_step::HitlDecisionOutput, ActivityError> {
        self.execute_step(&input.run_id, "hitl_decision", 1, &input, || async {
            Ok(step_catalog::run_hitl_decision(self.as_ref(), &input))
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_neo4j_backwrite_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::Neo4jBackwriteInput,
    ) -> Result<operations::Neo4jBackwriteOutput, ActivityError> {
        step_catalog::run_neo4j_backwrite(self.as_ref(), &input)
            .await
            .map_err(Self::into_activity_error)
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_projection_reconcile_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: ReconcileTargetInputPayload,
    ) -> Result<ReconcileTargetReportRecord, ActivityError> {
        let run_id = input.run_id.clone();
        let step_name = match input.target_system.as_str() {
            "neo4j" => "neo4j_sync",
            "qdrant" => "voyage_qdrant_sync",
            _ => "projection_reconcile",
        };
        self.execute_step(&run_id, step_name, 1, &input, || async {
            operations::projection_reconcile_impl(
                &input.target_system,
                input.dry_run,
                input.max_retry_count,
                input.batch_limit,
                input.requeue_base_delay_sec,
                input.requeue_jitter_sec,
            )
            .await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_projection_sync_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::ProjectionSyncInput,
    ) -> Result<operations::ProjectionSyncOutput, ActivityError> {
        let run_id = input.run_id.clone();
        let step_name = input.step_name.clone();
        self.execute_step(&run_id, &step_name, 1, &input, || async {
            operations::projection_sync_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_projection_barrier_audit_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: ProjectionBarrierAuditInputPayload,
    ) -> Result<ProjectionBarrierAuditOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        let step_name = if input.checkpoint.trim().is_empty() {
            "projection_barrier_audit"
        } else {
            input.checkpoint.as_str()
        };
        self.execute_step(&run_id, step_name, 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            let status = repo.load_projection_barrier_status(&run_id).await?;
            Ok(ProjectionBarrierAuditOutputPayload {
                run_id: run_id.clone(),
                checkpoint: input.checkpoint.clone(),
                blocked_events: status.blocked_events,
                max_open_lag_ms: status.max_open_lag_ms,
                status: if status.blocked_events > 0 {
                    "blocked"
                } else {
                    "clear"
                }
                .to_string(),
            })
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn load_semantic_section_sample_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::SemanticSectionSampleInput,
    ) -> Result<operations::SemanticSectionSampleOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(
            &run_id,
            "load_semantic_section_sample",
            1,
            &input,
            || async { operations::load_semantic_section_sample_impl(self.as_ref(), &input).await },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_seo_preflight_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::SeoPreflightInput,
    ) -> Result<operations::SeoPreflightOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "seo_preflight", 1, &input, || async {
            operations::seo_preflight_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_whole_page_semantic_pass_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::WholePageSemanticPassInput,
    ) -> Result<operations::WholePageSemanticPassOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "whole_page_semantic_pass", 1, &input, || async {
            operations::whole_page_semantic_pass_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_sectioning_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::SectioningInput,
    ) -> Result<operations::SectioningOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "sectioning", 1, &input, || async {
            operations::sectioning_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_page_utility_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::PageUtilitySweepInput,
    ) -> Result<operations::PageUtilitySweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "page_utility_classifier", 1, &input, || async {
            operations::page_utility_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_dom_block_relevance_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::DomBlockRelevanceSweepInput,
    ) -> Result<operations::DomBlockRelevanceSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "dom_block_relevance_filter", 1, &input, || async {
            operations::dom_block_relevance_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_sectioning_contract_gate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::SectioningContractGateInput,
    ) -> Result<operations::SectioningContractGateOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "sectioning_contract_gate", 1, &input, || async {
            operations::sectioning_contract_gate_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_cas_gate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::CasGateInput,
    ) -> Result<operations::CasGateOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "cas_gate", 1, &input, || async {
            operations::cas_gate_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_raw_evidence_register_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::RawEvidenceRegisterInput,
    ) -> Result<operations::RawEvidenceRegisterOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "raw_evidence_register", 1, &input, || async {
            operations::raw_evidence_register_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_layer_router_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::LayerRouterSweepInput,
    ) -> Result<operations::LayerRouterSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "layer_router", 1, &input, || async {
            operations::layer_router_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_subspan_layer_router_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::SubspanLayerRouterInput,
    ) -> Result<operations::SubspanLayerRouterOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "subspan_layer_router", 1, &input, || async {
            operations::subspan_layer_router_impl(&input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_entity_span_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::EntitySpanSweepInput,
    ) -> Result<operations::EntitySpanSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "entity_span_detection", 1, &input, || async {
            operations::entity_span_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_canonical_mapping_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::CanonicalMappingSweepInput,
    ) -> Result<operations::CanonicalMappingSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "canonical_mapping", 1, &input, || async {
            operations::canonical_mapping_sweep_impl(&input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_ontology_intake_gate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::OntologyIntakeGateInput,
    ) -> Result<operations::OntologyIntakeGateOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "ontology_intake_gate", 1, &input, || async {
            operations::ontology_intake_gate_impl(&input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_procedural_extraction_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::ProceduralExtractionSweepInput,
    ) -> Result<operations::ProceduralExtractionSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "procedural_extraction", 1, &input, || async {
            operations::procedural_extraction_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_operational_extraction_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::OperationalExtractionSweepInput,
    ) -> Result<operations::OperationalExtractionSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "operational_extraction", 1, &input, || async {
            operations::operational_extraction_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_editorial_extraction_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::EditorialExtractionSweepInput,
    ) -> Result<operations::EditorialExtractionSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "editorial_extraction", 1, &input, || async {
            operations::editorial_extraction_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_seo_signal_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::SeoSignalExtractionSweepInput,
    ) -> Result<operations::SeoSignalExtractionSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "seo_signal_extraction", 1, &input, || async {
            operations::seo_signal_extraction_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_commercial_signal_extraction_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::CommercialSignalExtractionSweepInput,
    ) -> Result<operations::CommercialSignalExtractionSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "commercial_signal_extraction", 1, &input, || async {
            operations::commercial_signal_extraction_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_extraction_schema_validate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::ExtractionSchemaValidateInput,
    ) -> Result<operations::ExtractionSchemaValidateOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "extraction_schema_validate", 1, &input, || async {
            operations::extraction_schema_validate_impl(&input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_candidate_validation_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::CandidateValidationInput,
    ) -> Result<operations::CandidateValidationOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "candidate_validation", 1, &input, || async {
            operations::candidate_validation_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_triple_builder_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::TripleBuilderSweepInput,
    ) -> Result<operations::TripleBuilderSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "triple_builder", 1, &input, || async {
            operations::triple_builder_sweep_impl(&input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_completeness_judge_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::CompletenessJudgeSweepInput,
    ) -> Result<operations::CompletenessJudgeSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "completeness_judge", 1, &input, || async {
            operations::completeness_judge_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_resolution_loop_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::ResolutionLoopInput,
    ) -> Result<operations::ResolutionLoopOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "resolution_loop", 1, &input, || async {
            operations::resolution_loop_impl(&input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_contradiction_gate_sweep_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::ContradictionGateSweepInput,
    ) -> Result<operations::ContradictionGateSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "contradiction_gate", 1, &input, || async {
            operations::contradiction_gate_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_truth_adjudication_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::TruthAdjudicationSweepInput,
    ) -> Result<operations::TruthAdjudicationSweepOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "truth_adjudication", 1, &input, || async {
            operations::truth_adjudication_sweep_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_verified_truth_write_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::VerifiedTruthWriteInput,
    ) -> Result<operations::VerifiedTruthWriteOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "verified_truth_write", 1, &input, || async {
            operations::verified_truth_write_impl(self.as_ref(), &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn load_seo_site_build_input(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<SeoSiteBuildInputPayload, ActivityError> {
        self.execute_step(&run_id, "load_seo_site_build_input", 1, &run_id, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::seo_runtime::load_site_build_input(&repo, &run_id).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn load_verified_support_bundle(
        self: Arc<Self>,
        _ctx: ActivityContext,
        workflow_input: String,
    ) -> Result<Vec<SeoVerifiedFactSupportState>, ActivityError> {
        let request: VerifiedSupportBundleRequest =
            serde_json::from_str(&workflow_input).map_err(|err| {
                Self::into_activity_error(DomainError::ValidationFailure {
                    message: format!("invalid verified support bundle request: {err}"),
                })
            })?;
        let timer = crate::metrics::ActivityTimer::start("load_verified_support_bundle");
        let repo = SqlxSeoRuntimeRepository::new(&self.pool);
        match seo_application::seo_runtime::load_verified_support_bundle(&repo, &request).await {
            Ok(bundle) => {
                crate::metrics::global()
                    .support_bundle_events_total
                    .with_label_values(&[if bundle.is_empty() { "empty" } else { "loaded" }])
                    .inc();
                timer.record_success();
                Ok(bundle)
            }
            Err(err) => {
                let class = err.class();
                let error_text = format!("{err:?}");
                let outcome = if error_text.contains("empty") {
                    "empty"
                } else {
                    "failed"
                };
                crate::metrics::global()
                    .support_bundle_events_total
                    .with_label_values(&[outcome])
                    .inc();
                timer.record_failure(class.as_str());
                if outcome == "empty" {
                    return Ok(Vec::new());
                }
                Err(Self::activity_error_from_domain(&err))
            }
        }
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_serp_ingest_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: SerpIngestInputPayload,
    ) -> Result<SerpIngestOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "serp_ingest", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::planning::run_serp_ingest(&repo, &repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_crawl_sources_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: CrawlSourcesInputPayload,
    ) -> Result<CrawlSourcesOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "crawl_sources", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::crawl_ingest::run_crawl_sources(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_raw_knowledge_ingestion_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: RawKnowledgeIngestionInputPayload,
    ) -> Result<RawKnowledgeIngestionOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "raw_knowledge_ingestion", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            let report =
                seo_application::crawl_ingest::run_raw_knowledge_ingestion(&repo, &input).await?;
            observe_projection_barrier(&self.pool, "raw_knowledge_ingestion", &input.run_id)
                .await?;
            Ok(report)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_serp_normalize_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: SerpNormalizeInputPayload,
    ) -> Result<SerpNormalizeOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "serp_normalize", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::planning::run_serp_normalize(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_opportunity_build_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: OpportunityBuildInputPayload,
    ) -> Result<OpportunityBuildOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "opportunity_build", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::planning::run_opportunity_build(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_ia_build_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: IaBuildInputPayload,
    ) -> Result<IaBuildOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "ia_build", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::planning::run_ia_build(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_link_recommend_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: LinkRecommendInputPayload,
    ) -> Result<LinkRecommendOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "link_recommend", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::planning::run_link_recommend(&repo, &repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_global_site_reconcile_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: GlobalSiteReconcileInputPayload,
    ) -> Result<GlobalSiteReconcileOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "global_site_reconcile", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::planning::run_global_site_reconcile(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_draft_assemble_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: DraftAssembleInputPayload,
    ) -> Result<DraftAssembleOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "draft_assemble", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::drafting::run_draft_assemble(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_draft_normalize_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: DraftNormalizeInputPayload,
    ) -> Result<DraftNormalizeOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "draft_normalize", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::drafting::run_draft_normalize(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_editorial_draft_generate(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: EditorialDraftGenerateInputPayload,
    ) -> Result<EditorialDraftGenerateOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "editorial_draft_generate", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            let output =
                seo_application::drafting::run_editorial_draft_generate(&repo, &input).await?;
            let outcome = if output.provider_key == "deterministic_fixture"
                && output.status.contains("fallback")
            {
                "fallback"
            } else {
                "selected"
            };
            crate::metrics::global()
                .editorial_provider_events_total
                .with_label_values(&[&output.provider_key, outcome])
                .inc();
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_content_contract_validate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: ContentContractValidateInputPayload,
    ) -> Result<ContentContractValidateOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "content_contract_validate", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::drafting::run_content_contract_validate(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_draft_qa_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: DraftQaInputPayload,
    ) -> Result<DraftQaOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "draft_qa", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::drafting::run_draft_qa(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_truth_admissibility_gate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::TruthAdmissibilityGateInput,
    ) -> Result<operations::TruthAdmissibilityGateOutput, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "truth_admissibility_gate", 1, &input, || async {
            seo_application::seo_runtime::truth_admissibility_gate(
                &input.verified_support,
                &input.context_key,
                &input.applicant_profile,
            )?;
            Ok(operations::TruthAdmissibilityGateOutput {
                context_key: input.context_key.clone(),
                applicant_profile: input.applicant_profile.clone(),
                admissible_support_count: input.verified_support.len(),
                status: "admissible".to_string(),
            })
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_cms_publish_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: CmsPublishInputPayload,
    ) -> Result<CmsPublishOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "cms_publish", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::review_publish::run_cms_publish(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_cms_request_review_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: CmsPublishInputPayload,
    ) -> Result<CmsPublishOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "cms_request_review", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::review_publish::run_cms_publish(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_human_approval_wait_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: operations::HumanApprovalWaitInput,
    ) -> Result<CmsApprovalDecision, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "human_approval_wait", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::review_publish::load_cms_approval_decision(
                &repo,
                &input.page_node_key,
                &input.revision_id,
            )
            .await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_cms_publish_approved_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: CmsPublishInputPayload,
    ) -> Result<CmsPublishOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "cms_publish_approved", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::review_publish::run_cms_publish(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn load_cms_approval_decision(
        self: Arc<Self>,
        _ctx: ActivityContext,
        workflow_input: String,
    ) -> Result<CmsApprovalDecision, ActivityError> {
        let mut parts = workflow_input.splitn(3, '|');
        let run_id = parts.next().unwrap_or_default().to_string();
        let page_node_key = parts.next().unwrap_or_default().to_string();
        let revision_id = parts.next().unwrap_or_default().to_string();
        self.execute_step(
            &run_id,
            "load_cms_approval_decision",
            1,
            &workflow_input,
            || async {
                let repo = SqlxSeoRuntimeRepository::new(&self.pool);
                seo_application::review_publish::load_cms_approval_decision(
                    &repo,
                    &page_node_key,
                    &revision_id,
                )
                .await
            },
        )
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_rebuild_detect_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: RebuildDetectInputPayload,
    ) -> Result<RebuildDetectOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "rebuild_detect", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::rebuild_detect::execute(&repo, &input).await
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_publish_materialize_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: PublishMaterializeInputPayload,
    ) -> Result<PublishMaterializeOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "publish_materialize", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            let persisted =
                seo_application::review_publish::run_publish_materialize(&repo, &input).await?;
            let manifest_json = persisted
                .publish_artifact
                .as_ref()
                .map(|artifact| artifact.manifest_json.as_str())
                .unwrap_or("");
            let build_scope = if manifest_json.contains("\"build_scope\":\"site\"")
                || manifest_json.contains("\"build_scope\": \"site\"")
            {
                "site"
            } else {
                "page"
            };
            let outcome = if persisted.materialization_status.contains("failed") {
                "failed"
            } else {
                "built"
            };
            crate::metrics::global()
                .publish_build_scope_total
                .with_label_values(&[build_scope, outcome])
                .inc();
            Ok(persisted)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_render_preview_validate_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: RenderPreviewValidateInputPayload,
    ) -> Result<RenderPreviewValidateOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "render_preview_validate", 1, &input, || async {
            let output = seo_application::review_publish::run_render_preview_validate(&input);
            if output.verdict != "render_ready" {
                if output.blocking_reasons.is_empty() {
                    crate::metrics::global()
                        .render_validation_failures_total
                        .with_label_values(&["blocked"])
                        .inc();
                } else {
                    for reason in &output.blocking_reasons {
                        crate::metrics::global()
                            .render_validation_failures_total
                            .with_label_values(&[reason])
                            .inc();
                    }
                }
            }
            Ok(output)
        })
        .await
    }

    #[allow(dead_code)]
    #[activity]
    pub async fn run_finalize_publish_step(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: FinalizePublishInputPayload,
    ) -> Result<FinalizePublishOutputPayload, ActivityError> {
        let run_id = input.run_id.clone();
        self.execute_step(&run_id, "finalize_publish", 1, &input, || async {
            let repo = SqlxSeoRuntimeRepository::new(&self.pool);
            seo_application::review_publish::run_finalize_publish(&repo, &input).await
        })
        .await
    }

    #[activity]
    pub async fn test_step_prepare(
        self: Arc<Self>,
        _ctx: ActivityContext,
        workflow_id: String,
    ) -> Result<String, ActivityError> {
        Ok(operations::test_step_prepare_impl(&workflow_id))
    }

    #[activity]
    pub async fn test_step_finalize(
        self: Arc<Self>,
        _ctx: ActivityContext,
        prepared_token: String,
    ) -> Result<String, ActivityError> {
        Ok(operations::test_step_finalize_impl(&prepared_token))
    }

    #[activity]
    pub async fn finalize_run(
        self: Arc<Self>,
        _ctx: ActivityContext,
        run_id: String,
    ) -> Result<String, ActivityError> {
        self.execute_step(&run_id, "finalize_run", 1, &run_id, || async {
            content_generation::finalize_run_impl(self.as_ref(), &run_id).await
        })
        .await
    }

    #[activity]
    pub async fn check_data_freshness(
        self: Arc<Self>,
        _ctx: ActivityContext,
        threshold_input: String,
    ) -> Result<String, ActivityError> {
        let timer = crate::metrics::ActivityTimer::start("check_data_freshness");
        match operations::check_data_freshness_impl(self.as_ref(), &threshold_input).await {
            Ok(report) => {
                timer.record_success();
                Ok(report)
            }
            Err(err) => {
                let class = err.class();
                timer.record_failure(class.as_str());
                Err(Self::activity_error_from_domain(&err))
            }
        }
    }
}
